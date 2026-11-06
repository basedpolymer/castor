//! `audio.denoise`: RNNoise (ffmpeg `arnndn`, speech model fetched once) for voice, or an FFT
//! denoiser (`afftdn`) for general steady noise. Works on audio and on video soundtracks.

use castor_core::download::{self, Asset};
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, units};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Denoise {
    meta: ToolMeta,
}

impl Denoise {
    pub fn new() -> Self {
        Denoise {
            meta: meta(
                "audio.denoise",
                Family::Audio,
                InputSpec::kinds(AV),
                "MP3 · M4A · WAV · FLAC · MP4",
                &["noise", "hiss", "hum", "background noise", "clean", "rnnoise", "voice", "podcast", "fan"],
                "clean",
                false,
            ),
        }
    }
}

/// RNNoise model "somnolent-hogwash" (speech in recording noise) from GregorR/rnnoise-models.
/// The repository states the models are not creative work and not subject to copyright.
pub static RNNOISE_MODEL: Asset = Asset {
    id: "rnnoise/sh.rnnn",
    url: "https://raw.githubusercontent.com/GregorR/rnnoise-models/3eee541a283fd3b8f81b85b1748e3b9ccbefa04d/somnolent-hogwash-2018-09-01/sh.rnnn",
    sha256: "70bb6685eb0c2a1d18e2918dca3fbfbd39317010b1802eb1b6ea73a92f3fdec0",
    size: 297646,
    license: "public-domain",
};

impl Tool for Denoise {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("method", vec![Choice::key("voice"), Choice::key("general")], "voice"),
            OptionSpec::int("strength", 1, 10, 6).slider(),
            OptionSpec::toggle("highpass", true).advanced(),
        ]
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        if v.str("method") == "voice" { vec![RNNOISE_MODEL.clone()] } else { Vec::new() }
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let s = v.int("strength").clamp(1, 10) as f64;
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let mut chain = Vec::new();
        if v.bool("highpass") {
            chain.push("highpass=f=70".to_string());
        }
        let voice = v.str("method") == "voice" && engine_media::has_filter("arnndn");
        if voice {
            let model = download::ensure(&RNNOISE_MODEL, &cx.cancel_token(), &|_, _| {})?;
            std::fs::copy(&model, tmp.join("model.rnnn"))?;
            // mix: 1 = fully denoised; lower strengths blend some of the original back in.
            chain.push(format!("arnndn=m=model.rnnn:mix={:.2}", 0.5 + s * 0.05));
        } else {
            chain.push(format!("afftdn=nr={:.0}:nf=-40:tn=1", 6.0 + s * 2.4));
        }
        let sr = if p.sample_rate > 0 { p.sample_rate } else { 48000 };
        let af = format!("{},aresample={sr}", chain.join(","));
        let ext = reencode_ext(&ext_of(input), p.has_video);
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let dst = std::path::absolute(out.path())?;
        let mut ff = Ffmpeg::new().input(std::path::absolute(&input.path)?);
        if p.has_video {
            ff = ff.args(stream_map(&p, &ext, true, false)).args(["-map", "0:a:0", "-c:v", "copy", "-af", &af]).args(audio_args(&ext, 192, &p));
        } else {
            ff = ff.args(super::convert_args_same(&p, &ext)).args(["-af", &af]);
        }
        ff.args(mux_args(&ext)).arg(&dst).cwd(&tmp).duration(p.duration).run(cx)?;
        let size = size_of(out.path());
        let how = tr(if voice { "media.note.rnnoise" } else { "media.note.fft" });
        Ok(vec![Output::new(out.keep()).with_note(format!("{how} · {}", units::bytes(size)))])
    }
}
