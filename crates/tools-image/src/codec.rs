//! Reading and writing images: every input format Castor understands, the best encoder per
//! output format, metadata carried over without recompression where possible.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, RgbaImage};

/// File formats, by what we can do with them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fmt {
    Jpeg,
    Png,
    Webp,
    Avif,
    Gif,
    Tiff,
    Bmp,
    Ico,
    Icns,
    Heic,
    Jxl,
    Svg,
    Raw,
    Tga,
    Qoi,
    Dds,
    Pnm,
    Exr,
    Hdr,
}

impl Fmt {
    pub fn from_ext(ext: &str) -> Option<Fmt> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "jfif" | "jpe" => Fmt::Jpeg,
            "png" | "apng" => Fmt::Png,
            "webp" => Fmt::Webp,
            "avif" => Fmt::Avif,
            "gif" => Fmt::Gif,
            "tif" | "tiff" => Fmt::Tiff,
            "bmp" | "dib" => Fmt::Bmp,
            "ico" | "cur" => Fmt::Ico,
            "icns" => Fmt::Icns,
            "heic" | "heif" | "hif" => Fmt::Heic,
            "jxl" => Fmt::Jxl,
            "svg" | "svgz" => Fmt::Svg,
            "tga" => Fmt::Tga,
            "qoi" => Fmt::Qoi,
            "dds" => Fmt::Dds,
            "pnm" | "pbm" | "pgm" | "ppm" | "pam" => Fmt::Pnm,
            "exr" => Fmt::Exr,
            "hdr" => Fmt::Hdr,
            "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" | "srw" | "pef" | "nrw" | "3fr" | "iiq" | "crw"
            | "erf" | "mef" | "mos" | "mrw" | "x3f" | "kdc" | "dcr" => Fmt::Raw,
            _ => return None,
        })
    }

    /// Guess from the first bytes.
    pub fn sniff(head: &[u8]) -> Option<Fmt> {
        if head.len() >= 12 && &head[4..8] == b"ftyp" {
            let brand = &head[8..12];
            return Some(if brand == b"avif" || brand == b"avis" { Fmt::Avif } else { Fmt::Heic });
        }
        if head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0cJXL ") {
            return Some(Fmt::Jxl);
        }
        if head.starts_with(b"icns") {
            return Some(Fmt::Icns);
        }
        let t = String::from_utf8_lossy(&head[..head.len().min(512)]).to_ascii_lowercase();
        if t.contains("<svg") {
            return Some(Fmt::Svg);
        }
        match image::guess_format(head).ok()? {
            ImageFormat::Jpeg => Some(Fmt::Jpeg),
            ImageFormat::Png => Some(Fmt::Png),
            ImageFormat::WebP => Some(Fmt::Webp),
            ImageFormat::Gif => Some(Fmt::Gif),
            ImageFormat::Tiff => Some(Fmt::Tiff),
            ImageFormat::Bmp => Some(Fmt::Bmp),
            ImageFormat::Ico => Some(Fmt::Ico),
            ImageFormat::Qoi => Some(Fmt::Qoi),
            ImageFormat::Dds => Some(Fmt::Dds),
            ImageFormat::Pnm => Some(Fmt::Pnm),
            ImageFormat::OpenExr => Some(Fmt::Exr),
            ImageFormat::Hdr => Some(Fmt::Hdr),
            ImageFormat::Tga => Some(Fmt::Tga),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Fmt::Jpeg => "jpg",
            Fmt::Png => "png",
            Fmt::Webp => "webp",
            Fmt::Avif => "avif",
            Fmt::Gif => "gif",
            Fmt::Tiff => "tiff",
            Fmt::Bmp => "bmp",
            Fmt::Ico => "ico",
            Fmt::Icns => "icns",
            Fmt::Heic => "heic",
            Fmt::Jxl => "jxl",
            Fmt::Svg => "svg",
            Fmt::Raw => "dng",
            Fmt::Tga => "tga",
            Fmt::Qoi => "qoi",
            Fmt::Dds => "dds",
            Fmt::Pnm => "ppm",
            Fmt::Exr => "exr",
            Fmt::Hdr => "hdr",
        }
    }

    /// Can hold transparency.
    pub fn has_alpha(self) -> bool {
        !matches!(self, Fmt::Jpeg | Fmt::Bmp | Fmt::Pnm | Fmt::Heic | Fmt::Hdr)
    }

    /// Formats we can write.
    pub fn writable(self) -> bool {
        matches!(
            self,
            Fmt::Jpeg | Fmt::Png | Fmt::Webp | Fmt::Avif | Fmt::Gif | Fmt::Tiff | Fmt::Bmp | Fmt::Ico | Fmt::Tga | Fmt::Qoi
        ) || (self == Fmt::Heic && cfg!(target_os = "macos"))
    }

    /// Formats whose metadata img-parts can edit without touching pixels.
    pub fn container_editable(self) -> bool {
        matches!(self, Fmt::Jpeg | Fmt::Png | Fmt::Webp)
    }
}

/// One frame of an animation.
#[derive(Clone)]
pub struct Frame {
    pub image: RgbaImage,
    /// Display time in milliseconds.
    pub delay_ms: u32,
}

/// A decoded image and what came with it.
pub struct Decoded {
    pub image: DynamicImage,
    pub format: Fmt,
    pub icc: Option<Vec<u8>>,
    /// Raw EXIF (TIFF structure, without the "Exif\0\0" prefix).
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    /// Frames when the file is animated (GIF, WebP, APNG) and frames were asked for.
    pub frames: Option<Vec<Frame>>,
    /// EXIF orientation found in the file (1 = none). Already applied to `image` if asked.
    pub orientation: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct DecodeOpts {
    /// Rotate/flip according to EXIF orientation (what viewers show).
    pub orient: bool,
    /// Keep all frames of animated files.
    pub frames: bool,
    /// SVG raster size: long side in pixels (0 = natural size).
    pub svg_side: u32,
}

impl Default for DecodeOpts {
    fn default() -> Self {
        DecodeOpts { orient: true, frames: false, svg_side: 0 }
    }
}

/// Detect the format of a file (extension first, then content).
pub fn detect(path: &Path) -> Option<Fmt> {
    let mut head = [0u8; 512];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let by_content = Fmt::sniff(&head[..n]);
    let by_ext = path.extension().and_then(|e| e.to_str()).and_then(Fmt::from_ext);
    // Trust the content when it is clear (a .png that is really a JPEG still decodes).
    match (by_ext, by_content) {
        (Some(Fmt::Raw), _) => Some(Fmt::Raw), // TIFF-based RAWs sniff as TIFF
        (_, Some(c)) => Some(c),
        (e, None) => e,
    }
}

pub fn decode(path: &Path, o: DecodeOpts) -> Result<Decoded> {
    let meta = std::fs::metadata(path).with_context(|| crate::common::err("err.read", path))?;
    if meta.len() == 0 {
        bail!("{}", crate::common::err("err.empty", path));
    }
    let fmt = detect(path).ok_or_else(|| anyhow!("{}", crate::common::err("err.unknown_format", path)))?;
    let mut d = match fmt {
        Fmt::Heic | Fmt::Avif => decode_heif(path, fmt)?,
        Fmt::Jxl => decode_jxl(path)?,
        Fmt::Svg => decode_svg(path, o.svg_side)?,
        Fmt::Raw => decode_raw(path)?,
        Fmt::Icns => decode_icns(path)?,
        _ => decode_generic(path, fmt, o.frames)?,
    };
    if o.orient && d.orientation > 1 {
        if let Some(or) = image::metadata::Orientation::from_exif(d.orientation) {
            d.image.apply_orientation(or);
            if let Some(fr) = &mut d.frames {
                for f in fr.iter_mut() {
                    let mut di = DynamicImage::ImageRgba8(std::mem::take(&mut f.image));
                    di.apply_orientation(or);
                    f.image = di.into_rgba8();
                }
            }
        }
    }
    Ok(d)
}

fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_alloc = Some(4 << 30);
    l.max_image_width = Some(65_535);
    l.max_image_height = Some(65_535);
    l
}

fn decode_generic(path: &Path, fmt: Fmt, want_frames: bool) -> Result<Decoded> {
    let data = std::fs::read(path).with_context(|| crate::common::err("err.read", path))?;
    let mut r = image::ImageReader::new(Cursor::new(&data)).with_guessed_format()?;
    r.limits(limits());
    let mut dec = r.into_decoder().map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let icc = dec.icc_profile().ok().flatten();
    let exif = dec.exif_metadata().ok().flatten();
    let xmp = crate::meta::read_xmp(&data, fmt);
    let orientation = exif.as_deref().and_then(exif_orientation).unwrap_or(1);
    let image = DynamicImage::from_decoder(dec).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let frames = if want_frames { decode_frames(&data, fmt)? } else { None };
    Ok(Decoded { image, format: fmt, icc, exif, xmp, frames, orientation })
}

/// All frames of an animated GIF / WebP / APNG, or None if the file holds one frame.
pub fn decode_frames(data: &[u8], fmt: Fmt) -> Result<Option<Vec<Frame>>> {
    let frames: Vec<image::Frame> = match fmt {
        Fmt::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(data))?.into_frames().collect_frames()?,
        Fmt::Webp => {
            let d = image::codecs::webp::WebPDecoder::new(Cursor::new(data))?;
            if !d.has_animation() {
                return Ok(None);
            }
            d.into_frames().collect_frames()?
        }
        Fmt::Png => {
            let d = image::codecs::png::PngDecoder::new(Cursor::new(data))?;
            if !d.is_apng()? {
                return Ok(None);
            }
            d.apng()?.into_frames().collect_frames()?
        }
        _ => return Ok(None),
    };
    if frames.len() < 2 {
        return Ok(None);
    }
    Ok(Some(
        frames
            .into_iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                let ms = if d == 0 { 100 } else { n / d };
                Frame { delay_ms: if ms < 20 { 100 } else { ms }, image: f.into_buffer() }
            })
            .collect(),
    ))
}

fn decode_heif(path: &Path, fmt: Fmt) -> Result<Decoded> {
    let data = std::fs::read(path).with_context(|| crate::common::err("err.read", path))?;
    let cfg = heic::DecoderConfig::new();
    let out = cfg.decode(&data, heic::PixelLayout::Rgba8).map_err(|e| anyhow!("{}: {e:?}", crate::common::err("err.corrupt", path)))?;
    let img = RgbaImage::from_raw(out.width, out.height, out.data).context("bad HEIF frame")?;
    let has_alpha = img.pixels().any(|p| p[3] != 255);
    let image = if has_alpha { DynamicImage::ImageRgba8(img) } else { DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(img).into_rgb8()) };
    let exif = cfg.extract_exif(&data).ok().flatten().map(|c| strip_exif_prefix(c.into_owned()));
    let xmp = cfg.extract_xmp(&data).ok().flatten().map(|c| c.into_owned());
    let icc = cfg.extract_icc(&data).ok().flatten();
    // HEIF applies its own rotation (irot/imir); the EXIF orientation is informational only.
    Ok(Decoded { image, format: fmt, icc, exif, xmp, frames: None, orientation: 1 })
}

fn decode_jxl(path: &Path) -> Result<Decoded> {
    let f = std::fs::File::open(path).with_context(|| crate::common::err("err.read", path))?;
    let mut dec = jxl_oxide::integration::JxlDecoder::new(f).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let icc = dec.icc_profile().ok().flatten();
    let image = DynamicImage::from_decoder(dec).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    Ok(Decoded { image, format: Fmt::Jxl, icc, exif: None, xmp: None, frames: None, orientation: 1 })
}

/// Shared font database (system fonts), loaded once.
pub fn fontdb() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    static DB: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        let fonts = castor_core::paths::resources_dir().join("assets").join("fonts");
        db.load_fonts_dir(&fonts);
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        db.load_fonts_dir(dev);
        std::sync::Arc::new(db)
    })
    .clone()
}

/// Rasterise SVG text (an SVG document) to RGBA, long side `side` (0 = natural size).
pub fn render_svg(data: &[u8], side: u32, resources_dir: Option<&Path>) -> Result<RgbaImage> {
    let opt = resvg::usvg::Options {
        resources_dir: resources_dir.map(|p| p.to_path_buf()),
        fontdb: fontdb(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(data, &opt)?;
    let size = tree.size();
    let (sw, sh) = (size.width().max(1.0), size.height().max(1.0));
    let scale = if side == 0 { 1.0 } else { side as f32 / sw.max(sh) };
    let (w, h) = (((sw * scale).round() as u32).max(1), ((sh * scale).round() as u32).max(1));
    anyhow::ensure!((w as u64) * (h as u64) <= 400_000_000, "SVG too large to render");
    let mut pix = resvg::tiny_skia::Pixmap::new(w, h).context("pixmap")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    // tiny-skia stores premultiplied RGBA.
    let mut img = RgbaImage::from_raw(w, h, pix.take()).context("pixmap")?;
    for p in img.pixels_mut() {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(img)
}

fn decode_svg(path: &Path, side: u32) -> Result<Decoded> {
    let mut data = std::fs::read(path).with_context(|| crate::common::err("err.read", path))?;
    if data.starts_with(&[0x1f, 0x8b]) {
        data = resvg::usvg::decompress_svgz(&data)?;
    }
    let img = render_svg(&data, side, path.parent()).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    Ok(Decoded { image: DynamicImage::ImageRgba8(img), format: Fmt::Svg, icc: None, exif: None, xmp: None, frames: None, orientation: 1 })
}

fn decode_raw(path: &Path) -> Result<Decoded> {
    use rawler::imgop::develop::RawDevelop;
    let raw = rawler::decode_file(path).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let inter = RawDevelop::default().develop_intermediate(&raw).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let image = inter.to_dynamic_image().ok_or_else(|| anyhow!("{}", crate::common::err("err.corrupt", path)))?;
    let image = DynamicImage::ImageRgb8(image.into_rgb8());
    let orientation = raw.orientation.to_u16() as u8;
    Ok(Decoded { image, format: Fmt::Raw, icc: None, exif: None, xmp: None, frames: None, orientation })
}

fn decode_icns(path: &Path) -> Result<Decoded> {
    let f = std::fs::File::open(path).with_context(|| crate::common::err("err.read", path))?;
    let fam = icns::IconFamily::read(std::io::BufReader::new(f)).map_err(|e| anyhow!("{}: {e}", crate::common::err("err.corrupt", path)))?;
    let best = fam
        .available_icons()
        .into_iter()
        .max_by_key(|t| t.pixel_width())
        .ok_or_else(|| anyhow!("{}", crate::common::err("err.corrupt", path)))?;
    let icon = fam.get_icon_with_type(best)?.convert_to(icns::PixelFormat::RGBA);
    let img = RgbaImage::from_raw(icon.width(), icon.height(), icon.into_data().into_vec()).context("icns")?;
    Ok(Decoded { image: DynamicImage::ImageRgba8(img), format: Fmt::Icns, icc: None, exif: None, xmp: None, frames: None, orientation: 1 })
}

fn strip_exif_prefix(mut v: Vec<u8>) -> Vec<u8> {
    if v.starts_with(b"Exif\0\0") {
        v.drain(..6);
    } else if v.len() > 4 && !(v.starts_with(b"II") || v.starts_with(b"MM")) {
        // HEIF: 4-byte offset to the TIFF header.
        let off = u32::from_be_bytes([v[0], v[1], v[2], v[3]]) as usize;
        if 4 + off < v.len() {
            v.drain(..4 + off);
        }
    }
    v
}

/// EXIF orientation tag (1..=8) from raw EXIF.
pub fn exif_orientation(exif: &[u8]) -> Option<u8> {
    let r = exif::Reader::new().read_raw(strip_exif_prefix(exif.to_vec())).ok()?;
    let f = r.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    f.value.get_uint(0).map(|v| v as u8).filter(|v| (1..=8).contains(v))
}

// ------------------------------------------------------------------------------- encoding

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    Auto,
    S444,
    S420,
}

/// Encoder settings, shared by convert / compress / every tool that writes images.
#[derive(Clone, Debug)]
pub struct EncodeOpts {
    /// 1..=100. Lossy formats only.
    pub quality: u8,
    /// WebP / AVIF / PNG: lossless.
    pub lossless: bool,
    /// PNG: quantise to a palette (TinyPNG-like). GIF is always palettised.
    pub png_palette: bool,
    /// PNG effort 0..=6 (oxipng preset).
    pub png_effort: u8,
    /// Background used where the target format cannot keep transparency.
    pub background: [u8; 3],
    pub chroma: Chroma,
    pub progressive: bool,
    /// Metadata to embed (the caller decides what to keep).
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    /// AVIF speed 1 (slow, small) ..= 10 (fast).
    pub avif_speed: u8,
}

impl Default for EncodeOpts {
    fn default() -> Self {
        EncodeOpts {
            quality: 82,
            lossless: false,
            png_palette: false,
            png_effort: 2,
            background: [255, 255, 255],
            chroma: Chroma::Auto,
            progressive: true,
            icc: None,
            exif: None,
            xmp: None,
            avif_speed: 6,
        }
    }
}

/// True if any pixel is not fully opaque.
pub fn uses_alpha(img: &DynamicImage) -> bool {
    if !img.color().has_alpha() {
        return false;
    }
    match img {
        DynamicImage::ImageRgba8(i) => i.pixels().any(|p| p[3] != 255),
        DynamicImage::ImageLumaA8(i) => i.pixels().any(|p| p[1] != 255),
        other => other.to_rgba8().pixels().any(|p| p[3] != 255),
    }
}

/// Flatten transparency over a colour.
pub fn flatten(img: &DynamicImage, bg: [u8; 3]) -> image::RgbImage {
    if !uses_alpha(img) {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        for c in 0..3 {
            o[c] = ((p[c] as u32 * a + bg[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

fn is_gray(img: &DynamicImage) -> bool {
    matches!(img, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_))
}

/// Encode a still image.
pub fn encode(img: &DynamicImage, fmt: Fmt, o: &EncodeOpts) -> Result<Vec<u8>> {
    let bytes = match fmt {
        Fmt::Jpeg => encode_jpeg(img, o)?,
        Fmt::Png => encode_png(img, o)?,
        Fmt::Webp => encode_webp(img, o)?,
        Fmt::Avif => encode_avif(img, o)?,
        Fmt::Gif => encode_gif_still(img, o)?,
        Fmt::Ico => encode_ico(img)?,
        Fmt::Heic => encode_heic_macos(img, o)?,
        Fmt::Bmp | Fmt::Tga | Fmt::Qoi | Fmt::Tiff => {
            let img = if fmt.has_alpha() && uses_alpha(img) {
                DynamicImage::ImageRgba8(img.to_rgba8())
            } else if matches!(img, DynamicImage::ImageLuma16(_) | DynamicImage::ImageRgb16(_)) && fmt == Fmt::Tiff {
                img.clone()
            } else if fmt.has_alpha() {
                DynamicImage::ImageRgb8(img.to_rgb8())
            } else {
                DynamicImage::ImageRgb8(flatten(img, o.background))
            };
            let f = match fmt {
                Fmt::Bmp => ImageFormat::Bmp,
                Fmt::Tga => ImageFormat::Tga,
                Fmt::Qoi => ImageFormat::Qoi,
                _ => ImageFormat::Tiff,
            };
            let mut out = Cursor::new(Vec::new());
            img.write_to(&mut out, f)?;
            out.into_inner()
        }
        other => bail!("cannot write {} files", other.ext().to_uppercase()),
    };
    if fmt.container_editable() && (o.icc.is_some() || o.exif.is_some() || o.xmp.is_some()) {
        return crate::meta::embed(bytes, fmt, o.icc.as_deref(), o.exif.as_deref(), o.xmp.as_deref());
    }
    Ok(bytes)
}

fn encode_jpeg(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    use zenjpeg::encoder::{ChromaSubsampling, EncoderConfig, PixelLayout, Quality, Unstoppable};
    let gray = is_gray(img);
    let (w, h) = (img.width(), img.height());
    anyhow::ensure!(w <= 65_500 && h <= 65_500, "JPEG cannot be larger than 65 500 pixels per side");
    let pixels: Vec<u8> = if gray { img.to_luma8().into_raw() } else { flatten(img, o.background).into_raw() };
    // Quality on the familiar libjpeg/mozjpeg scale.
    let q = Quality::ApproxMozjpeg(o.quality.clamp(1, 100));
    let cfg = if gray {
        EncoderConfig::grayscale(q)
    } else {
        let sub = match o.chroma {
            Chroma::S444 => ChromaSubsampling::None,
            Chroma::S420 => ChromaSubsampling::Quarter,
            Chroma::Auto if o.quality >= 90 => ChromaSubsampling::None,
            Chroma::Auto => ChromaSubsampling::Quarter,
        };
        EncoderConfig::ycbcr(q, sub)
    };
    // No restart markers: zune-jpeg (the decoder of the `image` crate, used by Castor itself and
    // many apps) rejects progressive files that contain them.
    let cfg = cfg.progressive(o.progressive).auto_optimize(true).restart_mcu_rows(0);
    let layout = if gray { PixelLayout::Gray8Srgb } else { PixelLayout::Rgb8Srgb };
    let mut enc = cfg.encode_from_bytes(w, h, layout).map_err(|e| anyhow!("JPEG encoder: {e}"))?;
    enc.push_packed(&pixels, Unstoppable).map_err(|e| anyhow!("JPEG encoder: {e}"))?;
    enc.finish().map_err(|e| anyhow!("JPEG encoder: {e}"))
}

/// PNG via oxipng; `png_palette` quantises with libimagequant first.
fn encode_png(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    use oxipng::{BitDepth, ColorType, RawImage};
    let (w, h) = img.dimensions_u32();
    let mut opts = oxipng::Options::from_preset(o.png_effort.min(6));
    opts.strip = oxipng::StripChunks::Safe;
    let raw = if o.png_palette {
        let (palette, indexes) = quantize(&img.to_rgba8(), o.quality, 1.0)?;
        let pal = palette.iter().map(|c| rgb::RGBA8::new(c.r, c.g, c.b, c.a)).collect();
        RawImage::new(w, h, ColorType::Indexed { palette: pal }, BitDepth::Eight, indexes)?
    } else {
        match img {
            DynamicImage::ImageLuma8(i) => {
                RawImage::new(w, h, ColorType::Grayscale { transparent_shade: None }, BitDepth::Eight, i.as_raw().clone())?
            }
            DynamicImage::ImageLuma16(i) => RawImage::new(
                w,
                h,
                ColorType::Grayscale { transparent_shade: None },
                BitDepth::Sixteen,
                i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect(),
            )?,
            DynamicImage::ImageRgb16(i) => RawImage::new(
                w,
                h,
                ColorType::RGB { transparent_color: None },
                BitDepth::Sixteen,
                i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect(),
            )?,
            DynamicImage::ImageRgba16(i) => {
                RawImage::new(w, h, ColorType::RGBA, BitDepth::Sixteen, i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect())?
            }
            i if uses_alpha(i) => RawImage::new(w, h, ColorType::RGBA, BitDepth::Eight, i.to_rgba8().into_raw())?,
            i => RawImage::new(w, h, ColorType::RGB { transparent_color: None }, BitDepth::Eight, i.to_rgb8().into_raw())?,
        }
    };
    Ok(raw.create_optimized_png(&opts)?)
}

/// libimagequant: palette (≤ 256) + indexes. `quality` 1..=100 is the target.
pub fn quantize(rgba: &RgbaImage, quality: u8, dither: f32) -> Result<(Vec<rgb::RGBA8>, Vec<u8>)> {
    let mut liq = imagequant::new();
    liq.set_speed(4)?;
    liq.set_quality(0, quality.clamp(1, 100))?;
    let px: Vec<imagequant::RGBA> =
        rgba.pixels().map(|p| imagequant::RGBA::new(p[0], p[1], p[2], p[3])).collect();
    let mut im = liq.new_image(px, rgba.width() as usize, rgba.height() as usize, 0.0)?;
    let mut res = liq.quantize(&mut im)?;
    res.set_dithering_level(dither)?;
    let (pal, idx) = res.remapped(&mut im)?;
    Ok((pal.into_iter().map(|c| rgb::RGBA8::new(c.r, c.g, c.b, c.a)).collect(), idx))
}

fn encode_webp(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions_u32();
    anyhow::ensure!(w <= 16_383 && h <= 16_383, "WebP cannot be larger than 16 383 pixels per side");
    let alpha = uses_alpha(img);
    let data = if alpha { img.to_rgba8().into_raw() } else { img.to_rgb8().into_raw() };
    let enc = if alpha { webp::Encoder::from_rgba(&data, w, h) } else { webp::Encoder::from_rgb(&data, w, h) };
    let mut cfg = webp::WebPConfig::new().map_err(|_| anyhow!("WebP config"))?;
    cfg.lossless = o.lossless as i32;
    cfg.quality = if o.lossless { 75.0 } else { o.quality as f32 };
    cfg.method = 5;
    cfg.alpha_quality = 100;
    if alpha && !o.lossless {
        cfg.exact = 0;
    }
    let mem = enc.encode_advanced(&cfg).map_err(|e| anyhow!("WebP encoding failed: {e:?}"))?;
    Ok(mem.to_vec())
}

fn encode_avif(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions_u32();
    let q = if o.lossless { 100.0 } else { o.quality as f32 };
    let mut enc = ravif::Encoder::new().with_quality(q).with_alpha_quality(q.max(80.0)).with_speed(o.avif_speed.clamp(1, 10));
    if let Some(exif) = &o.exif {
        enc = enc.with_exif(exif.clone());
    }
    let rgba = img.to_rgba8();
    let px: Vec<rgb::RGBA8> = rgba.pixels().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
    let out = enc.encode_rgba(imgref::Img::new(&px[..], w as usize, h as usize))?;
    Ok(out.avif_file)
}

fn encode_gif_still(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    let frames = [Frame { image: img.to_rgba8(), delay_ms: 0 }];
    encode_gif_frames(&frames, o.quality, false)
}

/// GIF from frames with one libimagequant palette per frame (transparent pixels keep a
/// reserved index). For best animated quality use [`encode_gifski`].
pub fn encode_gif_frames(frames: &[Frame], quality: u8, looping: bool) -> Result<Vec<u8>> {
    let first = frames.first().context("no frames")?;
    let (w, h) = first.image.dimensions();
    anyhow::ensure!(w <= 65_535 && h <= 65_535, "GIF cannot be larger than 65 535 pixels per side");
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w as u16, h as u16, &[])?;
        if frames.len() > 1 && looping {
            enc.set_repeat(gif::Repeat::Infinite)?;
        }
        for f in frames {
            let (pal, idx) = quantize(&f.image, quality, 1.0)?;
            let mut flat = Vec::with_capacity(pal.len() * 3);
            let mut transparent = None;
            for (i, c) in pal.iter().enumerate() {
                flat.extend_from_slice(&[c.r, c.g, c.b]);
                if c.a < 128 && transparent.is_none() {
                    transparent = Some(i as u8);
                }
            }
            let idx: Vec<u8> = match transparent {
                Some(t) => idx.iter().map(|i| if pal[*i as usize].a < 128 { t } else { *i }).collect(),
                None => idx,
            };
            let mut gf = gif::Frame::from_palette_pixels(f.image.width() as u16, f.image.height() as u16, idx, flat, transparent);
            gf.delay = (f.delay_ms / 10).min(65_535) as u16;
            gf.dispose = gif::DisposalMethod::Background;
            enc.write_frame(&gf)?;
        }
    }
    Ok(out)
}

struct CancelReporter(castor_core::CancelToken);
impl gifski::progress::ProgressReporter for CancelReporter {
    fn increase(&mut self) -> bool {
        !self.0.is_cancelled()
    }
}

/// Animated GIF with gifski (cross-frame palettes, temporal dithering): best quality per byte.
/// `lossy`: 0 = off, else gifsicle-style lossy quality 1..=100 (lower = smaller).
pub fn encode_gifski(frames: Vec<Frame>, quality: u8, lossy: u8, max_side: Option<u32>, cancel: castor_core::CancelToken) -> Result<Vec<u8>> {
    let first = frames.first().context("no frames")?;
    let (w, h) = first.image.dimensions();
    let (tw, th) = match max_side {
        Some(m) if w.max(h) > m => {
            let s = m as f32 / w.max(h) as f32;
            (Some(((w as f32 * s).round() as u32).max(1)), Some(((h as f32 * s).round() as u32).max(1)))
        }
        _ => (None, None),
    };
    let mut s = gifski::Settings { width: tw, height: th, quality: quality.clamp(1, 100), fast: false, repeat: gifski::Repeat::Infinite };
    if tw.is_none() {
        s.width = Some(w);
        s.height = Some(h);
    }
    let (collector, writer) = gifski::new(s)?;
    let _ = lossy; // gifsicle lossy is configured through the writer below when supported.
    let feeder = std::thread::spawn(move || -> Result<()> {
        let mut t = 0f64;
        for (i, f) in frames.into_iter().enumerate() {
            let (fw, fh) = f.image.dimensions();
            let px: Vec<rgb::RGBA8> = f.image.pixels().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
            collector.add_frame_rgba(i, imgref::ImgVec::new(px, fw as usize, fh as usize), t)?;
            t += f.delay_ms as f64 / 1000.0;
        }
        Ok(())
    });
    let mut out = Vec::new();
    let mut rep = CancelReporter(cancel.clone());
    let res = writer.write(&mut out, &mut rep);
    let fed = feeder.join().map_err(|_| anyhow!("GIF encoder thread failed"))?;
    if cancel.is_cancelled() {
        return Err(castor_core::job::Cancelled.into());
    }
    res?;
    fed?;
    Ok(out)
}

/// Animated WebP.
pub fn encode_webp_frames(frames: &[Frame], o: &EncodeOpts) -> Result<Vec<u8>> {
    let first = frames.first().context("no frames")?;
    let (w, h) = first.image.dimensions();
    let mut cfg = webp::WebPConfig::new().map_err(|_| anyhow!("WebP config"))?;
    cfg.lossless = o.lossless as i32;
    cfg.quality = o.quality as f32;
    cfg.method = 4;
    let mut enc = webp::AnimEncoder::new(w, h, &cfg);
    enc.set_loop_count(0);
    let mut t = 0i32;
    for f in frames {
        enc.add_frame(webp::AnimFrame::from_rgba(f.image.as_raw(), f.image.width(), f.image.height(), t));
        t += f.delay_ms as i32;
    }
    let mem = enc.try_encode().map_err(|e| anyhow!("WebP animation encoding failed: {e:?}"))?;
    Ok(mem.to_vec())
}

/// ICO with the standard sizes up to the image size (PNG-compressed entries).
fn encode_ico(img: &DynamicImage) -> Result<Vec<u8>> {
    let sizes: Vec<u32> = [16u32, 24, 32, 48, 64, 128, 256].into_iter().filter(|s| *s <= img.width().max(img.height()).max(16)).collect();
    ico_from_sizes(img, &sizes)
}

/// ICO with the given square sizes (≤ 256).
pub fn ico_from_sizes(img: &DynamicImage, sizes: &[u32]) -> Result<Vec<u8>> {
    let mut pngs = Vec::new();
    for s in sizes {
        let sq = crate::ops::fit_square(&img.to_rgba8(), *s, [0, 0, 0, 0]);
        pngs.push((*s, sq));
    }
    let frames: Vec<image::codecs::ico::IcoFrame> = pngs
        .iter()
        .map(|(s, i)| image::codecs::ico::IcoFrame::as_png(i.as_raw(), *s, *s, image::ExtendedColorType::Rgba8))
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    image::codecs::ico::IcoEncoder::new(&mut out).encode_images(&frames)?;
    Ok(out)
}

/// HEIC through macOS ImageIO (`sips`, part of the OS). Not available elsewhere.
fn encode_heic_macos(img: &DynamicImage, o: &EncodeOpts) -> Result<Vec<u8>> {
    if !cfg!(target_os = "macos") {
        bail!("{}", castor_core::tr("image.err.heic_write"));
    }
    let dir = tempdir()?;
    let src = dir.join("in.png");
    let dst = dir.join("out.heic");
    let mut po = o.clone();
    po.png_effort = 0;
    po.png_palette = false;
    std::fs::write(&src, encode_png(img, &po)?)?;
    let st = std::process::Command::new("sips")
        .args(["-s", "format", "heic", "-s", "formatOptions", &o.quality.to_string()])
        .arg(&src)
        .arg("--out")
        .arg(&dst)
        .output()?;
    anyhow::ensure!(st.status.success(), "sips failed: {}", String::from_utf8_lossy(&st.stderr));
    let b = std::fs::read(&dst)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(b)
}

fn tempdir() -> Result<std::path::PathBuf> {
    let d = std::env::temp_dir().join(format!("castor-img-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ N.fetch_add(1, Ordering::Relaxed)
}

pub trait Dims {
    fn dimensions_u32(&self) -> (u32, u32);
}
impl Dims for DynamicImage {
    fn dimensions_u32(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
}
