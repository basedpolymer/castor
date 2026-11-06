//! Whole-document tools: encrypt, unlock, flatten, repair, metadata, grayscale.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::Permission;
use engine_pdf::optimize::{optimize, Optimize, Recolor};

use crate::common::*;

pub struct Encrypt {
    meta: ToolMeta,
}

impl Encrypt {
    pub fn new() -> Self {
        Encrypt { meta: meta("pdf.encrypt", Family::Pdf, PDF_IN, "PDF", &["password", "protect", "lock", "secure", "aes"], "protected") }
    }
}

fn random_password() -> String {
    // Not secret-grade randomness is needed for an owner password nobody will type, but avoid
    // predictability anyway: hash time, address entropy and the process id.
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        h.write_usize(i);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

impl Tool for Encrypt {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::password("user_password"),
            OptionSpec::password("owner_password").advanced(),
            OptionSpec::toggle("allow_print", true),
            OptionSpec::toggle("allow_copy", true),
            OptionSpec::toggle("allow_edit", false),
            OptionSpec::toggle("allow_annotate", true),
            OptionSpec::toggle("allow_forms", true).advanced(),
            OptionSpec::toggle("allow_assemble", false).advanced(),
            OptionSpec::password("password").common().advanced(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let user = v.str("user_password").to_string();
        let mut owner = v.str("owner_password").to_string();
        let restricted = !(v.bool("allow_print")
            && v.bool("allow_copy")
            && v.bool("allow_edit")
            && v.bool("allow_annotate")
            && v.bool("allow_forms")
            && v.bool("allow_assemble"));
        anyhow::ensure!(!user.is_empty() || !owner.is_empty() || restricted, "{}", tr("pdf.encrypt.err.no_password"));
        if owner.is_empty() {
            owner = if restricted { random_password() } else { user.clone() };
        }
        let mut perm = Permission::ACCESSIBILITY;
        if v.bool("allow_print") {
            perm |= Permission::PRINT | Permission::PRINT_HQ;
        }
        if v.bool("allow_copy") {
            perm |= Permission::COPY;
        }
        if v.bool("allow_edit") {
            perm |= Permission::MODIFY;
        }
        if v.bool("allow_annotate") {
            perm |= Permission::ANNOTATE;
        }
        if v.bool("allow_forms") {
            perm |= Permission::FORM;
        }
        if v.bool("allow_assemble") {
            perm |= Permission::ASSEMBLE;
        }
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        engine_pdf::stamp_producer(&mut doc)?;
        cx.progress(0.3);
        let out = out_for(cx, input, "pdf");
        let save = engine_pdf::Save {
            encrypt: Some(engine_pdf::Encrypt { user_password: user.clone(), owner_password: owner, permissions: perm.bits() }),
            ..Default::default()
        };
        engine_pdf::save_pdf(&doc, &out, &save)?;
        Ok(vec![Output::new(out).with_note(tr(if user.is_empty() { "pdf.encrypt.note.restrict" } else { "pdf.encrypt.note.aes" }))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Unlock {
    meta: ToolMeta,
}

impl Unlock {
    pub fn new() -> Self {
        Unlock { meta: meta("pdf.unlock", Family::Pdf, PDF_IN, "PDF", &["decrypt", "remove password", "unprotect", "open"], "unlocked") }
    }
}

impl Tool for Unlock {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::password("password").common()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let was_encrypted = doc.trailer()?.get_dict("Encrypt")?.is_some();
        anyhow::ensure!(was_encrypted, "{}", tr("pdf.unlock.err.not_encrypted"));
        engine_pdf::stamp_producer(&mut doc)?;
        cx.progress(0.4);
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save::default())?;
        Ok(vec![Output::new(out)])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Flatten {
    meta: ToolMeta,
}

impl Flatten {
    pub fn new() -> Self {
        Flatten { meta: meta("pdf.flatten", Family::Pdf, PDF_IN, "PDF", &["bake", "forms", "annotations", "fields", "lock in"], "flattened") }
    }
}

impl Tool for Flatten {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::choice("what", vec![Choice::key("all"), Choice::key("forms"), Choice::key("annotations")], "all"), password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let what = v.str("what");
        let (annots, widgets) = (what != "forms", what != "annotations");
        // Make sure every widget has an appearance before baking it.
        if widgets {
            let n = doc.page_count()?;
            for i in 0..n {
                cx.check_cancel()?;
                let page = doc.load_pdf_page(i)?;
                for mut w in page.widgets() {
                    let _ = w.update();
                }
            }
        }
        cx.progress(0.3);
        doc.bake(annots, widgets)?;
        if widgets {
            doc.catalog()?.dict_delete("AcroForm")?;
        }
        engine_pdf::stamp_producer(&mut doc)?;
        cx.progress(0.7);
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        Ok(vec![Output::new(out)])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Repair {
    meta: ToolMeta,
}

impl Repair {
    pub fn new() -> Self {
        Repair { meta: meta("pdf.repair", Family::Pdf, PDF_IN, "PDF", &["fix", "broken", "corrupt", "damaged", "recover", "xref"], "repaired") }
    }
}

impl Tool for Repair {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        cx.message(tr("pdf.repair.msg"));
        let out = out_for(cx, input, "pdf");
        // MuPDF rebuilds the cross-reference table on load when it is broken; saving with
        // garbage collection and clean content streams writes a fresh, consistent file.
        let o = Optimize { garbage: 4, clean: true, object_streams: false, ..Default::default() };
        optimize(&input.path, &out, password(v), &o)?;
        cx.progress(0.8);
        let n = engine_pdf::page_count(&out).unwrap_or(0);
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &n.to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Metadata {
    meta: ToolMeta,
}

impl Metadata {
    pub fn new() -> Self {
        Metadata {
            meta: meta("pdf.metadata", Family::Pdf, PDF_IN, "PDF", &["title", "author", "properties", "info", "xmp", "privacy", "clean"], "meta"),
        }
    }
}

const FIELDS: &[(&str, &str)] = &[("title", "Title"), ("author", "Author"), ("subject", "Subject"), ("keywords", "Keywords"), ("creator", "Creator")];

impl Tool for Metadata {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut v = vec![OptionSpec::toggle("strip", false)];
        for (k, _) in FIELDS {
            v.push(OptionSpec::text(k, ""));
        }
        v.push(password_opt());
        v
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let before: Vec<String> = FIELDS
            .iter()
            .filter_map(|(_, k)| {
                let val = engine_pdf::get_info(&doc, k);
                (!val.is_empty()).then(|| format!("{k}: {val}"))
            })
            .collect();
        if v.bool("strip") {
            let mut t = doc.trailer()?;
            t.dict_delete("Info")?;
            let mut cat = doc.catalog()?;
            cat.dict_delete("Metadata")?;
            cat.dict_delete("PieceInfo")?;
            let n = doc.page_count()?;
            for i in 0..n {
                let mut p = doc.find_page(i)?;
                p.dict_delete("PieceInfo")?;
                p.dict_delete("Metadata")?;
            }
        }
        let mut changed = 0;
        for (opt, key) in FIELDS {
            let val = v.str(opt).trim();
            if !val.is_empty() {
                engine_pdf::set_info(&mut doc, key, val)?;
                changed += 1;
            }
        }
        if !v.bool("strip") {
            engine_pdf::set_info(&mut doc, "ModDate", &engine_pdf::pdf_date_now())?;
            if changed > 0 {
                // An XMP packet would now contradict the Info dictionary: drop it.
                doc.catalog()?.dict_delete("Metadata")?;
            }
        }
        cx.progress(0.5);
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        let note = if before.is_empty() { tr("pdf.metadata.note.none") } else { before.join(" · ") };
        Ok(vec![Output::new(out).with_note(note)])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Grayscale {
    meta: ToolMeta,
}

impl Grayscale {
    pub fn new() -> Self {
        Grayscale {
            meta: meta("pdf.grayscale", Family::Pdf, PDF_IN, "PDF", &["black and white", "gray", "grey", "monochrome", "print", "ink"], "gray"),
        }
    }
}

impl Tool for Grayscale {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        cx.message(tr("pdf.grayscale.msg"));
        let out = out_for(cx, input, "pdf");
        let o = Optimize { recolor: Recolor::Gray, garbage: 3, ..Default::default() };
        optimize(&input.path, &out, password(v), &o)?;
        let note = size_change(input.size, file_size(&out));
        Ok(vec![Output::new(out).with_note(note)])
    }
}
