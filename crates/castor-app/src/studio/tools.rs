//! The studios' saves and exports run through the runner like any tool (progress, cancel,
//! history, undo), as two app-side tools that the registry does not list: `studio.pdf` and
//! `studio.media`. Their only option is the edit script (JSON) the studio built.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use castor_core::job::Output;
use castor_core::options::{OptionSpec, Value, Values};
use castor_core::{Family, FileInfo, FileKind, InputSpec, Job, Registry, Result, RunCx, Tool, ToolMeta, ToolStatus, tr, tr_args, units};

use super::media_engine::{self, MediaEdit};
use super::pdf_engine::{self, PdfEdit};

pub const PDF_ID: &str = "studio.pdf";
pub const MEDIA_ID: &str = "studio.media";

fn meta(id: &'static str, family: Family, kinds: &'static [FileKind], output: &'static str, suffix: &'static str) -> ToolMeta {
    ToolMeta {
        id,
        family,
        inputs: InputSpec::kinds(kinds),
        output,
        keywords: &[],
        status: ToolStatus::Ready,
        requires: &[],
        suffix,
        in_place: false,
        serial: true,
    }
}

/// Values for a studio run: the script, and whether to replace the original.
pub fn values(script: String, overwrite: bool) -> Values {
    let mut v = Values::default();
    v.set("script", Value::Text(script));
    v.set("overwrite", Value::Bool(overwrite));
    v
}

/// Where originals replaced by a studio are kept for undo.
fn backup_dir() -> PathBuf {
    castor_core::paths::data_dir().join("undo").join("studios")
}

/// Replace `target` by `fresh`, keeping a copy of the original for undo.
fn replace_with_backup(fresh: &Path, target: &Path) -> Result<Output> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = castor_core::history::new_run_id();
    let name = target.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let backup = dir.join(format!("{stamp}-{name}"));
    std::fs::copy(target, &backup)?;
    engine_pdf::persist(fresh, target)?;
    Ok(Output::new(target).with_undo(serde_json::json!({ "backup": backup, "target": target })))
}

fn undo_replace(journal: &serde_json::Value) -> Result<()> {
    let backup = journal.get("backup").and_then(|v| v.as_str()).map(PathBuf::from);
    let target = journal.get("target").and_then(|v| v.as_str()).map(PathBuf::from);
    let (Some(b), Some(t)) = (backup, target) else { anyhow::bail!("{}", tr("app.studio.err.undo")) };
    anyhow::ensure!(b.exists(), "{}", tr("app.studio.err.undo"));
    std::fs::copy(&b, &t)?;
    let _ = std::fs::remove_file(&b);
    Ok(())
}

// ---- PDF --------------------------------------------------------------------------------

pub struct PdfStudioTool {
    meta: ToolMeta,
}

impl PdfStudioTool {
    pub fn new() -> Self {
        Self::with_suffix("edited")
    }
    /// Output names end with `-{suffix}` ("-edited", or "-pages" for an extract).
    pub fn with_suffix(suffix: &'static str) -> Self {
        PdfStudioTool { meta: meta(PDF_ID, Family::Pdf, &[FileKind::Pdf], "PDF", suffix) }
    }
}

impl Tool for PdfStudioTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::text("script", ""), OptionSpec::toggle("overwrite", false)]
    }
    fn plan(&self, inputs: &[FileInfo], _values: &Values) -> Result<Vec<Job>> {
        Ok(inputs.first().cloned().map(Job::single).into_iter().collect())
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let edit: PdfEdit = serde_json::from_str(v.str("script"))?;
        let (out, done) = if v.bool("overwrite") {
            let tmp = cx.tmp_dir().join("edited.pdf");
            let done = pdf_engine::apply(&edit, &tmp, cx)?;
            (replace_with_backup(&tmp, &input.path)?, done)
        } else {
            let out = cx.output_path(Some(input), "pdf");
            let done = pdf_engine::apply(&edit, &out, cx)?;
            (Output::new(out), done)
        };
        let mut parts = vec![tr_args("app.pdf.note.pages", &[("count", &done.pages.to_string())])];
        for (n, key) in [(done.annotations, "app.pdf.note.annotations"), (done.signatures, "app.pdf.note.signatures"), (done.redactions, "app.pdf.note.redactions"), (done.fields, "app.pdf.note.fields")] {
            if n > 0 {
                parts.push(tr_args(key, &[("count", &n.to_string())]));
            }
        }
        Ok(vec![out.with_note(parts.join(" · "))])
    }
    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        undo_replace(journal)
    }
}

// ---- video / audio ----------------------------------------------------------------------

pub struct MediaStudioTool {
    meta: ToolMeta,
    registry: Arc<Registry>,
}

impl MediaStudioTool {
    pub fn new(registry: Arc<Registry>) -> Self {
        MediaStudioTool { meta: meta(MEDIA_ID, Family::Video, &[FileKind::Video, FileKind::Audio], "MP4 · MP3 · SRT", "edited"), registry }
    }
}

impl Tool for MediaStudioTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::text("script", "")]
    }
    fn plan(&self, inputs: &[FileInfo], _values: &Values) -> Result<Vec<Job>> {
        Ok(inputs.first().cloned().map(Job::single).into_iter().collect())
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let edit: MediaEdit = serde_json::from_str(v.str("script"))?;
        let outs = media_engine::export(&edit, input, &self.registry, cx)?;
        let kept: f64 = edit.kept();
        let note = if edit.has_cuts() {
            tr_args("app.media.note.cut", &[("before", &units::duration(edit.duration)), ("after", &units::duration(kept))])
        } else {
            String::new()
        };
        Ok(outs.into_iter().map(|o| if note.is_empty() || o.note.is_some() { o } else { o.with_note(note.clone()) }).collect())
    }
}

/// The studio tool with this id (history: undo, names).
pub fn by_id(id: &str, registry: Arc<Registry>) -> Option<Arc<dyn Tool>> {
    match id {
        PDF_ID => Some(Arc::new(PdfStudioTool::new())),
        MEDIA_ID => Some(Arc::new(MediaStudioTool::new(registry))),
        _ => None,
    }
}
