//! image.optimize-gif (gifski re-encode, resize, frame-rate cap) and image.optimize-svg
//! (remove editor data, metadata and comments, round numbers, minify).

use anyhow::{Context, Result};
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{InputSpec, Job, Output, RunCx, Tool, ToolMeta, ToolStatus};

use crate::codec::{self, Fmt, Frame};
use crate::{common, ops};

// ------------------------------------------------------------------ GIF

pub struct OptimizeGif {
    meta: ToolMeta,
}

impl OptimizeGif {
    pub fn new() -> Self {
        let mut meta = common::meta(
            "image.optimize-gif",
            "GIF",
            &["gif", "animated", "optimize", "compress", "smaller", "gifski", "meme"],
            "min",
            ToolStatus::Ready,
            &[],
            false,
        );
        meta.inputs = InputSpec { kinds: &[], extensions: &["gif"], min: 1, max: None, folders: false };
        OptimizeGif { meta }
    }
}

impl Tool for OptimizeGif {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            common::quality_option(70),
            OptionSpec::int("max_width", 0, 4000, 0).unit("px"),
            OptionSpec::choice("fps", vec![Choice::key("keep"), Choice::lit("20", "20"), Choice::lit("15", "15"), Choice::lit("10", "10")], "keep"),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let src = std::fs::read(&input.path).with_context(|| common::err("err.read", &input.path))?;
        let frames = match codec::decode_frames(&src, Fmt::Gif).map_err(|e| anyhow::anyhow!("{}: {e}", common::err("err.corrupt", &input.path)))? {
            Some(f) => f,
            None => {
                let img = image::load_from_memory(&src).map_err(|e| anyhow::anyhow!("{}: {e}", common::err("err.corrupt", &input.path)))?;
                vec![Frame { image: img.to_rgba8(), delay_ms: 100 }]
            }
        };
        cx.progress(0.2);
        let frames = match v.str("fps") {
            "keep" | "" => frames,
            f => cap_fps(frames, f.parse().unwrap_or(15)),
        };
        let mw = v.int("max_width").max(0) as u32;
        let frames: Vec<Frame> = if mw > 0 && frames[0].image.width() > mw {
            let (w, h) = ops::fit_size(frames[0].image.width(), frames[0].image.height(), mw, u32::MAX / 2);
            frames.into_iter().map(|f| Frame { image: ops::resize_rgba(&f.image, w, h), delay_ms: f.delay_ms }).collect()
        } else {
            frames
        };
        let bytes = codec::encode_gifski(frames, v.int("quality").clamp(1, 100) as u8, 0, None, cx.cancel_token())?;
        let out = cx.output_path(Some(input), "gif");
        if bytes.len() >= src.len() {
            common::write(&out, &src)?;
            return Ok(vec![Output::new(out).with_note(castor_core::tr("image.compress.kept"))]);
        }
        common::write(&out, &bytes)?;
        Ok(vec![Output::new(out).with_note(common::size_note(src.len() as u64, bytes.len() as u64))])
    }
}

/// Merge frames so that no frame is shorter than 1/fps (durations are summed, never lost).
fn cap_fps(frames: Vec<Frame>, fps: u32) -> Vec<Frame> {
    let min_ms = 1000 / fps.max(1);
    let mut out: Vec<Frame> = Vec::new();
    let mut acc = 0u32;
    for f in frames {
        match out.last_mut() {
            Some(last) if acc < min_ms => {
                last.delay_ms += f.delay_ms;
                acc += f.delay_ms;
            }
            _ => {
                acc = f.delay_ms;
                out.push(f);
            }
        }
    }
    out
}

// ------------------------------------------------------------------ SVG

pub struct OptimizeSvg {
    meta: ToolMeta,
}

impl OptimizeSvg {
    pub fn new() -> Self {
        let mut meta = common::meta(
            "image.optimize-svg",
            "SVG",
            &["svg", "minify", "optimize", "svgo", "clean", "vector", "smaller"],
            "min",
            ToolStatus::Ready,
            &[],
            false,
        );
        meta.inputs = InputSpec { kinds: &[castor_core::FileKind::Svg], extensions: &["svg"], min: 1, max: None, folders: false };
        OptimizeSvg { meta }
    }
}

impl Tool for OptimizeSvg {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::int("precision", 0, 6, 3).slider(), OptionSpec::toggle("keep_title", true), OptionSpec::toggle("pretty", false)]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut src = std::fs::read(&input.path).with_context(|| common::err("err.read", &input.path))?;
        if src.starts_with(&[0x1f, 0x8b]) {
            src = resvg::usvg::decompress_svgz(&src)?;
        }
        let text = String::from_utf8(src.clone()).map_err(|_| anyhow::anyhow!("{}", common::err("err.corrupt", &input.path)))?;
        let out_text = optimize_svg(&text, v.int("precision").clamp(0, 8) as usize, v.bool("keep_title"), v.bool("pretty"))
            .map_err(|e| anyhow::anyhow!("{}: {e}", common::err("err.corrupt", &input.path)))?;
        // Safety net: the result must still parse and render.
        crate::codec::render_svg(out_text.as_bytes(), 64, None).context(castor_core::tr("image.optimize-svg.broken"))?;
        let out = cx.output_path(Some(input), "svg");
        let (before, after) = (input.size.max(src.len() as u64), out_text.len() as u64);
        if after >= before {
            common::write(&out, &src)?;
            return Ok(vec![Output::new(out).with_note(castor_core::tr("image.compress.kept"))]);
        }
        common::write(&out, out_text.as_bytes())?;
        Ok(vec![Output::new(out).with_note(common::size_note(before, after))])
    }
}

const DROP_NS: &[&str] = &[
    "http://www.inkscape.org/namespaces/inkscape",
    "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd",
    "http://ns.adobe.com/AdobeIllustrator/10.0/",
    "http://ns.adobe.com/Graphs/1.0/",
    "http://ns.adobe.com/AdobeSVGViewerExtensions/3.0/",
    "http://ns.adobe.com/Variables/1.0/",
    "http://ns.adobe.com/SaveForWeb/1.0/",
    "http://ns.adobe.com/Extensibility/1.0/",
    "http://ns.adobe.com/Flows/1.0/",
    "http://ns.adobe.com/ImageReplacement/1.0/",
    "http://ns.adobe.com/GenericCustomNamespace/1.0/",
    "http://ns.adobe.com/XPath/1.0/",
    "http://www.bohemiancoding.com/sketch/ns",
    "http://www.figma.com/figma/ns",
    "http://purl.org/dc/elements/1.1/",
    "http://creativecommons.org/ns#",
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
];
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Attributes whose numbers can be rounded.
fn numeric_attr(name: &str) -> bool {
    matches!(
        name,
        "d" | "points" | "x" | "y" | "x1" | "y1" | "x2" | "y2" | "cx" | "cy" | "r" | "rx" | "ry" | "width" | "height" | "transform"
            | "viewBox" | "stroke-width" | "fx" | "fy" | "gradientTransform" | "patternTransform" | "stroke-dashoffset" | "stroke-dasharray"
    )
}

/// Round every number in an attribute value to `p` decimals, dropping trailing zeros and
/// redundant separators ("M 10.0, 20" → "M10 20").
pub fn round_numbers(s: &str, p: usize) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut prev_num = false;
    let digit = |k: usize| k < b.len() && b[k].is_ascii_digit();
    while i < b.len() {
        let c = b[i];
        let starts = c.is_ascii_digit() || (c == b'.' && digit(i + 1)) || ((c == b'-' || c == b'+') && (digit(i + 1) || (i + 1 < b.len() && b[i + 1] == b'.' && digit(i + 2))));
        if starts {
            let st = i;
            i += 1;
            let mut dot = c == b'.';
            while i < b.len() {
                if b[i].is_ascii_digit() {
                    i += 1;
                } else if b[i] == b'.' && !dot {
                    dot = true;
                    i += 1;
                } else if (b[i] == b'e' || b[i] == b'E') && (digit(i + 1) || ((i + 1 < b.len()) && (b[i + 1] == b'-' || b[i + 1] == b'+') && digit(i + 2))) {
                    i += 2;
                    while digit(i) {
                        i += 1;
                    }
                    break;
                } else {
                    break;
                }
            }
            let tok = &s[st..i];
            let f = tok.parse::<f64>().map(|n| fmt_num(n, p)).unwrap_or_else(|_| tok.to_string());
            if prev_num && !f.starts_with('-') {
                out.push(' ');
            }
            out.push_str(&f);
            prev_num = true;
        } else if c == 0x20 || c == 0x2c || c.is_ascii_whitespace() {
            i += 1;
        } else {
            // Any other character (command letter, parenthesis, unit, %): copied as is.
            let ch = s[i..].chars().next().unwrap_or(' ');
            // Transform lists need a separator between items: "rotate(4) scale(2)".
            if ch.is_alphabetic() && out.ends_with(')') {
                out.push(' ');
            }
            out.push(ch);
            i += ch.len_utf8();
            prev_num = false;
        }
    }
    out
}

fn fmt_num(n: f64, p: usize) -> String {
    let mut s = format!("{:.*}", p, n);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    // "0.5" → ".5", "-0.5" → "-.5"
    if let Some(r) = s.strip_prefix("0.") {
        s = format!(".{r}");
    } else if let Some(r) = s.strip_prefix("-0.") {
        s = format!("-.{r}");
    }
    s
}

fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}
fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn optimize_svg(src: &str, precision: usize, keep_title: bool, pretty: bool) -> Result<String> {
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(src, opt)?;
    let root = doc.root_element();
    // Prefixes used on the root for namespaces we keep.
    let mut out = String::with_capacity(src.len() / 2);
    write_node(root, &mut out, precision, keep_title, pretty, 0, true);
    if pretty {
        out.push('\n');
    }
    Ok(out)
}

fn prefix_for(node: roxmltree::Node, uri: &str) -> Option<String> {
    if uri == XML_NS {
        return Some("xml".into());
    }
    node.lookup_prefix(uri).map(|p| p.to_string())
}

fn write_node(node: roxmltree::Node, out: &mut String, p: usize, keep_title: bool, pretty: bool, depth: usize, is_root: bool) {
    let ns = node.tag_name().namespace().unwrap_or(SVG_NS);
    if DROP_NS.contains(&ns) {
        return;
    }
    let name = node.tag_name().name();
    if name == "metadata" || (!keep_title && (name == "title" || name == "desc")) {
        return;
    }
    // Empty groups and defs carry nothing.
    let children: Vec<_> = node.children().filter(|c| c.is_element() || (c.is_text() && !c.text().unwrap_or("").trim().is_empty())).collect();
    if (name == "g" || name == "defs") && children.is_empty() && node.attributes().all(|a| a.name() != "id") {
        return;
    }
    let qual = |uri: &str, local: &str| -> String {
        if uri == SVG_NS || uri.is_empty() {
            local.to_string()
        } else {
            match prefix_for(node, uri) {
                Some(px) if !px.is_empty() => format!("{px}:{local}"),
                _ => local.to_string(),
            }
        }
    };
    if pretty && !is_root {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    }
    let tag = qual(ns, name);
    out.push('<');
    out.push_str(&tag);
    if is_root {
        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
        let uses_xlink = node.descendants().any(|d| d.attributes().any(|a| a.namespace() == Some(XLINK_NS)));
        if uses_xlink {
            out.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
        }
    }
    for a in node.attributes() {
        let ans = a.namespace().unwrap_or("");
        if DROP_NS.contains(&ans) {
            continue;
        }
        let an = a.name();
        // Editor leftovers and defaults that change nothing.
        if ans.is_empty() && (an.starts_with("data-name") || an == "enable-background" || (an == "version" && is_root) || an == "xml:space") {
            continue;
        }
        let key = if ans.is_empty() { an.to_string() } else if ans == XLINK_NS { format!("xlink:{an}") } else if ans == XML_NS { format!("xml:{an}") } else { qual(ans, an) };
        let val = if ans.is_empty() && numeric_attr(an) { round_numbers(a.value(), p) } else if an == "style" { a.value().split(';').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(";") } else { a.value().to_string() };
        out.push(' ');
        out.push_str(&key);
        out.push_str("=\"");
        out.push_str(&esc_attr(&val));
        out.push('"');
    }
    if children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    let mut had_elem = false;
    for c in node.children() {
        if c.is_element() {
            write_node(c, out, p, keep_title, pretty, depth + 1, false);
            had_elem = true;
        } else if c.is_text() {
            let t = c.text().unwrap_or("");
            let keep_ws = matches!(name, "text" | "tspan" | "textPath" | "style" | "title" | "desc" | "script");
            if keep_ws {
                if name == "style" || name == "script" {
                    out.push_str("<![CDATA[");
                    out.push_str(&t.replace("]]>", "]]]]><![CDATA[>"));
                    out.push_str("]]>");
                } else {
                    out.push_str(&esc_text(t));
                }
            } else if !t.trim().is_empty() {
                out.push_str(&esc_text(t.trim()));
            }
        }
    }
    if pretty && had_elem {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    }
    out.push_str("</");
    out.push_str(&tag);
    out.push('>');
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounding() {
        assert_eq!(round_numbers("M 10.123456 20.5 L -0.50001,3e2", 2), "M10.12 20.5L-.5 300");
        assert_eq!(round_numbers("translate(1.0000 , 2.26) rotate(4)", 1), "translate(1 2.3) rotate(4)");
    }
    #[test]
    fn strips_editor_data() {
        let src = r#"<?xml version="1.0"?><!-- c --><svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="10" height="10" inkscape:version="1"><metadata>m</metadata><g></g><rect x="1.0000" y="2" width="3" height="4" inkscape:label="r"/></svg>"#;
        let o = optimize_svg(src, 3, true, false).unwrap();
        assert_eq!(o, r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect x="1" y="2" width="3" height="4"/></svg>"#);
    }
    #[test]
    fn fps_cap_keeps_duration() {
        let f = |d| Frame { image: image::RgbaImage::new(1, 1), delay_ms: d };
        let out = cap_fps(vec![f(20), f(20), f(20), f(20), f(20)], 20);
        assert_eq!(out.iter().map(|f| f.delay_ms).sum::<u32>(), 100);
        assert!(out.len() < 5);
    }
}
