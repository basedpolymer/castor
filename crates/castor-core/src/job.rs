//! Jobs, outputs and the run context handed to [`crate::Tool::run`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::files::FileInfo;

/// One unit of work: usually one input file, sometimes many (merge), sometimes none (QR code).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Job {
    pub inputs: Vec<FileInfo>,
}

impl Job {
    pub fn single(f: FileInfo) -> Job {
        Job { inputs: vec![f] }
    }
    pub fn many(fs: Vec<FileInfo>) -> Job {
        Job { inputs: fs }
    }
    pub fn empty() -> Job {
        Job { inputs: Vec::new() }
    }
    /// The first input. Panics never: returns an error if the job has none.
    pub fn input(&self) -> crate::Result<&FileInfo> {
        self.inputs.first().ok_or_else(|| anyhow::anyhow!("this tool needs an input file"))
    }
    /// Short label for progress lists: file name, "3 files", or "".
    pub fn label(&self) -> String {
        match self.inputs.len() {
            0 => String::new(),
            1 => self.inputs[0].name(),
            n => crate::tr_args("common.n_files", &[("count", &n.to_string())]),
        }
    }
}

/// A file produced by a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub path: PathBuf,
    /// Short human note shown next to the result ("OCR: 12 pages, fr+en").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// For in-place tools: what `Tool::undo` needs to revert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<serde_json::Value>,
}

impl Output {
    pub fn new(path: impl Into<PathBuf>) -> Output {
        Output { path: path.into(), note: None, undo: None }
    }
    pub fn with_note(mut self, note: impl Into<String>) -> Output {
        self.note = Some(note.into());
        self
    }
    pub fn with_undo(mut self, undo: serde_json::Value) -> Output {
        self.undo = Some(undo);
        self
    }
}

/// Error returned when a run was cancelled. Check with `err.is::<Cancelled>()`.
#[derive(Debug, Clone, Copy)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

/// Shared cancellation flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

type ProgressFn = Arc<dyn Fn(f32) + Send + Sync>;
type MessageFn = Arc<dyn Fn(String) + Send + Sync>;

/// Everything a running job may use. Cheap to clone.
#[derive(Clone)]
pub struct RunCx {
    out_dir: PathBuf,
    tmp_dir: PathBuf,
    suffix: String,
    cancel: CancelToken,
    progress: ProgressFn,
    message: MessageFn,
}

impl RunCx {
    pub fn new(out_dir: PathBuf, tmp_dir: PathBuf, suffix: impl Into<String>, cancel: CancelToken) -> Self {
        RunCx {
            out_dir,
            tmp_dir,
            suffix: suffix.into(),
            cancel,
            progress: Arc::new(|_| {}),
            message: Arc::new(|_| {}),
        }
    }
    pub fn on_progress(mut self, f: impl Fn(f32) + Send + Sync + 'static) -> Self {
        self.progress = Arc::new(f);
        self
    }
    pub fn on_message(mut self, f: impl Fn(String) + Send + Sync + 'static) -> Self {
        self.message = Arc::new(f);
        self
    }

    /// Report progress of the current job, 0.0..=1.0.
    pub fn progress(&self, frac: f32) {
        (self.progress)(frac.clamp(0.0, 1.0))
    }
    /// Report a short status line ("Page 3 of 12", "Downloading model…").
    pub fn message(&self, msg: impl Into<String>) {
        (self.message)(msg.into())
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    /// `cx.check_cancel()?` in every long loop.
    pub fn check_cancel(&self) -> crate::Result<()> {
        if self.cancel.is_cancelled() { Err(Cancelled.into()) } else { Ok(()) }
    }
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }
    /// Folder where outputs go (created by the runner).
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }
    /// Scratch folder for this job, deleted afterwards.
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    /// `{out_dir}/{stem}-{suffix}.{ext}`, made unique (" (2)", " (3)"…). Never overwrites.
    /// With no input, uses `fallback_stem`-like naming from the suffix ("qr-code.png").
    pub fn output_path(&self, input: Option<&FileInfo>, ext: &str) -> PathBuf {
        let stem = match input {
            Some(f) if self.suffix.is_empty() => f.stem(),
            Some(f) => format!("{}-{}", f.stem(), self.suffix),
            None => if self.suffix.is_empty() { "castor".to_string() } else { self.suffix.clone() },
        };
        self.output_named(&format!("{stem}.{ext}"))
    }

    /// `{out_dir}/{file_name}`, made unique. Never overwrites.
    pub fn output_named(&self, file_name: &str) -> PathBuf {
        unique_path(&self.out_dir.join(sanitize_file_name(file_name)))
    }
}

/// Make a path unique by appending " (2)", " (3)"… before the extension.
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let dir = p.parent().unwrap_or(Path::new("."));
    for i in 2.. {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

/// Replace characters that are invalid in file names on Windows or macOS.
pub fn sanitize_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "untitled".into() } else { s }
}
