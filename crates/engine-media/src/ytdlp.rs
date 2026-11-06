//! yt-dlp (Unlicense): the official release binary, fetched on first use into
//! `downloads_dir()/bin/`, verified against the release's `SHA2-256SUMS`, updated with `-U`.
//! Used by `create.video-download` (wave 2). Network access is explicit and only happens here.

use std::path::{Path, PathBuf};

use castor_core::download::{Asset, fetch, sha256_file};
use castor_core::{RunCx, tr};

const REPO: &str = "https://github.com/yt-dlp/yt-dlp/releases";

/// Release file name for this OS.
pub fn release_file() -> &'static str {
    if cfg!(windows) {
        if cfg!(target_arch = "aarch64") { "yt-dlp_arm64.exe" } else { "yt-dlp.exe" }
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    }
}

/// Local path of the binary (present or not).
pub fn path() -> PathBuf {
    let name = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    castor_core::paths::downloads_dir().join("bin").join(name)
}

/// Description for the consent dialog (no SHA pin: the binary self-updates; the release
/// checksum file is verified instead).
pub fn asset() -> Asset {
    Asset {
        id: if cfg!(windows) { "bin/yt-dlp.exe" } else { "bin/yt-dlp" },
        url: if cfg!(windows) {
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
        } else {
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
        },
        sha256: "",
        size: if cfg!(windows) { 18_000_000 } else { 36_000_000 },
        license: "Unlicense",
    }
}

/// The yt-dlp binary, downloading and verifying it on first use.
pub fn ensure(cx: &RunCx) -> anyhow::Result<PathBuf> {
    let dest = path();
    if dest.is_file() {
        return Ok(dest);
    }
    cx.message(tr("media.msg.fetching_ytdlp"));
    let tmp = tempfile::tempdir()?;
    let cancel = cx.cancel_token();
    // Resolve the latest tag once so the binary and the checksum file match.
    let api = tmp.path().join("latest.json");
    fetch("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest", &api, "", 0, &cancel, &|_, _| {})?;
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&api)?)?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow::anyhow!("yt-dlp: no release tag"))?.to_string();
    anyhow::ensure!(tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'), "yt-dlp: unexpected tag");
    let sums_path = tmp.path().join("SHA2-256SUMS");
    fetch(&format!("{REPO}/download/{tag}/SHA2-256SUMS"), &sums_path, "", 0, &cancel, &|_, _| {})?;
    let sums = std::fs::read_to_string(&sums_path)?;
    let file = release_file();
    let expected = sums
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let h = it.next()?;
            let n = it.next()?.trim_start_matches('*');
            (n == file).then(|| h.to_ascii_lowercase())
        })
        .ok_or_else(|| anyhow::anyhow!("yt-dlp: {file} not listed in SHA2-256SUMS"))?;
    // Pass the hash to fetch: it verifies before the atomic rename.
    fetch(&format!("{REPO}/download/{tag}/{file}"), &dest, &expected, 0, &cancel, &|done, total| {
        if total > 0 {
            cx.progress(done as f32 / total as f32 * 0.9);
        }
    })?;
    debug_assert_eq!(sha256_file(&dest).unwrap_or_default(), expected);
    make_executable(&dest)?;
    Ok(dest)
}

#[cfg(unix)]
fn make_executable(p: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p)?.permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(p, perm)?;
    Ok(())
}
#[cfg(not(unix))]
fn make_executable(_: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// `yt-dlp -U` (self-update; the program verifies its own update).
pub fn update(cx: &RunCx) -> anyhow::Result<String> {
    let bin = ensure(cx)?;
    let mut cmd = castor_core::process::command(&bin);
    cmd.arg("-U");
    let f = castor_core::process::run(cmd, &cx.cancel_token(), |_| {}, |_| {})?;
    Ok(f.stdout.trim().to_string())
}

/// Installed version string ("2026.10.01").
pub fn version(cx: &RunCx) -> anyhow::Result<String> {
    let bin = ensure(cx)?;
    let mut cmd = castor_core::process::command(&bin);
    cmd.arg("--version");
    let f = castor_core::process::run(cmd, &cx.cancel_token(), |_| {}, |_| {})?;
    Ok(f.stdout.trim().to_string())
}

/// What to download.
#[derive(Clone, Debug)]
pub struct DownloadOptions {
    /// Audio only (best audio, converted to `audio_format`).
    pub audio_only: bool,
    /// "mp3", "m4a", "opus"… when `audio_only`.
    pub audio_format: String,
    /// Max video height (0 = best).
    pub max_height: u32,
    /// Download a whole playlist when the URL is one.
    pub playlist: bool,
    /// Also fetch subtitles (embedded).
    pub subtitles: bool,
    pub out_dir: PathBuf,
}

/// Download `url` with yt-dlp (ffmpeg from [`crate::ffmpeg`] for merging). Returns the files.
pub fn download(url: &str, opts: &DownloadOptions, cx: &RunCx) -> anyhow::Result<Vec<PathBuf>> {
    let bin = ensure(cx)?;
    let ff = crate::ffmpeg()?;
    std::fs::create_dir_all(&opts.out_dir)?;
    let mut cmd = castor_core::process::command(&bin);
    cmd.arg("--ffmpeg-location").arg(&ff);
    cmd.args(["--newline", "--no-colors", "--no-mtime", "--restrict-filenames", "--windows-filenames"]);
    cmd.args(["--progress-template", "download:castor-progress %(progress._percent_str)s"]);
    cmd.args(["--print", "after_move:filepath"]);
    cmd.arg("-P").arg(&opts.out_dir);
    cmd.args(["-o", "%(title).150B [%(id)s].%(ext)s"]);
    if !opts.playlist {
        cmd.arg("--no-playlist");
    }
    if opts.audio_only {
        cmd.args(["-x", "--audio-format", if opts.audio_format.is_empty() { "mp3" } else { &opts.audio_format }]);
    } else {
        let f = if opts.max_height > 0 {
            format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b", h = opts.max_height)
        } else {
            "bv*+ba/b".to_string()
        };
        cmd.args(["-f", &f, "--merge-output-format", "mp4/mkv"]);
    }
    if opts.subtitles {
        cmd.args(["--embed-subs", "--sub-langs", "all,-live_chat"]);
    }
    cmd.arg("--").arg(url);
    let mut files = Vec::new();
    let res = castor_core::process::run(
        cmd,
        &cx.cancel_token(),
        |line| {
            if let Some(p) = line.strip_prefix("castor-progress") {
                if let Ok(v) = p.trim().trim_end_matches('%').trim().parse::<f32>() {
                    cx.progress(v / 100.0);
                }
            } else {
                let p = PathBuf::from(line.trim());
                if p.is_absolute() {
                    files.push(p);
                }
            }
        },
        |_| {},
    );
    if let Err(e) = res {
        if e.is::<castor_core::job::Cancelled>() {
            return Err(e);
        }
        return Err(anyhow::Error::msg(format!("{e}")).context(tr("media.err.ytdlp")));
    }
    Ok(files.into_iter().filter(|p| p.is_file()).collect())
}
