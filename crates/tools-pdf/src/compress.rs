//! pdf.compress — presets light / balanced / strong / max / custom. Never larger than the input.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::optimize::{optimize, Optimize, Recolor};

use crate::common::*;

pub struct Compress {
    meta: ToolMeta,
}

impl Compress {
    pub fn new() -> Self {
        Compress {
            meta: meta("pdf.compress", Family::Pdf, PDF_IN, "PDF", &["shrink", "smaller", "reduce", "size", "optimize", "email"], "compressed"),
        }
    }
}

/// Settings of a preset (or of the custom controls).
fn settings(v: &Values) -> Optimize {
    let mut o = Optimize { garbage: 4, object_streams: true, effort: 100, subset_fonts: v.bool("subset_fonts"), ..Default::default() };
    match v.str("preset") {
        "light" => {}
        "strong" => {
            o.image_dpi = 120;
            o.image_threshold_dpi = 160;
            o.jpeg_quality = 60;
            o.lossless_to_jpeg = true;
            o.rewrite_images = true;
            o.bitonal_dpi = 300;
        }
        "max" => {
            o.image_dpi = 96;
            o.image_threshold_dpi = 110;
            o.jpeg_quality = 45;
            o.lossless_to_jpeg = true;
            o.rewrite_images = true;
            o.bitonal_dpi = 200;
            o.recolor = Recolor::Rgb;
        }
        "custom" => {
            o.image_dpi = v.int("image_dpi").clamp(0, 1200) as u32;
            o.image_threshold_dpi = (o.image_dpi as f32 * 1.3) as u32;
            o.jpeg_quality = v.int("jpeg_quality").clamp(5, 100) as u8;
            o.lossless_to_jpeg = v.bool("lossless_to_jpeg");
            o.rewrite_images = true;
        }
        _ => {
            // balanced
            o.image_dpi = 150;
            o.image_threshold_dpi = 225;
            o.jpeg_quality = 75;
            o.rewrite_images = true;
        }
    }
    if v.bool("grayscale") {
        o.recolor = Recolor::Gray;
    }
    o
}

/// Remove optional parts of the document in place (safe API). Returns true if anything changed.
fn strip(doc: &mut mupdf::pdf::PdfDocument, v: &Values, max: bool) -> Result<bool> {
    let mut changed = false;
    let mut cat = doc.catalog()?;
    if v.bool("strip_metadata") {
        for k in ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate", "Trapped"] {
            engine_pdf::set_info(doc, k, "")?;
        }
        cat.dict_delete("Metadata")?;
        changed = true;
    }
    if v.bool("strip_bookmarks") {
        doc.delete_outlines()?;
        cat.dict_delete("PageMode")?;
        changed = true;
    }
    if v.bool("strip_tags") || max {
        cat.dict_delete("StructTreeRoot")?;
        cat.dict_delete("MarkInfo")?;
        cat.dict_delete("PieceInfo")?;
        changed = true;
    }
    if v.bool("strip_attachments") {
        if let Some(mut names) = cat.get_dict("Names")? {
            names.dict_delete("EmbeddedFiles")?;
        }
        cat.dict_delete("AF")?;
        changed = true;
    }
    // Page thumbnails are never worth their weight.
    let n = doc.page_count()?;
    for i in 0..n {
        let mut p = doc.find_page(i)?;
        if p.get_dict("Thumb")?.is_some() {
            p.dict_delete("Thumb")?;
            changed = true;
        }
        if max && p.get_dict("PieceInfo")?.is_some() {
            p.dict_delete("PieceInfo")?;
            changed = true;
        }
    }
    Ok(changed)
}

impl Tool for Compress {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "preset",
                vec![Choice::key("light"), Choice::key("balanced"), Choice::key("strong"), Choice::key("max"), Choice::key("custom")],
                "balanced",
            ),
            OptionSpec::int("image_dpi", 36, 600, 150).slider().unit("dpi").when(Cond::Eq("preset", "custom")),
            OptionSpec::int("jpeg_quality", 10, 95, 70).slider().when(Cond::Eq("preset", "custom")),
            OptionSpec::toggle("lossless_to_jpeg", false).when(Cond::Eq("preset", "custom")),
            OptionSpec::toggle("grayscale", false),
            OptionSpec::toggle("strip_metadata", false).advanced(),
            OptionSpec::toggle("strip_bookmarks", false).advanced(),
            OptionSpec::toggle("strip_tags", false).advanced(),
            OptionSpec::toggle("strip_attachments", false).advanced(),
            OptionSpec::toggle("subset_fonts", true).advanced(),
            password_opt(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let before = file_size(&input.path);
        let out = out_for(cx, input, "pdf");
        let pw = password(v);
        cx.message(tr("pdf.compress.msg.analyse"));
        // Pass 1 (only if needed): strip optional parts.
        let mut doc = engine_pdf::open_pdf(&input.path, pw)?;
        let source = {
            let changed = strip(&mut doc, v, v.str("preset") == "max")?;
            if changed {
                let tmp = cx.tmp_dir().join("stripped.pdf");
                let save = engine_pdf::Save { garbage: 1, keep_encryption: !pw.is_empty(), ..Default::default() };
                engine_pdf::save_pdf(&doc, &tmp, &save)?;
                tmp
            } else {
                input.path.clone()
            }
        };
        drop(doc);
        cx.check_cancel()?;
        cx.progress(0.15);
        cx.message(tr("pdf.compress.msg.images"));
        // Pass 2: images, fonts, structure.
        let candidate = cx.tmp_dir().join("candidate.pdf");
        // Some images (damaged JBIG2, exotic filters) cannot be decoded for recompression: retry
        // without touching bitonal images, then without touching images at all.
        let mut o = settings(v);
        if let Err(e) = optimize(&source, &candidate, pw, &o) {
            cx.check_cancel()?;
            o.bitonal_dpi = 0;
            if optimize(&source, &candidate, pw, &o).is_err() {
                o.rewrite_images = false;
                o.image_dpi = 0;
                optimize(&source, &candidate, pw, &o).map_err(|_| e)?;
            }
        }
        cx.check_cancel()?;
        cx.progress(0.95);
        let after = file_size(&candidate);
        if after == 0 || after >= before {
            std::fs::copy(&input.path, &out)?;
            return Ok(vec![Output::new(out).with_note(tr("pdf.compress.note.not_smaller"))]);
        }
        std::fs::copy(&candidate, &out)?;
        Ok(vec![Output::new(out).with_note(size_change(before, after))])
    }
}
