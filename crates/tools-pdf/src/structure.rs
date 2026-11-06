//! Turn the layout of a PDF into a document structure (headings, paragraphs, lists, simple
//! tables, code, images) for Markdown and EPUB output.

use std::collections::HashMap;

use castor_core::{Result, RunCx};
use engine_pdf::mupdf::{self, Document, TextPageFlags};
use engine_pdf::text::{self, Block, Line, Span};

#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Clone, Debug)]
pub enum Elem {
    Heading(u8, String),
    Para(Vec<Run>),
    Item { ordered: Option<String>, runs: Vec<Run> },
    Code(String),
    Table(Vec<Vec<String>>),
    /// PNG bytes, width, height.
    Image(Vec<u8>, u32, u32),
    PageBreak(usize),
}

pub struct Options {
    pub images: bool,
    pub page_breaks: bool,
    pub min_image: u32,
}

fn norm_key(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
}

fn is_bullet(s: &str) -> Option<(Option<String>, String)> {
    let t = s.trim_start();
    for b in ['•', '◦', '▪', '‣', '●', '○', '■', '□', '–', '-', '*', '·', '➢', '►', '✓', '✔'] {
        if let Some(rest) = t.strip_prefix(b) {
            if rest.starts_with(char::is_whitespace) || b != '-' && b != '*' {
                return Some((None, rest.trim_start().to_string()));
            }
        }
    }
    // "1." "2)" "a)" "iv."
    let mut chars = t.char_indices();
    let mut end = 0;
    for (i, c) in chars.by_ref() {
        if c.is_ascii_digit() || (i < 4 && c.is_ascii_lowercase()) {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if end > 0 && end <= 3 {
        let rest = &t[end..];
        if let Some(r) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            if r.starts_with(' ') {
                let num = &t[..end];
                if num.chars().all(|c| c.is_ascii_digit()) || num.len() == 1 {
                    return Some((Some(format!("{num}{}", &rest[..1])), r.trim_start().to_string()));
                }
            }
        }
    }
    None
}

fn runs_of(spans: &[Span]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for s in spans {
        let r = Run { text: s.text.clone(), bold: s.bold, italic: s.italic, code: s.mono };
        match out.last_mut() {
            Some(l) if l.bold == r.bold && l.italic == r.italic && l.code == r.code => l.text.push_str(&r.text),
            _ => out.push(r),
        }
    }
    out
}

fn join_runs(mut a: Vec<Run>, b: Vec<Run>) -> Vec<Run> {
    if let Some(l) = a.last_mut() {
        // Remove a soft hyphen at the end of the line when the word continues.
        if l.text.ends_with('-') && b.first().is_some_and(|f| f.text.starts_with(|c: char| c.is_lowercase())) {
            l.text.pop();
        } else if !l.text.ends_with(char::is_whitespace) {
            l.text.push(' ');
        }
    }
    for r in b {
        match a.last_mut() {
            Some(l) if l.bold == r.bold && l.italic == r.italic && l.code == r.code => l.text.push_str(&r.text),
            _ => a.push(r),
        }
    }
    a
}

pub fn runs_text(r: &[Run]) -> String {
    r.iter().map(|x| x.text.as_str()).collect()
}

/// Lines repeated on many pages near the top or bottom: running headers / footers.
fn furniture(all: &[(usize, f32, Vec<Block>)]) -> HashMap<String, usize> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for (_, h, blocks) in all {
        for b in blocks {
            if let Block::Text { lines, .. } = b {
                for l in lines {
                    if l.bbox.y1 < h * 0.09 || l.bbox.y0 > h * 0.91 {
                        *count.entry(norm_key(&l.text())).or_default() += 1;
                    }
                }
            }
        }
    }
    let pages = all.len();
    count.retain(|k, n| !k.is_empty() && pages >= 3 && *n * 2 >= pages);
    count
}

/// Analyse the whole document.
pub fn analyse(doc: &Document, opts: &Options, cx: &RunCx, from: f32, to: f32) -> Result<Vec<Elem>> {
    let n = doc.page_count()?.max(0) as usize;
    let mut all = Vec::with_capacity(n);
    let mut sizes: HashMap<i32, usize> = HashMap::new();
    for i in 0..n {
        cx.check_cancel()?;
        let page = doc.load_page(i as i32)?;
        let h = page.bounds()?.height();
        let blocks = text::blocks(&page, false)?;
        for b in &blocks {
            if let Block::Text { lines, .. } = b {
                for l in lines {
                    for s in &l.spans {
                        *sizes.entry((s.size * 2.0).round() as i32).or_default() += s.text.chars().filter(|c| !c.is_whitespace()).count();
                    }
                }
            }
        }
        all.push((i, h, blocks));
        crate::common::prog(cx, i + 1, n, from, from + (to - from) * 0.4);
    }
    let body = sizes.iter().max_by_key(|(_, c)| **c).map(|(s, _)| *s as f32 / 2.0).unwrap_or(10.0);
    // Heading sizes: noticeably larger than the body, by frequency-limited clusters.
    let mut hsizes: Vec<f32> = sizes
        .iter()
        .filter(|(s, c)| (**s as f32 / 2.0) > body * 1.12 && **c >= 2)
        .map(|(s, _)| *s as f32 / 2.0)
        .collect();
    hsizes.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let mut levels: Vec<f32> = Vec::new();
    for s in hsizes {
        if levels.last().is_none_or(|l| *l - s > 0.9) {
            levels.push(s);
        }
    }
    levels.truncate(4);
    let level_of = |size: f32| -> Option<u8> {
        if size <= body * 1.12 {
            return None;
        }
        levels.iter().position(|l| (size - l).abs() <= 0.9).map(|i| i as u8 + 1).or(Some(levels.len().clamp(1, 5) as u8))
    };
    let furn = furniture(&all);

    let mut out: Vec<Elem> = Vec::new();
    let mut img_count = 0usize;
    for (pi, h, blocks) in &all {
        cx.check_cancel()?;
        if *pi > 0 && opts.page_breaks {
            out.push(Elem::PageBreak(*pi));
        }
        for b in blocks {
            let Block::Text { lines, .. } = b else { continue };
            let lines: Vec<&Line> = lines
                .iter()
                .filter(|l| {
                    let t = l.text();
                    let edge = l.bbox.y1 < h * 0.09 || l.bbox.y0 > h * 0.91;
                    let pure_number = t.trim().chars().all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '/' || c == '-');
                    !(edge && (furn.contains_key(&norm_key(&t)) || pure_number))
                })
                .collect();
            if lines.is_empty() {
                continue;
            }
            // Code: every line monospaced.
            if lines.iter().all(|l| l.spans.iter().filter(|s| !s.text.trim().is_empty()).all(|s| s.mono)) && lines.len() >= 2 {
                out.push(Elem::Code(lines.iter().map(|l| l.text().trim_end().to_string()).collect::<Vec<_>>().join("\n")));
                continue;
            }
            // Table: ≥2 lines with the same number (≥2) of gap-separated cells.
            let cells = |l: &Line| -> Vec<String> {
                let mut c: Vec<String> = vec![String::new()];
                for s in &l.spans {
                    if s.gap_before {
                        c.push(String::new());
                    }
                    c.last_mut().unwrap().push_str(&s.text);
                }
                c.into_iter().map(|x| x.trim().to_string()).collect()
            };
            let counts: Vec<usize> = lines.iter().map(|l| cells(l).len()).collect();
            if lines.len() >= 2 && counts[0] >= 2 && counts.iter().all(|c| *c == counts[0]) {
                out.push(Elem::Table(lines.iter().map(|l| cells(l)).collect()));
                continue;
            }
            let mut para: Option<Vec<Run>> = None;
            let mut item: Option<(Option<String>, Vec<Run>)> = None;
            let flush = |out: &mut Vec<Elem>, para: &mut Option<Vec<Run>>, item: &mut Option<(Option<String>, Vec<Run>)>| {
                if let Some(p) = para.take() {
                    if !runs_text(&p).trim().is_empty() {
                        out.push(Elem::Para(p));
                    }
                }
                if let Some((o, r)) = item.take() {
                    out.push(Elem::Item { ordered: o, runs: r });
                }
            };
            for l in lines {
                let t = l.text();
                if t.trim().is_empty() {
                    continue;
                }
                if let Some(lv) = level_of(l.size()) {
                    flush(&mut out, &mut para, &mut item);
                    // Multi-line headings: merge with the previous heading of the same level.
                    if let Some(Elem::Heading(pl, pt)) = out.last_mut() {
                        if *pl == lv {
                            pt.push(' ');
                            pt.push_str(t.trim());
                            continue;
                        }
                    }
                    out.push(Elem::Heading(lv, t.trim().to_string()));
                    continue;
                }
                if let Some((ord, rest)) = is_bullet(&t) {
                    flush(&mut out, &mut para, &mut item);
                    let mut runs = runs_of(&l.spans);
                    // Drop the bullet text from the first run(s).
                    let mut skip = t.len() - rest.len();
                    for r in runs.iter_mut() {
                        if skip == 0 {
                            break;
                        }
                        let k = skip.min(r.text.len());
                        let k = (0..=k).rev().find(|i| r.text.is_char_boundary(*i)).unwrap_or(0);
                        r.text.drain(..k);
                        skip -= k;
                        if k == 0 {
                            break;
                        }
                    }
                    runs.retain(|r| !r.text.is_empty());
                    item = Some((ord, runs));
                    continue;
                }
                let runs = runs_of(&l.spans);
                if let Some((o, r)) = item.take() {
                    item = Some((o, join_runs(r, runs)));
                } else {
                    para = Some(match para.take() {
                        Some(p) => join_runs(p, runs),
                        None => runs,
                    });
                }
            }
            flush(&mut out, &mut para, &mut item);
        }
        if opts.images {
            let page = doc.load_page(*pi as i32)?;
            let tp = page.to_text_page(TextPageFlags::PRESERVE_IMAGES)?;
            for b in tp.blocks() {
                if b.r#type() != mupdf::text_page::TextBlockType::Image {
                    continue;
                }
                let bb = b.bounds();
                if bb.width() < opts.min_image as f32 && bb.height() < opts.min_image as f32 {
                    continue;
                }
                let Some(img) = b.image() else { continue };
                let Ok(pix) = img.to_pixmap() else { continue };
                let mut png = Vec::new();
                // Gray/RGB only; others via RGB conversion by rendering the pixmap.
                if pix.n() as usize - pix.alpha() as usize >= 4 {
                    continue;
                }
                if pix.write_to(&mut png, mupdf::ImageFormat::PNG).is_ok() {
                    img_count += 1;
                    out.push(Elem::Image(png, pix.width(), pix.height()));
                }
            }
        }
        crate::common::prog(cx, *pi + 1, n, from + (to - from) * 0.4, to);
    }
    let _ = img_count;
    // Join paragraphs split by a page break mid-sentence.
    let mut merged: Vec<Elem> = Vec::with_capacity(out.len());
    for e in out {
        if let Elem::Para(r) = &e {
            let starts_lower = runs_text(r).trim_start().starts_with(|c: char| c.is_lowercase());
            let prev_para = merged.iter().rev().find(|x| !matches!(x, Elem::PageBreak(_) | Elem::Image(..)));
            if starts_lower {
                if let Some(Elem::Para(pr)) = prev_para {
                    let t = runs_text(pr);
                    let open = !t.trim_end().ends_with(['.', '!', '?', ':', '…', '"', '»', ')']);
                    if open {
                        // Find it again mutably.
                        let idx = merged.iter().rposition(|x| matches!(x, Elem::Para(_))).unwrap();
                        if let Elem::Para(pr) = &mut merged[idx] {
                            let joined = join_runs(std::mem::take(pr), r.clone());
                            *pr = joined;
                        }
                        continue;
                    }
                }
            }
        }
        merged.push(e);
    }
    Ok(merged)
}

fn md_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '_' | '`' | '[' | ']' | '<' | '>' | '#' | '|' | '\\') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

fn md_runs(r: &[Run]) -> String {
    let mut o = String::new();
    for x in r {
        let t = x.text.replace('\u{ad}', "");
        let core = t.trim();
        if core.is_empty() {
            o.push_str(&t);
            continue;
        }
        let lead = &t[..t.len() - t.trim_start().len()];
        let trail = &t[t.trim_end().len()..];
        o.push_str(lead);
        if x.code {
            o.push('`');
            o.push_str(core);
            o.push('`');
        } else {
            let mark = match (x.bold, x.italic) {
                (true, true) => "***",
                (true, false) => "**",
                (false, true) => "*",
                _ => "",
            };
            o.push_str(mark);
            o.push_str(&md_escape(core));
            o.push_str(mark);
        }
        o.push_str(trail);
    }
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Markdown text; images are written by `save_image(index, png) -> relative path`.
pub fn to_markdown(elems: &[Elem], mut save_image: impl FnMut(usize, &[u8]) -> Result<String>) -> Result<String> {
    let mut o = String::new();
    let mut img = 0;
    let mut prev_item = false;
    for e in elems {
        let is_item = matches!(e, Elem::Item { .. });
        if prev_item && !is_item {
            o.push('\n');
        }
        match e {
            Elem::Heading(l, t) => {
                o.push_str(&format!("{} {}\n\n", "#".repeat(*l as usize), md_escape(t)));
            }
            Elem::Para(r) => {
                let t = md_runs(r);
                if !t.is_empty() {
                    o.push_str(&t);
                    o.push_str("\n\n");
                }
            }
            Elem::Item { ordered, runs } => {
                let mark = ordered.clone().unwrap_or_else(|| "-".into());
                o.push_str(&format!("{mark} {}\n", md_runs(runs)));
            }
            Elem::Code(c) => o.push_str(&format!("```\n{c}\n```\n\n")),
            Elem::Table(rows) => {
                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                for (i, r) in rows.iter().enumerate() {
                    let cells: Vec<String> = (0..cols).map(|c| md_escape(r.get(c).map(|s| s.as_str()).unwrap_or(""))).collect();
                    o.push_str(&format!("| {} |\n", cells.join(" | ")));
                    if i == 0 {
                        o.push_str(&format!("|{}\n", " --- |".repeat(cols)));
                    }
                }
                o.push('\n');
            }
            Elem::Image(png, _, _) => {
                img += 1;
                let rel = save_image(img, png)?;
                o.push_str(&format!("![]({rel})\n\n"));
            }
            Elem::PageBreak(_) => o.push_str("---\n\n"),
        }
        prev_item = is_item;
    }
    Ok(o.trim_end().to_string() + "\n")
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html_runs(r: &[Run]) -> String {
    let mut o = String::new();
    for x in r {
        let t = xml_escape(&x.text.replace('\u{ad}', ""));
        let t = match (x.code, x.bold, x.italic) {
            (true, ..) => format!("<code>{t}</code>"),
            (_, true, true) => format!("<strong><em>{t}</em></strong>"),
            (_, true, false) => format!("<strong>{t}</strong>"),
            (_, false, true) => format!("<em>{t}</em>"),
            _ => t,
        };
        o.push_str(&t);
    }
    o
}

/// XHTML body fragments, split into chapters at level-1 (or level-2) headings.
/// Returns (title, body) per chapter; images referenced as `images/img-N.png` via `save_image`.
pub fn to_xhtml_chapters(elems: &[Elem], mut save_image: impl FnMut(usize, &[u8])) -> Vec<(String, String)> {
    let split_level = if elems.iter().any(|e| matches!(e, Elem::Heading(1, _))) { 1 } else { 2 };
    let mut chapters: Vec<(String, String)> = Vec::new();
    let mut cur = (String::new(), String::new());
    let mut img = 0;
    let mut in_list: Option<bool> = None;
    let close_list = |body: &mut String, in_list: &mut Option<bool>| {
        if let Some(ord) = in_list.take() {
            body.push_str(if ord { "</ol>\n" } else { "</ul>\n" });
        }
    };
    for e in elems {
        if !matches!(e, Elem::Item { .. }) {
            close_list(&mut cur.1, &mut in_list);
        }
        match e {
            Elem::Heading(l, t) => {
                if *l <= split_level && !cur.1.trim().is_empty() {
                    chapters.push(std::mem::take(&mut cur));
                }
                if *l <= split_level && cur.0.is_empty() {
                    cur.0 = t.clone();
                }
                cur.1.push_str(&format!("<h{l}>{}</h{l}>\n", xml_escape(t)));
            }
            Elem::Para(r) => cur.1.push_str(&format!("<p>{}</p>\n", html_runs(r))),
            Elem::Item { ordered, runs } => {
                let ord = ordered.is_some();
                if in_list != Some(ord) {
                    close_list(&mut cur.1, &mut in_list);
                    cur.1.push_str(if ord { "<ol>\n" } else { "<ul>\n" });
                    in_list = Some(ord);
                }
                cur.1.push_str(&format!("<li>{}</li>\n", html_runs(runs)));
            }
            Elem::Code(c) => cur.1.push_str(&format!("<pre><code>{}</code></pre>\n", xml_escape(c))),
            Elem::Table(rows) => {
                cur.1.push_str("<table>\n");
                for (i, r) in rows.iter().enumerate() {
                    let tag = if i == 0 { "th" } else { "td" };
                    cur.1.push_str("<tr>");
                    for c in r {
                        cur.1.push_str(&format!("<{tag}>{}</{tag}>", xml_escape(c)));
                    }
                    cur.1.push_str("</tr>\n");
                }
                cur.1.push_str("</table>\n");
            }
            Elem::Image(png, _, _) => {
                img += 1;
                save_image(img, png);
                cur.1.push_str(&format!("<p class=\"img\"><img src=\"images/img-{img}.png\" alt=\"\"/></p>\n"));
            }
            Elem::PageBreak(_) => {}
        }
    }
    close_list(&mut cur.1, &mut in_list);
    if !cur.1.trim().is_empty() {
        chapters.push(cur);
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bullets() {
        assert_eq!(is_bullet("• hello").unwrap().1, "hello");
        assert_eq!(is_bullet("12. twelve").unwrap().0.as_deref(), Some("12."));
        assert!(is_bullet("-5 degrees").is_none());
        assert!(is_bullet("2024. A year").is_none());
    }
}
