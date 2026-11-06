//! `video.split` / `audio.split`: N equal parts, fixed length, scene changes (video) or pauses
//! (audio).

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args, units};
use engine_media::{Ffmpeg, Probe};

use crate::common::*;

pub struct Split {
    meta: ToolMeta,
    audio: bool,
}

impl Split {
    pub fn video() -> Self {
        Split {
            meta: meta(
                "video.split",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["cut", "parts", "segments", "chunks", "scenes", "chapters"],
                "part",
                true,
            ),
            audio: false,
        }
    }
    pub fn audio() -> Self {
        Split {
            meta: meta(
                "audio.split",
                Family::Audio,
                InputSpec::kinds(AUDIO),
                "MP3 · M4A · WAV · FLAC",
                &["cut", "parts", "segments", "chunks", "pauses", "silence", "tracks"],
                "part",
                false,
            ),
            audio: true,
        }
    }
}

/// Segments `(start, end)` from cut points, dropping cuts that would make parts shorter than
/// `min_len`.
pub fn segments_from_cuts(cuts: &[f64], duration: f64, min_len: f64) -> Vec<(f64, f64)> {
    let mut pts: Vec<f64> = Vec::new();
    let mut last = 0.0;
    let mut sorted: Vec<f64> = cuts.iter().copied().filter(|c| *c > 0.0 && *c < duration).collect();
    sorted.sort_by(f64::total_cmp);
    for c in sorted {
        if c - last >= min_len && duration - c >= min_len {
            pts.push(c);
            last = c;
        }
    }
    let mut segs = Vec::new();
    let mut a = 0.0;
    for c in pts {
        segs.push((a, c));
        a = c;
    }
    segs.push((a, duration));
    segs
}

impl Tool for Split {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut by = vec![Choice::key("parts"), Choice::key("length")];
        by.push(if self.audio { Choice::key("pauses") } else { Choice::key("scenes") });
        let mut o = vec![
            OptionSpec::choice("by", by, "parts"),
            OptionSpec::int("parts", 2, 500, 2).when(Cond::Eq("by", "parts")),
            OptionSpec::text("length", "60").placeholder("media.ph.time").when(Cond::Eq("by", "length")),
        ];
        if self.audio {
            o.push(OptionSpec::int("noise", -70, -10, -35).slider().unit("dB").when(Cond::Eq("by", "pauses")));
            o.push(OptionSpec::float("pause", 0.2, 10.0, 1.0).step(0.1).unit("s").when(Cond::Eq("by", "pauses")));
            o.push(OptionSpec::float("min_part", 0.0, 3600.0, 10.0).unit("s").when(Cond::Eq("by", "pauses")));
        } else {
            o.push(OptionSpec::int("sensitivity", 1, 100, 60).slider().when(Cond::Eq("by", "scenes")));
            o.push(OptionSpec::float("min_part", 0.0, 3600.0, 2.0).unit("s").when(Cond::Eq("by", "scenes")).advanced());
            o.push(cut_option());
        }
        o
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        if self.audio { need_audio(&p)? } else { need_video(&p)? }
        let dur = p.duration;
        anyhow::ensure!(dur > 0.0, tr("media.err.no_duration"));
        let (segs, analysis) = match v.str("by") {
            "length" => {
                let len = time_opt(v, "length")?.filter(|l| *l > 0.0).ok_or_else(|| anyhow::anyhow!(tr("media.err.bad_length")))?;
                anyhow::ensure!(dur / len <= 2000.0, tr("media.err.too_many_parts"));
                let cuts: Vec<f64> = (1..).map(|i| i as f64 * len).take_while(|c| *c < dur - 0.05).collect();
                (segments_from_cuts(&cuts, dur, 0.05), 0.0)
            }
            "scenes" => {
                // Sensitivity 1..100 → scdet threshold 30..2 (higher sensitivity = more cuts).
                let s = v.int("sensitivity").clamp(1, 100) as f64;
                let thr = 30.0 - (s - 1.0) * 28.0 / 99.0;
                cx.message(tr("media.msg.finding_scenes"));
                let cuts = engine_media::analyze::scenes(&input.path, thr, v.float("min_part").max(0.2), dur, cx, (0.0, 0.3))?;
                (segments_from_cuts(&cuts, dur, v.float("min_part").max(0.2)), 0.3)
            }
            "pauses" => {
                cx.message(tr("media.msg.finding_pauses"));
                let sil = engine_media::analyze::silences(&input.path, v.int("noise") as f64, v.float("pause").max(0.1), dur, cx, (0.0, 0.2))?;
                let cuts: Vec<f64> = sil.iter().filter(|(a, b)| *a > 0.01 && *b < dur - 0.01).map(|(a, b)| (a + b) / 2.0).collect();
                (segments_from_cuts(&cuts, dur, v.float("min_part").max(0.1)), 0.2)
            }
            _ => {
                let n = v.int("parts").clamp(2, 500) as usize;
                let cuts: Vec<f64> = (1..n).map(|i| dur * i as f64 / n as f64).collect();
                (segments_from_cuts(&cuts, dur, 0.0), 0.0)
            }
        };
        if segs.len() < 2 {
            anyhow::bail!(tr("media.err.nothing_to_split"));
        }
        write_parts(input, &p, &segs, self.audio || v.str("cut") != "fast", self.audio, cx, analysis)
    }
}

/// Write each `(start, end)` segment as its own file.
pub fn write_parts(
    input: &castor_core::FileInfo,
    p: &Probe,
    segs: &[(f64, f64)],
    exact: bool,
    audio_only: bool,
    cx: &RunCx,
    base: f32,
) -> Result<Vec<Output>> {
    let ext = if exact { reencode_ext(&ext_of(input), !audio_only && p.has_video) } else { ext_of(input) };
    let total: f64 = segs.iter().map(|(a, b)| b - a).sum();
    let digits = segs.len().to_string().len().max(2);
    let mut done = 0.0;
    let mut outs = Vec::new();
    for (i, (a, b)) in segs.iter().enumerate() {
        cx.check_cancel()?;
        cx.message(tr_args("media.msg.part", &[("n", &(i + 1).to_string()), ("total", &segs.len().to_string())]));
        let len = b - a;
        let lo = base + (1.0 - base) * (done / total) as f32;
        let hi = base + (1.0 - base) * ((done + len) / total) as f32;
        let name = format!("{}-{}-{:0digits$}.{ext}", input.stem(), cx.suffix(), i + 1);
        let out = Partial::new(cx.output_named(&name));
        let mut ff = Ffmpeg::new().args(["-ss", &format!("{a:.3}")]).input(&input.path).args(["-t", &format!("{len:.3}")]);
        if !exact {
            ff = ff.args(["-map", "0:v?", "-map", "0:a?", "-map", "0:s?", "-c", "copy", "-avoid_negative_ts", "make_zero"]);
        } else if audio_only || !p.has_video {
            ff = ff.args(["-map", "0:a", "-vn"]).args(super::trim::audio_reencode(p, &ext));
        } else {
            let enc = edit_encoder(p, &ext);
            let mut vf = Vec::new();
            finish_filters(p, &enc, &mut vf);
            let (va, _) = video_args(p, &enc);
            ff = ff.args(stream_map(p, &ext, true, true)).args(["-vf", &vf.join(",")]).args(va).args(edit_audio(p, &ext, false));
        }
        ff.args(["-map_metadata", "0", "-map_chapters", "-1"])
            .args(mux_args(&ext))
            .arg(out.path())
            .duration(len)
            .range(lo, hi)
            .run(cx)?;
        done += len;
        let note = format!("{} → {}", units::timecode(*a), units::timecode(*b));
        outs.push(Output::new(out.keep()).with_note(note));
    }
    Ok(outs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segs() {
        assert_eq!(segments_from_cuts(&[5.0, 10.0], 15.0, 0.0), vec![(0.0, 5.0), (5.0, 10.0), (10.0, 15.0)]);
        assert_eq!(segments_from_cuts(&[1.0, 5.0, 14.5], 15.0, 2.0), vec![(0.0, 5.0), (5.0, 15.0)]);
    }
}
