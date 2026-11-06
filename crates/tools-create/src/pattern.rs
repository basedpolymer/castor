//! `create.pattern` (seamless patterns and textures) and `create.watermark-tile` (repeating
//! text watermarks with transparency). Both are drawn as SVG tiles: export the tile itself
//! (seamless) or a filled canvas, in PNG, SVG or JPG.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};

use crate::common::{self, Rng, xml};
use crate::render::{png, style};

pub struct PatternTool {
    meta: ToolMeta,
}
pub struct WatermarkTileTool {
    meta: ToolMeta,
}

impl PatternTool {
    pub fn new() -> Self {
        PatternTool {
            meta: common::meta(
                "create.pattern",
                InputSpec::none(),
                "PNG · SVG · JPG",
                &["pattern", "texture", "seamless", "tile", "background", "polka", "stripes", "terrazzo", "geometric"],
                "pattern",
            ),
        }
    }
}

impl WatermarkTileTool {
    pub fn new() -> Self {
        WatermarkTileTool {
            meta: common::meta(
                "create.watermark-tile",
                InputSpec::none(),
                "PNG · SVG",
                &["watermark", "tile", "overlay", "confidential", "draft", "copyright", "proof", "repeat"],
                "watermark",
            ),
        }
    }
}

pub const KINDS: &[&str] = &["dots", "grid", "diamonds", "checker", "stripes", "zigzag", "waves", "hexagons", "triangles", "crosses", "terrazzo", "confetti", "scales", "herringbone"];

fn out_options() -> Vec<OptionSpec> {
    vec![
        OptionSpec::choice("output", ["tile", "canvas"].map(Choice::key).to_vec(), "canvas"),
        OptionSpec::size("size", 1920, 1080).when(Cond::Eq("output", "canvas")),
        OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("svg", "SVG"), Choice::lit("jpg", "JPG")], "png"),
        OptionSpec::text("file_name", "").advanced(),
    ]
}

impl Tool for PatternTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = vec![
            OptionSpec::choice("kind", KINDS.iter().map(|k| Choice::key(k)).collect(), "diamonds"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("color2", "#2f6db5"),
            OptionSpec::color("background", "#ffeca4"),
            OptionSpec::int("tile", 8, 1000, 120).unit("px"),
            OptionSpec::float("stroke", 0.5, 40.0, 3.0).unit("px"),
            OptionSpec::int("angle", -90, 90, 0).slider().unit("°"),
            OptionSpec::int("seed", 0, 999_999, 1).when(Cond::Eq("kind", "terrazzo")),
        ];
        o.extend(out_options());
        o
    }
    fn preview(&self, _i: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        preview(&pattern_tile(v), v, max_side)
    }
    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        export(&pattern_tile(v), v, cx)
    }
}

impl Tool for WatermarkTileTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = vec![
            OptionSpec::text("text", "CONFIDENTIAL"),
            OptionSpec::text("text2", ""),
            OptionSpec::choice("font", ["sans", "serif", "mono", "chorus", "system"].map(Choice::key).to_vec(), "sans"),
            OptionSpec::font("family", "").when(Cond::Eq("font", "system")),
            OptionSpec::int("font_size", 8, 400, 36).unit("px"),
            OptionSpec::color("color", "#1b161640"),
            OptionSpec::color("background", "#00000000"),
            OptionSpec::int("angle", -90, 90, -30).slider().unit("°"),
            OptionSpec::int("spacing", 0, 400, 80).slider().unit("%"),
            OptionSpec::toggle("stagger", true),
        ];
        o.extend(out_options());
        o
    }
    fn preview(&self, _i: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        preview(&watermark_tile(v)?, v, max_side)
    }
    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        export(&watermark_tile(v)?, v, cx)
    }
}

/// A seamless tile: SVG content of size (w, h), plus a background colour.
pub struct Tile {
    pub w: f32,
    pub h: f32,
    pub body: String,
    pub bg: [u8; 4],
    pub angle: f32,
    pub system_fonts: bool,
}

impl Tile {
    /// SVG of the tile alone (seamless when repeated). Rotation is applied on canvases only.
    pub fn tile_svg(&self) -> String {
        let (bg, bga) = style::svg_paint(self.bg);
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"{bg}\" fill-opacity=\"{bga}\"/>{b}</svg>",
            w = self.w,
            h = self.h,
            b = self.body
        )
    }

    /// SVG of a canvas filled with the (optionally rotated) tile.
    pub fn canvas_svg(&self, cw: u32, ch: u32) -> String {
        let (bg, bga) = style::svg_paint(self.bg);
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {cw} {ch}\" width=\"{cw}\" height=\"{ch}\"><defs><pattern id=\"t\" width=\"{w}\" height=\"{h}\" patternUnits=\"userSpaceOnUse\" patternTransform=\"rotate({a} {cx} {cy})\">{b}</pattern></defs><rect width=\"{cw}\" height=\"{ch}\" fill=\"{bg}\" fill-opacity=\"{bga}\"/><rect width=\"{cw}\" height=\"{ch}\" fill=\"url(#t)\"/></svg>",
            w = self.w,
            h = self.h,
            a = self.angle,
            cx = cw as f32 / 2.0,
            cy = ch as f32 / 2.0,
            b = self.body
        )
    }
}

fn preview(t: &Tile, v: &Values, max_side: u32) -> Result<Option<Preview>> {
    let (svg, w, h) = if v.str("output") == "tile" {
        // Show 3 × 3 tiles so the seams can be judged.
        (t.canvas_svg((t.w * 3.0) as u32, (t.h * 3.0) as u32), t.w * 3.0, t.h * 3.0)
    } else {
        let (w, h) = v.size("size");
        (t.canvas_svg(w, h), w as f32, h as f32)
    };
    let k = (max_side as f32 / w.max(h)).min(1.0);
    let pm = png::render_svg(svg.as_bytes(), (w * k).max(1.0) as u32, (h * k).max(1.0) as u32, t.system_fonts)?;
    let cap = if v.str("output") == "tile" { format!("{} × 9", common::px(t.w as u32, t.h as u32)) } else { common::px(w as u32, h as u32) };
    Ok(Some(common::preview(png::encode_pixmap(&pm)?, common::figure(&cap))))
}

fn export(t: &Tile, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
    let fmt = v.str("format");
    let (svg, w, h) = if v.str("output") == "tile" {
        (t.tile_svg(), t.w.round() as u32, t.h.round() as u32)
    } else {
        let (w, h) = v.size("size");
        anyhow::ensure!((16..=16384).contains(&w) && (16..=16384).contains(&h), "{}", tr("create.common.err.size"));
        (t.canvas_svg(w, h), w, h)
    };
    let out = common::out_named(cx, v.str("file_name"), fmt);
    match fmt {
        "svg" => {
            let tree = png::parse_svg(svg.as_bytes(), t.system_fonts)?;
            common::write(&out, tree.to_string(&resvg::usvg::WriteOptions::default()).as_bytes())?;
        }
        "jpg" => {
            let pm = png::render_svg(svg.as_bytes(), w, h, t.system_fonts)?;
            let rgb = png::flatten(&png::pixmap_to_rgba(&pm), [255, 255, 255]);
            crate::gradient::save_rgb(&rgb, &out, "jpg")?;
        }
        _ => common::write(&out, &png::encode_pixmap(&png::render_svg(svg.as_bytes(), w, h, t.system_fonts)?)?)?,
    }
    Ok(vec![Output::new(out).with_note(common::px(w, h))])
}

fn f(v: f32) -> String {
    format!("{v:.2}")
}

/// Pattern tiles. Every shape that crosses an edge is drawn again on the other side.
pub fn pattern_tile(v: &Values) -> Tile {
    let s = v.int("tile").clamp(8, 1000) as f32;
    let sw = v.float("stroke").clamp(0.5, 40.0) as f32;
    let (c1, a1) = style::svg_paint(v.color("color"));
    let (c2, a2) = style::svg_paint(v.color("color2"));
    let fill1 = format!("fill=\"{c1}\" fill-opacity=\"{a1}\"");
    let fill2 = format!("fill=\"{c2}\" fill-opacity=\"{a2}\"");
    let stroke1 = format!("fill=\"none\" stroke=\"{c1}\" stroke-opacity=\"{a1}\" stroke-width=\"{sw}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"");
    let (mut w, mut h) = (s, s);
    let body = match v.str("kind") {
        "dots" => format!(
            "<circle cx=\"{q}\" cy=\"{q}\" r=\"{r}\" {fill1}/><circle cx=\"{t}\" cy=\"{t}\" r=\"{r}\" {fill2}/>",
            q = f(s * 0.25),
            t = f(s * 0.75),
            r = f(s * 0.11)
        ),
        "grid" => format!("<path d=\"M0 0H{s}M0 0V{s}M0 {s}H{s}M{s} 0V{s}\" {stroke1}/>"),
        "diamonds" => {
            let r = s * 0.16;
            let c = s / 2.0;
            let faint = format!("fill=\"none\" stroke=\"{c1}\" stroke-opacity=\"{}\" stroke-width=\"{sw}\"", a1 * 0.25);
            format!(
                "<path d=\"M{c} {a}L{b} {c}L{c} {d}L{e} {c}Z\" {stroke1}/><path d=\"M0 {m}L{m} 0M{s} {n}L{n} {s}\" {faint}/>",
                a = f(c - r),
                b = f(c + r),
                d = f(c + r),
                e = f(c - r),
                m = f(s * 0.0),
                n = f(s)
            )
        }
        "checker" => format!("<rect width=\"{h2}\" height=\"{h2}\" {fill1}/><rect x=\"{h2}\" y=\"{h2}\" width=\"{h2}\" height=\"{h2}\" {fill1}/>", h2 = f(s / 2.0)),
        "stripes" => format!("<rect width=\"{s}\" height=\"{}\" {fill1}/><rect y=\"{}\" width=\"{s}\" height=\"{}\" {fill2}/>", f(s * 0.25), f(s * 0.5), f(s * 0.12)),
        "zigzag" => {
            h = s * 0.5;
            format!("<path d=\"M0 {a}L{q} {b}L{hh} {a}L{t} {b}L{s} {a}\" {stroke1}/>", a = f(h * 0.7), b = f(h * 0.3), q = f(s * 0.25), hh = f(s * 0.5), t = f(s * 0.75))
        }
        "waves" => {
            h = s * 0.5;
            format!(
                "<path d=\"M0 {m}C{a} {lo} {b} {lo} {hh} {m}S{c} {hi} {s} {m}\" {stroke1}/>",
                m = f(h * 0.5),
                a = f(s * 0.17),
                b = f(s * 0.33),
                lo = f(h * 0.1),
                hh = f(s * 0.5),
                c = f(s * 0.83),
                hi = f(h * 0.9)
            )
        }
        "hexagons" => {
            // Pointy-top hexagon grid: tile is s × s·√3.
            let r = s / 3.0_f32.sqrt();
            h = s * 3.0_f32.sqrt();
            w = s;
            let hex = |cx: f32, cy: f32| {
                let pts: Vec<String> = (0..6)
                    .map(|i| {
                        let a = std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
                        format!("{},{}", f(cx + r * 0.92 * a.cos()), f(cy + r * 0.92 * a.sin()))
                    })
                    .collect();
                format!("<polygon points=\"{}\" {stroke1}/>", pts.join(" "))
            };
            [(0.0, 0.0), (s, 0.0), (s / 2.0, h / 2.0), (0.0, h), (s, h)].iter().map(|(x, y)| hex(*x, *y)).collect()
        }
        "triangles" => {
            h = s * 0.866;
            format!(
                "<path d=\"M0 {h}L{hs} 0L{s} {h}Z\" {fill1}/><path d=\"M{hs} 0L{s} {h}L{s1} 0Z\" {fill2}/><path d=\"M-{hs} 0L0 {h}L{hs} 0Z\" {fill2}/>",
                h = f(h),
                hs = f(s / 2.0),
                s1 = f(s * 1.5)
            )
        }
        "crosses" => {
            let a = s * 0.12;
            format!("<path d=\"M{c} {m1}V{m2}M{m1} {c}H{m2}\" {stroke1}/>", c = f(s / 2.0), m1 = f(s / 2.0 - a), m2 = f(s / 2.0 + a))
        }
        "scales" => {
            h = s * 0.5;
            format!(
                "<path d=\"M0 {h}A{r} {r} 0 0 1 {s} {h}M-{hs} 0A{r} {r} 0 0 1 {hs} 0M{hs} 0A{r} {r} 0 0 1 {s3} 0\" {stroke1}/>",
                h = f(h),
                r = f(s / 2.0),
                hs = f(s / 2.0),
                s3 = f(s * 1.5)
            )
        }
        "herringbone" => {
            let a = s * 0.25;
            format!(
                "<path d=\"M0 {a}L{a} 0M{a} {s}L0 {b}M{hs} {a}L{b} 0M{b} {s}L{hs} {b}M{hs} {s}L{s} {hs}M{hs} 0L{s} {a}\" {stroke1}/>",
                a = f(a),
                b = f(s - a),
                hs = f(s / 2.0)
            )
        }
        kind @ ("terrazzo" | "confetti") => {
            let mut rng = Rng::new(v.int("seed").max(0) as u64);
            let mut o = String::new();
            let n = if kind == "terrazzo" { 14 } else { 22 };
            for i in 0..n {
                let (x, y) = (rng.range(0.0, s), rng.range(0.0, s));
                let fill = if i % 3 == 0 { &fill2 } else { &fill1 };
                let shape = if kind == "terrazzo" {
                    let r = rng.range(s * 0.03, s * 0.09);
                    let k = (5 + (rng.next_u64() % 3)) as usize;
                    let pts: Vec<String> = (0..k)
                        .map(|j| {
                            let a = j as f32 / k as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
                            let rr = r * rng.range(0.6, 1.2);
                            format!("{},{}", f(rr * a.cos()), f(rr * a.sin()))
                        })
                        .collect();
                    format!("<polygon points=\"{}\" {fill}/>", pts.join(" "))
                } else {
                    let (l, t) = (rng.range(s * 0.04, s * 0.09), rng.range(s * 0.015, s * 0.03));
                    format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" transform=\"rotate({})\" {fill}/>", f(-l / 2.0), f(-t / 2.0), f(l), f(t), f(t / 2.0), rng.range(0.0, 180.0))
                };
                // Draw at (x, y) and its wrapped copies so the tile is seamless.
                for dx in [-s, 0.0, s] {
                    for dy in [-s, 0.0, s] {
                        o.push_str(&format!("<g transform=\"translate({} {})\">{shape}</g>", f(x + dx), f(y + dy)));
                    }
                }
            }
            o
        }
        _ => String::new(),
    };
    Tile { w, h, body, bg: v.color("background"), angle: v.int("angle") as f32, system_fonts: false }
}

/// Watermark tile: the text (and an optional second line) centred, spaced and staggered.
pub fn watermark_tile(v: &Values) -> Result<Tile> {
    let text = v.str("text").trim();
    anyhow::ensure!(!text.is_empty(), "{}", tr("create.watermark-tile.err.text"));
    let text2 = v.str("text2").trim();
    let size = v.int("font_size").clamp(8, 400) as f32;
    let fam = match v.str("font") {
        "serif" => "Latin Modern Roman".to_string(),
        "mono" => "Latin Modern Mono".to_string(),
        "chorus" => "TeX Gyre Chorus".to_string(),
        "system" if !v.str("family").trim().is_empty() => v.str("family").trim().to_string(),
        _ => "Latin Modern Sans".to_string(),
    };
    // Measure the text with the real fonts.
    let probe = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" height=\"1000\"><text x=\"10\" y=\"500\" font-family=\"{fam}\" font-size=\"{size}\" font-weight=\"bold\">{}</text></svg>",
        xml(if text2.chars().count() > text.chars().count() { text2 } else { text }),
        fam = xml(&fam)
    );
    let system = v.str("font") == "system";
    let tree = png::parse_svg(probe.as_bytes(), system)?;
    let tw = tree.root().abs_bounding_box().width().max(size);
    let lines = if text2.is_empty() { 1.0 } else { 2.2 };
    let gap = v.int("spacing").clamp(0, 400) as f32 / 100.0;
    let w = tw * (1.0 + gap);
    let h = size * lines * (1.0 + gap) + size;
    let (c, a) = style::svg_paint(v.color("color"));
    let t = |x: f32, y: f32, s: &str, sz: f32, bold: bool| {
        format!(
            "<text x=\"{x}\" y=\"{y}\" text-anchor=\"middle\" font-family=\"{}\" font-size=\"{sz}\"{} letter-spacing=\"{}\" fill=\"{c}\" fill-opacity=\"{a}\">{}</text>",
            xml(&fam),
            if bold { " font-weight=\"bold\"" } else { "" },
            sz * 0.04,
            xml(s)
        )
    };
    let block = |cx: f32, cy: f32| {
        let mut o = t(cx, cy + size * 0.35 - if text2.is_empty() { 0.0 } else { size * 0.55 }, text, size, true);
        if !text2.is_empty() {
            o.push_str(&t(cx, cy + size * 0.95, text2, size * 0.6, false));
        }
        o
    };
    let mut body = block(w / 2.0, h / 4.0);
    if v.bool("stagger") {
        // Second row offset by half a tile; wrapped at the edges.
        body.push_str(&block(0.0, h * 0.75));
        body.push_str(&block(w, h * 0.75));
    } else {
        body.push_str(&block(w / 2.0, h * 0.75));
    }
    Ok(Tile { w, h, body, bg: v.color("background"), angle: v.int("angle") as f32, system_fonts: system })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_pattern_renders() {
        let t = PatternTool::new();
        for k in KINDS {
            let mut v = Values::defaults(&t.options());
            v.set("kind", castor_core::Value::Text(k.to_string()));
            let tile = pattern_tile(&v);
            png::render_svg(tile.canvas_svg(200, 100).as_bytes(), 200, 100, false).unwrap_or_else(|e| panic!("{k}: {e}"));
            png::render_svg(tile.tile_svg().as_bytes(), tile.w as u32, tile.h as u32, false).unwrap();
        }
        let w = WatermarkTileTool::new();
        let v = Values::defaults(&w.options());
        let tile = watermark_tile(&v).unwrap();
        assert!(tile.w > 100.0);
    }
}
