//! Locating and running ffmpeg / ffprobe.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Instant;

use castor_core::job::Cancelled;
use castor_core::{RunCx, tr, tr_args, units};
use parking_lot::Mutex;

/// Located ffmpeg binary, or a clear (localised) error.
///
/// Search order: `CASTOR_FFMPEG` (dev override) → bundled resources → downloads → PATH.
pub fn ffmpeg() -> anyhow::Result<PathBuf> {
    locate("ffmpeg", "CASTOR_FFMPEG")
}

/// Located ffprobe binary (next to ffmpeg when possible).
pub fn ffprobe() -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os("CASTOR_FFPROBE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    // Prefer the ffprobe that sits next to the ffmpeg we use, so both come from the same build.
    if let Ok(ff) = ffmpeg() {
        let name = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
        if let Some(p) = ff.parent().map(|d| d.join(name)).filter(|p| p.is_file()) {
            return Ok(p);
        }
    }
    locate("ffprobe", "CASTOR_FFPROBE")
}

fn locate(name: &str, env: &str) -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os(env).map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    castor_core::paths::find_program(name)
        .ok_or_else(|| anyhow::anyhow!(tr_args("media.err.missing_program", &[("program", name)])))
}

/// User-facing error kinds recognised in ffmpeg's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    /// Not a media file, damaged or truncated.
    Unreadable,
    /// The output folder cannot be written.
    Permission,
    /// The disk is full.
    DiskFull,
    /// The encoder could not be opened (e.g. hardware encoder not usable).
    Encoder,
    /// Anything else.
    Other,
}

impl MediaError {
    pub fn classify(stderr: &str) -> MediaError {
        let s = stderr.to_ascii_lowercase();
        if s.contains("no space left") {
            MediaError::DiskFull
        } else if s.contains("permission denied") {
            MediaError::Permission
        } else if s.contains("invalid data found")
            || s.contains("moov atom not found")
            || s.contains("could not find codec parameters")
            || s.contains("does not contain any stream")
            || s.contains("end of file")
            || s.contains("no such file")
        {
            MediaError::Unreadable
        } else if s.contains("error while opening encoder")
            || s.contains("could not open encoder")
            || s.contains("cannot load")
            || s.contains("no capable devices")
            || s.contains("openencodesessionex failed")
            || s.contains("no device available")
        {
            MediaError::Encoder
        } else {
            MediaError::Other
        }
    }
    pub fn message(self) -> String {
        tr(match self {
            MediaError::Unreadable => "media.err.unreadable",
            MediaError::Permission => "media.err.permission",
            MediaError::DiskFull => "media.err.disk_full",
            MediaError::Encoder => "media.err.encoder",
            MediaError::Other => "media.err.ffmpeg",
        })
    }
}

/// Escape a value for use inside an ffmpeg filter graph option (`drawtext=text='…'`).
pub fn filter_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(c, '\\' | ':' | '\'' | ',' | '[' | ']' | ';' | '=' | '%') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Builder for one ffmpeg invocation with progress.
///
/// ```ignore
/// Ffmpeg::new().input(src).args(["-c:v", "libx264"]).output(dst).duration(d).run(cx)?;
/// ```
pub struct Ffmpeg {
    pre: Vec<OsString>,
    args: Vec<OsString>,
    duration: f64,
    range: (f32, f32),
    cwd: Option<PathBuf>,
    loglevel: &'static str,
    eta: bool,
}

impl Default for Ffmpeg {
    fn default() -> Self {
        Self::new()
    }
}

impl Ffmpeg {
    pub fn new() -> Self {
        Ffmpeg { pre: Vec::new(), args: Vec::new(), duration: 0.0, range: (0.0, 1.0), cwd: None, loglevel: "error", eta: true }
    }
    /// Append one argument.
    pub fn arg(mut self, a: impl AsRef<OsStr>) -> Self {
        self.args.push(a.as_ref().to_os_string());
        self
    }
    /// Append arguments.
    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(it.into_iter().map(|s| s.as_ref().to_os_string()));
        self
    }
    /// Append `-i path`.
    pub fn input(self, p: impl AsRef<OsStr>) -> Self {
        self.arg("-i").arg(p)
    }
    /// Expected output duration in seconds, for progress (0 = unknown).
    pub fn duration(mut self, d: f64) -> Self {
        self.duration = d;
        self
    }
    /// Map this run's progress to `lo..hi` of the job (multi-pass tools).
    pub fn range(mut self, lo: f32, hi: f32) -> Self {
        self.range = (lo, hi);
        self
    }
    /// Working directory: lets filter graphs refer to files by plain relative names
    /// (`subtitles=captions.ass`), avoiding Windows path escaping in filter syntax.
    pub fn cwd(mut self, d: impl Into<PathBuf>) -> Self {
        self.cwd = Some(d.into());
        self
    }
    /// Log filters' info output on stderr (silencedetect, cropdetect, loudnorm…).
    pub fn info_log(mut self) -> Self {
        self.loglevel = "info";
        self
    }
    /// Do not post ETA messages (short helper runs).
    pub fn quiet(mut self) -> Self {
        self.eta = false;
        self
    }
    /// Arguments placed before everything else (rarely needed: global options).
    pub fn global(mut self, a: impl AsRef<OsStr>) -> Self {
        self.pre.push(a.as_ref().to_os_string());
        self
    }

    /// Run and return the full stderr (only meaningful with [`Self::info_log`]).
    pub fn run(self, cx: &RunCx) -> anyhow::Result<String> {
        self.run_with(cx, |_| {})
    }

    /// Run, calling `on_stderr` for each stderr line. Returns all stderr lines joined.
    pub fn run_with(self, cx: &RunCx, mut on_stderr: impl FnMut(&str) + Send) -> anyhow::Result<String> {
        let ff = ffmpeg()?;
        let mut cmd = castor_core::process::command(&ff);
        cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", self.loglevel, "-progress", "pipe:1", "-nostats"]);
        cmd.args(&self.pre);
        cmd.args(&self.args);
        if let Some(d) = &self.cwd {
            cmd.current_dir(d);
        }
        tracing::debug!(target: "engine_media", "ffmpeg {:?}", self.args);
        let (lo, hi) = self.range;
        let duration = self.duration;
        let started = Instant::now();
        let last_msg = Mutex::new(Instant::now());
        let want_eta = self.eta;
        let all_err = Mutex::new(String::new());
        let res = castor_core::process::run(
            cmd,
            &cx.cancel_token(),
            |line| {
                let Some((k, v)) = line.split_once('=') else { return };
                if k == "out_time_us" || k == "out_time_ms" {
                    // Both are microseconds (the "_ms" name is a historical misnomer).
                    let Ok(us) = v.trim().parse::<i64>() else { return };
                    if duration > 0.0 && us >= 0 {
                        let f = ((us as f64 / 1e6) / duration).clamp(0.0, 1.0) as f32;
                        let overall = lo + (hi - lo) * f;
                        cx.progress(overall);
                        if want_eta {
                            let mut lm = last_msg.lock();
                            let el = started.elapsed().as_secs_f64();
                            if lm.elapsed().as_secs_f64() > 5.0 && f > 0.02 && el > 4.0 {
                                *lm = Instant::now();
                                let left = el / f as f64 * (1.0 - f as f64);
                                cx.message(tr_args(
                                    "media.msg.progress",
                                    &[("pct", &format!("{:.0}", overall * 100.0)), ("eta", &units::duration(left))],
                                ));
                            }
                        }
                    }
                } else if k == "progress" && v.trim() == "end" {
                    cx.progress(hi);
                }
            },
            |line| {
                on_stderr(line);
                let mut a = all_err.lock();
                if a.len() < 4_000_000 {
                    a.push_str(line);
                    a.push('\n');
                }
            },
        );
        match res {
            Ok(_) => Ok(all_err.into_inner()),
            Err(e) if e.is::<Cancelled>() => Err(e),
            Err(e) => {
                let details = format!("{e}");
                let kind = MediaError::classify(&details);
                Err(anyhow::Error::msg(details).context(kind.message()))
            }
        }
    }
}

/// Run ffmpeg with `args` (without the program name), progress mapped to `cx.progress()`
/// using the expected output `duration` in seconds.
pub fn run_ffmpeg(args: &[OsString], duration: f64, cx: &RunCx) -> anyhow::Result<()> {
    Ffmpeg::new().args(args).duration(duration).run(cx).map(|_| ())
}

/// Does the located ffmpeg have this filter (e.g. "vidstabdetect", "subtitles")? Cached.
pub fn has_filter(name: &str) -> bool {
    static LIST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| {
        ffmpeg()
            .ok()
            .and_then(|ff| run_simple(&ff, &[OsStr::new("-hide_banner"), OsStr::new("-filters")], &castor_core::CancelToken::new()).ok())
            .unwrap_or_default()
    });
    list.lines().any(|l| l.split_whitespace().nth(1) == Some(name))
}

/// Run a short helper command (ffprobe, test encodes) and return stdout; no progress.
pub(crate) fn run_simple(program: &Path, args: &[&OsStr], cancel: &castor_core::CancelToken) -> anyhow::Result<String> {
    let mut cmd = castor_core::process::command(program);
    cmd.args(args);
    let f = castor_core::process::run(cmd, cancel, |_| {}, |_| {})?;
    Ok(f.stdout)
}
