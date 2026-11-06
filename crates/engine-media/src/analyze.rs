//! Analysis passes: silences, scene changes, black bars, loudness.

use std::path::Path;

use castor_core::RunCx;

use crate::Ffmpeg;

/// Silent intervals `(start, end)` in seconds of the first audio track: quieter than `noise_db`
/// (e.g. -35) for at least `min_len` seconds. A trailing silence ends at `duration`.
pub fn silences(path: &Path, noise_db: f64, min_len: f64, duration: f64, cx: &RunCx, range: (f32, f32)) -> anyhow::Result<Vec<(f64, f64)>> {
    let af = format!("silencedetect=noise={noise_db}dB:d={min_len}");
    let log = Ffmpeg::new()
        .input(path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-af", &af, "-f", "null", "-"])
        .duration(duration)
        .range(range.0, range.1)
        .info_log()
        .quiet()
        .run(cx)?;
    Ok(parse_silences(&log, duration))
}

pub fn parse_silences(log: &str, duration: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    for line in log.lines() {
        if let Some(i) = line.find("silence_start:") {
            start = line[i + 14..].split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(i) = line.find("silence_end:") {
            let end: Option<f64> = line[i + 12..].split_whitespace().next().and_then(|s| s.parse().ok());
            if let (Some(s), Some(e)) = (start.take(), end) {
                out.push((s.max(0.0), e));
            }
        }
    }
    if let Some(s) = start {
        if duration > s {
            out.push((s.max(0.0), duration));
        }
    }
    out
}

/// Scene-change times in seconds (`threshold` 0..100, scdet score; ~10 is a good default).
/// Changes closer than `min_gap` seconds to the previous one are ignored.
pub fn scenes(path: &Path, threshold: f64, min_gap: f64, duration: f64, cx: &RunCx, range: (f32, f32)) -> anyhow::Result<Vec<f64>> {
    // Analyse a small copy of the picture: much faster, same cuts.
    let vf = format!("scale=w=320:h=-2:flags=fast_bilinear,scdet=threshold={threshold}");
    let log = Ffmpeg::new()
        .input(path)
        .args(["-map", "0:V:0", "-an", "-sn", "-dn", "-vf", &vf, "-f", "null", "-"])
        .duration(duration)
        .range(range.0, range.1)
        .info_log()
        .quiet()
        .run(cx)?;
    let mut out: Vec<f64> = Vec::new();
    for line in log.lines() {
        if let Some(i) = line.find("lavfi.scd.time:") {
            if let Some(t) = line[i + 15..].split(|c: char| c == ',' || c.is_whitespace()).find(|s| !s.is_empty()).and_then(|s| s.parse::<f64>().ok())
            {
                if t > min_gap && out.last().is_none_or(|l| t - l >= min_gap) && (duration <= 0.0 || duration - t >= min_gap * 0.5) {
                    out.push(t);
                }
            }
        }
    }
    Ok(out)
}

/// Black-bar detection: the crop rectangle `(w, h, x, y)` that keeps the picture, in coded
/// (pre-rotation) pixels, or None if nothing to remove.
pub fn black_bars(path: &Path, width: u32, height: u32, duration: f64, cx: &RunCx) -> anyhow::Result<Option<(u32, u32, u32, u32)>> {
    // Keyframes only: fast on long files, enough samples to find the widest picture.
    let log = Ffmpeg::new()
        .args(["-skip_frame", "nokey"])
        .input(path)
        .args(["-map", "0:V:0", "-an", "-sn", "-dn", "-vf", "cropdetect=limit=24:round=2:reset=0:skip=0", "-f", "null", "-"])
        .duration(duration)
        .range(0.0, 0.3)
        .info_log()
        .quiet()
        .run(cx)?;
    let last = log.lines().rev().find_map(|l| l.find("crop=").map(|i| l[i + 5..].trim().to_string()));
    let Some(c) = last else { return Ok(None) };
    let v: Vec<i64> = c.split(':').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 4 || v[0] <= 0 || v[1] <= 0 {
        return Ok(None);
    }
    let (w, h, x, y) = (v[0] as u32, v[1] as u32, v[2].max(0) as u32, v[3].max(0) as u32);
    if (w >= width.saturating_sub(4)) && (h >= height.saturating_sub(4)) {
        return Ok(None);
    }
    Ok(Some((w, h, x, y)))
}

/// EBU R128 measurement (loudnorm first pass).
#[derive(Clone, Debug, Default)]
pub struct Loudness {
    /// Integrated loudness, LUFS.
    pub i: f64,
    /// True peak, dBTP.
    pub tp: f64,
    /// Loudness range, LU.
    pub lra: f64,
    pub thresh: f64,
    pub offset: f64,
}

/// Measure loudness of the first audio track (targets are only used for the offset).
pub fn loudness(path: &Path, target_i: f64, target_tp: f64, target_lra: f64, duration: f64, cx: &RunCx, range: (f32, f32)) -> anyhow::Result<Loudness> {
    let af = format!("loudnorm=I={target_i}:TP={target_tp}:LRA={target_lra}:print_format=json");
    let log = Ffmpeg::new()
        .input(path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-af", &af, "-f", "null", "-"])
        .duration(duration)
        .range(range.0, range.1)
        .info_log()
        .quiet()
        .run(cx)?;
    parse_loudness(&log).ok_or_else(|| anyhow::anyhow!(castor_core::tr("media.err.loudness")))
}

pub fn parse_loudness(log: &str) -> Option<Loudness> {
    let start = log.rfind("\"input_i\"")?;
    let open = log[..start].rfind('{')?;
    let close = open + log[open..].find('}')?;
    let v: serde_json::Value = serde_json::from_str(&log[open..=close]).ok()?;
    let f = |k: &str| v[k].as_str().and_then(|s| s.trim().parse::<f64>().ok());
    Some(Loudness { i: f("input_i")?, tp: f("input_tp")?, lra: f("input_lra")?, thresh: f("input_thresh")?, offset: f("target_offset").unwrap_or(0.0) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silences_parse() {
        let log = "[silencedetect @ 0x] silence_start: 1.5\n[silencedetect @ 0x] silence_end: 3.25 | silence_duration: 1.75\n[silencedetect @ 0x] silence_start: 9\n";
        assert_eq!(parse_silences(log, 10.0), vec![(1.5, 3.25), (9.0, 10.0)]);
    }
    #[test]
    fn loudness_parse() {
        let log = "x\n[Parsed_loudnorm_0 @ 0] \n{\n\t\"input_i\" : \"-21.81\",\n\t\"input_tp\" : \"-17.69\",\n\t\"input_lra\" : \"0.00\",\n\t\"input_thresh\" : \"-31.81\",\n\t\"target_offset\" : \"-0.00\"\n}\n[out#0]";
        let l = parse_loudness(log).unwrap();
        assert!((l.i + 21.81).abs() < 1e-9 && (l.tp + 17.69).abs() < 1e-9);
    }
}
