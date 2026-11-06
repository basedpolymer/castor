//! Drawing on existing PDF pages: text (base-14 or embedded fonts, any angle), rectangles and
//! images, with opacity, over or under the page content.
//!
//! All coordinates are in *visible page space*: points, origin at the top-left of the page as
//! displayed (rotation and crop box applied), y pointing down — the same space MuPDF reports for
//! `Page::bounds`, text and search results.

use std::collections::HashMap;
use std::fmt::Write;

use anyhow::Result;
use mupdf::pdf::{InsertFontOptions, PdfDocument, PdfObject, PdfPage};
use mupdf::{Font, Rect};

use crate::fonts::Face;

/// Affine matrix [a b c d e f], PDF convention (row vectors): p' = p × M.
pub type M = [f32; 6];

pub const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`.
pub fn concat(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn translate(x: f32, y: f32) -> M {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// Visual counter-clockwise rotation in a y-down space.
pub fn rotate_ccw(deg: f32) -> M {
    let (s, c) = deg.to_radians().sin_cos();
    [c, -s, s, c, 0.0, 0.0]
}

pub fn apply(m: M, x: f32, y: f32) -> (f32, f32) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

fn num(v: f32) -> String {
    let v = if v.abs() < 1e-5 { 0.0 } else { v };
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

fn mat(m: M) -> String {
    m.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// RGB 0..1.
pub type Color = [f32; 3];

pub fn color_from_rgba(c: [u8; 4]) -> (Color, f32) {
    ([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0], c[3] as f32 / 255.0)
}

/// Windows-1252 (WinAnsi) byte for a character, as used by MuPDF simple fonts.
pub fn winansi(ch: char) -> Option<u8> {
    let c = ch as u32;
    if (0x20..0x7f).contains(&c) || (0xa0..=0xff).contains(&c) {
        return Some(c as u8);
    }
    const HIGH: &[(char, u8)] = &[
        ('€', 0x80),
        ('‚', 0x82),
        ('ƒ', 0x83),
        ('„', 0x84),
        ('…', 0x85),
        ('†', 0x86),
        ('‡', 0x87),
        ('ˆ', 0x88),
        ('‰', 0x89),
        ('Š', 0x8a),
        ('‹', 0x8b),
        ('Œ', 0x8c),
        ('Ž', 0x8e),
        ('‘', 0x91),
        ('’', 0x92),
        ('“', 0x93),
        ('”', 0x94),
        ('•', 0x95),
        ('–', 0x96),
        ('—', 0x97),
        ('˜', 0x98),
        ('™', 0x99),
        ('š', 0x9a),
        ('›', 0x9b),
        ('œ', 0x9c),
        ('ž', 0x9e),
        ('Ÿ', 0x9f),
    ];
    HIGH.iter().find(|(c2, _)| *c2 == ch).map(|(_, b)| *b)
}

/// A loaded face: resource name on the page, font for metrics, and how to encode text.
struct LoadedFont {
    res: String,
    font: Font,
    simple: bool,
}

/// Width of `text` in points at `size`.
pub fn text_width(font: &Font, simple: bool, text: &str, size: f32) -> f32 {
    let mut w = 0.0;
    for ch in text.chars() {
        let ch = if simple && winansi(ch).is_none() { '?' } else { ch };
        if let Ok(g) = font.encode_character(ch as i32) {
            w += font.advance_glyph(g).unwrap_or(0.5);
        }
    }
    w * size
}

/// Collects drawing operations for one page, then writes them as one content stream.
pub struct Canvas {
    /// Visible page space → PDF user space.
    to_user: M,
    ops: String,
    fonts: HashMap<Face, LoadedFont>,
    gstates: HashMap<u32, String>,
    images: usize,
    pub bounds: Rect,
}

impl Canvas {
    pub fn new(page: &PdfPage) -> Result<Canvas> {
        let ctm = page.ctm()?;
        let inv = ctm.invert().ok_or_else(|| anyhow::anyhow!("page transform is not invertible"))?;
        Ok(Canvas {
            to_user: [inv.a, inv.b, inv.c, inv.d, inv.e, inv.f],
            ops: String::new(),
            fonts: HashMap::new(),
            gstates: HashMap::new(),
            images: 0,
            bounds: page.bounds()?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    fn font(&mut self, doc: &mut PdfDocument, page: &mut PdfPage, face: Face) -> Result<&LoadedFont> {
        if !self.fonts.contains_key(&face) {
            let lf = match face.bytes() {
                Some(bytes) => {
                    let mut o = InsertFontOptions::new(face.name());
                    o.fontfile = Some(bytes);
                    o.simple = false;
                    let (res, _, _) = page.insert_font(doc, &o)?;
                    LoadedFont { res, font: face.load()?, simple: false }
                }
                None => {
                    let alias = match face {
                        Face::HelveticaBold => "hebo",
                        Face::Times => "tiro",
                        Face::TimesBold => "tibo",
                        Face::Courier => "cour",
                        _ => "helv",
                    };
                    let (res, _, _) = page.insert_font(doc, &InsertFontOptions::new(alias))?;
                    LoadedFont { res, font: face.load()?, simple: true }
                }
            };
            self.fonts.insert(face, lf);
        }
        Ok(&self.fonts[&face])
    }

    fn gstate(&mut self, doc: &mut PdfDocument, page: &mut PdfPage, opacity: f32) -> Result<Option<String>> {
        if opacity >= 0.999 {
            return Ok(None);
        }
        let key = (opacity.clamp(0.0, 1.0) * 1000.0) as u32;
        if let Some(n) = self.gstates.get(&key) {
            return Ok(Some(n.clone()));
        }
        let op = key as f32 / 1000.0;
        let name = page.register_ext_gstate(doc, Some(op), Some(op))?.unwrap_or_default();
        self.gstates.insert(key, name.clone());
        Ok(Some(name))
    }

    /// Measure `text` with `face`.
    pub fn measure(&mut self, doc: &mut PdfDocument, page: &mut PdfPage, face: Face, size: f32, text: &str) -> Result<f32> {
        let f = self.font(doc, page, face)?;
        Ok(text_width(&f.font, f.simple, text, size))
    }

    /// Font ascender/descender (em units, descender negative).
    pub fn metrics(&mut self, doc: &mut PdfDocument, page: &mut PdfPage, face: Face) -> Result<(f32, f32)> {
        let f = self.font(doc, page, face)?;
        Ok((f.font.ascender(), f.font.descender()))
    }

    /// Draw one line of text whose baseline starts at (`x`, `y`), rotated `angle` degrees
    /// counter-clockwise around that point.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        doc: &mut PdfDocument,
        page: &mut PdfPage,
        face: Face,
        size: f32,
        text: &str,
        x: f32,
        y: f32,
        angle: f32,
        color: Color,
        opacity: f32,
    ) -> Result<()> {
        let gs = self.gstate(doc, page, opacity)?;
        let f = self.font(doc, page, face)?;
        let mut hex = String::new();
        for ch in text.chars() {
            if f.simple {
                let b = winansi(ch).unwrap_or(b'?');
                let _ = write!(hex, "{b:02x}");
            } else {
                let g = f.font.encode_character(ch as i32).unwrap_or(0);
                let _ = write!(hex, "{:04x}", g.clamp(0, 0xffff));
            }
        }
        let res = f.res.clone();
        let (s, c) = angle.to_radians().sin_cos();
        // Text space (y up) → visible space (y down), rotated ccw.
        let tm: M = [c, -s, -s, -c, x, y];
        let _ = write!(self.ops, "q\n");
        if let Some(gs) = gs {
            let _ = writeln!(self.ops, "{gs} gs");
        }
        let _ = write!(
            self.ops,
            "{} {} {} rg\nBT\n{res} {} Tf\n{} Tm\n<{hex}> Tj\nET\nQ\n",
            num(color[0]),
            num(color[1]),
            num(color[2]),
            num(size),
            mat(tm)
        );
        Ok(())
    }

    /// Fill (and/or stroke) a rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &mut self,
        doc: &mut PdfDocument,
        page: &mut PdfPage,
        r: Rect,
        fill: Option<Color>,
        stroke: Option<(Color, f32)>,
        opacity: f32,
    ) -> Result<()> {
        let gs = self.gstate(doc, page, opacity)?;
        let _ = write!(self.ops, "q\n");
        if let Some(gs) = gs {
            let _ = writeln!(self.ops, "{gs} gs");
        }
        if let Some(c) = fill {
            let _ = writeln!(self.ops, "{} {} {} rg", num(c[0]), num(c[1]), num(c[2]));
        }
        if let Some((c, w)) = stroke {
            let _ = writeln!(self.ops, "{} {} {} RG {} w", num(c[0]), num(c[1]), num(c[2]), num(w));
        }
        let op = match (fill.is_some(), stroke.is_some()) {
            (true, true) => "B",
            (true, false) => "f",
            (false, true) => "S",
            _ => "n",
        };
        let _ = write!(self.ops, "{} {} {} {} re {op}\nQ\n", num(r.x0), num(r.y0), num(r.x1 - r.x0), num(r.y1 - r.y0));
        Ok(())
    }

    /// Draw an image into `r`, rotated `angle` degrees counter-clockwise around the rect centre.
    pub fn image(
        &mut self,
        doc: &mut PdfDocument,
        page: &mut PdfPage,
        img: &mupdf::Image,
        r: Rect,
        angle: f32,
        opacity: f32,
    ) -> Result<()> {
        let obj = doc.add_image(img)?;
        self.image_obj(doc, page, &obj, r, angle, opacity)
    }

    /// Same as [`Canvas::image`] with an image object already added to the document (reuse it
    /// across pages to store the image once).
    pub fn image_obj(
        &mut self,
        doc: &mut PdfDocument,
        page: &mut PdfPage,
        obj: &PdfObject,
        r: Rect,
        angle: f32,
        opacity: f32,
    ) -> Result<()> {
        let gs = self.gstate(doc, page, opacity)?;
        let mut res = page.resources()?;
        let mut xo = match res.get_dict("XObject")? {
            Some(x) if x.is_dict()? => x,
            _ => {
                let d = doc.new_dict()?;
                res.dict_put("XObject", d)?;
                res.get_dict("XObject")?.ok_or_else(|| anyhow::anyhow!("XObject"))?
            }
        };
        let name = loop {
            let n = format!("CastorIm{}", self.images);
            self.images += 1;
            if xo.get_dict(n.as_str())?.is_none() {
                break n;
            }
        };
        xo.dict_put(name.as_str(), obj.clone())?;
        let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        // Unit square (y up) → centred box (y down) → rotation → position.
        let m = concat(concat([w, 0.0, 0.0, -h, -w / 2.0, h / 2.0], rotate_ccw(angle)), translate(cx, cy));
        let _ = write!(self.ops, "q\n");
        if let Some(gs) = gs {
            let _ = writeln!(self.ops, "{gs} gs");
        }
        let _ = write!(self.ops, "{} cm\n/{name} Do\nQ\n", mat(m));
        Ok(())
    }

    /// Raw operators in visible page space (advanced use: OCR text layer).
    pub fn raw(&mut self, ops: &str) {
        self.ops.push_str(ops);
    }

    /// Write the collected operations to the page, over (`overlay`) or under the content.
    pub fn commit(self, doc: &mut PdfDocument, page: &mut PdfPage, overlay: bool) -> Result<()> {
        if self.ops.is_empty() {
            return Ok(());
        }
        if overlay {
            page.wrap_contents(doc)?;
        }
        let content = format!("q\n{} cm\n{}Q\n", mat(self.to_user), self.ops);
        page.insert_contents(doc, content.as_bytes(), overlay)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrices() {
        let m = concat(translate(1.0, 2.0), rotate_ccw(90.0));
        let (x, y) = apply(m, 0.0, 0.0);
        assert!((x - 2.0).abs() < 1e-5 && (y + 1.0).abs() < 1e-5, "{x} {y}");
        assert_eq!(num(1.50), "1.5");
        assert_eq!(num(-0.000001), "0");
        assert_eq!(winansi('é'), Some(0xe9));
        assert_eq!(winansi('€'), Some(0x80));
    }
}
