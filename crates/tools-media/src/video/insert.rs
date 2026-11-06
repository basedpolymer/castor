//! `video.insert-clips`: intro, a clip inserted at the first pause after X seconds, outro.
//! Everything is normalised to the main video's size, frame rate and a stereo 48 kHz track.

use castor_core::options::{OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args, units};
use engine_media::{Ffmpeg, Probe};

use crate::common::*;

pub struct InsertClips {
    meta: ToolMeta,
}

impl InsertClips {
    pub fn new() -> Self {
        InsertClips {
            meta: meta(
                "video.insert-clips",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4",
                &["intro", "outro", "ad", "sponsor", "bumper", "insert", "splice"],
                "edited",
                true,
            ),
        }
    }
}

const VID: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v", "avi"];

/// One piece of the final timeline.
struct Piece {
    input: usize,
    range: Option<(f64, f64)>,
    has_audio: bool,
    duration: f64,
}

/// Where to insert: the middle of the first pause starting at or after `after` seconds.
pub fn insertion_point(silences: &[(f64, f64)], after: f64, duration: f64) -> (f64, bool) {
    match silences.iter().find(|(a, _)| *a >= after) {
        Some((a, b)) => (((a + b) / 2.0).min(duration), true),
        None => (after.min(duration), false),
    }
}

impl Tool for InsertClips {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::file("intro", VID),
            OptionSpec::file("clip", VID),
            OptionSpec::text("after", "1:00").placeholder("media.ph.time"),
            OptionSpec::file("outro", VID),
            quality_option("best").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let dur = p.duration;
        let get = |k: &str| -> Result<Option<(std::path::PathBuf, Probe)>> {
            match v.path(k) {
                None => Ok(None),
                Some(path) => {
                    anyhow::ensure!(path.is_file(), tr_args("media.err.missing_file", &[("file", &path.display().to_string())]));
                    let q = engine_media::probe(path)?;
                    need_video(&q)?;
                    Ok(Some((path.to_path_buf(), q)))
                }
            }
        };
        let (intro, clip, outro) = (get("intro")?, get("clip")?, get("outro")?);
        anyhow::ensure!(intro.is_some() || clip.is_some() || outro.is_some(), tr("media.err.no_clips"));

        let mut files: Vec<(std::path::PathBuf, Probe)> = vec![(input.path.clone(), p.clone())];
        let mut pieces = Vec::new();
        let add_file = |f: (std::path::PathBuf, Probe), files: &mut Vec<(std::path::PathBuf, Probe)>| {
            files.push(f);
            files.len() - 1
        };
        let mut note_at = None;
        if let Some(f) = intro {
            let d = f.1.duration;
            let a = f.1.has_audio;
            let i = add_file(f, &mut files);
            pieces.push(Piece { input: i, range: None, has_audio: a, duration: d });
        }
        let mut base = 0.0;
        if let Some(f) = clip {
            let after = time_opt(v, "after")?.unwrap_or(0.0);
            let (at, found) = if p.has_audio {
                cx.message(tr("media.msg.finding_pauses"));
                let sil = engine_media::analyze::silences(&input.path, -35.0, 0.4, dur, cx, (0.0, 0.15))?;
                insertion_point(&sil, after, dur)
            } else {
                (after.min(dur), false)
            };
            base = 0.15;
            note_at = Some((at, found));
            if at > 0.05 {
                pieces.push(Piece { input: 0, range: Some((0.0, at)), has_audio: p.has_audio, duration: at });
            }
            let d = f.1.duration;
            let a = f.1.has_audio;
            let i = add_file(f, &mut files);
            pieces.push(Piece { input: i, range: None, has_audio: a, duration: d });
            if dur - at > 0.05 {
                pieces.push(Piece { input: 0, range: Some((at, dur)), has_audio: p.has_audio, duration: dur - at });
            }
        } else {
            pieces.push(Piece { input: 0, range: None, has_audio: p.has_audio, duration: dur });
        }
        if let Some(f) = outro {
            let d = f.1.duration;
            let a = f.1.has_audio;
            let i = add_file(f, &mut files);
            pieces.push(Piece { input: i, range: None, has_audio: a, duration: d });
        }

        let (w, h) = p.display_size();
        let (w, h) = (w / 2 * 2, h / 2 * 2);
        let fps = if p.fps > 0.0 && p.fps <= 120.0 { p.fps } else { 30.0 };
        let total: f64 = pieces.iter().map(|x| x.duration).sum();
        let mut fc = String::new();
        // Main input used twice: split it.
        let main_uses = pieces.iter().filter(|x| x.input == 0).count();
        if main_uses > 1 {
            fc.push_str(&format!("[0:{}]split={main_uses}", p.video_index.unwrap_or(0)));
            for k in 0..main_uses {
                fc.push_str(&format!("[mv{k}]"));
            }
            fc.push(';');
            if p.has_audio {
                fc.push_str(&format!("[0:a:0]asplit={main_uses}"));
                for k in 0..main_uses {
                    fc.push_str(&format!("[ma{k}]"));
                }
                fc.push(';');
            }
        }
        let mut cat = String::new();
        let mut k = 0;
        for (n, pc) in pieces.iter().enumerate() {
            let (vin, ain) = if pc.input == 0 && main_uses > 1 {
                let r = (format!("[mv{k}]"), format!("[ma{k}]"));
                k += 1;
                r
            } else {
                (format!("[{}:v:0]", pc.input), format!("[{}:a:0]", pc.input))
            };
            let (vt, at) = match pc.range {
                Some((a, b)) => (format!("trim=start={a:.3}:end={b:.3},setpts=PTS-STARTPTS,"), format!("atrim=start={a:.3}:end={b:.3},asetpts=PTS-STARTPTS,")),
                None => (String::new(), String::new()),
            };
            fc.push_str(&format!(
                "{vin}{vt}scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps={fps:.4},format=yuv420p[v{n}];"
            ));
            if pc.has_audio {
                fc.push_str(&format!("{ain}{at}aformat=sample_rates=48000:channel_layouts=stereo[a{n}];"));
            } else {
                fc.push_str(&format!("anullsrc=r=48000:cl=stereo,atrim=duration={:.3}[a{n}];", pc.duration.max(0.04)));
            }
            cat.push_str(&format!("[v{n}][a{n}]"));
        }
        let enc = VideoEnc::quality(VCodec::H264, level(v));
        let mut tail = Vec::new();
        finish_filters(&p, &enc, &mut tail);
        fc.push_str(&format!("{cat}concat=n={}:v=1:a=1[vc][aout];[vc]{}[vout]", pieces.len(), tail.join(",")));
        let tmp = cx.tmp_dir();
        std::fs::write(tmp.join("graph.txt"), &fc)?;
        let out = Partial::new(cx.output_path(Some(input), "mp4"));
        let mut ff = Ffmpeg::new();
        for (f, _) in &files {
            ff = ff.input(f);
        }
        let (va, _) = video_args(&p, &enc);
        ff.arg("-/filter_complex")
            .arg(tmp.join("graph.txt"))
            .args(["-map", "[vout]", "-map", "[aout]"])
            .args(va)
            .args(audio_args("mp4", 192, &Probe::default()))
            .args(mux_args("mp4"))
            .arg(out.path())
            .duration(total)
            .range(base, 1.0)
            .run(cx)?;
        let note = match note_at {
            Some((t, true)) => tr_args("media.note.inserted_pause", &[("at", &units::timecode(t))]),
            Some((t, false)) => tr_args("media.note.inserted_no_pause", &[("at", &units::timecode(t))]),
            None => units::duration(total),
        };
        Ok(vec![Output::new(out.keep()).with_note(note)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point() {
        assert_eq!(insertion_point(&[(2.0, 3.0), (31.0, 32.0)], 30.0, 60.0), (31.5, true));
        assert_eq!(insertion_point(&[(2.0, 3.0)], 30.0, 60.0), (30.0, false));
        assert_eq!(insertion_point(&[], 90.0, 60.0), (60.0, false));
    }
}
