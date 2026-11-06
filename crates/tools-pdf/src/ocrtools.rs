//! OCR tools: pdf.ocr (searchable PDF), image.extract-text (TXT / Markdown), and the page OCR
//! helper shared with image.to-pdf and image.images-to-pdf.

use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::{PdfDocument, PdfObject};
use engine_pdf::ocr::{self, Engine, OcrWord};
use engine_pdf::{scan, textlayer};

use crate::common::*;

pub fn languages_opt() -> OptionSpec {
    OptionSpec::language("languages", ocr::languages(), true, "eng")
}

/// OCR an 8-bit gray image, trying other orientations when the result looks like noise.
/// Returns the words and the clockwise quarter-turns (0..3) applied to the image first.
pub fn recognize_best(engine: &mut Engine, img: &image::GrayImage, dpi: u32, cx: &RunCx, from: f32, to: f32) -> Result<(Vec<OcrWord>, u32, image::GrayImage)> {
    let run = |engine: &mut Engine, im: &image::GrayImage| -> Result<(Vec<OcrWord>, f32)> {
        let pcx = cx.clone();
        engine.recognize_gray(im.as_raw(), im.width(), im.height(), im.width() as usize, dpi, cx.cancel_token(), move |f| {
            pcx.progress(from + (to - from) * f)
        })
    };
    let (words, conf) = run(engine, img)?;
    // Good enough, or a blank page: keep it. Upside-down or sideways text gives garbage with a
    // low mean confidence (< 45), readable but poor scans usually stay above.
    if conf >= 45.0 || words.is_empty() {
        return Ok((words, 0, img.clone()));
    }
    // Poor result: maybe the scan is upside down or sideways.
    let mut best = (words, conf, 0u32, img.clone());
    for turns in [2u32, 1, 3] {
        cx.check_cancel()?;
        let im = match turns {
            1 => image::imageops::rotate90(img),
            2 => image::imageops::rotate180(img),
            _ => image::imageops::rotate270(img),
        };
        let (w, c) = run(engine, &im)?;
        if c > best.1 + 10.0 {
            best = (w, c, turns, im);
        }
        if best.1 >= 70.0 {
            break;
        }
    }
    Ok((best.0, best.2, best.3))
}

/// Map a point of an image turned by `turns` clockwise quarter-turns back to the image before
/// the turn (`w` × `h` = size before the turn).
fn unturn_point(x: f32, y: f32, turns: u32, w: f32, h: f32) -> (f32, f32) {
    match turns {
        1 => (y, h - x),
        2 => (w - x, h - y),
        3 => (w - y, x),
        _ => (x, y),
    }
}

pub struct PageOcr {
    pub text: String,
    pub words: usize,
}

/// Render page `index`, OCR it and add an invisible text layer. `font` = the glyph-less font
/// object (add it once per document with `textlayer::add_glyphless_font`).
#[allow(clippy::too_many_arguments)]
pub fn ocr_pdf_page(
    doc: &mut PdfDocument,
    index: usize,
    font: &PdfObject,
    engine: &mut Engine,
    dpi: u32,
    deskew: bool,
    cx: &RunCx,
    from: f32,
    to: f32,
) -> Result<PageOcr> {
    let page = doc.load_page(index as i32)?;
    let pix = engine_pdf::render(&page, dpi as f32, true)?;
    let gray = engine_pdf::pixmap_to_gray(&pix);
    drop(pix);
    drop(page);
    let (w, h) = gray.dimensions();
    let skew = if deskew { scan::estimate_skew(&gray, 8.0) } else { 0.0 };
    let skew = if skew.abs() >= 0.15 { skew } else { 0.0 };
    let straight = if skew != 0.0 { scan::rotate_gray(&gray, -skew) } else { gray };
    let (words, turns, upright) = recognize_best(engine, &straight, dpi, cx, from, to)?;
    let text = ocr::words_to_text(&words);
    // Place words in the upright image, then undo the quarter turns and the deskew.
    let k = 72.0 / dpi as f32;
    let mut placed = textlayer::place_words(&words, dpi as f32, upright.width(), upright.height(), 0.0);
    let turn_angle = match turns {
        1 => 90.0,
        2 => 180.0,
        3 => -90.0,
        _ => 0.0,
    };
    for p in placed.iter_mut() {
        let (x, y) = unturn_point(p.x / k, p.y / k, turns, w as f32, h as f32);
        let (x, y) = scan::unrotate_point(x, y, w, h, -skew);
        p.x = x * k;
        p.y = y * k;
        p.angle += turn_angle + skew;
    }
    let mut pdfpage = doc.load_pdf_page(index as i32)?;
    textlayer::add_text_layer(doc, &mut pdfpage, font, &placed)?;
    Ok(PageOcr { text, words: words.len() })
}

/// True if the page already has real text (not just a few stray characters).
pub fn page_has_text(doc: &PdfDocument, index: usize) -> bool {
    doc.load_page(index as i32)
        .ok()
        .and_then(|p| engine_pdf::text::page_text(&p).ok())
        .map(|t| t.chars().filter(|c| c.is_alphanumeric()).count() > 20)
        .unwrap_or(false)
}

// ------------------------------------------------------------------------------------------

pub struct OcrPdf {
    meta: ToolMeta,
}

impl OcrPdf {
    pub fn new() -> Self {
        let mut m = meta("pdf.ocr", Family::Pdf, PDF_IN, "PDF · TXT", &["searchable", "scan", "recognize", "tesseract", "text layer", "copy text"], "ocr");
        m.serial = true;
        OcrPdf { meta: m }
    }
}

impl Tool for OcrPdf {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            languages_opt(),
            OptionSpec::toggle("skip_text", true),
            OptionSpec::toggle("deskew", true),
            OptionSpec::toggle("sidecar", false),
            OptionSpec::choice("dpi", vec![Choice::lit("200", "200 dpi"), Choice::lit("300", "300 dpi"), Choice::lit("400", "400 dpi")], "300")
                .advanced(),
            OptionSpec::pages("pages").common().advanced(),
            password_opt(),
        ]
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        ocr::assets(v.str("languages"))
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let dpi: u32 = v.str("dpi").parse().unwrap_or(300);
        let mut engine = Engine::new(v.str("languages"), cx)?;
        let font = textlayer::add_glyphless_font(&mut doc)?;
        let mut text = String::new();
        let (mut done, mut skipped, mut words) = (0usize, 0usize, 0usize);
        for (k, &p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            page_msg(cx, k, sel.len());
            let from = 0.05 + 0.9 * k as f32 / sel.len() as f32;
            let to = 0.05 + 0.9 * (k + 1) as f32 / sel.len() as f32;
            if v.bool("skip_text") && page_has_text(&doc, p) {
                skipped += 1;
                if v.bool("sidecar") {
                    if let Ok(page) = doc.load_page(p as i32) {
                        text.push_str(&engine_pdf::text::page_text(&page).unwrap_or_default());
                    }
                    text.push('\u{c}');
                }
                cx.progress(to);
                continue;
            }
            let r = ocr_pdf_page(&mut doc, p, &font, &mut engine, dpi, v.bool("deskew"), cx, from, to)?;
            words += r.words;
            done += 1;
            if v.bool("sidecar") {
                text.push_str(&r.text);
                text.push_str("\n\u{c}");
            }
        }
        drop(engine);
        engine_pdf::stamp_producer(&mut doc)?;
        let out = out_for(cx, input, "pdf");
        let mut cleanup = Cleanup::new();
        cleanup.add(&out);
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        let note = tr_args(
            "pdf.ocr.note",
            &[("pages", &done.to_string()), ("skipped", &skipped.to_string()), ("words", &words.to_string()), ("langs", v.str("languages"))],
        );
        let mut outs = vec![Output::new(&out).with_note(note)];
        if v.bool("sidecar") {
            let txt = out_for(cx, input, "txt");
            cleanup.add(&txt);
            std::fs::write(&txt, text)?;
            outs.push(Output::new(txt));
        }
        cleanup.keep();
        Ok(outs)
    }
}

// ------------------------------------------------------------------------------------------

pub struct ExtractText {
    meta: ToolMeta,
}

impl ExtractText {
    pub fn new() -> Self {
        let mut m = meta(
            "image.extract-text",
            Family::Image,
            InputSpec::kinds(&[FileKind::Image]),
            "TXT · MD",
            &["ocr", "recognize", "read text", "screenshot", "copy text", "tesseract"],
            "",
        );
        m.serial = true;
        ExtractText { meta: m }
    }
}

/// Decode an image file to gray, honouring EXIF orientation.
pub fn load_gray(path: &std::path::Path) -> Result<image::GrayImage> {
    Ok(crate::images::load_oriented(path)?.to_luma8())
}

impl Tool for ExtractText {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            languages_opt(),
            OptionSpec::choice("format", vec![Choice::lit("txt", "TXT"), Choice::lit("md", "Markdown")], "txt"),
            OptionSpec::toggle("deskew", true).advanced(),
        ]
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        ocr::assets(v.str("languages"))
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut gray = load_gray(&input.path)?;
        // Small screenshots read better upscaled to a print-like resolution.
        if gray.width().max(gray.height()) < 1600 {
            let s = 2;
            gray = image::imageops::resize(&gray, gray.width() * s, gray.height() * s, image::imageops::FilterType::CatmullRom);
        }
        if v.bool("deskew") {
            let a = scan::estimate_skew(&gray, 8.0);
            if a.abs() >= 0.15 {
                gray = scan::rotate_gray(&gray, -a);
            }
        }
        let mut engine = Engine::new(v.str("languages"), cx)?;
        let (words, _, _) = recognize_best(&mut engine, &gray, 300, cx, 0.05, 0.95)?;
        drop(engine);
        let mut text = ocr::words_to_text(&words);
        let ext = if v.str("format") == "md" { "md" } else { "txt" };
        if ext == "md" {
            // Paragraphs are separated by blank lines already; join wrapped lines inside them.
            text = text.split("\n\n").map(|p| p.replace('\n', " ")).collect::<Vec<_>>().join("\n\n");
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let out = out_for(cx, input, ext);
        std::fs::write(&out, &text)?;
        let note = tr_args("image.extract-text.note", &[("words", &words.len().to_string())]);
        Ok(vec![Output::new(out).with_note(note)])
    }
}

/// Values helper for tools that embed OCR (images to PDF): visible only when OCR is on.
pub fn ocr_opts() -> Vec<OptionSpec> {
    vec![OptionSpec::toggle("ocr", false), languages_opt().when(Cond::True("ocr"))]
}
