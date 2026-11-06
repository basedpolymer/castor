//! `video.mute`: drop every audio track, no re-encoding.

use castor_core::options::{OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Mute {
    meta: ToolMeta,
}

impl Mute {
    pub fn new() -> Self {
        Mute {
            meta: meta(
                "video.mute",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["silent", "remove audio", "no sound", "strip audio", "silence"],
                "muted",
                false,
            ),
        }
    }
}

impl Tool for Mute {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        Vec::new()
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, _v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let ext = ext_of(input);
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let mut map = stream_map(&p, &ext, true, false);
        // Subtitle codec conversion from stream_map is fine; everything else is copied.
        map.splice(0..0, ["-c".to_string(), "copy".to_string()]);
        Ffmpeg::new().input(&input.path).args(map).arg("-an").args(mux_args(&ext)).arg(out.path()).duration(p.duration).run(cx)?;
        let size = size_of(out.path());
        let path = out.keep();
        let note = if p.has_audio { size_note(input.size, size) } else { tr("media.note.had_no_audio") };
        Ok(vec![Output::new(path).with_note(note)])
    }
}
