//! `video.speed` / `audio.speed`: 0.25–4×, pitch kept (atempo) or shifted (resampling).

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, units};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Speed {
    meta: ToolMeta,
    audio: bool,
}

impl Speed {
    pub fn video() -> Self {
        Speed {
            meta: meta(
                "video.speed",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["fast", "slow", "motion", "timelapse", "faster", "slower", "tempo"],
                "speed",
                true,
            ),
            audio: false,
        }
    }
    pub fn audio() -> Self {
        Speed {
            meta: meta(
                "audio.speed",
                Family::Audio,
                InputSpec::kinds(AUDIO),
                "MP3 · M4A · WAV · FLAC",
                &["fast", "slow", "tempo", "pitch", "nightcore", "faster", "slower", "podcast"],
                "speed",
                false,
            ),
            audio: true,
        }
    }
}

/// Audio filter changing speed by `f`, keeping the pitch (atempo chain) or not (resample).
pub fn audio_speed_filter(f: f64, keep_pitch: bool, sample_rate: u32) -> String {
    if !keep_pitch {
        let sr = if sample_rate > 0 { sample_rate } else { 48000 };
        return format!("asetrate={},aresample={sr}", (sr as f64 * f).round() as u64);
    }
    let mut parts = Vec::new();
    let mut r = f;
    while r > 2.0 {
        parts.push("atempo=2".to_string());
        r /= 2.0;
    }
    while r < 0.5 {
        parts.push("atempo=0.5".to_string());
        r /= 0.5;
    }
    parts.push(format!("atempo={r:.6}"));
    parts.join(",")
}

impl Tool for Speed {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::float("factor", 0.25, 4.0, if self.audio { 1.25 } else { 2.0 }).step(0.05).slider().unit("×"),
            OptionSpec::choice("pitch", vec![Choice::key("keep"), Choice::key("shift")], "keep"),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        if self.audio { need_audio(&p)? } else { need_video(&p)? }
        let f = v.float("factor").clamp(0.25, 4.0);
        anyhow::ensure!((f - 1.0).abs() > 1e-3, tr("media.err.nothing_to_do"));
        let keep = v.str("pitch") != "shift";
        let ext = reencode_ext(&ext_of(input), p.has_video);
        let new_dur = p.duration / f;
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let af = audio_speed_filter(f, keep, p.sample_rate);
        if self.audio || !p.has_video {
            Ffmpeg::new()
                .input(&input.path)
                .args(["-map", "0:a", "-vn", "-af", &af, "-map_metadata", "0"])
                .args(super::trim::audio_reencode(&p, &ext))
                .args(mux_args(&ext))
                .arg(out.path())
                .duration(new_dur)
                .run(cx)?;
        } else {
            let enc = edit_encoder(&p, &ext);
            let fps = if p.fps > 0.0 && p.fps < 240.0 { format!(",fps={:.4}", p.fps) } else { String::new() };
            let mut vf = vec![format!("setpts=PTS/{f:.6}{fps}")];
            finish_filters(&p, &enc, &mut vf);
            let (va, _) = video_args(&p, &enc);
            let mut ff = Ffmpeg::new().input(&input.path).args(["-map", &format!("0:{}", p.video_index.unwrap_or(0))]);
            if p.has_audio {
                ff = ff.args(["-map", "0:a", "-af", &af]).args(audio_args(&ext, 192, &p));
            }
            ff.args(["-vf", &vf.join(",")])
                .args(va)
                .args(["-map_metadata", "0"])
                .args(mux_args(&ext))
                .arg(out.path())
                .duration(new_dur)
                .run(cx)?;
        }
        let path = out.keep();
        let note = format!("{} → {} ({f}×)", units::duration(p.duration), units::duration(new_dur));
        Ok(vec![Output::new(path).with_note(note)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atempo_chain() {
        assert_eq!(audio_speed_filter(4.0, true, 0), "atempo=2,atempo=2.000000");
        assert_eq!(audio_speed_filter(0.25, true, 0), "atempo=0.5,atempo=0.500000");
        assert_eq!(audio_speed_filter(1.5, false, 44100), "asetrate=66150,aresample=44100");
    }
}
