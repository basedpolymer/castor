//! `video.remove-silences`: detect pauses on the soundtrack and cut them out (video + audio
//! kept in sync), keeping a little padding so speech does not sound clipped.

use castor_core::options::{OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args, units};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct RemoveSilences {
    meta: ToolMeta,
}

impl RemoveSilences {
    pub fn new() -> Self {
        RemoveSilences {
            meta: meta(
                "video.remove-silences",
                Family::Video,
                InputSpec::kinds(AV),
                "MP4 · MOV · MKV · MP3 · WAV",
                &["jump cut", "pauses", "dead air", "silence", "tighten", "podcast", "vlog", "auto cut"],
                "tight",
                true,
            ),
        }
    }
}

/// Segments to keep: the complement of `silences` (each shrunk by `pad` on both sides).
pub fn keep_segments(silences: &[(f64, f64)], duration: f64, pad: f64) -> Vec<(f64, f64)> {
    let mut keep = Vec::new();
    let mut t = 0.0;
    for &(a, b) in silences {
        let (cut_a, cut_b) = (if a <= 0.0 { 0.0 } else { a + pad }, if b >= duration { duration } else { b - pad });
        if cut_b - cut_a < 0.05 {
            continue;
        }
        if cut_a > t {
            keep.push((t, cut_a));
        }
        t = cut_b;
    }
    if duration - t > 0.02 {
        keep.push((t, duration));
    }
    keep
}

impl Tool for RemoveSilences {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::int("noise", -70, -10, -35).slider().unit("dB"),
            OptionSpec::float("min_silence", 0.2, 10.0, 0.7).step(0.1).unit("s"),
            OptionSpec::float("padding", 0.0, 1.0, 0.15).step(0.05).unit("s"),
            quality_option("best").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let dur = p.duration;
        anyhow::ensure!(dur > 0.0, tr("media.err.no_duration"));
        cx.message(tr("media.msg.finding_pauses"));
        let sil = engine_media::analyze::silences(&input.path, v.int("noise") as f64, v.float("min_silence").max(0.1), dur, cx, (0.0, 0.2))?;
        let pad = v.float("padding").clamp(0.0, 1.0);
        let keep = keep_segments(&sil, dur, pad);
        let kept: f64 = keep.iter().map(|(a, b)| b - a).sum();
        if keep.is_empty() || dur - kept < 0.1 {
            let o = super::compress::keep_original(input, cx)?.with_note(tr("media.note.no_silences"));
            return Ok(vec![o]);
        }
        let expr: String = keep.iter().map(|(a, b)| format!("between(t,{a:.3},{b:.3})")).collect::<Vec<_>>().join("+");
        let ext = reencode_ext(&ext_of(input), p.has_video);
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let src = std::path::absolute(&input.path)?;
        let video = p.has_video;
        let (graph, ext) = if video {
            let enc = VideoEnc::quality(edit_encoder(&p, &ext).codec, level(v));
            let ext = output_ext(&ext, enc.codec);
            let mut tail = Vec::new();
            finish_filters(&p, &enc, &mut tail);
            let fps = if p.fps > 0.0 { p.fps } else { 30.0 };
            (
                format!(
                    "[0:{}]fps={fps:.4},select='{expr}',setpts=N/({fps:.4})/TB,{}[vout];[0:a:0]aselect='{expr}',asetpts=N/SR/TB[aout]",
                    p.video_index.unwrap_or(0),
                    tail.join(",")
                ),
                ext,
            )
        } else {
            (format!("[0:a:0]aselect='{expr}',asetpts=N/SR/TB[aout]"), ext)
        };
        std::fs::write(tmp.join("graph.txt"), graph)?;
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let dst = std::path::absolute(out.path())?;
        let mut ff = Ffmpeg::new().input(&src).args(["-/filter_complex", "graph.txt"]);
        if video {
            let enc = VideoEnc::quality(edit_encoder(&p, &ext).codec, level(v));
            let (va, _) = video_args(&p, &enc);
            ff = ff.args(["-map", "[vout]", "-map", "[aout]"]).args(va).args(audio_args(&ext, 192, &p));
        } else {
            ff = ff.args(["-map", "[aout]"]).args(super::trim::audio_reencode(&p, &ext));
        }
        ff.args(["-map_metadata", "0"]).args(mux_args(&ext)).arg(&dst).cwd(&tmp).duration(dur).range(0.2, 1.0).run(cx)?;
        let removed = dur - kept;
        let cuts = keep.len().saturating_sub(1) + usize::from(keep.first().is_some_and(|k| k.0 > 0.0)) + usize::from(keep.last().is_some_and(|k| k.1 < dur));
        let note = tr_args(
            "media.note.silences_removed",
            &[("count", &cuts.to_string()), ("removed", &units::duration(removed)), ("before", &units::duration(dur)), ("after", &units::duration(kept))],
        );
        Ok(vec![Output::new(out.keep()).with_note(note)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps() {
        let k = keep_segments(&[(0.0, 1.0), (3.0, 5.0), (9.0, 10.0)], 10.0, 0.1);
        assert_eq!(k, vec![(0.9, 3.1), (4.9, 9.1)]);
    }
}
