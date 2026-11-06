//! The [`Tool`] contract.
//!
//! A tool is a value implementing [`Tool`]. It never touches the UI: the app reads
//! [`ToolMeta`] + [`Tool::options`] and generates the screen; the CLI and chains use the same.
//!
//! Display strings are NOT in the code. They come from the locale files, by key:
//! - `{id}.name`  — short name, e.g. "Compress PDF"
//! - `{id}.desc`  — one line, e.g. "Shrink the file while text stays sharp."
//! - `{id}.opt.{key}` / `{id}.opt.{key}.help` / `{id}.opt.{key}.{choice}` — see [`crate::options`].
//! - `{id}.done`  — optional witty one-liner shown when a run completes (may use %{count}, %{saved}).

use crate::files::{FileInfo, FileKind};
use crate::job::{Job, Output, RunCx};
use crate::options::{OptionSpec, Values};
use crate::Result;

/// Tool families: the tabs of the tools home, and the colour of the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Pdf,
    Image,
    Video,
    Audio,
    Create,
    Files,
}

impl Family {
    pub const ALL: [Family; 6] =
        [Family::Pdf, Family::Image, Family::Video, Family::Audio, Family::Create, Family::Files];

    pub fn key(self) -> &'static str {
        match self {
            Family::Pdf => "pdf",
            Family::Image => "image",
            Family::Video => "video",
            Family::Audio => "audio",
            Family::Create => "create",
            Family::Files => "files",
        }
    }

    /// Localised family name (`family.{key}` in locales/app.yml).
    pub fn name(self) -> String {
        crate::tr(&format!("family.{}", self.key()))
    }
}

/// Maturity of a tool. `Soon` tools are listed (greyed) but cannot run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolStatus {
    Ready,
    Beta,
    Soon,
}

/// Something a tool needs before it can run. The app checks these and offers to
/// download/locate what is missing (models, sidecar binaries) before the first run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Bundled ffmpeg + ffprobe (engine-media).
    Ffmpeg,
    /// PDFium library (engine-pdf).
    Pdfium,
    /// A downloadable ML model, by id from the engine-ml model catalog.
    Model(&'static str),
    /// whisper.cpp runtime (engine-media).
    Whisper,
    /// yt-dlp, fetched and kept up to date on first use (engine-media).
    YtDlp,
    /// A Chromium-based browser already installed (Chrome, Edge, Brave, Chromium).
    Chromium,
    /// LibreOffice already installed.
    LibreOffice,
    /// Network access at run time (e.g. video downloader). Shown to the user.
    Network,
}

/// Which input files a tool takes.
#[derive(Clone, Debug)]
pub struct InputSpec {
    /// Accepted kinds. Empty = decide by `extensions` only.
    pub kinds: &'static [FileKind],
    /// Extra accepted extensions, lowercase without dot (e.g. "epub"). Checked in addition to kinds.
    pub extensions: &'static [&'static str],
    /// Minimum number of files for one run (0 = tool can run with no input, e.g. QR code).
    pub min: usize,
    /// Maximum number of files (None = unlimited).
    pub max: Option<usize>,
    /// Accepts folders as input (e.g. Sort & route, Batch rename, Create archive).
    pub folders: bool,
}

impl InputSpec {
    pub const fn kinds(kinds: &'static [FileKind]) -> Self {
        InputSpec { kinds, extensions: &[], min: 1, max: None, folders: false }
    }
    pub const fn none() -> Self {
        InputSpec { kinds: &[], extensions: &[], min: 0, max: None, folders: false }
    }
    pub const fn any() -> Self {
        InputSpec { kinds: &[FileKind::Any], extensions: &[], min: 1, max: None, folders: true }
    }
    pub const fn with_extensions(mut self, ext: &'static [&'static str]) -> Self {
        self.extensions = ext;
        self
    }
    pub const fn min(mut self, n: usize) -> Self {
        self.min = n;
        self
    }
    pub const fn max(mut self, n: usize) -> Self {
        self.max = Some(n);
        self
    }
    pub const fn folders(mut self) -> Self {
        self.folders = true;
        self
    }

    pub fn accepts(&self, f: &FileInfo) -> bool {
        if f.is_dir {
            return self.folders;
        }
        if self.kinds.contains(&FileKind::Any) {
            return true;
        }
        if self.kinds.contains(&f.kind) {
            return true;
        }
        let ext = f.extension();
        !ext.is_empty() && self.extensions.iter().any(|e| *e == ext)
    }
}

/// Static description of a tool.
#[derive(Clone, Debug)]
pub struct ToolMeta {
    /// Stable id, `family.verb-noun`, e.g. "pdf.compress", "image.remove-background".
    /// Used as i18n key prefix, CLI name, chain step id, history id. Never rename.
    pub id: &'static str,
    pub family: Family,
    pub inputs: InputSpec,
    /// Short output description shown on the card, not translated: "PDF", "JPG · PNG · WEBP".
    pub output: &'static str,
    /// Extra English search keywords (the localised name/desc are searched too).
    pub keywords: &'static [&'static str],
    pub status: ToolStatus,
    pub requires: &'static [Requirement],
    /// Default suffix for output names: `report-compressed.pdf`.
    pub suffix: &'static str,
    /// The tool edits its inputs in place (rename, dates, sort) and supports undo.
    pub in_place: bool,
    /// Prefer running jobs one at a time (heavy ffmpeg / ML jobs). Default false = parallel.
    pub serial: bool,
}

impl ToolMeta {
    pub fn name(&self) -> String {
        crate::tr(&format!("{}.name", self.id))
    }
    pub fn desc(&self) -> String {
        crate::tr(&format!("{}.desc", self.id))
    }
}

/// A file tool. Implementations must be cheap to construct and stateless:
/// everything per-run lives in [`Values`] and [`RunCx`].
pub trait Tool: Send + Sync + 'static {
    fn meta(&self) -> &ToolMeta;

    /// Declarative options. Order = display order.
    fn options(&self) -> Vec<OptionSpec> {
        Vec::new()
    }

    /// Whether this tool takes this file. Default: [`InputSpec::accepts`].
    fn accepts(&self, file: &FileInfo) -> bool {
        self.meta().inputs.accepts(file)
    }

    /// Group inputs into jobs. Default: one job per input (1 → 1).
    /// Merge-like tools return a single job holding every input (N → 1);
    /// tools that need no input return one empty job.
    fn plan(&self, inputs: &[FileInfo], _values: &Values) -> Result<Vec<Job>> {
        if inputs.is_empty() {
            return Ok(vec![Job::empty()]);
        }
        Ok(inputs.iter().cloned().map(Job::single).collect())
    }

    /// Run one job. Must poll `cx.check_cancel()?` in long loops, report `cx.progress(..)`,
    /// write only through `cx.output_path(..)`/`cx.out_dir()` (or in place if `meta.in_place`),
    /// and return every file it produced.
    fn run(&self, job: &Job, values: &Values, cx: &RunCx) -> Result<Vec<Output>>;

    /// Undo an in-place run (rename, dates…). `journal` is what `run` stored in `Output::undo`.
    fn undo(&self, _journal: &serde_json::Value) -> Result<()> {
        anyhow::bail!("undo not supported")
    }

    /// Live preview for the options panel, rendered on a background thread and debounced by the
    /// app (≈150 ms after the last change). `input` = the selected file, if any. The image must
    /// fit in `max_side` pixels. Return `Ok(None)` when a preview makes no sense.
    /// Create tools (QR code, gradients…) should implement it: it is their main view.
    fn preview(&self, _input: Option<&FileInfo>, _values: &Values, _max_side: u32) -> Result<Option<Preview>> {
        Ok(None)
    }

    /// One-time downloads this run would need with these values (models, OCR languages,
    /// whisper models…). The app asks for consent, showing names and total size of those not
    /// yet present, before starting the run; `run` then calls `download::ensure` itself.
    fn downloads(&self, _values: &Values) -> Vec<crate::download::Asset> {
        Vec::new()
    }

    /// Why the tool cannot run on this machine right now (e.g. "LibreOffice is not installed"),
    /// or None. Shown on the card and the tool screen; the run button is disabled.
    fn unavailable(&self) -> Option<String> {
        None
    }
}

/// A rendered preview: PNG bytes (keeps the core free of image dependencies) and an optional
/// caption shown under it in the house style ("Figure 1 / 1080 × 1080, scans at 12 cm").
#[derive(Clone, Debug)]
pub struct Preview {
    pub png: Vec<u8>,
    pub caption: Option<String>,
}
