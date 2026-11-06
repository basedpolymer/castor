//! `video.add-audio`: replace the soundtrack or mix a new one in (volumes, loop), video copied.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct AddAudio {
    meta: ToolMeta,
}

impl AddAudio {
    pub fn new() -> Self {
        AddAudio {
            meta: meta(
                "video.add-audio",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["music", "soundtrack", "background music", "voiceover", "replace audio", "dub", "mix"],
                "audio",
                true,
            ),
        }
    }
}

const AUDIO_EXT: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "aiff", "aif", "wma", "mp4", "mov", "mkv", "webm"];

impl Tool for AddAudio {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::file("audio", AUDIO_EXT),
            OptionSpec::choice("mode", vec![Choice::key("replace"), Choice::key("mix")], "replace"),
            OptionSpec::int("volume", 0, 200, 100).slider().unit("%"),
            OptionSpec::int("original", 0, 200, 100).slider().unit("%").when(Cond::Eq("mode", "mix")),
            OptionSpec::toggle("loop", true),
            OptionSpec::float("fade", 0.0, 10.0, 0.0).step(0.5).unit("s").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let audio = v.path("audio").ok_or_else(|| anyhow::anyhow!(tr("media.err.no_audio_file")))?;
        anyhow::ensure!(audio.is_file(), tr("media.err.no_audio_file"));
        let ap = engine_media::probe(audio)?;
        need_audio(&ap)?;
        let ext = match ext_of(input).as_str() {
            e @ ("mp4" | "mov" | "mkv" | "webm" | "m4v") => e.to_string(),
            _ => "mp4".to_string(),
        };
        let dur = p.duration;
        let new_vol = v.int("volume").clamp(0, 200) as f64 / 100.0;
        let orig_vol = v.int("original").clamp(0, 200) as f64 / 100.0;
        let fade = v.float("fade").clamp(0.0, 10.0);
        let mut a1 = format!("[1:a:0]volume={new_vol:.3},aformat=sample_rates=48000:channel_layouts=stereo");
        if fade > 0.0 && dur > fade * 2.0 {
            a1.push_str(&format!(",afade=t=in:d={fade},afade=t=out:st={:.3}:d={fade}", dur - fade));
        }
        let graph = if v.str("mode") == "mix" && p.has_audio {
            format!(
                "{a1}[n];[0:a:0]volume={orig_vol:.3},aformat=sample_rates=48000:channel_layouts=stereo[o];[o][n]amix=inputs=2:duration=first:dropout_transition=0:normalize=0[aout]"
            )
        } else {
            format!("{a1},apad[aout]")
        };
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let mut ff = Ffmpeg::new().input(&input.path);
        if v.bool("loop") && ap.duration > 0.0 && ap.duration < dur {
            ff = ff.args(["-stream_loop", "-1"]);
        }
        ff.input(audio)
            .args(["-filter_complex", &graph])
            .args(["-map", &format!("0:{}", p.video_index.unwrap_or(0)), "-map", "[aout]", "-c:v", "copy"])
            .args(audio_args(&ext, 192, &engine_media::Probe::default()))
            .args(["-t", &format!("{dur:.3}"), "-map_metadata", "0"])
            .args(mux_args(&ext))
            .arg(out.path())
            .duration(dur)
            .run(cx)?;
        let s = size_of(out.path());
        Ok(vec![Output::new(out.keep()).with_note(castor_core::units::bytes(s))])
    }
}
