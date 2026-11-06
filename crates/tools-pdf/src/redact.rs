//! pdf.redact — true removal of text (and images / line art under the boxes) for search terms,
//! presets (emails, phones, IBAN, card numbers, French NIR / SIRET / SIREN, dates) and imported
//! lists, plus metadata. Verified by re-extracting the text.

use std::path::Path;

use castor_core::options::{OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::{
    AnnotationArea, AnnotationQuadPoints, PdfRedactImageMethod, PdfRedactLineArtMethod, PdfRedactOptions, PdfRedactTextMethod,
};
use engine_pdf::mupdf::{Quad, TextPageFlags};
use regex::{Regex, RegexBuilder};

use crate::common::*;

pub struct Redact {
    meta: ToolMeta,
}

impl Redact {
    pub fn new() -> Self {
        Redact {
            meta: meta("pdf.redact", Family::Pdf, PDF_IN, "PDF", &["censor", "black out", "anonymize", "gdpr", "remove text", "privacy"], "redacted"),
        }
    }
}

const MONTHS: &str = "january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec|janvier|février|fevrier|mars|avril|mai|juin|juillet|août|aout|septembre|octobre|novembre|décembre|decembre";

/// A preset: regex and an optional validator for the digits.
struct Preset {
    key: &'static str,
    pattern: String,
    check: Option<fn(&str) -> bool>,
}

fn digits(s: &str) -> Vec<u32> {
    s.chars().filter_map(|c| c.to_digit(10)).collect()
}

pub fn luhn(s: &str) -> bool {
    let d = digits(s);
    if d.len() < 9 {
        return false;
    }
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &x)| {
            if i % 2 == 1 {
                let y = x * 2;
                if y > 9 { y - 9 } else { y }
            } else {
                x
            }
        })
        .sum();
    sum % 10 == 0
}

pub fn iban_ok(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
    if t.len() < 15 || t.len() > 34 {
        return false;
    }
    let r = format!("{}{}", &t[4..], &t[..4]);
    let mut rem: u64 = 0;
    for c in r.chars() {
        let v = if c.is_ascii_digit() {
            c as u64 - '0' as u64
        } else if c.is_ascii_uppercase() {
            c as u64 - 'A' as u64 + 10
        } else {
            return false;
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

fn card_ok(s: &str) -> bool {
    let n = digits(s).len();
    (13..=19).contains(&n) && luhn(s)
}

fn siret_ok(s: &str) -> bool {
    let n = digits(s).len();
    (n == 9 || n == 14) && luhn(s)
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { key: "emails", pattern: r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}".into(), check: None },
        Preset {
            key: "phones",
            pattern: [
                r"(?:\+|00)\d{1,3}[\s.\-]?(?:\(0\)[\s.\-]?)?\d{1,4}(?:[\s.\-]?\d{2,4}){2,5}",
                r"\b0[1-9](?:[\s.\-]?\d{2}){4}\b",
                r"\(?\b\d{3}\)?[\s.\-]\d{3}[\s.\-]\d{4}\b",
            ]
            .join("|"),
            check: None,
        },
        Preset { key: "iban", pattern: r"\b[A-Z]{2}\d{2}(?:[ ]?[A-Z0-9]{4}){2,7}(?:[ ]?[A-Z0-9]{1,4})?\b".into(), check: Some(iban_ok) },
        Preset { key: "cards", pattern: r"\b\d(?:[ \-]?\d){12,18}\b".into(), check: Some(card_ok) },
        Preset {
            key: "fr_ids",
            pattern: [
                // NIR (sécurité sociale), with or without the key.
                r"\b[12][ ]?\d{2}[ ]?(?:0[1-9]|1[0-2]|[2-9]\d)[ ]?(?:\d{2}|2[ABab])[ ]?\d{3}[ ]?\d{3}(?:[ ]?\d{2})?\b",
            ]
            .join("|"),
            check: None,
        },
        Preset { key: "fr_ids", pattern: r"\b\d{3}[ ]?\d{3}[ ]?\d{3}(?:[ ]?\d{5})?\b".into(), check: Some(siret_ok) },
        Preset {
            key: "dates",
            pattern: format!(
                r"(?i)\b\d{{1,2}}[/.\-]\d{{1,2}}[/.\-]\d{{2,4}}\b|\b\d{{4}}-\d{{2}}-\d{{2}}\b|\b\d{{1,2}}(?:er|st|nd|rd|th)?\s+(?:{MONTHS})\.?\s+\d{{4}}\b|\b(?:{MONTHS})\.?\s+\d{{1,2}}(?:st|nd|rd|th)?,?\s+\d{{4}}\b"
            ),
            check: None,
        },
    ]
}

/// Terms from a TXT / CSV / TSV file: one per line, or every cell.
fn read_list(p: &Path) -> Result<Vec<String>> {
    let s = std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let sep: Option<char> = match ext.as_str() {
        "csv" => Some(if s.lines().next().is_some_and(|l| l.matches(';').count() > l.matches(',').count()) { ';' } else { ',' }),
        "tsv" => Some('\t'),
        _ => None,
    };
    let mut out = Vec::new();
    for line in s.lines() {
        match sep {
            Some(c) => out.extend(line.split(c).map(|x| x.trim().trim_matches('"').to_string())),
            None => out.push(line.trim().to_string()),
        }
    }
    out.retain(|t| !t.is_empty());
    Ok(out)
}

/// Every matcher selected by the options.
fn matchers(v: &Values) -> Result<Vec<(Regex, Option<fn(&str) -> bool>)>> {
    let mut terms: Vec<String> = v.str("terms").lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if let Some(p) = v.path("list") {
        terms.extend(read_list(p)?);
    }
    let mut out = Vec::new();
    for t in terms {
        // Whitespace inside a term matches any run of whitespace (line breaks included).
        let body = t.split_whitespace().map(regex::escape).collect::<Vec<_>>().join(r"\s+");
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let pat = if v.bool("whole_word") {
            let pre = if word(t.chars().next()) { r"\b" } else { "" };
            let post = if word(t.chars().last()) { r"\b" } else { "" };
            format!("{pre}{body}{post}")
        } else {
            body
        };
        let re = RegexBuilder::new(&pat).case_insensitive(!v.bool("case_sensitive")).build()?;
        out.push((re, None));
    }
    for p in presets() {
        if v.bool(p.key) {
            out.push((Regex::new(&p.pattern)?, p.check));
        }
    }
    anyhow::ensure!(!out.is_empty(), "{}", tr("pdf.redact.err.nothing"));
    Ok(out)
}

/// Text of a page with, for each char, its quad. Lines of a block are joined by '\n'.
fn page_chars(page: &mupdf::Page) -> Result<(String, Vec<Option<Quad>>)> {
    let tp = page.to_text_page(TextPageFlags::empty())?;
    let mut text = String::new();
    let mut quads: Vec<Option<Quad>> = Vec::new();
    for b in tp.blocks() {
        for l in b.lines() {
            for c in l.chars() {
                if let Some(ch) = c.char() {
                    text.push(ch);
                    for _ in 0..ch.len_utf8() {
                        quads.push(Some(c.quad()));
                    }
                }
            }
            text.push('\n');
            quads.push(None);
        }
        text.push('\n');
        quads.push(None);
    }
    Ok((text, quads))
}

/// Find matches: per text line, one quad spanning the matched characters.
fn find(page: &mupdf::Page, ms: &[(Regex, Option<fn(&str) -> bool>)]) -> Result<Vec<Vec<Quad>>> {
    let (text, quads) = page_chars(page)?;
    let mut hits = Vec::new();
    for (re, check) in ms {
        for m in re.find_iter(&text) {
            if check.is_some_and(|c| !c(m.as_str())) {
                continue;
            }
            // Group the chars of the match by line (a None marks a line break).
            let mut lines: Vec<Quad> = Vec::new();
            let mut cur: Option<Quad> = None;
            for q in &quads[m.start()..m.end()] {
                match (q, cur.as_mut()) {
                    (Some(q), None) => cur = Some(q.clone()),
                    (Some(q), Some(c)) => {
                        c.ur = q.ur;
                        c.lr = q.lr;
                        // Keep the tallest extent of the line.
                        c.ul.y = c.ul.y.min(q.ul.y);
                        c.ll.y = c.ll.y.max(q.ll.y);
                        c.ur.y = c.ur.y.min(q.ur.y);
                        c.lr.y = c.lr.y.max(q.lr.y);
                    }
                    (None, _) => {
                        if let Some(c) = cur.take() {
                            lines.push(c);
                        }
                    }
                }
            }
            if let Some(c) = cur {
                lines.push(c);
            }
            if !lines.is_empty() {
                hits.push(lines);
            }
        }
    }
    Ok(hits)
}

impl Tool for Redact {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::multiline("terms", ""),
            OptionSpec::toggle("case_sensitive", false),
            OptionSpec::toggle("whole_word", true),
            OptionSpec::toggle("emails", false),
            OptionSpec::toggle("phones", false),
            OptionSpec::toggle("iban", false),
            OptionSpec::toggle("cards", false),
            OptionSpec::toggle("fr_ids", false),
            OptionSpec::toggle("dates", false),
            OptionSpec::file("list", &["txt", "csv", "tsv"]).advanced(),
            OptionSpec::color("fill", "#000000ff").advanced(),
            OptionSpec::toggle("strip_metadata", true).advanced(),
            OptionSpec::pages("pages").common().advanced(),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let ms = matchers(v)?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let (fill, _) = color(v, "fill");
        let opts = PdfRedactOptions {
            black_boxes: false,
            image_method: PdfRedactImageMethod::Pixels,
            line_art: PdfRedactLineArtMethod::RemoveIfCovered,
            text: PdfRedactTextMethod::Remove,
        };
        let (mut total, mut pages_hit) = (0usize, 0usize);
        for (k, &p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            page_msg(cx, k, sel.len());
            let hits = {
                let page = doc.load_page(p as i32)?;
                find(&page, &ms)?
            };
            if hits.is_empty() {
                continue;
            }
            let mut page = doc.load_pdf_page(p as i32)?;
            let mut boxes = Vec::new();
            for h in &hits {
                for q in h {
                    let r = mupdf::Rect {
                        x0: q.ul.x.min(q.ll.x),
                        y0: q.ul.y.min(q.ur.y),
                        x1: q.ur.x.max(q.lr.x),
                        y1: q.ll.y.max(q.lr.y),
                    };
                    boxes.push(r);
                }
                page.add_redact_annotation(AnnotationArea::QuadPoints(AnnotationQuadPoints::new(h.iter().cloned())))?;
            }
            page.apply_redactions_with_options(opts)?;
            // Paint the boxes ourselves (MuPDF's black boxes ignore the colour).
            let mut canvas = engine_pdf::draw::Canvas::new(&page)?;
            for r in boxes {
                canvas.rect(&mut doc, &mut page, r, Some(fill), None, 1.0)?;
            }
            canvas.commit(&mut doc, &mut page, true)?;
            total += hits.len();
            pages_hit += 1;
            prog(cx, k + 1, sel.len(), 0.0, 0.85);
        }
        if v.bool("strip_metadata") {
            doc.trailer()?.dict_delete("Info")?;
            doc.catalog()?.dict_delete("Metadata")?;
        }
        engine_pdf::set_info(&mut doc, "Producer", engine_pdf::PRODUCER)?;
        let out = out_for(cx, input, "pdf");
        let mut cleanup = Cleanup::new();
        cleanup.add(&out);
        // garbage 4 so removed objects (old images, text) are really gone from the file.
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { garbage: 4, keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        // Verify: nothing searchable must remain.
        cx.message(tr("pdf.redact.msg.verify"));
        let check = engine_pdf::open_pdf(&out, password(v))?;
        for &p in &sel {
            let page = check.load_page(p as i32)?;
            let left = find(&page, &ms)?;
            anyhow::ensure!(left.is_empty(), "{}", tr_args("pdf.redact.err.verify", &[("page", &(p + 1).to_string())]));
        }
        cleanup.keep();
        let note = tr_args("pdf.redact.note", &[("count", &total.to_string()), ("pages", &pages_hit.to_string())]);
        Ok(vec![Output::new(out).with_note(note)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validators() {
        assert!(luhn("4111 1111 1111 1111"));
        assert!(!luhn("4111 1111 1111 1112"));
        assert!(iban_ok("FR76 3000 6000 0112 3456 7890 189"));
        assert!(!iban_ok("FR76 3000 6000 0112 3456 7890 188"));
        assert!(siret_ok("732 829 320 00074"));
    }
}
