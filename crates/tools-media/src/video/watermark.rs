//! `video.watermark`: an image (logo) or a line of text, anchored, with margin and opacity.

use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Watermark {
    meta: ToolMeta,
}

impl Watermark {
    pub fn new() -> Self {
        Watermark {
            meta: meta(
                "video.watermark",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["logo", "brand", "overlay", "copyright", "stamp", "text"],
                "watermarked",
                true,
            ),
        }
    }
}

/// Overlay position expressions for an anchor and a margin in pixels.
/// `W/H` = main size, `w/h` = overlay size (names as in the overlay/drawtext filters).
pub fn position(anchor: Anchor, m: u32, w: &str, h: &str, big_w: &str, big_h: &str) -> (String, String) {
    let (gx, gy) = anchor.grid();
    let x = match gx {
        0 => format!("{m}"),
        1 => format!("({big_w}-{w})/2"),
        _ => format!("{big_w}-{w}-{m}"),
    };
    let y = match gy {
        0 => format!("{m}"),
        1 => format!("({big_h}-{h})/2"),
        _ => format!("{big_h}-{h}-{m}"),
    };
    (x, y)
}

impl Tool for Watermark {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("kind", vec![Choice::key("image"), Choice::key("text")], "image"),
            OptionSpec::file("image", &["png", "jpg", "jpeg", "webp", "bmp", "gif"]).when(Cond::Eq("kind", "image")),
            OptionSpec::text("text", "© Castor").when(Cond::Eq("kind", "text")),
            OptionSpec::font("font", crate::fonts::DEFAULT_FAMILY).when(Cond::Eq("kind", "text")),
            OptionSpec::color("color", "#ffffff").when(Cond::Eq("kind", "text")).common(),
            OptionSpec::int("size", 1, 100, 15).slider().unit("%"),
            OptionSpec::anchor("position", Anchor::BottomRight).common(),
            OptionSpec::int("margin", 0, 25, 3).slider().unit("%"),
            OptionSpec::int("opacity", 5, 100, 80).slider().unit("%").common(),
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
        let (dw, dh) = p.display_size();
        let margin = ((dw.min(dh) as f64) * v.int("margin") as f64 / 100.0).round() as u32;
        let opacity = v.int("opacity").clamp(5, 100) as f64 / 100.0;
        let size = v.int("size").clamp(1, 100) as f64 / 100.0;
        let anchor = v.anchor("position");
        let ext = ext_of(input);
        let enc = VideoEnc::quality(edit_encoder(&p, &ext).codec, level(v));
        let ext = output_ext(&ext, enc.codec);
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let src = std::path::absolute(&input.path)?;
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let dst = std::path::absolute(out.path())?;
        let mut tail = Vec::new();
        finish_filters(&p, &enc, &mut tail);
        let (va, _) = video_args(&p, &enc);
        let vi = p.video_index.unwrap_or(0);
        let mut ff = Ffmpeg::new().input(&src);
        let graph = if v.str("kind") == "text" {
            let text = v.str("text").trim().to_string();
            anyhow::ensure!(!text.is_empty(), tr("media.err.no_text"));
            let font = crate::fonts::prepare(v.str("font"), false, &tmp)?;
            std::fs::write(tmp.join("wm.txt"), text.as_bytes())?;
            let [r, g, b, a] = v.color("color");
            let alpha = (a as f64 / 255.0) * opacity;
            let fs = ((dh as f64) * size * 0.5).max(8.0).round();
            let (x, y) = position(anchor, margin, "text_w", "text_h", "w", "h");
            format!(
                "[0:{vi}]drawtext=fontfile={}:textfile=wm.txt:expansion=none:fontsize={fs}:fontcolor=0x{r:02x}{g:02x}{b:02x}@{alpha:.3}:shadowx=2:shadowy=2:shadowcolor=black@{:.3}:x={x}:y={y},{}[vout]",
                font.file,
                alpha * 0.4,
                tail.join(",")
            )
        } else {
            let img = v.path("image").ok_or_else(|| anyhow::anyhow!(tr("media.err.no_image")))?;
            anyhow::ensure!(img.is_file(), tr("media.err.no_image"));
            let ip = engine_media::probe(img).map_err(|_| anyhow::anyhow!(tr("media.err.bad_image")))?;
            anyhow::ensure!(ip.width > 0 && ip.height > 0, tr("media.err.bad_image"));
            let w = ((dw as f64 * size).round() as u32).max(2);
            let h = ((w as f64 * ip.height as f64 / ip.width as f64).round() as u32).max(2);
            let (x, y) = position(anchor, margin, "overlay_w", "overlay_h", "main_w", "main_h");
            ff = ff.args(["-loop", "1"]).input(std::path::absolute(img)?);
            format!(
                "[1:v]scale={w}:{h}:flags=lanczos,format=rgba,colorchannelmixer=aa={opacity:.3}[wm];[0:{vi}][wm]overlay=x={x}:y={y}:shortest=1:format=auto,{}[vout]",
                tail.join(",")
            )
        };
        let script = tmp.join("graph.txt");
        std::fs::write(&script, graph)?;
        ff.arg("-/filter_complex")
            .arg("graph.txt")
            .args(["-map", "[vout]", "-map", "0:a?"])
            .args(va)
            .args(edit_audio(&p, &ext, false))
            .args(["-map_metadata", "0"])
            .args(mux_args(&ext))
            .arg(&dst)
            .cwd(&tmp)
            .duration(p.duration)
            .run(cx)?;
        let s = size_of(out.path());
        Ok(vec![Output::new(out.keep()).with_note(castor_core::units::bytes(s))])
    }
}
