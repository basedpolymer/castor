//! Thumbnails, single frames and waveform peaks.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

use castor_core::job::Cancelled;
use castor_core::{CancelToken, tr};

use crate::ff::{ffmpeg, run_simple};
use crate::probe::{StreamKind, probe};

/// Scale filter that keeps the aspect ratio and fits the longest side in `max_side`
/// (never upscales).
pub(crate) fn fit_filter(max_side: u32) -> String {
    format!("scale=w='if(gte(iw,ih),min(iw,{m}),-2)':h='if(gte(iw,ih),-2,min(ih,{m}))'", m = max_side.max(2))
}

/// PNG of the frame at `at` seconds, longest side ≤ `max_side`. `extra` is a filter chain
/// applied after scaling (e.g. a GIF palette preview), with `[in]`/`[out]` implied.
pub fn frame_png(path: &Path, at: f64, max_side: u32, extra: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("frame.png");
    let mut vf = fit_filter(max_side);
    if let Some(e) = extra.filter(|e| !e.is_empty()) {
        vf = format!("{vf},{e}");
    }
    let args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{:.3}", at.max(0.0)).into(),
        "-i".into(),
        path.as_os_str().into(),
        "-map".into(),
        "0:V:0?".into(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        vf.into(),
        "-update".into(),
        "1".into(),
        "-c:v".into(),
        "png".into(),
        out.as_os_str().into(),
    ];
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
    run_simple(&ffmpeg()?, &refs, &CancelToken::new())
        .map_err(|e| anyhow::Error::msg(format!("{e}")).context(tr("media.err.unreadable")))?;
    let bytes = std::fs::read(&out).map_err(|_| anyhow::anyhow!(tr("media.err.no_frame")))?;
    Ok(bytes)
}

/// PNG thumbnail at `at` seconds (default 10 % in), longest side ≤ `max_side`.
/// Audio files: their cover art if any, else a drawn waveform.
pub fn thumbnail(path: &Path, at: Option<f64>, max_side: u32) -> anyhow::Result<Vec<u8>> {
    let p = probe(path)?;
    if p.has_video {
        let mut t = at.unwrap_or(p.duration * 0.1);
        if p.duration > 0.0 {
            t = t.min((p.duration - 0.05).max(0.0));
        }
        // Some files cannot seek precisely near the end: fall back to the first frame.
        return frame_png(path, t, max_side, None).or_else(|_| frame_png(path, 0.0, max_side, None));
    }
    if let Some(cover) = p.streams.iter().find(|s| s.kind == StreamKind::Picture) {
        let dir = tempfile::tempdir()?;
        let out = dir.path().join("cover.png");
        let map = format!("0:{}", cover.index);
        let vf = fit_filter(max_side);
        let args: Vec<OsString> = vec![
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-i".into(),
            path.as_os_str().into(),
            "-map".into(),
            map.into(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            vf.into(),
            "-update".into(),
            "1".into(),
            out.as_os_str().into(),
        ];
        let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
        if run_simple(&ffmpeg()?, &refs, &CancelToken::new()).is_ok() {
            if let Ok(b) = std::fs::read(&out) {
                return Ok(b);
            }
        }
    }
    // Waveform picture, 2:1.
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("wave.png");
    let (w, h) = (max_side.max(16), (max_side / 2).max(8));
    let fc = format!("[0:a:0]aformat=channel_layouts=mono,showwavespic=s={w}x{h}:colors=0x3d6fb4:scale=sqrt[v]");
    let args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        path.as_os_str().into(),
        "-filter_complex".into(),
        fc.into(),
        "-map".into(),
        "[v]".into(),
        "-frames:v".into(),
        "1".into(),
        "-update".into(),
        "1".into(),
        out.as_os_str().into(),
    ];
    let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
    run_simple(&ffmpeg()?, &refs, &CancelToken::new())
        .map_err(|e| anyhow::Error::msg(format!("{e}")).context(tr("media.err.unreadable")))?;
    Ok(std::fs::read(&out)?)
}

/// Waveform peaks: `n` buckets of (min, max) in -1..1 over the whole file (first audio track,
/// mixed to mono). Streams the decoded audio; memory use is O(n).
pub fn waveform(path: &Path, n: usize, cancel: &CancelToken) -> anyhow::Result<Vec<(f32, f32)>> {
    anyhow::ensure!(n > 0, "n must be > 0");
    let p = crate::probe::probe_cancel(path, cancel)?;
    anyhow::ensure!(p.has_audio, tr("media.err.no_audio"));
    // Peaks need the real waveform: decimating with a low-pass filter would flatten loud high
    // notes. Native rate for short files; 16 kHz (speech/music energy) for long ones.
    let native = if p.sample_rate > 0 { p.sample_rate.clamp(8000, 48000) } else { 44100 };
    let rate: u32 = if p.duration > 0.0 && p.duration * native as f64 > 30_000_000.0 { 16000 } else { native };
    let total = if p.duration > 0.0 { (p.duration * rate as f64).ceil() as u64 } else { 0 };
    let mut cmd = castor_core::process::command(ffmpeg()?);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-ar", &rate.to_string(), "-f", "s16le", "-acodec", "pcm_s16le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take().unwrap();
    let mut peaks: Vec<(f32, f32)> = vec![(0.0, 0.0); n];
    let mut samples: Vec<i16> = Vec::new(); // only used when the duration is unknown
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Option<u8> = None;
    let mut idx: u64 = 0;
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        let got = out.read(&mut buf)?;
        if got == 0 {
            break;
        }
        let mut bytes: &[u8] = &buf[..got];
        let mut pair = [0u8; 2];
        if let Some(c) = carry.take() {
            pair[0] = c;
            pair[1] = bytes[0];
            bytes = &bytes[1..];
            push(i16::from_le_bytes(pair), &mut idx, total, n, &mut peaks, &mut samples);
        }
        let mut chunks = bytes.chunks_exact(2);
        for c in &mut chunks {
            push(i16::from_le_bytes([c[0], c[1]]), &mut idx, total, n, &mut peaks, &mut samples);
        }
        if let [b] = chunks.remainder() {
            carry = Some(*b);
        }
    }
    let _ = child.wait();
    if total == 0 && !samples.is_empty() {
        let len = samples.len();
        for (i, s) in samples.iter().enumerate() {
            let b = (i * n / len).min(n - 1);
            let v = *s as f32 / 32768.0;
            peaks[b].0 = peaks[b].0.min(v);
            peaks[b].1 = peaks[b].1.max(v);
        }
    }
    Ok(peaks)
}

fn push(s: i16, idx: &mut u64, total: u64, n: usize, peaks: &mut [(f32, f32)], samples: &mut Vec<i16>) {
    if total == 0 {
        samples.push(s);
        return;
    }
    let b = ((*idx * n as u64) / total).min(n as u64 - 1) as usize;
    let v = s as f32 / 32768.0;
    let p = &mut peaks[b];
    if v < p.0 {
        p.0 = v;
    }
    if v > p.1 {
        p.1 = v;
    }
    *idx += 1;
}
