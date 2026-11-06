//! `files.identify` ("What is this file?"): the real type of files with no or wrong
//! extensions. Engines, in order: Magika (Google's content-type model, Apache-2.0, bundled,
//! run with tract — pure Rust, like engine-ml's fallback backend), the `file-format` signature
//! database, `infer`. Report as TXT / CSV / JSON; "fix extensions" renames mismatched files
//! (previewed, undoable). [`identify`] is also the fast helper the app can use for routing.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};
use serde::Serialize;

use crate::common::{self, Cell, MoveJournal};
use crate::magika_table as mt;

pub struct TypeInfo {
    pub label: &'static str,
    pub mime: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    pub extensions: &'static [&'static str],
    pub is_text: bool,
}

/// What a file really is.
#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    /// Short label ("pdf", "png", "python", "zip"…).
    pub label: String,
    pub description: String,
    pub mime: String,
    /// Magika group: document, image, video, audio, archive, code, text, executable…
    pub group: String,
    /// 0..1 (1 for signature matches and rules).
    pub confidence: f32,
    /// Usual extensions, best first (lowercase, no dot).
    pub extensions: Vec<String>,
    pub is_text: bool,
    /// Which engine decided: "magika", "file-format", "infer", "rule".
    pub engine: &'static str,
    /// Second opinion from the signature database, if it differs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl Identity {
    /// Does the file's current extension fit what it is?
    pub fn extension_fits(&self, path: &Path) -> bool {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext.is_empty() {
            return false;
        }
        if self.extensions.iter().any(|e| *e == ext) {
            return true;
        }
        // Text is text: a ".txt" holding Markdown or a ".log" holding text is fine.
        self.is_text && matches!(ext.as_str(), "txt" | "text" | "log" | "md" | "csv" | "tsv" | "conf" | "cfg" | "ini")
    }
}

const MODEL: &[u8] = include_bytes!("../assets/magika/model.onnx");
const BEG: usize = 1024;
const END: usize = 1024;
const BLOCK: usize = 4096;
const PAD: i32 = 256;
const MIN_DL: usize = 8;

type Plan = tract_onnx::prelude::TypedSimplePlan;

fn model() -> Result<&'static std::sync::Arc<Plan>> {
    static M: OnceLock<std::result::Result<std::sync::Arc<Plan>, String>> = OnceLock::new();
    M.get_or_init(|| {
        use tract_onnx::prelude::*;
        (|| -> TractResult<std::sync::Arc<Plan>> {
            let m = tract_onnx::onnx().model_for_read(&mut &*MODEL)?.with_input_fact(0, i32::fact([1, BEG + END]).into())?;
            m.into_optimized()?.into_runnable()
        })()
        .map_err(|e| format!("{e:#}"))
    })
    .as_ref()
    .map_err(|e| anyhow!("Magika model: {e}"))
}

fn is_ws(x: u8) -> bool {
    x.is_ascii_whitespace() || x == 0x0b
}

/// Magika features: first and last bytes with surrounding whitespace stripped, padded.
fn features(path: &Path) -> Result<Option<(Vec<i32>, Vec<u8>)>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let len = f.metadata()?.len();
    if len == 0 {
        return Ok(None);
    }
    let n = (BLOCK as u64).min(len) as usize;
    let mut beg = vec![0u8; n];
    f.read_exact(&mut beg)?;
    let mut end = vec![0u8; n];
    f.seek(SeekFrom::Start(len - n as u64))?;
    f.read_exact(&mut end)?;
    let b = {
        let s = beg.iter().position(|x| !is_ws(*x)).unwrap_or(beg.len());
        &beg[s..]
    };
    let e = {
        let s = end.iter().rposition(|x| !is_ws(*x)).map(|i| i + 1).unwrap_or(0);
        &end[..s]
    };
    let mut feat = vec![PAD; BEG + END];
    let k = b.len().min(BEG);
    for i in 0..k {
        feat[i] = b[i] as i32;
    }
    let k = e.len().min(END);
    for i in 0..k {
        feat[BEG + END - k + i] = e[e.len() - k + i] as i32;
    }
    Ok(Some((feat, beg)))
}

fn from_table(i: usize, confidence: f32, engine: &'static str) -> Identity {
    let t = &mt::TYPES[i];
    Identity {
        label: t.label.to_string(),
        description: t.description.to_string(),
        mime: t.mime.to_string(),
        group: t.group.to_string(),
        confidence,
        extensions: t.extensions.iter().map(|e| e.to_lowercase()).collect(),
        is_text: t.is_text,
        engine,
        signature: None,
    }
}

fn magika(path: &Path) -> Result<Identity> {
    let Some((feat, first)) = features(path)? else { return Ok(from_table(mt::EMPTY, 1.0, "rule")) };
    if feat[MIN_DL - 1] == PAD {
        // Too small for the model: text or unknown.
        let i = if std::str::from_utf8(&first).is_ok() { mt::TXT } else { mt::UNKNOWN };
        return Ok(from_table(i, 1.0, "rule"));
    }
    use tract_onnx::prelude::*;
    let plan = model()?;
    let t = Tensor::from_shape(&[1, BEG + END], &feat)?;
    let out = plan.run(tvec!(t.into_tvalue()))?;
    let view = out[0].try_as_plain_ram()?;
    let scores = view.as_slice::<f32>()?;
    let (best, score) = scores.iter().enumerate().fold((0, f32::MIN), |a, (i, s)| if *s > a.1 { (i, *s) } else { a });
    let ct = *mt::LABELS.get(best).ok_or_else(|| anyhow!("Magika: label {best} out of range"))? as usize;
    let ct = if score < mt::THRESHOLDS[ct] {
        if mt::TYPES[ct].is_text { mt::TXT } else { mt::UNKNOWN }
    } else {
        mt::OVERWRITE[ct] as usize
    };
    Ok(from_table(ct, score, "magika"))
}

/// Identify a file: Magika first, then signatures. Fast (one 4 KB read + a 3 MB model loaded
/// once per process); safe to call from the app to route extension-less files.
pub fn identify(path: &Path) -> Result<Identity> {
    let mut id = magika(path).unwrap_or_else(|_| from_table(mt::UNKNOWN, 0.0, "magika"));
    let ff = file_format::FileFormat::from_file(path).ok();
    if let Some(ff) = &ff {
        let name = ff.name().to_string();
        if *ff != file_format::FileFormat::ArbitraryBinaryData && *ff != file_format::FileFormat::PlainText {
            // Signatures are exact: prefer them when Magika is unsure or sees only "unknown".
            if id.label == "unknown" || id.confidence < 0.6 {
                id = Identity {
                    label: ff.extension().to_string(),
                    description: name.clone(),
                    mime: ff.media_type().to_string(),
                    group: format!("{:?}", ff.kind()).to_lowercase(),
                    confidence: 1.0,
                    extensions: vec![ff.extension().to_string()],
                    is_text: false,
                    engine: "file-format",
                    signature: None,
                };
            } else if !id.extensions.iter().any(|e| e == ff.extension()) {
                id.signature = Some(format!("{name} (.{})", ff.extension()));
            }
        }
    }
    if id.label == "unknown" {
        if let Ok(Some(t)) = infer::get_from_path(path) {
            id = Identity {
                label: t.extension().to_string(),
                description: t.mime_type().to_string(),
                mime: t.mime_type().to_string(),
                group: format!("{:?}", t.matcher_type()).to_lowercase(),
                confidence: 0.9,
                extensions: vec![t.extension().to_string()],
                is_text: false,
                engine: "infer",
                signature: None,
            };
        }
    }
    Ok(id)
}

pub struct Identify {
    meta: ToolMeta,
}

impl Identify {
    pub fn new() -> Self {
        Identify {
            meta: common::meta(
                "files.identify",
                InputSpec::kinds(common::ANY).folders(),
                "TXT · CSV · JSON",
                &["identify", "file type", "what is this file", "unknown", "extension", "magic", "trid", "mime", "detect"],
                "file-types",
                true,
            ),
        }
    }
}

impl Tool for Identify {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("report", vec![Choice::lit("txt", "TXT"), Choice::lit("csv", "CSV"), Choice::lit("json", "JSON"), Choice::key("none")], "txt"),
            OptionSpec::toggle("only_mismatches", false),
            OptionSpec::toggle("fix_extensions", false),
            OptionSpec::int("min_confidence", 50, 100, 90).slider().unit("%").advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        let files = common::expand_inputs(inputs, true);
        anyhow::ensure!(!files.is_empty(), "{}", tr("files.err.no_files"));
        Ok(vec![Job::many(files)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let files = if f.is_dir { common::expand_inputs(std::slice::from_ref(f), true) } else { vec![f.clone()] };
        let list: Vec<(FileInfo, Identity)> = files.iter().take(100).filter_map(|f| identify(&f.path).ok().map(|i| (f.clone(), i))).collect();
        let min = v.int("min_confidence") as f32 / 100.0;
        let rows: Vec<Vec<Cell>> = list
            .iter()
            .map(|(f, id)| {
                let fix = suggestion(&f.path, id, min);
                vec![
                    (f.name(), None),
                    (id.description.clone(), None),
                    (format!("{:.0} %", id.confidence * 100.0), None),
                    match &fix {
                        Some(e) => (format!(".{e}"), Some(common::BAD)),
                        None if id.extension_fits(&f.path) => ("✓".into(), Some(common::GOOD)),
                        None => ("?".into(), None),
                    },
                ]
            })
            .collect();
        let png = common::table_png(
            &tr_args("files.identify.caption", &[("count", &files.len().to_string())]),
            &[&tr("files.col.file"), &tr("files.identify.col.type"), &tr("files.identify.col.confidence"), &tr("files.identify.col.extension")],
            &rows,
            max_side,
            files.len().saturating_sub(100),
        )?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let min = v.int("min_confidence") as f32 / 100.0;
        let mut results: Vec<(PathBuf, Identity)> = Vec::new();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            if i % 32 == 0 {
                cx.message(f.name());
            }
            match identify(&f.path) {
                Ok(id) => results.push((f.path.clone(), id)),
                Err(e) => tracing_free_warn(&format!("{}: {e:#}", f.path.display())),
            }
            cx.progress((i + 1) as f32 / job.inputs.len() as f32 * 0.9);
        }
        let mut outs = Vec::new();
        let mismatched = results.iter().filter(|(p, id)| !id.extension_fits(p)).count();
        let fmt = v.str("report");
        if fmt != "none" {
            let report = cx.output_named(&format!("file-types.{fmt}"));
            let shown: Vec<&(PathBuf, Identity)> = results.iter().filter(|(p, id)| !v.bool("only_mismatches") || !id.extension_fits(p)).collect();
            std::fs::write(&report, report_text(&shown, fmt, min)?)?;
            outs.push(Output::new(report).with_note(tr_args(
                "files.identify.note",
                &[("count", &results.len().to_string()), ("mismatched", &mismatched.to_string())],
            )));
        }
        if v.bool("fix_extensions") {
            let moves: Vec<(PathBuf, PathBuf)> = results
                .iter()
                .filter_map(|(p, id)| {
                    let e = suggestion(p, id, min)?;
                    let stem = p.file_stem()?.to_string_lossy().into_owned();
                    // "photo" → "photo.jpg"; "photo.png" (really a JPEG) → "photo.jpg".
                    let name = if p.extension().is_some() { format!("{stem}.{e}") } else { format!("{}.{e}", p.file_name()?.to_string_lossy()) };
                    Some((p.clone(), castor_core::job::unique_path(&p.with_file_name(name))))
                })
                .collect();
            if !moves.is_empty() {
                let journal = common::apply_moves(&moves, cx)?;
                let undo = serde_json::to_value(&journal)?;
                let start = outs.len();
                outs.extend(journal.moves.iter().map(|(a, b)| Output::new(b).with_note(format!("← {}", a.file_name().unwrap_or_default().to_string_lossy()))));
                outs[start].undo = Some(undo);
            }
        }
        anyhow::ensure!(!outs.is_empty(), "{}", tr("files.identify.err.nothing"));
        Ok(outs)
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let j: MoveJournal = serde_json::from_value(journal.clone())?;
        common::undo_moves(&j)
    }
}

fn tracing_free_warn(_msg: &str) {}

/// The extension to give a file, if its current one is wrong and we are confident enough.
/// Text files keep their extension unless they have none (a ".txt" of Python stays ".txt").
pub fn suggestion(p: &Path, id: &Identity, min: f32) -> Option<String> {
    if id.extension_fits(p) || id.confidence < min || matches!(id.label.as_str(), "unknown" | "empty" | "txt" | "directory" | "symlink") {
        return None;
    }
    let has_ext = p.extension().is_some_and(|e| !e.is_empty());
    if id.is_text && has_ext {
        return None;
    }
    id.extensions.first().cloned().filter(|e| !e.is_empty())
}

fn report_text(rows: &[&(PathBuf, Identity)], fmt: &str, min: f32) -> Result<String> {
    Ok(match fmt {
        "json" => serde_json::to_string_pretty(
            &rows
                .iter()
                .map(|(p, id)| serde_json::json!({"path": p, "type": id, "extension_ok": id.extension_fits(p), "suggested_extension": suggestion(p, id, min)}))
                .collect::<Vec<_>>(),
        )?,
        "csv" => {
            let mut s = String::from("path,label,description,mime,group,confidence,engine,extension_ok,suggested_extension\n");
            for (p, id) in rows {
                let q = |x: &str| format!("\"{}\"", x.replace('"', "\"\""));
                s.push_str(&format!(
                    "{},{},{},{},{},{:.3},{},{},{}\n",
                    q(&p.to_string_lossy()),
                    q(&id.label),
                    q(&id.description),
                    q(&id.mime),
                    q(&id.group),
                    id.confidence,
                    id.engine,
                    id.extension_fits(p),
                    suggestion(p, id, min).unwrap_or_default()
                ));
            }
            s
        }
        _ => {
            let mut s = String::new();
            for (p, id) in rows {
                s.push_str(&format!("{}\n", p.display()));
                s.push_str(&format!("    {} — {} ({:.0} %, {})\n", id.description, id.mime, id.confidence * 100.0, id.engine));
                if let Some(sig) = &id.signature {
                    s.push_str(&format!("    signature: {sig}\n"));
                }
                if let Some(e) = suggestion(p, id, min) {
                    s.push_str(&format!("    → .{e}\n"));
                }
                s.push('\n');
            }
            s
        }
    })
}
