//! Text extraction with layout information (for Markdown, EPUB, redaction, compare).

use anyhow::Result;
use mupdf::{Page, Rect, TextCharFlags, TextPageFlags};

/// A styled run of characters on a line.
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub bbox: Rect,
    /// A wide horizontal gap precedes this span (column separator in tables).
    pub gap_before: bool,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub bbox: Rect,
    pub spans: Vec<Span>,
}

impl Line {
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
    /// Dominant font size (weighted by characters).
    pub fn size(&self) -> f32 {
        let mut best = (0usize, 0.0f32);
        let mut acc: Vec<(f32, usize)> = Vec::new();
        for s in &self.spans {
            let n = s.text.chars().filter(|c| !c.is_whitespace()).count();
            match acc.iter_mut().find(|(sz, _)| (*sz - s.size).abs() < 0.3) {
                Some(e) => e.1 += n,
                None => acc.push((s.size, n)),
            }
        }
        for (sz, n) in acc {
            if n > best.0 {
                best = (n, sz);
            }
        }
        best.1
    }
    pub fn all_bold(&self) -> bool {
        self.spans.iter().filter(|s| !s.text.trim().is_empty()).all(|s| s.bold)
            && self.spans.iter().any(|s| !s.text.trim().is_empty())
    }
}

#[derive(Clone, Debug)]
pub enum Block {
    Text { bbox: Rect, lines: Vec<Line> },
    Image { bbox: Rect },
}

/// Layout of a page: blocks in MuPDF's reading order.
pub fn blocks(page: &Page, images: bool) -> Result<Vec<Block>> {
    let mut flags = TextPageFlags::DEHYPHENATE;
    if images {
        flags |= TextPageFlags::PRESERVE_IMAGES;
    }
    let tp = page.to_text_page(flags)?;
    let mut out = Vec::new();
    for b in tp.blocks() {
        match b.r#type() {
            mupdf::text_page::TextBlockType::Text => {
                let mut ls = Vec::new();
                for l in b.lines() {
                    let mut spans: Vec<Span> = Vec::new();
                    let mut last_font: Option<(String, bool, bool)> = None;
                    for ch in l.chars() {
                        let Some(c) = ch.char() else { continue };
                        let f = ch.flags();
                        let (italic, mono, fbold) = match ch.font() {
                            Some(font) => {
                                let name = font.name().to_string();
                                let lower = name.to_ascii_lowercase();
                                let it = font.is_italic() || lower.contains("italic") || lower.contains("oblique");
                                let mo = font.is_monospaced() || lower.contains("mono") || lower.contains("courier");
                                let bo = font.is_bold() || lower.contains("bold") || lower.contains("black");
                                last_font = Some((name, it, mo));
                                (it, mo, bo)
                            }
                            None => last_font.as_ref().map(|(_, i, m)| (*i, *m, false)).unwrap_or((false, false, false)),
                        };
                        let bold = fbold || f.contains(TextCharFlags::BOLD);
                        let q = ch.quad();
                        let r = Rect {
                            x0: q.ul.x.min(q.ll.x),
                            y0: q.ul.y.min(q.ur.y),
                            x1: q.ur.x.max(q.lr.x),
                            y1: q.ll.y.max(q.lr.y),
                        };
                        let size = ch.size();
                        let gap = spans.last().is_some_and(|s| r.x0 - s.bbox.x1 > size.max(1.0) * 1.2);
                        let same = !gap
                            && spans.last().is_some_and(|s| {
                                (s.size - size).abs() < 0.1 && s.bold == bold && s.italic == italic && s.mono == mono
                            });
                        if same {
                            let s = spans.last_mut().unwrap();
                            s.text.push(c);
                            s.bbox = s.bbox.union(&r);
                        } else {
                            spans.push(Span { text: c.to_string(), size, bold, italic, mono, bbox: r, gap_before: gap });
                        }
                    }
                    if !spans.is_empty() {
                        ls.push(Line { bbox: l.bounds(), spans });
                    }
                }
                if !ls.is_empty() {
                    out.push(Block::Text { bbox: b.bounds(), lines: ls });
                }
            }
            mupdf::text_page::TextBlockType::Image => out.push(Block::Image { bbox: b.bounds() }),
            _ => {}
        }
    }
    Ok(out)
}

/// Plain text of a page (MuPDF's reading order).
pub fn page_text(page: &Page) -> Result<String> {
    let tp = page.to_text_page(TextPageFlags::empty())?;
    Ok(tp.to_text()?)
}
