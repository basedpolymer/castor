//! Pixels in and out: PNG encoding, SVG rasterisation (resvg with the bundled fonts),
//! QR/barcode decoding helpers.

use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use image::{ImageEncoder, RgbaImage};
use resvg::tiny_skia::Pixmap;
use resvg::usvg;

/// Encode a tiny-skia pixmap (premultiplied) as PNG.
pub fn encode_pixmap(pm: &Pixmap) -> Result<Vec<u8>> {
    pm.encode_png().map_err(|e| anyhow!("PNG encoding failed: {e}"))
}

/// Pixmap → straight-alpha RGBA image.
pub fn pixmap_to_rgba(pm: &Pixmap) -> RgbaImage {
    let mut img = RgbaImage::new(pm.width(), pm.height());
    for (dst, src) in img.pixels_mut().zip(pm.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    img
}

/// RGBA image → PNG bytes (fast compression for previews, default otherwise).
pub fn encode_rgba(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

/// RGB image → PNG bytes.
pub fn encode_rgb(img: &image::RgbImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

/// Font database with the bundled fonts (Latin Modern, Chorus) and, optionally, the system's.
fn fontdb(system: bool) -> Arc<usvg::fontdb::Database> {
    static BUNDLED: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    static ALL: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let make = |sys: bool| {
        let mut db = if sys { super::typ::system_db().clone() } else { usvg::fontdb::Database::new() };
        for f in super::typ::bundled_font_data() {
            db.load_font_data(f.to_vec());
        }
        db.set_serif_family("Latin Modern Roman");
        db.set_sans_serif_family("Latin Modern Sans");
        db.set_monospace_family("Latin Modern Mono");
        Arc::new(db)
    };
    if system { ALL.get_or_init(|| make(true)).clone() } else { BUNDLED.get_or_init(|| make(false)).clone() }
}

/// Parse an SVG document (text is converted to paths with the bundled fonts, and the system's
/// when `system_fonts`).
pub fn parse_svg(svg: &[u8], system_fonts: bool) -> Result<usvg::Tree> {
    let mut opt = usvg::Options::default();
    opt.fontdb = fontdb(system_fonts);
    opt.font_family = "Latin Modern Sans".into();
    usvg::Tree::from_data(svg, &opt).map_err(|e| anyhow!("invalid SVG: {e}"))
}

/// Render an SVG to a pixmap of exactly `w` × `h` pixels (stretched to fit).
pub fn render_svg(svg: &[u8], w: u32, h: u32, system_fonts: bool) -> Result<Pixmap> {
    let tree = parse_svg(svg, system_fonts)?;
    let mut pm = Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| anyhow!("image too large"))?;
    let s = tree.size();
    let ts = resvg::tiny_skia::Transform::from_scale(w as f32 / s.width(), h as f32 / s.height());
    resvg::render(&tree, ts, &mut pm.as_mut());
    Ok(pm)
}

/// Render an SVG with its longest side = `side` pixels.
pub fn render_svg_fit(svg: &[u8], side: u32, system_fonts: bool) -> Result<Pixmap> {
    let tree = parse_svg(svg, system_fonts)?;
    let s = tree.size();
    let k = side as f32 / s.width().max(s.height());
    let (w, h) = ((s.width() * k).round().max(1.0) as u32, (s.height() * k).round().max(1.0) as u32);
    let mut pm = Pixmap::new(w, h).ok_or_else(|| anyhow!("image too large"))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    Ok(pm)
}

/// Load a raster or SVG image file as RGBA (logos, screenshots, covers).
pub fn load_image(path: &std::path::Path, max_side: u32) -> Result<RgbaImage> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "svg" {
        let data = std::fs::read(path)?;
        let pm = render_svg_fit(&data, max_side.min(4096), true)?;
        return Ok(pixmap_to_rgba(&pm));
    }
    let img = image::ImageReader::open(path)
        .map_err(|e| anyhow!("{}: {e}", path.display()))?
        .with_guessed_format()?
        .decode()
        .map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let img = if img.width().max(img.height()) > max_side { img.thumbnail(max_side, max_side) } else { img };
    Ok(img.to_rgba8())
}

/// Composite `img` over a solid colour (for JPEG or decoding).
pub fn flatten(img: &RgbaImage, bg: [u8; 3]) -> image::RgbImage {
    let mut out = image::RgbImage::new(img.width(), img.height());
    for (d, s) in out.pixels_mut().zip(img.pixels()) {
        let a = s[3] as u32;
        let mix = |c: u8, b: u8| ((c as u32 * a + b as u32 * (255 - a)) / 255) as u8;
        *d = image::Rgb([mix(s[0], bg[0]), mix(s[1], bg[1]), mix(s[2], bg[2])]);
    }
    out
}

/// Decode every QR code visible in an image with rqrr; when rqrr finds nothing (it is strict
/// about rounded finder patterns), ask rxing (the ZXing port most phone scanners derive from).
pub fn decode_qr(img: &RgbaImage) -> Vec<String> {
    let rgb = flatten(img, [255, 255, 255]);
    let gray = image::DynamicImage::ImageRgb8(rgb).to_luma8();
    let (w, h) = gray.dimensions();
    let mut prepared = rqrr::PreparedImage::prepare(gray.clone());
    let found: Vec<String> = prepared.detect_grids().into_iter().filter_map(|g| g.decode().ok().map(|(_, s)| s)).collect();
    if !found.is_empty() {
        return found;
    }
    let mut hints = rxing::DecodeHints { TryHarder: Some(true), ..Default::default() };
    rxing::helpers::detect_in_luma_with_hints(gray.into_raw(), w, h, Some(rxing::BarcodeFormat::QR_CODE), &mut hints)
        .map(|r| vec![r.getText().to_string()])
        .unwrap_or_default()
}
