//! Downloads for models and helper programs: resumable, size + SHA-256 checked, cancellable.
//!
//! Catalog entries live in the engine crates; this module only moves bytes.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::job::{Cancelled, CancelToken};

/// Something that can be downloaded once and reused.
#[derive(Clone, Debug)]
pub struct Asset {
    /// Stable id, also the file name under `downloads_dir()` (e.g. "models/birefnet-lite.onnx").
    pub id: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256, or "" to skip the check (only for self-updating tools like yt-dlp).
    pub sha256: &'static str,
    /// Expected size in bytes (shown to the user before downloading). 0 = unknown.
    pub size: u64,
    /// SPDX licence of the asset, shown in Settings → Models.
    pub license: &'static str,
}

impl Asset {
    pub fn path(&self) -> PathBuf {
        crate::paths::downloads_dir().join(self.id)
    }
    pub fn is_present(&self) -> bool {
        self.path().is_file()
    }
}

/// Download `asset` if missing. `progress(done, total)` is called as bytes arrive.
pub fn ensure(asset: &Asset, cancel: &CancelToken, progress: &dyn Fn(u64, u64)) -> crate::Result<PathBuf> {
    let dest = asset.path();
    if dest.is_file() {
        return Ok(dest);
    }
    fetch(asset.url, &dest, asset.sha256, asset.size, cancel, progress)?;
    Ok(dest)
}

/// Download `url` to `dest` (atomic: `.part` then rename). Resumes a previous `.part`.
pub fn fetch(
    url: &str,
    dest: &Path,
    sha256: &str,
    expected: u64,
    cancel: &CancelToken,
    progress: &dyn Fn(u64, u64),
) -> crate::Result<()> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    let part = dest.with_extension(format!(
        "{}.part",
        dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(concat!("Castor/", env!("CARGO_PKG_VERSION"), " (+https://basedpolymer.eu/castor)"))
        .build()
        .into();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call()?;
    let status = resp.status().as_u16();
    if have > 0 && status != 206 {
        have = 0; // server ignored the range: start over
    }
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| n + have)
        .unwrap_or(expected);
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part)?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = have;
    let mut last = std::time::Instant::now();
    loop {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        if last.elapsed().as_millis() > 100 {
            progress(done, len);
            last = std::time::Instant::now();
        }
    }
    f.flush()?;
    drop(f);
    progress(done, len);

    if !sha256.is_empty() {
        let got = sha256_file(&part)?;
        if !got.eq_ignore_ascii_case(sha256) {
            let _ = std::fs::remove_file(&part);
            anyhow::bail!("download of {url} is corrupted (checksum mismatch)");
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

pub fn sha256_file(p: &Path) -> crate::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
