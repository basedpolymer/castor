//! Images ⇄ PDF: pdf.to-images, pdf.extract-images, image.images-to-pdf, image.to-pdf.

use std::collections::HashSet;
use std::path::Path;

use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::draw::Canvas;
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::mupdf::{Rect, Size};
use engine_pdf::{ocr, scan, textlayer};

use crate::common::*;
use crate::ocrtools;

/// Decode an image file with the `image` crate, applying the EXIF orientation.
pub fn load_oriented(path: &Path) -> Result<image::DynamicImage> {
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
        .with_guessed_format()?;
    let mut dec = reader.into_decoder().map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.image")))?;
    let orient = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.image")))?;
    img.apply_orientation(orient);
    Ok(img)
}

fn exif_orientation(path: &Path) -> image::metadata::Orientation {
    use image::ImageDecoder;
    image::ImageReader::open(path)
        .ok()
        .and_then(|r| r.with_guessed_format().ok())
        .and_then(|r| r.into_decoder().ok())
        .and_then(|mut d| d.orientation().ok())
        .unwrap_or(image::metadata::Orientation::NoTransforms)
}

/// An image ready to place: MuPDF image, its displayed size in pixels, rotation to apply (ccw
/// degrees) and resolution.
struct Placeable {
    img: mupdf::Image,
    w: u32,
    h: u32,
    angle: f32,
    dpi: f32,
}

/// JPEG files are embedded as they are (no re-encoding), rotated by their EXIF orientation;
/// anything else goes through MuPDF's decoders, or the `image` crate as a fallback.
fn placeable(path: &Path) -> Result<Placeable> {
    use image::metadata::Orientation as O;
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let is_jpeg = bytes.starts_with(&[0xff, 0xd8]);
    let orient = exif_orientation(path);
    let simple_turn = matches!(orient, O::NoTransforms | O::Rotate90 | O::Rotate180 | O::Rotate270);
    if simple_turn && (is_jpeg || orient == O::NoTransforms) {
        if let Ok(img) = mupdf::Image::from_bytes(&bytes) {
            let (rx, _) = img.resolution();
            let (w, h) = (img.width(), img.height());
            let (angle, dw, dh) = match orient {
                O::Rotate90 => (-90.0, h, w),
                O::Rotate180 => (180.0, w, h),
                O::Rotate270 => (90.0, h, w),
                _ => (0.0, w, h),
            };
            return Ok(Placeable { img, w: dw, h: dh, angle, dpi: if rx > 0 { rx as f32 } else { 96.0 } });
        }
    }
    let dynimg = load_oriented(path)?;
    let mut png = Vec::new();
    let enc = if dynimg.color().has_alpha() { dynimg.to_rgba8().into() } else { image::DynamicImage::ImageRgb8(dynimg.to_rgb8()) };
    enc.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    let img = mupdf::Image::from_bytes(&png)?;
    Ok(Placeable { w: img.width(), h: img.height(), img, angle: 0.0, dpi: 96.0 })
}

fn paper(name: &str) -> Option<(f32, f32)> {
    match name {
        "a4" => Some((595.28, 841.89)),
        "a3" => Some((841.89, 1190.55)),
        "a5" => Some((419.53, 595.28)),
        "letter" => Some((612.0, 792.0)),
        "legal" => Some((612.0, 1008.0)),
        _ => None,
    }
}

/// Add a page holding `p` to `doc` per the page-size options.
fn add_image_page(doc: &mut PdfDocument, p: &Placeable, v: &Values) -> Result<()> {
    let margin = mm(v.float("margin"));
    let (iw, ih) = (p.w as f32 * 72.0 / p.dpi, p.h as f32 * 72.0 / p.dpi);
    let (pw, ph) = match paper(v.str("page_size")) {
        Some((a, b)) => {
            let landscape = match v.str("orientation") {
                "portrait" => false,
                "landscape" => true,
                _ => p.w > p.h,
            };
            if landscape { (b, a) } else { (a, b) }
        }
        None => (iw + 2.0 * margin, ih + 2.0 * margin),
    };
    let mut page = doc.new_page(Size { width: pw, height: ph })?;
    let (aw, ah) = ((pw - 2.0 * margin).max(1.0), (ph - 2.0 * margin).max(1.0));
    let s = (aw / iw).min(ah / ih);
    let s = if paper(v.str("page_size")).is_some() && !v.bool("upscale") { s.min(1.0) } else { s };
    let (dw, dh) = (iw * s, ih * s);
    let (x, y) = ((pw - dw) / 2.0, (ph - dh) / 2.0);
    // The image is drawn unrotated into a box with its stored proportions, then turned.
    let r = if p.angle.abs() == 90.0 {
        let (cx, cy) = (x + dw / 2.0, y + dh / 2.0);
        Rect { x0: cx - dh / 2.0, y0: cy - dw / 2.0, x1: cx + dh / 2.0, y1: cy + dw / 2.0 }
    } else {
        Rect { x0: x, y0: y, x1: x + dw, y1: y + dh }
    };
    let mut canvas = Canvas::new(&page)?;
    canvas.image(doc, &mut page, &p.img, r, p.angle, 1.0)?;
    canvas.commit(doc, &mut page, true)?;
    Ok(())
}

fn page_size_opts() -> Vec<OptionSpec> {
    vec![
        OptionSpec::choice(
            "page_size",
            vec![
                Choice::key("image"),
                Choice::lit("a4", "A4"),
                Choice::lit("letter", "Letter"),
                Choice::lit("a5", "A5"),
                Choice::lit("a3", "A3"),
                Choice::lit("legal", "Legal"),
            ],
            "a4",
        ),
        OptionSpec::choice("orientation", vec![Choice::key("auto"), Choice::key("portrait"), Choice::key("landscape")], "auto")
            .when(Cond::Ne("page_size", "image")),
        OptionSpec::float("margin", 0.0, 100.0, 0.0).unit("mm"),
        OptionSpec::toggle("upscale", true).when(Cond::Ne("page_size", "image")).advanced(),
    ]
}

/// OCR every page of a freshly built document (if the option is on).
fn ocr_document(doc: &mut PdfDocument, v: &Values, cx: &RunCx, from: f32, to: f32) -> Result<usize> {
    if !v.bool("ocr") {
        return Ok(0);
    }
    let n = doc.page_count()?.max(0) as usize;
    let mut engine = ocr::Engine::new(v.str("languages"), cx)?;
    let font = textlayer::add_glyphless_font(doc)?;
    let mut words = 0;
    for i in 0..n {
        cx.check_cancel()?;
        let a = from + (to - from) * i as f32 / n as f32;
        let b = from + (to - from) * (i + 1) as f32 / n as f32;
        words += ocrtools::ocr_pdf_page(doc, i, &font, &mut engine, 300, false, cx, a, b)?.words;
    }
    Ok(words)
}

// ------------------------------------------------------------------------------------------

pub struct ToImages {
    meta: ToolMeta,
}

impl ToImages {
    pub fn new() -> Self {
        ToImages {
            meta: meta("pdf.to-images", Family::Pdf, PDF_IN, "PNG · JPG · WEBP · TIFF", &["convert", "jpg", "png", "render", "rasterize", "export"], ""),
        }
    }
}

impl Tool for ToImages {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP"), Choice::lit("tiff", "TIFF")],
                "png",
            )
            .common(),
            OptionSpec::int("dpi", 36, 1200, 150).slider().unit("dpi"),
            OptionSpec::int("quality", 30, 100, 90).slider().when(Cond::Eq("format", "jpg")).common(),
            OptionSpec::toggle("gray", false).advanced(),
            OptionSpec::pages("pages").common(),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let doc = engine_pdf::open_document(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let dpi = v.int("dpi").clamp(36, 1200) as f32;
        let fmt = v.str("format");
        let ext = match fmt {
            "jpg" => "jpg",
            "webp" => "webp",
            "tiff" => "tif",
            _ => "png",
        };
        let digits = n.to_string().len().max(2);
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        for (k, &p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            page_msg(cx, k, sel.len());
            let page = doc.load_page(p as i32)?;
            // Very large pages at high dpi: cap at ~120 megapixels.
            let b = page.bounds()?;
            let px = (b.width() * dpi / 72.0) * (b.height() * dpi / 72.0);
            let dpi = if px > 120e6 { dpi * (120e6 / px).sqrt() } else { dpi };
            let pix = engine_pdf::render(&page, dpi, v.bool("gray"))?;
            let img: image::DynamicImage = if v.bool("gray") {
                engine_pdf::pixmap_to_gray(&pix).into()
            } else {
                engine_pdf::pixmap_to_rgb(&pix).into()
            };
            drop(pix);
            let name = if sel.len() == 1 {
                format!("{}.{ext}", input.stem())
            } else {
                format!("{}-p{:0digits$}.{ext}", input.stem(), p + 1)
            };
            let out = cx.output_named(&name);
            cleanup.add(&out);
            let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);
            match fmt {
                "jpg" => {
                    let q = v.int("quality").clamp(1, 100) as u8;
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, q).encode_image(&img)?;
                }
                "webp" => img.write_to(&mut f, image::ImageFormat::WebP)?,
                "tiff" => img.write_to(&mut f, image::ImageFormat::Tiff)?,
                _ => img.write_to(&mut f, image::ImageFormat::Png)?,
            }
            drop(f);
            outs.push(Output::new(out));
            prog(cx, k + 1, sel.len(), 0.0, 1.0);
        }
        cleanup.keep();
        Ok(outs)
    }
}

// ------------------------------------------------------------------------------------------

pub struct ExtractImages {
    meta: ToolMeta,
}

impl ExtractImages {
    pub fn new() -> Self {
        ExtractImages {
            meta: meta("pdf.extract-images", Family::Pdf, PDF_IN, "JPG · PNG", &["pictures", "photos", "export images", "embedded", "save images"], ""),
        }
    }
}

impl Tool for ExtractImages {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::int("min_size", 0, 4000, 48).unit("px"), OptionSpec::pages("pages").common().advanced(), password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let min = v.int("min_size").max(0) as u32;
        let mut seen_xref: HashSet<i32> = HashSet::new();
        let mut seen_digest: HashSet<Vec<u8>> = HashSet::new();
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        let mut count = 0usize;
        for (k, &p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            page_msg(cx, k, sel.len());
            let page = doc.load_pdf_page(p as i32)?;
            // 1. JPEG / JPEG 2000 streams are written as they are (no quality loss).
            let mut passthrough_sizes: Vec<(u32, u32)> = Vec::new();
            for info in page.images().unwrap_or_default() {
                let f = info.filter.as_deref().unwrap_or("").trim_start_matches('/');
                let ext = match f {
                    "DCTDecode" => "jpg",
                    "JPXDecode" => "jp2",
                    _ => continue,
                };
                // CMYK JPEGs from PDFs are often stored inverted: decode those instead.
                if info.color_space.as_deref().is_some_and(|c| c.contains("CMYK")) {
                    continue;
                }
                passthrough_sizes.push((info.width, info.height));
                if info.width.max(info.height) < min || !seen_xref.insert(info.xref) {
                    continue;
                }
                let Ok(ex) = doc.extract_image(info.xref) else { continue };
                count += 1;
                let out = cx.output_named(&format!("{}-p{}-{}.{ext}", input.stem(), p + 1, count));
                cleanup.add(&out);
                std::fs::write(&out, &ex.encoded)?;
                outs.push(Output::new(out));
            }
            // 2. Everything else, decoded at native resolution.
            let tp = page.to_text_page(mupdf::TextPageFlags::PRESERVE_IMAGES)?;
            for block in tp.blocks() {
                if block.r#type() != mupdf::text_page::TextBlockType::Image {
                    continue;
                }
                let Some(img) = block.image() else { continue };
                let (w, h) = (img.width(), img.height());
                if w.max(h) < min || passthrough_sizes.contains(&(w, h)) {
                    continue;
                }
                let Ok(pix) = img.to_pixmap() else { continue };
                let digest = pix.digest().map(|d| d.as_bytes().to_vec()).unwrap_or_default();
                if !seen_digest.insert(digest) {
                    continue;
                }
                let rgb = pixmap_any_to_dynamic(&pix);
                count += 1;
                let out = cx.output_named(&format!("{}-p{}-{}.png", input.stem(), p + 1, count));
                cleanup.add(&out);
                rgb.save_with_format(&out, image::ImageFormat::Png)?;
                outs.push(Output::new(out));
            }
            prog(cx, k + 1, sel.len(), 0.0, 1.0);
        }
        anyhow::ensure!(!outs.is_empty(), "{}", tr("pdf.extract-images.err.none"));
        cleanup.keep();
        if let Some(o) = outs.first_mut() {
            o.note = Some(tr_args("pdf.extract-images.note", &[("count", &count.to_string())]));
        }
        Ok(outs)
    }
}

/// Convert any pixmap (gray, RGB, CMYK, with or without alpha) to an `image` buffer.
fn pixmap_any_to_dynamic(pix: &mupdf::Pixmap) -> image::DynamicImage {
    let n = pix.n() as usize;
    let alpha = pix.alpha();
    let colors = n - alpha as usize;
    if colors == 3 || colors == 1 {
        if alpha {
            let (w, h, stride) = (pix.width(), pix.height(), pix.stride() as usize);
            let s = pix.samples();
            let mut out = image::RgbaImage::new(w, h);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let q = &s[y * stride + x * n..];
                    let px = if colors == 3 { [q[0], q[1], q[2], q[3]] } else { [q[0], q[0], q[0], q[1]] };
                    out.put_pixel(x as u32, y as u32, image::Rgba(px));
                }
            }
            return out.into();
        }
        return if colors == 1 { engine_pdf::pixmap_to_gray(pix).into() } else { engine_pdf::pixmap_to_rgb(pix).into() };
    }
    // CMYK (or other): naive conversion.
    let (w, h, stride) = (pix.width(), pix.height(), pix.stride() as usize);
    let s = pix.samples();
    let mut out = image::RgbImage::new(w, h);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let q = &s[y * stride + x * n..];
            let px = if colors >= 4 {
                let k = q[3] as u32;
                let f = |c: u8| (255 - ((c as u32 + k).min(255))) as u8;
                [f(q[0]), f(q[1]), f(q[2])]
            } else {
                [q[0], q[0], q[0]]
            };
            out.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    out.into()
}

// ------------------------------------------------------------------------------------------

pub struct ImagesToPdf {
    meta: ToolMeta,
}

impl ImagesToPdf {
    pub fn new() -> Self {
        ImagesToPdf {
            meta: meta(
                "image.images-to-pdf",
                Family::Image,
                InputSpec::kinds(&[FileKind::Image]),
                "PDF",
                &["combine", "photos", "scans", "album", "jpg to pdf", "png to pdf"],
                "",
            ),
        }
    }
}

impl Tool for ImagesToPdf {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut v = page_size_opts();
        v.push(OptionSpec::choice("output", vec![Choice::key("one"), Choice::key("each")], "one"));
        v.extend(ocrtools::ocr_opts());
        v
    }
    fn plan(&self, inputs: &[FileInfo], v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", tr("pdf.err.no_input"));
        Ok(if v.str("output") == "each" { inputs.iter().cloned().map(Job::single).collect() } else { vec![Job::many(inputs.to_vec())] })
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        if v.bool("ocr") { ocr::assets(v.str("languages")) } else { Vec::new() }
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let first = job.input()?;
        let mut doc = PdfDocument::new();
        let n = job.inputs.len();
        let ocr_share = if v.bool("ocr") { 0.6 } else { 0.0 };
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            cx.message(format!("{} ({}/{n})", f.name(), i + 1));
            let p = placeable(&f.path).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            add_image_page(&mut doc, &p, v)?;
            prog(cx, i + 1, n, 0.0, 1.0 - ocr_share);
        }
        let words = ocr_document(&mut doc, v, cx, 1.0 - ocr_share, 1.0)?;
        engine_pdf::stamp_producer(&mut doc)?;
        let out = if n == 1 { cx.output_named(&format!("{}.pdf", first.stem())) } else { cx.output_named(&format!("{}-images.pdf", first.stem())) };
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save::default())?;
        let mut o = Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &n.to_string())]));
        if v.bool("ocr") {
            o.note = Some(format!("{} · {}", o.note.unwrap_or_default(), tr_args("image.extract-text.note", &[("words", &words.to_string())])));
        }
        Ok(vec![o])
    }
}

// ------------------------------------------------------------------------------------------

pub struct ScanToPdf {
    meta: ToolMeta,
}

impl ScanToPdf {
    pub fn new() -> Self {
        let mut m = meta(
            "image.to-pdf",
            Family::Image,
            InputSpec::kinds(&[FileKind::Image]),
            "PDF",
            &["scan", "document", "photo of a page", "receipt", "searchable", "clean up", "deskew"],
            "",
        );
        m.serial = true;
        m.status = castor_core::ToolStatus::Beta;
        ScanToPdf { meta: m }
    }
}

impl Tool for ScanToPdf {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut v = vec![
            OptionSpec::toggle("crop", true),
            OptionSpec::toggle("deskew", true),
            OptionSpec::choice("enhance", vec![Choice::key("auto"), Choice::key("bw"), Choice::key("none")], "auto"),
        ];
        v.extend(page_size_opts());
        v.push(OptionSpec::toggle("ocr", true));
        v.push(ocrtools::languages_opt().when(Cond::True("ocr")));
        v
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        if v.bool("ocr") { ocr::assets(v.str("languages")) } else { Vec::new() }
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        cx.message(tr("image.to-pdf.msg.clean"));
        let mut img = load_oriented(&input.path)?.to_rgb8();
        // Work at most at ~4000 px on the long side (≈ 400 dpi for A4).
        let longest = img.width().max(img.height());
        if longest > 4200 {
            let s = 4200.0 / longest as f32;
            img = image::imageops::resize(&img, (img.width() as f32 * s) as u32, (img.height() as f32 * s) as u32, image::imageops::FilterType::Triangle);
        }
        cx.check_cancel()?;
        if v.bool("crop") {
            let g = image::DynamicImage::ImageRgb8(img.clone()).to_luma8();
            if let Some((x0, y0, x1, y1)) = scan::document_bbox(&g) {
                img = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
            }
        }
        cx.progress(0.1);
        if v.bool("deskew") {
            let g = image::DynamicImage::ImageRgb8(img.clone()).to_luma8();
            let a = scan::estimate_skew(&g, 10.0);
            if a.abs() >= 0.15 {
                img = scan::rotate_rgb(&img, -a);
            }
        }
        cx.progress(0.2);
        cx.check_cancel()?;
        let encoded: Vec<u8> = match v.str("enhance") {
            "bw" => {
                let g = scan::binarize(&image::DynamicImage::ImageRgb8(scan::enhance(&img)).to_luma8());
                let mut b = Vec::new();
                image::DynamicImage::ImageLuma8(g).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)?;
                b
            }
            mode => {
                let im = if mode == "auto" { scan::enhance(&img) } else { img };
                let mut b = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut b, 85).encode_image(&image::DynamicImage::ImageRgb8(im))?;
                b
            }
        };
        cx.progress(0.3);
        let mimg = mupdf::Image::from_bytes(&encoded)?;
        // Resolution: assume the page fills A4 width if nothing better is known.
        let dpi = (mimg.width() as f32 / 8.27).clamp(100.0, 600.0);
        let p = Placeable { w: mimg.width(), h: mimg.height(), img: mimg, angle: 0.0, dpi };
        let mut doc = PdfDocument::new();
        add_image_page(&mut doc, &p, v)?;
        let words = ocr_document(&mut doc, v, cx, 0.35, 0.95)?;
        engine_pdf::stamp_producer(&mut doc)?;
        let out = cx.output_named(&format!("{}.pdf", input.stem()));
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save::default())?;
        let mut o = Output::new(out);
        if v.bool("ocr") {
            o = o.with_note(tr_args("image.extract-text.note", &[("words", &words.to_string())]));
        }
        Ok(vec![o])
    }
}
