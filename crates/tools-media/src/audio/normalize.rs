//! `audio.normalize`: EBU R128 two-pass loudness normalisation (a single gain when possible),
//! peak normalisation, or a plain gain. Works on audio and on the soundtrack of videos.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Normalize {
    meta: ToolMeta,
}

impl Normalize {
    pub fn new() -> Self {
        Normalize {
            meta: meta(
                "audio.normalize",
                Family::Audio,
                InputSpec::kinds(AV),
                "MP3 · M4A · WAV · FLAC · MP4",
                &["volume", "loudness", "lufs", "louder", "quieter", "level", "ebu r128", "podcast", "gain"],
                "normalized",
                false,
            ),
        }
    }
}

fn target(v: &Values) -> (f64, f64) {
    match v.str("target") {
        "streaming" => (-14.0, -1.0),
        "broadcast" => (-23.0, -1.0),
        "custom" => (v.float("lufs").clamp(-40.0, -5.0), v.float("peak").clamp(-9.0, 0.0)),
        _ => (-16.0, -1.5),
    }
}

/// Last JSON object printed by loudnorm.
fn last_json(log: &str) -> Option<serde_json::Value> {
    let end = log.rfind('}')?;
    let start = log[..end].rfind('{')?;
    serde_json::from_str(&log[start..=end]).ok()
}

impl Tool for Normalize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("loudness"), Choice::key("peak"), Choice::key("gain")], "loudness"),
            OptionSpec::choice(
                "target",
                vec![Choice::key("podcast"), Choice::key("streaming"), Choice::key("broadcast"), Choice::key("custom")],
                "podcast",
            )
            .when(Cond::Eq("mode", "loudness")),
            OptionSpec::float("lufs", -40.0, -5.0, -16.0).step(0.5).unit("LUFS").when(Cond::Eq("target", "custom")),
            OptionSpec::float("peak", -9.0, 0.0, -1.0).step(0.1).unit("dB").when(Cond::Ne("mode", "gain")),
            OptionSpec::float("gain", -30.0, 30.0, 3.0).step(0.5).unit("dB").when(Cond::Eq("mode", "gain")),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let ext = reencode_ext(&ext_of(input), p.has_video);
        let dur = p.duration;
        let sr = if p.sample_rate > 0 { p.sample_rate } else { 48000 };
        let (af, note) = match v.str("mode") {
            "gain" => {
                let g = v.float("gain").clamp(-30.0, 30.0);
                (format!("volume={g:.2}dB"), tr_args("media.note.gain", &[("gain", &format!("{g:+.1} dB"))]))
            }
            "peak" => {
                let tp = v.float("peak").clamp(-9.0, 0.0);
                cx.message(tr("media.msg.measuring"));
                let log = Ffmpeg::new()
                    .input(&input.path)
                    .args(["-map", "0:a:0", "-vn", "-af", "volumedetect", "-f", "null", "-"])
                    .info_log()
                    .quiet()
                    .duration(dur)
                    .range(0.0, 0.4)
                    .run(cx)?;
                let max = log
                    .lines()
                    .rev()
                    .find_map(|l| l.split("max_volume:").nth(1).and_then(|s| s.trim().trim_end_matches("dB").trim().parse::<f64>().ok()))
                    .ok_or_else(|| anyhow::anyhow!(tr("media.err.loudness")))?;
                anyhow::ensure!(max > -90.0, tr("media.err.silent"));
                let g = tp - max;
                (format!("volume={g:.2}dB"), tr_args("media.note.peak", &[("before", &format!("{max:.1}")), ("after", &format!("{tp:.1}"))]))
            }
            _ => {
                let (i, tp) = target(v);
                cx.message(tr("media.msg.measuring"));
                let m = engine_media::analyze::loudness(&input.path, i, tp, 11.0, dur, cx, (0.0, 0.4))?;
                anyhow::ensure!(m.i > -70.0, tr("media.err.silent"));
                (
                    format!(
                        "loudnorm=I={i}:TP={tp}:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true:print_format=json",
                        // loudnorm refuses linear mode when the measured range is exactly 0
                        // (steady tones); a constant-loudness signal is fine with one gain.
                        m.i, m.tp, m.lra.max(0.1), m.thresh, m.offset
                    ),
                    tr_args("media.note.loudness", &[("before", &format!("{:.1}", m.i)), ("after", &format!("{i:.1}")), ("peak", &format!("{tp:.1}"))]),
                )
            }
        };
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let mut ff = Ffmpeg::new().input(&input.path);
        if p.has_video {
            ff = ff
                .args(stream_map(&p, &ext, true, false))
                .args(["-map", "0:a:0", "-c:v", "copy", "-af", &af, "-ar", &sr.to_string()])
                .args(audio_args(&ext, 192, &p));
        } else {
            ff = ff.args(super::convert_args_same(&p, &ext)).args(["-af", &af, "-ar", &sr.to_string()]);
        }
        let log = ff.args(mux_args(&ext)).arg(out.path()).duration(dur).range(0.4, 1.0).info_log().run(cx)?;
        let mut note = note;
        if let Some(j) = last_json(&log) {
            if j["normalization_type"].as_str() == Some("dynamic") {
                note.push_str(" · ");
                note.push_str(&tr("media.note.dynamic"));
            }
        }
        Ok(vec![Output::new(out.keep()).with_note(note)])
    }
}
