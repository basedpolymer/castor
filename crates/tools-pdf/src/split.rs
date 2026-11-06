//! pdf.split — every page / ranges / every N pages / by size / by bookmarks.

use castor_core::options::{parse_page_range, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::pages;

use crate::common::*;

pub struct Split {
    meta: ToolMeta,
}

impl Split {
    pub fn new() -> Self {
        Split { meta: meta("pdf.split", Family::Pdf, PDF_IN, "PDF", &["separate", "divide", "burst", "cut", "pages"], "") }
    }
}

/// A part: 0-based pages and a name suffix.
struct Part {
    pages: Vec<usize>,
    label: String,
}

fn range_label(pages: &[usize]) -> String {
    match (pages.first(), pages.last()) {
        (Some(a), Some(b)) if a == b => format!("p{}", a + 1),
        (Some(a), Some(b)) if pages.len() == b - a + 1 => format!("p{}-{}", a + 1, b + 1),
        (Some(a), _) => format!("p{}-etc", a + 1),
        _ => "empty".into(),
    }
}

fn plan_parts(src: &mut PdfDocument, v: &Values, n: usize, cx: &RunCx) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    match v.str("mode") {
        "ranges" => {
            let spec = v.str("ranges");
            anyhow::ensure!(!spec.trim().is_empty(), "{}", tr("pdf.split.err.ranges"));
            for chunk in spec.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
                let p = parse_page_range(chunk, n).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?;
                if !p.is_empty() {
                    parts.push(Part { label: range_label(&p), pages: p });
                }
            }
        }
        "every_n" => {
            let k = v.int("every").max(1) as usize;
            for start in (0..n).step_by(k) {
                let p: Vec<usize> = (start..(start + k).min(n)).collect();
                parts.push(Part { label: range_label(&p), pages: p });
            }
        }
        "size" => {
            let limit = (v.float("max_mb").max(0.05) * 1_000_000.0) as u64;
            // Size of each page alone, minus the fixed overhead of a document.
            let base = {
                let mut d = PdfDocument::new();
                let mut b = Vec::new();
                d.new_page(mupdf::Size::A4)?;
                d.write_to(&mut b)?;
                b.len() as u64
            };
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_size = base;
            for i in 0..n {
                cx.check_cancel()?;
                prog(cx, i, n, 0.0, 0.3);
                let mut one = PdfDocument::new();
                pages::copy_pages(&mut one, src, &[i], None)?;
                let mut b = Vec::new();
                one.write_to(&mut b)?;
                let sz = (b.len() as u64).saturating_sub(base / 2);
                if !cur.is_empty() && cur_size + sz > limit {
                    parts.push(Part { label: range_label(&cur), pages: std::mem::take(&mut cur) });
                    cur_size = base;
                }
                cur.push(i);
                cur_size += sz;
            }
            if !cur.is_empty() {
                parts.push(Part { label: range_label(&cur), pages: cur });
            }
        }
        "bookmarks" => {
            let doc: &mupdf::Document = src;
            let tops: Vec<(String, usize)> = pages::outline_entries(doc)
                .into_iter()
                .filter(|(lvl, _, p)| *lvl == 0 && p.is_some())
                .map(|(_, t, p)| (t, p.unwrap()))
                .collect();
            anyhow::ensure!(!tops.is_empty(), "{}", tr("pdf.split.err.no_bookmarks"));
            let mut starts: Vec<(String, usize)> = Vec::new();
            if tops[0].1 > 0 {
                starts.push((String::new(), 0));
            }
            for t in tops {
                if starts.last().is_some_and(|(_, s)| *s >= t.1) {
                    continue;
                }
                starts.push(t);
            }
            for (k, (title, s)) in starts.iter().enumerate() {
                let e = starts.get(k + 1).map(|x| x.1).unwrap_or(n);
                let p: Vec<usize> = (*s..e).collect();
                let label = if title.trim().is_empty() {
                    range_label(&p)
                } else {
                    let t: String = title.chars().filter(|c| !c.is_control()).take(60).collect();
                    format!("{:02}-{}", k + 1, t.trim())
                };
                parts.push(Part { pages: p, label });
            }
        }
        _ => {
            for i in 0..n {
                parts.push(Part { pages: vec![i], label: format!("p{}", i + 1) });
            }
        }
    }
    anyhow::ensure!(!parts.is_empty(), "{}", tr("pdf.err.no_pages"));
    Ok(parts)
}

impl Tool for Split {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "mode",
                vec![Choice::key("every_page"), Choice::key("ranges"), Choice::key("every_n"), Choice::key("size"), Choice::key("bookmarks")],
                "every_page",
            ),
            OptionSpec::text("ranges", "").placeholder("pdf.split.opt.ranges.placeholder").when(Cond::Eq("mode", "ranges")),
            OptionSpec::int("every", 1, 10000, 2).when(Cond::Eq("mode", "every_n")),
            OptionSpec::float("max_mb", 0.1, 2000.0, 10.0).unit("MB").when(Cond::Eq("mode", "size")),
            password_opt(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut src = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = src.page_count()?.max(0) as usize;
        anyhow::ensure!(n > 0, "{}", tr("pdf.err.no_pages"));
        let src_outline = src.outlines().unwrap_or_default();
        let parts = plan_parts(&mut src, v, n, cx)?;
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        for (k, part) in parts.iter().enumerate() {
            cx.check_cancel()?;
            cx.message(format!("{} ({}/{})", part.label, k + 1, parts.len()));
            let mut dst = PdfDocument::new();
            pages::copy_pages(&mut dst, &mut src, &part.pages, None)?;
            let idx = &part.pages;
            let ol = pages::remap_outline(src_outline_clone(&src_outline), &|p| idx.iter().position(|q| *q == p));
            if !ol.is_empty() {
                dst.set_outlines(&ol)?;
            }
            engine_pdf::stamp_producer(&mut dst)?;
            let out = cx.output_named(&format!("{}-{}.pdf", input.stem(), part.label));
            cleanup.add(&out);
            engine_pdf::save_pdf(&dst, &out, &engine_pdf::Save::default())?;
            outs.push(Output::new(out));
            prog(cx, k + 1, parts.len(), 0.3, 1.0);
        }
        cleanup.keep();
        if let Some(first) = outs.first_mut() {
            first.note = Some(tr_args("pdf.split.note", &[("count", &parts.len().to_string())]));
        }
        Ok(outs)
    }
}

fn src_outline_clone(v: &[mupdf::Outline]) -> Vec<mupdf::Outline> {
    v.iter()
        .map(|o| mupdf::Outline { title: o.title.clone(), uri: o.uri.clone(), dest: o.dest, down: src_outline_clone(&o.down) })
        .collect()
}
