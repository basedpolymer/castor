//! Invisible text layers (OCR): Tesseract's glyph-less font, Identity-H with CIDs = UTF-16
//! code units and a ToUnicode map, text render mode 3 (invisible), each word horizontally
//! stretched to its box — the approach of Tesseract's own PDF renderer and OCRmyPDF.

use std::fmt::Write;

use anyhow::Result;
use mupdf::pdf::{PdfDocument, PdfObject, PdfPage};
use mupdf::Buffer;

use crate::draw::{self, M};

const CHAR_WIDTH: f32 = 500.0; // /DW, in 1/1000 em

const TO_UNICODE: &str = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<< /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identify-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfrange\n<0000> <FFFF> <0000>\nendbfrange\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";

fn stream(doc: &mut PdfDocument, data: &[u8], dict: Option<&PdfObject>) -> Result<PdfObject> {
    let mut buf = Buffer::with_capacity(data.len());
    std::io::Write::write_all(&mut buf, data)?;
    Ok(doc.add_stream(&buf, dict, false)?)
}

/// Add the glyph-less Type0 font to a document; returns its (indirect) font object.
pub fn add_glyphless_font(doc: &mut PdfDocument) -> Result<PdfObject> {
    let font_len = crate::fonts::GLYPHLESS.len();
    let ff_dict = doc.new_object_from_str(&format!("<< /Length1 {font_len} >>"))?;
    let fontfile = stream(doc, crate::fonts::GLYPHLESS, Some(&ff_dict))?;
    // CIDToGIDMap: every CID → glyph 1.
    let mut map = vec![0u8; 2 * 65536];
    for pair in map.chunks_exact_mut(2) {
        pair[1] = 1;
    }
    let cid2gid = stream(doc, &map, None)?;
    let tounicode = stream(doc, TO_UNICODE.as_bytes(), None)?;

    let mut desc = doc.new_object_from_str(
        "<< /Type /FontDescriptor /FontName /GlyphLessFont /Ascent 1000 /CapHeight 1000 /Descent -1 /Flags 5 /FontBBox [0 0 500 1000] /ItalicAngle 0 /StemV 80 >>",
    )?;
    desc.dict_put("FontFile2", fontfile)?;
    let desc = doc.add_object(&desc)?;

    let mut cid = doc.new_object_from_str(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 500 >>",
    )?;
    cid.dict_put("FontDescriptor", desc)?;
    cid.dict_put("CIDToGIDMap", cid2gid)?;
    let cid = doc.add_object(&cid)?;

    let mut t0 = doc.new_object_from_str("<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H >>")?;
    let mut arr = doc.new_array()?;
    arr.array_push(cid)?;
    t0.dict_put("DescendantFonts", arr)?;
    t0.dict_put("ToUnicode", tounicode)?;
    Ok(doc.add_object(&t0)?)
}

/// A word to place, in visible page space (points, y down).
#[derive(Clone, Debug)]
pub struct PlacedWord {
    pub text: String,
    /// Baseline start.
    pub x: f32,
    pub y: f32,
    /// Advance length along the baseline.
    pub width: f32,
    /// Font size (≈ line height).
    pub size: f32,
    /// Counter-clockwise angle of the baseline, degrees.
    pub angle: f32,
}

/// Register `font` on the page under a fresh name; returns "/Name".
fn register_font(doc: &mut PdfDocument, page: &PdfPage, font: &PdfObject) -> Result<String> {
    let mut res = page.resources()?;
    let mut fonts = match res.get_dict("Font")? {
        Some(f) if f.is_dict()? => f,
        _ => {
            let d = doc.new_dict()?;
            res.dict_put("Font", d)?;
            res.get_dict("Font")?.ok_or_else(|| anyhow::anyhow!("Font"))?
        }
    };
    let mut i = 0;
    let name = loop {
        let n = if i == 0 { "CastorOCR".to_string() } else { format!("CastorOCR{i}") };
        if fonts.get_dict(n.as_str())?.is_none() {
            break n;
        }
        i += 1;
    };
    fonts.dict_put(name.as_str(), font.clone())?;
    Ok(format!("/{name}"))
}

/// Write `words` as invisible text over the page.
pub fn add_text_layer(doc: &mut PdfDocument, page: &mut PdfPage, font: &PdfObject, words: &[PlacedWord]) -> Result<()> {
    if words.is_empty() {
        return Ok(());
    }
    let res = register_font(doc, page, font)?;
    let mut ops = String::from("BT\n3 Tr\n");
    for w in words {
        let mut hex = String::new();
        let mut n = 0usize;
        for u in w.text.encode_utf16() {
            let _ = write!(hex, "{u:04X}");
            n += 1;
        }
        if n == 0 || w.size <= 0.0 || w.width <= 0.0 {
            continue;
        }
        let natural = n as f32 * CHAR_WIDTH / 1000.0 * w.size;
        let tz = (w.width / natural * 100.0).clamp(1.0, 10000.0);
        let (s, c) = w.angle.to_radians().sin_cos();
        let tm: M = [c, -s, -s, -c, w.x, w.y];
        let _ = writeln!(
            ops,
            "{res} {:.2} Tf {:.2} Tz {} Tm <{hex}0020> Tj",
            w.size,
            tz,
            tm.iter().map(|v| format!("{v:.3}")).collect::<Vec<_>>().join(" ")
        );
    }
    ops.push_str("ET\n");
    let mut canvas = draw::Canvas::new(page)?;
    canvas.raw(&ops);
    canvas.commit(doc, page, true)
}

/// Convert OCR words (pixel boxes of a page rendered at `dpi`, after the image was turned with `scan::rotate_*(…,
/// rotated_by)` before recognition) into page-space words.
pub fn place_words(words: &[crate::ocr::OcrWord], dpi: f32, img_w: u32, img_h: u32, rotated_by: f32) -> Vec<PlacedWord> {
    let k = 72.0 / dpi;
    words
        .iter()
        .filter(|w| !w.text.is_empty())
        .map(|w| {
            // Baseline: bottom of the line box, minus a small descent allowance.
            let lh = (w.line_bbox[3] - w.line_bbox[1]).max(1.0);
            let base_y = w.line_bbox[3] - lh * 0.2;
            let (x0, y0) = crate::scan::unrotate_point(w.bbox[0], base_y, img_w, img_h, rotated_by);
            let (x1, y1) = crate::scan::unrotate_point(w.bbox[2], base_y, img_w, img_h, rotated_by);
            let width = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            let angle = -(y1 - y0).atan2(x1 - x0).to_degrees();
            PlacedWord { text: w.text.clone(), x: x0 * k, y: y0 * k, width: width * k, size: lh * k * 0.85, angle }
        })
        .collect()
}
