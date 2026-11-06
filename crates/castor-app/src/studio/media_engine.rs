//! Trim & Caption Studio, engine side: the edit (kept ranges, transcript words, caption cues
//! and style), interval arithmetic, cue remapping through the cuts, and the export, which calls
//! the registry's own tools (`video.trim`/`audio.trim`, `video.join`/`audio.merge`,
//! `video.captions`) so the result is exactly what those tools produce.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use castor_core::job::Output;
use castor_core::options::{Value, Values};
use castor_core::{FileInfo, Job, Registry, RunCx, tr};
use serde::{Deserialize, Serialize};

/// A transcribed word.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A caption cue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Export {
    /// The cut video or audio.
    #[default]
    Media,
    /// The cut video with the captions burnt in.
    Burn,
    Srt,
    Vtt,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaEdit {
    pub source: PathBuf,
    pub duration: f64,
    pub has_video: bool,
    /// Ranges kept, sorted, disjoint, in seconds.
    pub keep: Vec<(f64, f64)>,
    /// "exact" (re-encode, frame accurate) or "fast" (keyframes, no re-encode).
    pub cut: String,
    pub words: Vec<Word>,
    pub cues: Vec<Cue>,
    /// Caption look, as the options of `video.captions`.
    pub style: String,
    pub size: i64,
    pub position: String,
    pub color: String,
    pub export: Export,
}

impl MediaEdit {
    pub fn new(source: PathBuf, duration: f64, has_video: bool) -> MediaEdit {
        MediaEdit {
            source,
            duration,
            has_video,
            keep: vec![(0.0, duration)],
            cut: "exact".into(),
            words: Vec::new(),
            cues: Vec::new(),
            style: "classic".into(),
            size: 6,
            position: "bottom".into(),
            color: "#ffffffff".into(),
            export: Export::Media,
        }
    }

    pub fn kept(&self) -> f64 {
        self.keep.iter().map(|(a, b)| b - a).sum()
    }

    pub fn has_cuts(&self) -> bool {
        (self.duration - self.kept()).abs() > 0.01
    }

    /// Remove `[a, b)` from the kept ranges.
    pub fn cut_range(&mut self, a: f64, b: f64) {
        self.keep = subtract(&self.keep, a.min(b), a.max(b));
    }

    /// Put `[a, b)` back.
    pub fn restore_range(&mut self, a: f64, b: f64) {
        let (a, b) = (a.min(b).max(0.0), a.max(b).min(self.duration));
        let mut v = self.keep.clone();
        v.push((a, b));
        self.keep = merge(v);
    }

    /// Keep only `[a, b)` (in / out points).
    pub fn trim_to(&mut self, a: f64, b: f64) {
        let (a, b) = (a.min(b), a.max(b));
        self.keep = self.keep.iter().filter_map(|&(x, y)| {
            let (s, e) = (x.max(a), y.min(b));
            (e - s > 0.01).then_some((s, e))
        }).collect();
    }

    /// Is the instant `t` kept?
    pub fn is_kept(&self, t: f64) -> bool {
        self.keep.iter().any(|&(a, b)| t >= a && t < b)
    }

    /// A word is cut when most of it falls outside the kept ranges.
    pub fn word_cut(&self, w: &Word) -> bool {
        let len = (w.end - w.start).max(0.001);
        overlap(&self.keep, w.start, w.end) < len * 0.5
    }

    /// Time on the edited timeline for a source time (None when cut out).
    pub fn map_time(&self, t: f64) -> Option<f64> {
        let mut acc = 0.0;
        for &(a, b) in &self.keep {
            if t >= a && t <= b {
                return Some(acc + t - a);
            }
            acc += b - a;
        }
        None
    }

    /// Cues moved onto the edited timeline; cues entirely cut out are dropped.
    pub fn mapped_cues(&self) -> Vec<Cue> {
        let mut out = Vec::new();
        for c in &self.cues {
            let parts: Vec<(f64, f64)> =
                self.keep.iter().filter_map(|&(a, b)| { let (s, e) = (c.start.max(a), c.end.min(b)); (e - s > 0.05).then_some((s, e)) }).collect();
            let (Some(first), Some(last)) = (parts.first(), parts.last()) else { continue };
            let (Some(s), Some(e)) = (self.map_time(first.0), self.map_time(last.1)) else { continue };
            if e - s > 0.05 && !c.text.trim().is_empty() {
                out.push(Cue { start: s, end: e, text: c.text.clone() });
            }
        }
        out
    }
}

/// Merge overlapping or touching ranges.
pub fn merge(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.retain(|(a, b)| b - a > 0.001);
    v.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 + 0.001 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `v` minus `[a, b)`.
pub fn subtract(v: &[(f64, f64)], a: f64, b: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(x, y) in v {
        if b <= x || a >= y {
            out.push((x, y));
            continue;
        }
        if a - x > 0.01 {
            out.push((x, a));
        }
        if y - b > 0.01 {
            out.push((b, y));
        }
    }
    out
}

/// Keyframe times of the first video stream (empty for audio or when ffprobe fails).
pub fn keyframes(path: &Path) -> Vec<f64> {
    let Ok(ffprobe) = engine_media::ffprobe() else { return Vec::new() };
    let out = castor_core::process::command(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .arg(path)
        .output();
    let Ok(out) = out else { return Vec::new() };
    let mut v: Vec<f64> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().trim_end_matches(',').parse().ok()).collect();
    v.sort_by(f64::total_cmp);
    v
}

/// What a fast (keyframe) cut really keeps: each range starts at the keyframe at or before its
/// start; ranges that then overlap are merged, so no picture is ever repeated.
pub fn snap_fast(keep: &[(f64, f64)], keys: &[f64]) -> Vec<(f64, f64)> {
    if keys.is_empty() {
        return keep.to_vec();
    }
    merge(keep.iter().map(|&(a, b)| (keys.iter().copied().filter(|k| *k <= a + 0.001).last().unwrap_or(0.0), b)).collect())
}

fn overlap(v: &[(f64, f64)], a: f64, b: f64) -> f64 {
    v.iter().map(|&(x, y)| (y.min(b) - x.max(a)).max(0.0)).sum()
}

// ---- transcript -------------------------------------------------------------------------

#[derive(Deserialize)]
struct JWord {
    start: f64,
    end: f64,
    text: String,
}
#[derive(Deserialize)]
struct JSegment {
    start: f64,
    end: f64,
    text: String,
    #[serde(default)]
    words: Vec<JWord>,
}
#[derive(Deserialize)]
struct JTranscript {
    #[serde(default)]
    language: String,
    #[serde(default)]
    duration: f64,
    segments: Vec<JSegment>,
}

/// Words and caption cues from the JSON written by `audio.transcribe` (format "json").
pub fn parse_transcript(json: &str) -> Result<(Vec<Word>, Vec<Cue>)> {
    use engine_media::whisper;
    let t: JTranscript = serde_json::from_str(json)?;
    let wt = whisper::Transcript {
        language: t.language,
        duration: t.duration,
        segments: t
            .segments
            .iter()
            .map(|s| whisper::Segment {
                start: s.start,
                end: s.end,
                text: s.text.clone(),
                words: s.words.iter().map(|w| whisper::Word { start: w.start, end: w.end, text: w.text.clone(), p: 1.0 }).collect(),
            })
            .collect(),
    };
    let words: Vec<Word> = wt.words().filter(|w| !w.text.trim().is_empty()).map(|w| Word { start: w.start, end: w.end, text: w.text.trim().to_string() }).collect();
    let cues = engine_media::subs::cues_from_transcript(&wt, engine_media::subs::Layout::default())
        .into_iter()
        .map(|c| Cue { start: c.start, end: c.end, text: c.text })
        .collect();
    Ok((words, cues))
}

/// Cues from an SRT / VTT / ASS file.
pub fn read_subtitles(path: &Path) -> Result<Vec<Cue>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    Ok(engine_media::subs::parse(&text).into_iter().map(|c| Cue { start: c.start, end: c.end, text: c.text }).collect())
}

fn to_subs(cues: &[Cue]) -> Vec<engine_media::subs::Cue> {
    cues.iter().map(|c| engine_media::subs::Cue::new(c.start, c.end, c.text.clone())).collect()
}

// ---- export -----------------------------------------------------------------------------

/// A RunCx for a sub-step writing into `out_dir`, its progress mapped onto `[a, b]`.
fn sub_cx(cx: &RunCx, out_dir: &Path, tmp: &Path, suffix: &str, a: f32, b: f32) -> Result<RunCx> {
    std::fs::create_dir_all(out_dir)?;
    std::fs::create_dir_all(tmp)?;
    let (p, m) = (cx.clone(), cx.clone());
    Ok(RunCx::new(out_dir.to_path_buf(), tmp.to_path_buf(), suffix, cx.cancel_token())
        .on_progress(move |f| p.progress(a + (b - a) * f))
        .on_message(move |s| m.message(s)))
}

fn tool(reg: &Registry, id: &str) -> Result<std::sync::Arc<dyn castor_core::Tool>> {
    reg.get(id).with_context(|| format!("{id}: {}", tr("app.studio.err.tool_missing")))
}

fn move_into(src: &Path, dst: &Path) -> Result<()> {
    if std::fs::rename(src, dst).is_err() {
        std::fs::copy(src, dst)?;
        let _ = std::fs::remove_file(src);
    }
    Ok(())
}

fn secs(t: f64) -> Value {
    Value::Text(format!("{t:.3}"))
}

/// Cut the kept ranges out of the source into `dir`. Returns the file (the source itself when
/// nothing is cut).
fn cut(edit: &MediaEdit, input: &FileInfo, reg: &Registry, dir: &Path, cx: &RunCx, a: f32, b: f32) -> Result<PathBuf> {
    if !edit.has_cuts() {
        return Ok(input.path.clone());
    }
    anyhow::ensure!(!edit.keep.is_empty(), "{}", tr("app.media.err.nothing_kept"));
    let trim = tool(reg, if edit.has_video { "video.trim" } else { "audio.trim" })?;
    let mut tv = Values::defaults(&trim.options());
    tv.set("until", Value::Text("end".into()));
    tv.set("cut", Value::Text(edit.cut.clone()));
    // Fast cuts start on keyframes: plan the ranges as they will really come out.
    let ranges = if edit.cut == "fast" && edit.has_video { snap_fast(&edit.keep, &keyframes(&input.path)) } else { edit.keep.clone() };
    let n = ranges.len();
    let parts_end = if n == 1 { b } else { a + (b - a) * 0.8 };
    let mut parts = Vec::new();
    for (k, &(s, e)) in ranges.iter().enumerate() {
        cx.check_cancel()?;
        tv.set("start", secs(s));
        tv.set("end", secs(e));
        let pa = a + (parts_end - a) * k as f32 / n as f32;
        let pb = a + (parts_end - a) * (k + 1) as f32 / n as f32;
        let seg_dir = dir.join(format!("part{}", k + 1));
        let scx = sub_cx(cx, &seg_dir, &dir.join(format!("tmp{}", k + 1)), &format!("part{}", k + 1), pa, pb)?;
        let outs = trim.run(&Job::single(input.clone()), &tv, &scx)?;
        parts.push(outs.into_iter().next().context("trim")?.path);
    }
    if n == 1 {
        return Ok(parts.remove(0));
    }
    let join = tool(reg, if edit.has_video { "video.join" } else { "audio.merge" })?;
    let jv = Values::defaults(&join.options());
    let jcx = sub_cx(cx, &dir.join("joined"), &dir.join("tmpj"), "joined", parts_end, b)?;
    let files: Vec<FileInfo> = parts.iter().map(|p| FileInfo::from_path(p)).collect();
    let outs = join.run(&Job::many(files), &jv, &jcx)?;
    Ok(outs.into_iter().next().context("join")?.path)
}

/// Run the export described by `edit` on `input`.
pub fn export(edit: &MediaEdit, input: &FileInfo, reg: &Registry, cx: &RunCx) -> Result<Vec<Output>> {
    let work = cx.tmp_dir().join("studio");
    std::fs::create_dir_all(&work)?;
    match edit.export {
        Export::Srt | Export::Vtt => {
            let cues = to_subs(&edit.mapped_cues());
            anyhow::ensure!(!cues.is_empty(), "{}", tr("app.media.err.no_captions"));
            let (ext, text) = if edit.export == Export::Srt { ("srt", engine_media::subs::to_srt(&cues)) } else { ("vtt", engine_media::subs::to_vtt(&cues)) };
            let out = cx.output_path(Some(input), ext);
            std::fs::write(&out, text)?;
            Ok(vec![Output::new(out)])
        }
        Export::Media => {
            anyhow::ensure!(edit.has_cuts(), "{}", tr("app.media.err.no_change"));
            let file = cut(edit, input, reg, &work, cx, 0.0, 1.0)?;
            let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
            let out = cx.output_path(Some(input), &ext);
            move_into(&file, &out)?;
            Ok(vec![Output::new(out)])
        }
        Export::Burn => {
            anyhow::ensure!(edit.has_video, "{}", tr("app.media.err.burn_audio"));
            let cues = to_subs(&edit.mapped_cues());
            anyhow::ensure!(!cues.is_empty(), "{}", tr("app.media.err.no_captions"));
            let split = if edit.has_cuts() { 0.5 } else { 0.0 };
            let file = cut(edit, input, reg, &work, cx, 0.0, split)?;
            let srt = work.join("captions.srt");
            std::fs::write(&srt, engine_media::subs::to_srt(&cues))?;
            let cap = tool(reg, "video.captions")?;
            let mut v = Values::defaults(&cap.options());
            v.set("source", Value::Text("file".into()));
            v.set("subtitles", Value::Path(Some(srt)));
            v.set("mode", Value::Text("burn".into()));
            v.set("style", Value::Text(edit.style.clone()));
            v.set("size", Value::Int(edit.size));
            v.set("position", Value::Text(edit.position.clone()));
            v.set("color", Value::Text(edit.color.clone()));
            let ccx = sub_cx(cx, &work.join("captioned"), &work.join("tmpc"), "captioned", split, 1.0)?;
            let outs = cap.run(&Job::single(FileInfo::from_path(&file)), &v, &ccx)?;
            let made = outs.into_iter().find(|o| o.path.extension().is_some_and(|e| e != "srt")).context("captions")?;
            let ext = made.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
            let out = cx.output_path(Some(input), &ext);
            move_into(&made.path, &out)?;
            Ok(vec![Output::new(out)])
        }
    }
}

/// Cut the range `[a, b)` without re-encoding into a temporary file and open it with the
/// system player (Castor has no real-time player yet).
pub fn play_segment(path: &Path, a: f64, b: f64) -> Result<()> {
    let dir = castor_core::paths::cache_dir().join("studio-play");
    std::fs::create_dir_all(&dir)?;
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let out = dir.join(format!("segment.{ext}"));
    let _ = std::fs::remove_file(&out);
    let cx = RunCx::new(dir.clone(), dir.clone(), "", castor_core::CancelToken::new());
    engine_media::Ffmpeg::new()
        .args(["-ss", &format!("{a:.3}")])
        .input(path)
        .args(["-t", &format!("{:.3}", (b - a).max(0.1)), "-map", "0:v?", "-map", "0:a?", "-c", "copy", "-avoid_negative_ts", "make_zero"])
        .arg(&out)
        .duration(b - a)
        .run(&cx)?;
    castor_core::process::open(&out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals() {
        let mut e = MediaEdit::new("a.mp4".into(), 10.0, true);
        e.cut_range(2.0, 3.0);
        e.cut_range(5.0, 6.0);
        assert_eq!(e.keep, vec![(0.0, 2.0), (3.0, 5.0), (6.0, 10.0)]);
        assert!((e.kept() - 8.0).abs() < 1e-9);
        assert_eq!(e.map_time(3.5), Some(2.5));
        assert_eq!(e.map_time(2.5), None);
        e.restore_range(2.0, 3.0);
        assert_eq!(e.keep, vec![(0.0, 5.0), (6.0, 10.0)]);
        e.trim_to(1.0, 9.0);
        assert_eq!(e.keep, vec![(1.0, 5.0), (6.0, 9.0)]);
        // Keyframes every 4 s: (6, 9) starts at 4 and swallows the cut between 5 and 6.
        assert_eq!(snap_fast(&e.keep, &[0.0, 4.0, 8.0]), vec![(0.0, 9.0)]);
        assert_eq!(snap_fast(&[(1.0, 2.0), (8.5, 9.0)], &[0.0, 8.0]), vec![(0.0, 2.0), (8.0, 9.0)]);
    }

    #[test]
    fn cues_follow_cuts() {
        let mut e = MediaEdit::new("a.mp4".into(), 10.0, true);
        e.cues = vec![Cue { start: 1.0, end: 2.0, text: "a".into() }, Cue { start: 3.0, end: 4.0, text: "b".into() }, Cue { start: 6.0, end: 7.0, text: "c".into() }];
        e.cut_range(2.5, 4.5);
        let m = e.mapped_cues();
        assert_eq!(m.len(), 2, "the cue inside the cut is dropped");
        assert!((m[1].start - 4.0).abs() < 1e-9);
        let w = Word { start: 3.0, end: 3.4, text: "b".into() };
        assert!(e.word_cut(&w));
    }

    #[test]
    fn transcript_json() {
        let j = r#"{"language":"en","duration":3.0,"segments":[{"start":0.0,"end":2.0,"text":" Hello world.","words":[{"start":0.0,"end":0.5,"text":" Hello","p":0.9},{"start":0.6,"end":1.2,"text":" world.","p":0.9}]}]}"#;
        let (w, c) = parse_transcript(j).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[1].text, "world.");
        assert_eq!(c.len(), 1);
    }
}
