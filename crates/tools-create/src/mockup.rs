//! `create.mockup`: put a screenshot (or a screen recording) into a generic device — phone,
//! tablet, laptop, monitor or browser window, all drawn here (no brand shapes) — on one of 22
//! scenes (painted gradients, desk surfaces, studio, solid, transparent), flat or angled.
//! Images → PNG/JPG; videos → MP4 through ffmpeg (overlay, perspective for angled frames).

use std::path::Path;

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};
use image::RgbaImage;

use crate::common::{self, xml};
use crate::render::{png, raster, style};

pub struct MockupTool {
    meta: ToolMeta,
}

impl MockupTool {
    pub fn new() -> Self {
        let mut m = common::meta(
            "create.mockup",
            InputSpec::kinds(&[FileKind::Image, FileKind::Svg, FileKind::Video]).min(0),
            "PNG · JPG · MP4",
            &["mockup", "device", "phone", "laptop", "screenshot", "frame", "presentation", "app store", "browser"],
            "mockup",
        );
        m.requires = &[];
        MockupTool { meta: common::serial(m) }
    }
}

pub const DEVICES: &[&str] = &["phone", "tablet", "laptop", "desktop", "browser"];

impl Tool for MockupTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("device", DEVICES.iter().map(|d| Choice::key(d)).collect(), "laptop"),
            OptionSpec::choice("finish", ["graphite", "silver", "white"].map(Choice::key).to_vec(), "graphite"),
            OptionSpec::choice("scene", raster::SCENES.iter().map(|s| Choice::key(s)).collect(), "castor"),
            OptionSpec::color("scene_color", "#ffeca4").when(Cond::Eq("scene", "custom")),
            OptionSpec::choice("angle", ["flat", "left", "right", "low"].map(Choice::key).to_vec(), "flat"),
            OptionSpec::choice("shadow", ["none", "soft", "strong"].map(Choice::key).to_vec(), "soft"),
            OptionSpec::choice("fit", ["fill", "fit"].map(Choice::key).to_vec(), "fill"),
            OptionSpec::text("url", "basedpolymer.eu/castor").when(Cond::Eq("device", "browser")),
            OptionSpec::size("size", 1600, 1200),
            OptionSpec::int("padding", 0, 40, 10).slider().unit("%"),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG")], "png"),
            OptionSpec::int("seed", 0, 9999, 2).advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        Ok(if inputs.is_empty() { vec![Job::empty()] } else { inputs.iter().cloned().map(Job::single).collect() })
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let (w, h) = v.size("size");
        let (pw, ph) = crate::gradient::fit(w.max(64), h.max(64), max_side);
        let screen = match input {
            Some(f) if f.kind == FileKind::Video => Some(engine_media::frame_png(&f.path, 1.0, 1600, None).ok().and_then(|p| image::load_from_memory(&p).ok()).map(|i| i.to_rgba8())),
            Some(f) => Some(Some(png::load_image(&f.path, 2400)?)),
            None => None,
        }
        .flatten();
        let img = compose(v, screen.as_ref(), pw, ph)?.image;
        let cap = format!("{} · {}", tr(&format!("create.mockup.opt.device.{}", v.str("device"))), common::px(w, h));
        Ok(Some(common::preview(png::encode_rgba(&img)?, common::figure(&cap))))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let (w, h) = v.size("size");
        anyhow::ensure!((64..=8000).contains(&w) && (64..=8000).contains(&h), "{}", tr("create.common.err.size"));
        let input = job.inputs.first();
        if let Some(f) = input.filter(|f| f.kind == FileKind::Video) {
            return video(f, v, cx);
        }
        let screen = match input {
            Some(f) => Some(png::load_image(&f.path, 4096)?),
            None => None,
        };
        cx.progress(0.2);
        let c = compose(v, screen.as_ref(), w, h)?;
        cx.progress(0.8);
        let fmt = v.str("format");
        let out = cx.output_path(input, fmt);
        if fmt == "jpg" {
            crate::gradient::save_rgb(&png::flatten(&c.image, [255, 255, 255]), &out, "jpg")?;
        } else {
            common::write(&out, &png::encode_rgba(&c.image)?)?;
        }
        Ok(vec![Output::new(out).with_note(common::px(w, h))])
    }
}

/// A device drawing in its own units.
struct Device {
    w: f32,
    h: f32,
    /// Screen rectangle and corner radius.
    sx: f32,
    sy: f32,
    sw: f32,
    sh: f32,
    sr: f32,
    /// Everything under the screen (body, bezel).
    under: String,
    /// Everything over the screen (camera, browser chrome over nothing).
    over: String,
}

fn device(kind: &str, finish: &str, url: &str) -> Device {
    let (body, edge, bezel) = match finish {
        "silver" => ("#d6d6d9", "#a9a9ad", "#0f0f10"),
        "white" => ("#f4f2ee", "#cfccc5", "#fcfbf9"),
        _ => ("#2b2b2e", "#4a4a4f", "#0b0b0c"),
    };
    let rr = |x: f32, y: f32, w: f32, h: f32, r: f32, fill: &str| format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{fill}\"/>");
    let grad = |id: &str| {
        format!(
            "<defs><linearGradient id=\"{id}\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{edge}\"/><stop offset=\"0.5\" stop-color=\"{body}\"/><stop offset=\"1\" stop-color=\"{edge}\"/></linearGradient></defs>"
        )
    };
    match kind {
        "phone" => Device {
            w: 432.0,
            h: 860.0,
            sx: 28.0,
            sy: 22.0,
            sw: 376.0,
            sh: 816.0,
            sr: 46.0,
            under: format!(
                "{}{}{}{}{}{}",
                grad("b"),
                rr(0.0, 250.0, 8.0, 70.0, 3.0, edge),
                rr(0.0, 340.0, 8.0, 70.0, 3.0, edge),
                rr(424.0, 290.0, 8.0, 110.0, 3.0, edge),
                format!("<rect x=\"6\" y=\"0\" width=\"420\" height=\"860\" rx=\"68\" fill=\"url(#b)\"/>"),
                rr(16.0, 10.0, 400.0, 840.0, 58.0, bezel)
            ),
            over: "<circle cx=\"216\" cy=\"48\" r=\"9\" fill=\"#050505\"/><circle cx=\"216\" cy=\"48\" r=\"4\" fill=\"#1d2533\"/>".into(),
        },
        "tablet" => Device {
            w: 980.0,
            h: 1300.0,
            sx: 42.0,
            sy: 42.0,
            sw: 896.0,
            sh: 1216.0,
            sr: 18.0,
            under: format!("{}<rect width=\"980\" height=\"1300\" rx=\"60\" fill=\"url(#b)\"/>{}", grad("b"), rr(10.0, 10.0, 960.0, 1280.0, 52.0, bezel)),
            over: "<circle cx=\"490\" cy=\"25\" r=\"6\" fill=\"#1d2533\"/>".into(),
        },
        "laptop" => Device {
            w: 1500.0,
            h: 900.0,
            sx: 134.0,
            sy: 34.0,
            sw: 1232.0,
            sh: 772.0,
            sr: 6.0,
            under: format!(
                "{}<path d=\"M126 840V26a26 26 0 0 1 26-26H1348a26 26 0 0 1 26 26V840Z\" fill=\"url(#b)\"/>{}<path d=\"M0 840H1500V856C1500 872 1484 880 1460 880H40C16 880 0 872 0 856Z\" fill=\"url(#b)\"/><path d=\"M0 840H1500\" stroke=\"{edge}\" stroke-width=\"3\"/><path d=\"M660 840H840V846C840 850 836 852 832 852H668C664 852 660 850 660 846Z\" fill=\"{edge}\"/>",
                grad("b"),
                rr(136.0, 10.0, 1228.0, 830.0, 18.0, bezel)
            ),
            over: "<circle cx=\"750\" cy=\"20\" r=\"4\" fill=\"#1d2533\"/>".into(),
        },
        "desktop" => Device {
            w: 1400.0,
            h: 1060.0,
            sx: 20.0,
            sy: 20.0,
            sw: 1360.0,
            sh: 812.0,
            sr: 4.0,
            under: format!(
                "{}<path d=\"M640 860H760L780 1030H620Z\" fill=\"url(#b)\"/><rect x=\"500\" y=\"1026\" width=\"400\" height=\"26\" rx=\"13\" fill=\"url(#b)\"/><rect width=\"1400\" height=\"880\" rx=\"20\" fill=\"url(#b)\"/>{}",
                grad("b"),
                rr(8.0, 8.0, 1384.0, 836.0, 14.0, bezel)
            ),
            over: String::new(),
        },
        _ => {
            // Browser window: neutral chrome, three grey dots, an address field.
            let url = xml(url);
            Device {
                w: 1400.0,
                h: 920.0,
                sx: 1.0,
                sy: 58.0,
                sw: 1398.0,
                sh: 861.0,
                sr: 0.0,
                under: "<rect width=\"1400\" height=\"920\" rx=\"14\" fill=\"#ffffff\"/>".into(),
                over: format!(
                    "<path d=\"M0 14a14 14 0 0 1 14-14H1386a14 14 0 0 1 14 14V58H0Z\" fill=\"#f4f2ee\"/><path d=\"M0 58H1400\" stroke=\"#d9d6cf\" stroke-width=\"1.5\"/><circle cx=\"30\" cy=\"29\" r=\"7\" fill=\"#c9c6bf\"/><circle cx=\"54\" cy=\"29\" r=\"7\" fill=\"#c9c6bf\"/><circle cx=\"78\" cy=\"29\" r=\"7\" fill=\"#c9c6bf\"/><rect x=\"300\" y=\"14\" width=\"800\" height=\"30\" rx=\"15\" fill=\"#ffffff\" stroke=\"#d9d6cf\"/><text x=\"700\" y=\"34.5\" text-anchor=\"middle\" font-family=\"Latin Modern Sans\" font-size=\"15\" fill=\"#5c5a55\">{url}</text><rect x=\"0.75\" y=\"0.75\" width=\"1398.5\" height=\"918.5\" rx=\"14\" fill=\"none\" stroke=\"#d9d6cf\" stroke-width=\"1.5\"/>"
                ),
            }
        }
    }
}

/// The screen content when no file is given: a house-style placeholder.
fn placeholder_screen(w: u32, h: u32) -> Result<RgbaImage> {
    let s = (w.min(h) as f32) / 10.0;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fcfbf9\"/><rect width=\"{w}\" height=\"{}\" fill=\"#ffeca4\"/><path d=\"M{cx} {a}L{b} {cy}L{cx} {c}L{d} {cy}Z\" fill=\"none\" stroke=\"#1b1616\" stroke-width=\"{sw}\"/><text x=\"{cx}\" y=\"{ty}\" text-anchor=\"middle\" font-family=\"Latin Modern Sans\" font-size=\"{fs}\" fill=\"#5c5a55\">{}</text></svg>",
        h as f32 * 0.08,
        xml(&tr("create.mockup.placeholder")),
        cx = w as f32 / 2.0,
        cy = h as f32 * 0.45,
        a = h as f32 * 0.45 - s * 0.6,
        b = w as f32 / 2.0 + s * 0.6,
        c = h as f32 * 0.45 + s * 0.6,
        d = w as f32 / 2.0 - s * 0.6,
        sw = (s * 0.06).max(1.0),
        ty = h as f32 * 0.45 + s * 1.5,
        fs = s * 0.42,
    );
    Ok(png::pixmap_to_rgba(&png::render_svg(svg.as_bytes(), w, h, false)?))
}

pub struct Composed {
    pub image: RgbaImage,
    /// Screen corners in the output (tl, tr, br, bl).
    pub screen: [[f64; 2]; 4],
    /// Bottom layer (scene + shadow + device under the screen) and top layer (over the screen),
    /// for video compositing.
    pub bottom: RgbaImage,
    pub top: RgbaImage,
}

/// Compose a mockup of size (w, h). `screen`: the screenshot (None = placeholder).
pub fn compose(v: &Values, screen: Option<&RgbaImage>, w: u32, h: u32) -> Result<Composed> {
    let d = device(v.str("device"), v.str("finish"), v.str("url"));
    let pad = v.int("padding").clamp(0, 40) as f32 / 100.0;
    let angle = v.str("angle");
    let room = if angle == "flat" { 1.0 } else { 0.92 };
    let k = ((w as f32 * (1.0 - 2.0 * pad)) / d.w).min((h as f32 * (1.0 - 2.0 * pad)) / d.h) * room;
    let (dw, dh) = ((d.w * k).round().max(1.0) as u32, (d.h * k).round().max(1.0) as u32);
    // Screen pixels at the final scale.
    let (sw, sh) = ((d.sw * k).round().max(1.0) as u32, (d.sh * k).round().max(1.0) as u32);
    let shot = match screen {
        Some(s) => fit_into(s, sw, sh, v.str("fit") == "fit"),
        None => placeholder_screen(sw, sh)?,
    };
    // Device layers in device units, rasterised at scale k.
    let layer = |inner: &str| -> Result<RgbaImage> {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{dw}\" height=\"{dh}\">{inner}</svg>",
            d.w, d.h
        );
        Ok(png::pixmap_to_rgba(&png::render_svg(svg.as_bytes(), dw, dh, false)?))
    };
    let under = layer(&d.under)?;
    let over = layer(&d.over)?;
    // Screen with rounded corners (mask from an SVG rect).
    let mask = layer(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#fff\"/>", d.sx, d.sy, d.sw, d.sh, d.sr))?;
    let (ox, oy) = ((d.sx * k).round() as i64, (d.sy * k).round() as i64);
    let mut flat = under.clone();
    let mut placed = RgbaImage::new(dw, dh);
    image::imageops::overlay(&mut placed, &shot, ox, oy);
    for (p, m) in placed.pixels_mut().zip(mask.pixels()) {
        p[3] = ((p[3] as u32 * m[3] as u32) / 255) as u8;
    }
    raster::over(&mut flat, &placed, 0, 0);
    raster::over(&mut flat, &over, 0, 0);
    // A version without the screen, for video (bottom = under, top = over).
    let mut bottom = raster::scene(v.str("scene"), w, h, v.int("seed").max(0) as u64, v.color("scene_color"));
    let mut top = RgbaImage::new(w, h);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0 - if angle == "low" { h as f64 * 0.03 } else { 0.0 });
    let quad = match angle {
        "left" => Some(raster::tilt(cx, cy, dw as f64, dh as f64, 24.0, 6.0, 2.2)),
        "right" => Some(raster::tilt(cx, cy, dw as f64, dh as f64, -24.0, 6.0, 2.2)),
        "low" => Some(raster::tilt(cx, cy, dw as f64, dh as f64, 0.0, 22.0, 2.0)),
        _ => None,
    };
    let (x0, y0) = ((w as i64 - dw as i64) / 2, (h as i64 - dh as i64) / 2);
    let place = |img: &RgbaImage| -> RgbaImage {
        match quad {
            Some(q) => raster::warp(img, q, w, h),
            None => {
                let mut c = RgbaImage::new(w, h);
                image::imageops::overlay(&mut c, img, x0, y0);
                c
            }
        }
    };
    let flat_c = place(&flat);
    let under_c = place(&under);
    let over_c = place(&over);
    // Shadow from the device silhouette.
    let (blur, opacity, dy) = match v.str("shadow") {
        "strong" => (0.022, 0.55, 0.018),
        "none" => (0.0, 0.0, 0.0),
        _ => (0.04, 0.36, 0.025),
    };
    if blur > 0.0 && v.str("scene") != "transparent" || blur > 0.0 {
        let r = (w.min(h) as f32 * blur).max(2.0) as u32;
        let sh_img = raster::shadow(&flat_c, r, [20, 16, 14], opacity);
        raster::over(&mut bottom, &sh_img, -(r as i64), -(r as i64) + (h as f32 * dy) as i64);
    }
    let mut image = bottom.clone();
    raster::over(&mut image, &flat_c, 0, 0);
    raster::over(&mut bottom, &under_c, 0, 0);
    raster::over(&mut top, &over_c, 0, 0);
    let rect = [
        [(d.sx * k) as f64, (d.sy * k) as f64],
        [((d.sx + d.sw) * k) as f64, (d.sy * k) as f64],
        [((d.sx + d.sw) * k) as f64, ((d.sy + d.sh) * k) as f64],
        [(d.sx * k) as f64, ((d.sy + d.sh) * k) as f64],
    ];
    let screen = match quad {
        Some(q) => {
            // Map device-space screen corners through the same homography.
            rect.map(|p| map_point(q, p[0] / dw as f64, p[1] / dh as f64))
        }
        None => rect.map(|p| [p[0] + x0 as f64, p[1] + y0 as f64]),
    };
    let _ = style::INK;
    Ok(Composed { image, screen, bottom, top })
}

fn map_point(q: [[f64; 2]; 4], u: f64, v: f64) -> [f64; 2] {
    // Bilinear on a projective quad is not exact; use the same homography as the warp.
    let m = square_to_quad_pub(q);
    let z = m[6] * u + m[7] * v + m[8];
    [(m[0] * u + m[1] * v + m[2]) / z, (m[3] * u + m[4] * v + m[5]) / z]
}

fn square_to_quad_pub(q: [[f64; 2]; 4]) -> [f64; 9] {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q.map(|p| (p[0], p[1]));
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dx2 * dy1;
        ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
    };
    [x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0]
}

/// Resize a screenshot to exactly (w, h): crop to fill, or letterbox on black to fit.
fn fit_into(src: &RgbaImage, w: u32, h: u32, contain: bool) -> RgbaImage {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let k = if contain { (w as f32 / sw).min(h as f32 / sh) } else { (w as f32 / sw).max(h as f32 / sh) };
    let (rw, rh) = ((sw * k).round().max(1.0) as u32, (sh * k).round().max(1.0) as u32);
    let r = image::imageops::resize(src, rw, rh, image::imageops::FilterType::Lanczos3);
    let mut out = RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
    // Screenshots of tall pages: align to the top, not the middle.
    let y = if contain || rh <= h { (h as i64 - rh as i64) / 2 } else { 0 };
    image::imageops::overlay(&mut out, &r, (w as i64 - rw as i64) / 2, y);
    out
}

fn video(f: &FileInfo, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
    let ff = engine_media::ffmpeg()?;
    let probe = engine_media::probe(&f.path)?;
    let (w, h) = v.size("size");
    let (w, h) = (w & !1, h & !1);
    let c = compose(v, None, w, h)?;
    let tmp = cx.tmp_dir();
    let save = |img: &RgbaImage, name: &str| -> Result<std::path::PathBuf> {
        let p = tmp.join(name);
        std::fs::write(&p, png::encode_rgba(img)?)?;
        Ok(p)
    };
    let bottom = save(&c.bottom, "bottom.png")?;
    let top = save(&c.top, "top.png")?;
    // Mask of the screen quad (rounded corners come from the top layer).
    let mut mask = RgbaImage::new(w, h);
    fill_quad(&mut mask, c.screen);
    let mask_p = save(&mask, "mask.png")?;
    let s = c.screen;
    let (sw, sh) = (((s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).round().max(2.0) as u32, ((s[3][1] - s[0][1]).hypot(s[3][0] - s[0][0])).round().max(2.0) as u32);
    let fill = v.str("fit") != "fit";
    // Bring the video to the screen's aspect ratio (crop or letterbox), stretch it to the
    // canvas, then map the canvas corners onto the screen quad.
    let aspect = (sw as f64 / sh as f64).max(0.01);
    let (aw, ah) = if aspect >= w as f64 / h as f64 { (w, ((w as f64 / aspect) as u32).max(2) & !1) } else { (((h as f64 * aspect) as u32).max(2) & !1, h) };
    let scale = if fill {
        format!("scale={aw}:{ah}:force_original_aspect_ratio=increase,crop={aw}:{ah}")
    } else {
        format!("scale={aw}:{ah}:force_original_aspect_ratio=decrease,pad={aw}:{ah}:(ow-iw)/2:(oh-ih)/2:black")
    };
    let persp = format!(
        "perspective=x0={:.1}:y0={:.1}:x1={:.1}:y1={:.1}:x2={:.1}:y2={:.1}:x3={:.1}:y3={:.1}:sense=destination:interpolation=linear",
        s[0][0], s[0][1], s[1][0], s[1][1], s[3][0], s[3][1], s[2][0], s[2][1]
    );
    let graph = format!(
        "[0:v]{scale},setsar=1,scale={w}:{h},format=rgba,{persp}[sv];[3:v]format=gray,scale={w}:{h}[m];[sv][m]alphamerge[scr];[1:v][scr]overlay=0:0:format=auto:shortest=1[a];[a][2:v]overlay=0:0:format=auto:shortest=1,format=yuv420p[out]"
    );
    let out = cx.output_path(Some(f), "mp4");
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    for a in ["-hide_banner", "-y", "-i"] {
        args.push(a.into());
    }
    args.push(f.path.clone().into_os_string());
    for (p, l) in [(&bottom, true), (&top, true), (&mask_p, true)] {
        if l {
            args.push("-loop".into());
            args.push("1".into());
        }
        args.push("-i".into());
        args.push(p.clone().into_os_string());
    }
    let dur = format!("{:.3}", probe.duration.max(0.1));
    for a in ["-filter_complex", &graph, "-t", &dur, "-map", "[out]", "-map", "0:a?", "-c:v", "libx264", "-crf", "18", "-preset", "medium", "-c:a", "copy", "-shortest", "-movflags", "+faststart"] {
        args.push(a.into());
    }
    args.push(out.clone().into_os_string());
    let _ = ff;
    engine_media::run_ffmpeg(&args, probe.duration, cx).map_err(|e| anyhow!("{e:#}"))?;
    Ok(vec![Output::new(out).with_note(common::px(w, h))])
}

/// Rasterise a convex quad into the alpha channel (white, opaque inside).
fn fill_quad(img: &mut RgbaImage, q: [[f64; 2]; 4]) {
    let inside = |x: f64, y: f64| {
        let mut sign = 0i32;
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            let c = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
            let s = if c > 0.0 { 1 } else if c < 0.0 { -1 } else { 0 };
            if s != 0 {
                if sign == 0 {
                    sign = s;
                } else if s != sign {
                    return false;
                }
            }
        }
        true
    };
    for (x, y, p) in img.enumerate_pixels_mut() {
        if inside(x as f64 + 0.5, y as f64 + 0.5) {
            *p = image::Rgba([255, 255, 255, 255]);
        } else {
            *p = image::Rgba([0, 0, 0, 255]);
        }
    }
}

#[allow(dead_code)]
fn exists(p: &Path) -> bool {
    p.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_device_and_angle() {
        let t = MockupTool::new();
        for d in DEVICES {
            for a in ["flat", "left"] {
                let mut v = Values::defaults(&t.options());
                v.set("device", castor_core::Value::Text(d.to_string()));
                v.set("angle", castor_core::Value::Text(a.into()));
                let c = compose(&v, None, 320, 240).unwrap();
                assert_eq!(c.image.dimensions(), (320, 240));
            }
        }
    }
}
