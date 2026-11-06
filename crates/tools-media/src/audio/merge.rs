//! `audio.merge`: stitch files end to end (optional crossfade) or layer them (per-file volume).

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, units};
use engine_media::Ffmpeg;

use super::convert::{audio_encode, format_choices};
use crate::common::*;
use crate::video::extract_audio::format_ext;

pub struct Merge {
    meta: ToolMeta,
}

impl Merge {
    pub fn new() -> Self {
        Merge {
            meta: meta(
                "audio.merge",
                Family::Audio,
                InputSpec::kinds(AV).min(2),
                "MP3 · M4A · WAV · FLAC",
                &["join", "combine", "concatenate", "mix", "layer", "overlay", "stitch", "playlist", "crossfade"],
                "merged",
                true,
            ),
        }
    }
}

/// Per-file volumes from "100, 80, 50" (percent). Missing entries are 100 %.
pub fn parse_volumes(s: &str, n: usize) -> Vec<f64> {
    let mut v: Vec<f64> = s
        .split([',', ';', ' '])
        .filter(|x| !x.trim().is_empty())
        .map(|x| x.trim().trim_end_matches('%').parse::<f64>().unwrap_or(100.0).clamp(0.0, 400.0) / 100.0)
        .collect();
    v.resize(n, 1.0);
    v
}

impl Tool for Merge {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut formats = vec![Choice::key("auto")];
        formats.extend(format_choices());
        vec![
            OptionSpec::choice("mode", vec![Choice::key("stitch"), Choice::key("layer")], "stitch"),
            OptionSpec::float("crossfade", 0.0, 10.0, 0.0).step(0.5).unit("s").when(Cond::Eq("mode", "stitch")),
            OptionSpec::text("volumes", "").placeholder("media.ph.volumes").when(Cond::Eq("mode", "layer")),
            OptionSpec::choice("format", formats, "auto"),
            OptionSpec::choice(
                "bitrate",
                vec![Choice::lit("128", "128 kbps"), Choice::lit("192", "192 kbps"), Choice::lit("256", "256 kbps"), Choice::lit("320", "320 kbps")],
                "192",
            )
            .advanced(),
        ]
    }
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(inputs.len() >= 2, tr("media.err.need_two"));
        Ok(vec![Job::many(inputs.to_vec())])
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        anyhow::ensure!(job.inputs.len() >= 2, tr("media.err.need_two"));
        let mut ps = Vec::new();
        for f in &job.inputs {
            let p = probe(f).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            need_audio(&p).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            ps.push(p);
        }
        let n = ps.len();
        let first = &job.inputs[0];
        let fmt = match v.str("format") {
            "auto" => match first.extension().as_str() {
                e @ ("mp3" | "wav" | "flac" | "ogg" | "opus" | "aiff") => e.to_string(),
                _ => "m4a".to_string(),
            },
            f => f.to_string(),
        };
        let kbps: u32 = v.str("bitrate").parse().unwrap_or(192);
        // Highest sample rate among inputs (capped at 48 kHz for lossy formats).
        let mut sr = ps.iter().map(|p| p.sample_rate).max().unwrap_or(48000).max(8000);
        if matches!(fmt.as_str(), "mp3" | "m4a" | "ogg" | "opus") {
            sr = sr.min(48000);
        }
        let norm = format!("aformat=sample_fmts=fltp:sample_rates={sr}:channel_layouts=stereo");
        let mut fc = String::new();
        let (graph_out, total) = if v.str("mode") == "layer" {
            let vols = parse_volumes(v.str("volumes"), n);
            for (i, vol) in vols.iter().enumerate() {
                fc.push_str(&format!("[{i}:a:0]{norm},volume={vol:.3}[a{i}];"));
            }
            for i in 0..n {
                fc.push_str(&format!("[a{i}]"));
            }
            fc.push_str(&format!("amix=inputs={n}:duration=longest:dropout_transition=0:normalize=0,alimiter=limit=0.97:level=false[out]"));
            ("[out]", ps.iter().map(|p| p.duration).fold(0.0, f64::max))
        } else {
            let xf = v.float("crossfade").clamp(0.0, 10.0);
            for i in 0..n {
                fc.push_str(&format!("[{i}:a:0]{norm}[a{i}];"));
            }
            if xf > 0.0 {
                let mut prev = "a0".to_string();
                for i in 1..n {
                    let d = xf.min(ps[i - 1].duration / 2.0).min(ps[i].duration / 2.0).max(0.01);
                    let lab = format!("x{i}");
                    fc.push_str(&format!("[{prev}][a{i}]acrossfade=d={d:.3}:c1=tri:c2=tri[{lab}];"));
                    prev = lab;
                }
                fc.push_str(&format!("[{prev}]anull[out]"));
            } else {
                for i in 0..n {
                    fc.push_str(&format!("[a{i}]"));
                }
                fc.push_str(&format!("concat=n={n}:v=0:a=1[out]"));
            }
            ("[out]", ps.iter().map(|p| p.duration).sum::<f64>())
        };
        let script = cx.tmp_dir().join("graph.txt");
        std::fs::write(&script, &fc)?;
        let ext = format_ext(&fmt).to_string();
        let out = Partial::new(cx.output_path(Some(first), &ext));
        let mut ff = Ffmpeg::new();
        for f in &job.inputs {
            ff = ff.input(&f.path);
        }
        // Encoder settings: reuse the single-file encoder args minus their stream mapping.
        let mut enc = audio_encode(&ps[0], &fmt, kbps, None, Some(2));
        strip_maps(&mut enc);
        ff.arg("-/filter_complex").arg(&script).args(["-map", graph_out, "-vn"]).args(enc).arg(out.path()).duration(total).run(cx)?;
        let size = size_of(out.path());
        Ok(vec![Output::new(out.keep()).with_note(format!("{n} → 1 · {} · {}", units::duration(total), units::bytes(size)))])
    }
}

/// Remove `-map x`, `-c:v copy`, `-disposition…`, `-vn`, `-map_metadata 0` pairs from encoder args.
fn strip_maps(a: &mut Vec<String>) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "-map" | "-c:v" | "-disposition:v:0" | "-map_metadata" => i += 2,
            "-vn" | "-sn" | "-dn" => i += 1,
            _ => {
                out.push(a[i].clone());
                i += 1;
            }
        }
    }
    *a = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volumes() {
        assert_eq!(parse_volumes("100, 50%", 3), vec![1.0, 0.5, 1.0]);
        let mut a: Vec<String> = ["-map", "0:a:0", "-vn", "-c:a", "aac", "-b:a", "192k"].iter().map(|s| s.to_string()).collect();
        strip_maps(&mut a);
        assert_eq!(a, ["-c:a", "aac", "-b:a", "192k"]);
    }
}
