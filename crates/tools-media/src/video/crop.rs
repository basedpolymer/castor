//! `video.crop`: aspect-ratio presets with an anchor, exact size, or black-bar removal.

use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};

use crate::common::*;

pub struct Crop {
    meta: ToolMeta,
}

impl Crop {
    pub fn new() -> Self {
        Crop {
            meta: meta(
                "video.crop",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["aspect", "ratio", "vertical", "square", "reels", "tiktok", "shorts", "black bars", "letterbox"],
                "cropped",
                true,
            ),
        }
    }
}

/// Crop rectangle (w, h, x, y) of ratio `rw:rh` inside `w × h`, placed by `anchor`.
pub fn aspect_rect(w: u32, h: u32, rw: f64, rh: f64, anchor: Anchor) -> (u32, u32, u32, u32) {
    let r = rw / rh;
    let (cw, ch) = if w as f64 / h as f64 > r {
        (((h as f64 * r) / 2.0).floor() as u32 * 2, h / 2 * 2)
    } else {
        (w / 2 * 2, ((w as f64 / r) / 2.0).floor() as u32 * 2)
    };
    place(w, h, cw.max(2), ch.max(2), anchor)
}

fn place(w: u32, h: u32, cw: u32, ch: u32, anchor: Anchor) -> (u32, u32, u32, u32) {
    let (gx, gy) = anchor.grid();
    let x = match gx {
        0 => 0,
        1 => (w - cw) / 2,
        _ => w - cw,
    };
    let y = match gy {
        0 => 0,
        1 => (h - ch) / 2,
        _ => h - ch,
    };
    (cw, ch, x / 2 * 2, y / 2 * 2)
}

impl Tool for Crop {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "mode",
                vec![
                    Choice::lit("9:16", "9:16"),
                    Choice::lit("1:1", "1:1"),
                    Choice::lit("4:5", "4:5"),
                    Choice::lit("16:9", "16:9"),
                    Choice::lit("4:3", "4:3"),
                    Choice::lit("21:9", "21:9"),
                    Choice::key("auto"),
                    Choice::key("custom"),
                ],
                "9:16",
            ),
            OptionSpec::size("size", 1080, 1080).when(Cond::Eq("mode", "custom")),
            OptionSpec::anchor("anchor", Anchor::Center).when(Cond::Ne("mode", "auto")),
            quality_option("best"),
            hardware_option(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let (w, h) = p.display_size();
        let rect = match v.str("mode") {
            "auto" => {
                cx.message(tr("media.msg.finding_bars"));
                match engine_media::analyze::black_bars(&input.path, w, h, p.duration, cx)? {
                    Some(r) => r,
                    None => {
                        let o = super::compress::keep_original(input, cx)?.with_note(tr("media.note.no_bars"));
                        return Ok(vec![o]);
                    }
                }
            }
            "custom" => {
                let (cw, ch) = v.size("size");
                anyhow::ensure!(cw >= 2 && ch >= 2, tr("media.err.bad_size"));
                place(w, h, cw.min(w) / 2 * 2, ch.min(h) / 2 * 2, v.anchor("anchor"))
            }
            m => {
                let (a, b) = m.split_once(':').unwrap_or(("9", "16"));
                aspect_rect(w, h, a.parse().unwrap_or(9.0), b.parse().unwrap_or(16.0), v.anchor("anchor"))
            }
        };
        let (cw, ch, x, y) = rect;
        let f = format!("crop={cw}:{ch}:{x}:{y}");
        super::resize::reencode_with_filter(input, &p, v, cx, vec![f], level(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rects() {
        assert_eq!(aspect_rect(1920, 1080, 9.0, 16.0, Anchor::Center), (606, 1080, 656, 0));
        assert_eq!(aspect_rect(1920, 1080, 1.0, 1.0, Anchor::Left), (1080, 1080, 0, 0));
        assert_eq!(aspect_rect(1080, 1920, 16.0, 9.0, Anchor::Bottom), (1080, 606, 0, 1314));
    }
}
