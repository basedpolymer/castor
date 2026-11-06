//! image.compare — before / after: self-contained HTML slider, GIF wipe, side-by-side PNG/JPEG.

use anyhow::Result;
use base64::Engine;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt, Frame};
use crate::{common, ops};

pub struct Compare {
    meta: ToolMeta,
}

pub const LOOKS: &[&str] = &["classic", "minimal", "bold", "dark", "pill", "neon"];

struct Look {
    line: [u8; 4],
    width: f32,
    label_bg: Option<[u8; 4]>,
    label_fg: [u8; 4],
    canvas: [u8; 4],
    labels: bool,
}

fn look(id: &str) -> Look {
    match id {
        "minimal" => Look { line: [255, 255, 255, 230], width: 1.5, label_bg: None, label_fg: [255, 255, 255, 255], canvas: [255, 255, 255, 255], labels: false },
        "bold" => Look { line: [255, 255, 255, 255], width: 6.0, label_bg: Some([0, 0, 0, 200]), label_fg: [255, 255, 255, 255], canvas: [255, 255, 255, 255], labels: true },
        "dark" => Look { line: [20, 20, 20, 255], width: 3.0, label_bg: Some([20, 20, 20, 220]), label_fg: [240, 240, 240, 255], canvas: [24, 24, 26, 255], labels: true },
        "pill" => Look { line: [255, 255, 255, 255], width: 3.0, label_bg: Some([255, 255, 255, 235]), label_fg: [25, 25, 25, 255], canvas: [246, 246, 246, 255], labels: true },
        "neon" => Look { line: [0, 255, 200, 255], width: 3.0, label_bg: Some([10, 10, 30, 210]), label_fg: [0, 255, 200, 255], canvas: [10, 10, 30, 255], labels: true },
        _ => Look { line: [255, 255, 255, 255], width: 3.0, label_bg: Some([0, 0, 0, 140]), label_fg: [255, 255, 255, 255], canvas: [255, 255, 255, 255], labels: true },
    }
}

impl Compare {
    pub fn new() -> Self {
        let mut meta = common::meta(
            "image.compare",
            "HTML · GIF · PNG · JPG",
            &["compare", "before", "after", "slider", "diff", "side by side", "versus", "wipe"],
            "compare",
            ToolStatus::Ready,
            &[],
            false,
        );
        meta.inputs = InputSpec { min: 2, ..common::IMAGE_INPUTS };
        Compare { meta }
    }
}

impl Tool for Compare {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "output",
                vec![Choice::lit("html", "HTML"), Choice::lit("gif", "GIF"), Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG")],
                "html",
            ),
            OptionSpec::choice("look", LOOKS.iter().map(|l| Choice::key(l)).collect(), "classic"),
            OptionSpec::text("before", &castor_core::tr("image.compare.before")),
            OptionSpec::text("after", &castor_core::tr("image.compare.after")),
            OptionSpec::choice("layout", vec![Choice::key("wipe"), Choice::key("side"), Choice::key("stack")], "side")
                .when(Cond::Ne("output", "html")),
            OptionSpec::int("max_width", 320, 8000, 1600).unit("px").advanced(),
        ]
    }

    /// Pairs in order: (1, 2), (3, 4)…
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(inputs.len() >= 2, "{}", castor_core::tr("image.compare.need_two"));
        Ok(inputs.chunks(2).filter(|c| c.len() == 2).map(|c| Job::many(c.to_vec())).collect())
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        anyhow::ensure!(job.inputs.len() == 2, "{}", castor_core::tr("image.compare.need_two"));
        let (a, b) = load_pair(&job.inputs[0], &job.inputs[1], v.int("max_width").max(64) as u32)?;
        cx.progress(0.3);
        let first = &job.inputs[0];
        let lk = look(v.str("look"));
        let (out, bytes) = match v.str("output") {
            "html" => (cx.output_path(Some(first), "html"), html(&a, &b, v)?.into_bytes()),
            "gif" => {
                let frames = wipe_frames(&a, &b, v, &lk);
                (cx.output_path(Some(first), "gif"), codec::encode_gifski(frames, 85, 0, Some(900), cx.cancel_token())?)
            }
            fmt => {
                let img = still(&a, &b, v, &lk);
                let f = if fmt == "jpg" { Fmt::Jpeg } else { Fmt::Png };
                (cx.output_path(Some(first), f.ext()), codec::encode(&DynamicImage::ImageRgba8(img), f, &EncodeOpts { quality: 90, ..Default::default() })?)
            }
        };
        common::write(&out, &bytes)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        // With one selected file, show the look with the file on both sides (left desaturated).
        let Some(f) = input else { return Ok(None) };
        let a = common::load_preview(f, max_side)?.to_rgba8();
        let mut b = a.clone();
        for p in b.pixels_mut() {
            let l = ((p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10) as u8;
            *p = Rgba([l, l, l, p[3]]);
        }
        let lk = look(v.str("look"));
        let img = if v.str("output") == "html" { wipe(&b, &a, 0.5, v, &lk) } else { still(&b, &a, v, &lk) };
        common::preview(&img, max_side, None)
    }
}

/// Both images upright, the second at the size of the first, at most `max_w` wide.
fn load_pair(fa: &FileInfo, fb: &FileInfo, max_w: u32) -> Result<(RgbaImage, RgbaImage)> {
    let a = common::load(fa, DecodeOpts::default())?.image.to_rgba8();
    let b = common::load(fb, DecodeOpts::default())?.image.to_rgba8();
    let (w, h) = if a.width() > max_w { ops::fit_size(a.width(), a.height(), max_w, u32::MAX / 2) } else { a.dimensions() };
    let a = ops::resize_rgba(&a, w, h);
    // Fill the frame of A (same crop logic as a cover fit), centred.
    let s = (w as f64 / b.width() as f64).max(h as f64 / b.height() as f64);
    let (bw, bh) = (((b.width() as f64 * s).ceil() as u32).max(w), ((b.height() as f64 * s).ceil() as u32).max(h));
    let bb = ops::resize_rgba(&b, bw, bh);
    let b = image::imageops::crop_imm(&bb, (bw - w) / 2, (bh - h) / 2, w, h).to_image();
    Ok((a, b))
}

fn label(img: &mut RgbaImage, text: &str, right: bool, lk: &Look) {
    if !lk.labels || text.trim().is_empty() {
        return;
    }
    let size = (img.width().max(img.height()) as f32 / 40.0).clamp(12.0, 48.0);
    let Some(t) = ops::render_text(text, "", size, lk.label_fg, true) else { return };
    let pad = size * 0.5;
    let (bw, bh) = (t.width() as f32 + pad * 2.0, t.height() as f32 + pad * 1.4);
    let x = if right { img.width() as f32 - bw - size } else { size };
    let y = size;
    if let Some(bg) = lk.label_bg {
        ops::fill_rounded_rect(img, x, y, bw, bh, bh / 2.0, bg);
    }
    ops::overlay(img, &t, (x + pad) as i64, (y + (bh - t.height() as f32) / 2.0) as i64, 1.0);
}

/// A at the left of the divider, B at the right; divider at `pos` (0..1).
fn wipe(a: &RgbaImage, b: &RgbaImage, pos: f32, v: &Values, lk: &Look) -> RgbaImage {
    let (w, h) = a.dimensions();
    let split = (w as f32 * pos) as u32;
    let mut out = b.clone();
    let left = image::imageops::crop_imm(a, 0, 0, split.max(1), h).to_image();
    ops::overlay(&mut out, &left, 0, 0, 1.0);
    let lw = (lk.width * w as f32 / 1000.0).max(1.0);
    ops::fill_rounded_rect(&mut out, split as f32 - lw / 2.0, 0.0, lw, h as f32, 0.0, lk.line);
    if v.str("look") != "minimal" {
        let r = (w.min(h) as f32 / 28.0).max(8.0);
        ops::fill_circle(&mut out, split as f32, h as f32 / 2.0, r, lk.line);
        let inner = [lk.canvas[0], lk.canvas[1], lk.canvas[2], 255];
        ops::fill_circle(&mut out, split as f32, h as f32 / 2.0, r * 0.35, inner);
    }
    label(&mut out, v.str("before"), false, lk);
    label(&mut out, v.str("after"), true, lk);
    out
}

fn still(a: &RgbaImage, b: &RgbaImage, v: &Values, lk: &Look) -> RgbaImage {
    let (w, h) = a.dimensions();
    match v.str("layout") {
        "wipe" => wipe(a, b, 0.5, v, lk),
        layout => {
            let gap = (w.max(h) / 80).max(4);
            let stack = layout == "stack";
            let (cw, ch) = if stack { (w + 2 * gap, 2 * h + 3 * gap) } else { (2 * w + 3 * gap, h + 2 * gap) };
            let mut out = RgbaImage::from_pixel(cw, ch, Rgba(lk.canvas));
            let mut pa = a.clone();
            let mut pb = b.clone();
            label(&mut pa, v.str("before"), false, lk);
            label(&mut pb, v.str("after"), false, lk);
            ops::overlay(&mut out, &pa, gap as i64, gap as i64, 1.0);
            if stack {
                ops::overlay(&mut out, &pb, gap as i64, (h + 2 * gap) as i64, 1.0);
            } else {
                ops::overlay(&mut out, &pb, (w + 2 * gap) as i64, gap as i64, 1.0);
            }
            out
        }
    }
}

fn wipe_frames(a: &RgbaImage, b: &RgbaImage, v: &Values, lk: &Look) -> Vec<Frame> {
    let (w, h) = a.dimensions();
    let (sw, sh) = if w > 900 { ops::fit_size(w, h, 900, 900 * h / w.max(1)) } else { (w, h) };
    let (a, b) = (ops::resize_rgba(a, sw, sh), ops::resize_rgba(b, sw, sh));
    let n = 36;
    let mut frames = Vec::new();
    for i in 0..n {
        // Ease in-out from 0.1 to 0.9 and back.
        let t = i as f32 / n as f32;
        let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        let e = tri * tri * (3.0 - 2.0 * tri);
        let pos = 0.08 + 0.84 * e;
        let delay = if i == 0 || i == n / 2 { 700 } else { 50 };
        frames.push(Frame { image: wipe(&a, &b, pos, v, lk), delay_ms: delay });
    }
    frames
}

fn data_uri(img: &RgbaImage) -> Result<String> {
    let alpha = img.pixels().any(|p| p[3] != 255);
    let di = DynamicImage::ImageRgba8(img.clone());
    let (mime, bytes) = if alpha {
        ("image/png", codec::encode(&di, Fmt::Png, &EncodeOpts { png_effort: 1, ..Default::default() })?)
    } else {
        ("image/jpeg", codec::encode(&di, Fmt::Jpeg, &EncodeOpts { quality: 90, ..Default::default() })?)
    };
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn css_rgba(c: [u8; 4]) -> String {
    format!("rgba({},{},{},{:.3})", c[0], c[1], c[2], c[3] as f32 / 255.0)
}

/// Self-contained HTML page: both images inline, a draggable divider, keyboard accessible.
fn html(a: &RgbaImage, b: &RgbaImage, v: &Values) -> Result<String> {
    let lk = look(v.str("look"));
    let (w, h) = a.dimensions();
    let (ua, ub) = (data_uri(a)?, data_uri(b)?);
    let label_css = match lk.label_bg {
        Some(bg) => format!("background:{};color:{};border-radius:999px;padding:.3em .9em;", css_rgba(bg), css_rgba(lk.label_fg)),
        None => format!("color:{};text-shadow:0 1px 3px rgba(0,0,0,.6);", css_rgba(lk.label_fg)),
    };
    let labels = if lk.labels {
        format!(
            "<span class=\"lab l\">{}</span><span class=\"lab r\">{}</span>",
            ops::xml_escape(v.str("before")),
            ops::xml_escape(v.str("after"))
        )
    } else {
        String::new()
    };
    let handle = if v.str("look") == "minimal" { "" } else { "<div class=\"knob\"></div>" };
    Ok(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="generator" content="Castor"><title>{before} / {after}</title>
<style>
html,body{{margin:0;background:{canvas};font:600 15px/1.2 system-ui,-apple-system,"Segoe UI",sans-serif}}
.cmp{{position:relative;max-width:{w}px;margin:24px auto;aspect-ratio:{w}/{h};user-select:none;touch-action:none;overflow:hidden;border-radius:6px}}
.cmp img{{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;pointer-events:none}}
.cmp .a{{clip-path:inset(0 calc(100% - var(--p)) 0 0)}}
.line{{position:absolute;top:0;bottom:0;left:var(--p);width:{lw}px;margin-left:-{lwh}px;background:{line}}}
.knob{{position:absolute;top:50%;left:50%;width:42px;height:42px;margin:-21px 0 0 -21px;border-radius:50%;background:{line};box-shadow:0 2px 8px rgba(0,0,0,.35)}}
.lab{{position:absolute;top:14px;{label_css}}}
.lab.l{{left:14px}}.lab.r{{right:14px}}
input{{position:absolute;inset:0;width:100%;height:100%;opacity:0;cursor:ew-resize;margin:0}}
</style></head><body>
<div class="cmp" style="--p:50%">
<img class="b" src="{ub}" alt="{after}"><img class="a" src="{ua}" alt="{before}">
<div class="line">{handle}</div>{labels}
<input type="range" min="0" max="1000" value="500" aria-label="{before} / {after}">
</div>
<script>
document.querySelectorAll('.cmp').forEach(function(c){{var i=c.querySelector('input');i.addEventListener('input',function(){{c.style.setProperty('--p',(i.value/10)+'%')}});}});
</script></body></html>
"#,
        before = ops::xml_escape(v.str("before")),
        after = ops::xml_escape(v.str("after")),
        canvas = css_rgba(lk.canvas),
        line = css_rgba(lk.line),
        lw = lk.width,
        lwh = lk.width / 2.0,
    ))
}
