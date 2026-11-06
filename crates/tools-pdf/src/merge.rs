//! pdf.merge — N files → 1 PDF in queue order (PDFs and images), optional bookmark per file.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileInfo, FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::pages;

use crate::common::*;

pub struct Merge {
    meta: ToolMeta,
}

impl Merge {
    pub fn new() -> Self {
        Merge {
            meta: meta(
                "pdf.merge",
                Family::Pdf,
                InputSpec::kinds(&[FileKind::Pdf, FileKind::Image]).min(1),
                "PDF",
                &["combine", "join", "append", "concatenate", "bind"],
                "merged",
            ),
        }
    }
}

/// Open an input as a PDF document: PDFs directly, other formats converted by MuPDF.
pub fn open_as_pdf(f: &FileInfo, password: &str) -> Result<PdfDocument> {
    if f.kind == FileKind::Pdf {
        return engine_pdf::open_pdf(&f.path, password);
    }
    let doc = engine_pdf::open_document(&f.path, "")?;
    Ok(doc.convert_to_pdf(0, -1, 0)?)
}

impl Tool for Merge {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("bookmarks", vec![Choice::key("keep"), Choice::key("per_file"), Choice::key("none")], "per_file"),
            password_opt(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", tr("pdf.err.no_input"));
        Ok(vec![Job::many(inputs.to_vec())])
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        anyhow::ensure!(!job.inputs.is_empty(), "{}", tr("pdf.err.no_input"));
        let mode = v.str("bookmarks");
        let mut dst = PdfDocument::new();
        let mut outline = Vec::new();
        let n = job.inputs.len();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            cx.message(format!("{} ({}/{n})", f.name(), i + 1));
            let mut src = open_as_pdf(f, password(v)).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            let count = src.page_count()?.max(0) as usize;
            if count == 0 {
                continue;
            }
            let start = dst.page_count()?.max(0) as usize;
            let src_outline = if mode == "none" { Vec::new() } else { src.outlines().unwrap_or_default() };
            pages::copy_pages(&mut dst, &mut src, &(0..count).collect::<Vec<_>>(), None)?;
            let inner = pages::remap_outline(src_outline, &|p| (p < count).then_some(start + p));
            match mode {
                "per_file" => outline.push(pages::bookmark(&f.stem(), start, inner)),
                "keep" => outline.extend(inner),
                _ => {}
            }
            prog(cx, i + 1, n, 0.0, 0.9);
        }
        anyhow::ensure!(dst.page_count()? > 0, "{}", tr("pdf.err.no_pages"));
        if !outline.is_empty() {
            dst.set_outlines(&outline)?;
        }
        engine_pdf::stamp_producer(&mut dst)?;
        let first = &job.inputs[0];
        let out = if n == 1 { out_for(cx, first, "pdf") } else { cx.output_named(&format!("{}-{}.pdf", first.stem(), cx.suffix())) };
        engine_pdf::save_pdf(&dst, &out, &engine_pdf::Save::default())?;
        let pages_total = dst.page_count()?.to_string();
        Ok(vec![Output::new(out).with_note(tr_args("pdf.merge.note", &[("files", &n.to_string()), ("pages", &pages_total)]))])
    }
}
