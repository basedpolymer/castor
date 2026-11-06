//! `create.html-to-image` and `create.html-to-pdf`: render web pages, local HTML files or pasted
//! HTML with the Chromium-based browser already installed (Chrome, Edge, Brave, Chromium),
//! headless, with a throw-away profile.

use std::time::Duration;

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, Requirement, RunCx, Tool, ToolMeta, tr};
use serde_json::json;

use crate::common;
use crate::render::chrome::{self, Browser};

const HTML_EXT: &[&str] = &["html", "htm", "xhtml", "svg"];

pub struct HtmlToImage {
    meta: ToolMeta,
}
pub struct HtmlToPdf {
    meta: ToolMeta,
}

impl HtmlToImage {
    pub fn new() -> Self {
        HtmlToImage {
            meta: common::serial(common::with_requires(
                common::meta(
                    "create.html-to-image",
                    InputSpec::kinds(&[FileKind::Html]).with_extensions(HTML_EXT).min(0),
                    "PNG · JPG · WEBP",
                    &["html", "screenshot", "web page", "url", "website", "capture", "render"],
                    "",
                ),
                &[Requirement::Chromium],
            )),
        }
    }
}

impl HtmlToPdf {
    pub fn new() -> Self {
        HtmlToPdf {
            meta: common::serial(common::with_requires(
                common::meta(
                    "create.html-to-pdf",
                    InputSpec::kinds(&[FileKind::Html]).with_extensions(HTML_EXT).min(0),
                    "PDF",
                    &["html", "pdf", "print", "web page", "url", "website", "save as pdf"],
                    "",
                ),
                &[Requirement::Chromium],
            )),
        }
    }
}

fn source_options() -> Vec<OptionSpec> {
    vec![
        OptionSpec::choice("source", ["files", "url", "html"].map(Choice::key).to_vec(), "url"),
        OptionSpec::multiline("url", "https://example.com").when(Cond::Eq("source", "url")),
        OptionSpec::multiline("html", "<h1>Hello</h1>").when(Cond::Eq("source", "html")),
        OptionSpec::float("delay", 0.0, 30.0, 0.5).unit("s").advanced(),
    ]
}

/// What to load for this job: (url, output stem, input for naming).
fn targets<'a>(job: &'a Job, v: &Values, cx: &RunCx) -> Result<Vec<(String, String, Option<&'a FileInfo>)>> {
    if let Some(f) = job.inputs.first() {
        return Ok(vec![(chrome::file_url(&f.path), f.stem(), Some(f))]);
    }
    match v.str("source") {
        "html" => {
            let html = v.str("html");
            anyhow::ensure!(!html.trim().is_empty(), "{}", tr("create.html.err.empty"));
            let p = cx.tmp_dir().join("page.html");
            let doc = if html.to_ascii_lowercase().contains("<html") {
                html.to_string()
            } else {
                format!("<!doctype html><html><head><meta charset=\"utf-8\"></head><body>{html}</body></html>")
            };
            std::fs::write(&p, doc)?;
            Ok(vec![(chrome::file_url(&p), "page".to_string(), None)])
        }
        "files" => anyhow::bail!("{}", tr("create.html.err.no_file")),
        _ => {
            let urls: Vec<String> = v.str("url").lines().map(str::trim).filter(|l| !l.is_empty()).map(normalize_url).collect();
            anyhow::ensure!(!urls.is_empty(), "{}", tr("create.html.err.empty"));
            Ok(urls.into_iter().map(|u| (u.clone(), url_stem(&u), None)).collect())
        }
    }
}

pub fn normalize_url(u: &str) -> String {
    if u.contains("://") || u.starts_with("about:") || u.starts_with("data:") { u.to_string() } else { format!("https://{u}") }
}

/// "https://www.example.com/blog/post?x" → "example.com-blog-post".
pub fn url_stem(u: &str) -> String {
    let s = u.split("://").nth(1).unwrap_or(u);
    let s = s.trim_start_matches("www.").split(['?', '#']).next().unwrap_or(s).trim_end_matches('/');
    common::slug_keep_case(&s.replace('/', "-"))
}

fn unavailable() -> Option<String> {
    if chrome::find_browser().is_none() { Some(tr("create.html.err.no_browser")) } else { None }
}

impl Tool for HtmlToImage {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn unavailable(&self) -> Option<String> {
        unavailable()
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = source_options();
        o.extend([
            OptionSpec::size("viewport", 1280, 800),
            OptionSpec::toggle("full_page", true),
            OptionSpec::choice("scale", vec![Choice::lit("1", "1×"), Choice::lit("2", "2×"), Choice::lit("3", "3×")], "2"),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP")], "png"),
            OptionSpec::toggle("dark", false).advanced(),
            OptionSpec::toggle("transparent", false).advanced(),
        ]);
        o
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        Ok(if inputs.is_empty() { vec![Job::empty()] } else { inputs.iter().cloned().map(Job::single).collect() })
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let list = targets(job, v, cx)?;
        let (w, h) = v.size("viewport");
        let (w, h) = (w.clamp(200, 8000), h.clamp(200, 8000));
        let scale: f64 = v.str("scale").parse().unwrap_or(2.0);
        let fmt = v.str("format");
        cx.message(tr("create.html.msg.starting"));
        let mut b = Browser::launch(Some(cx))?;
        b.new_page()?;
        b.viewport(w, h, scale, v.bool("dark"), v.bool("transparent") && fmt == "png")?;
        let mut outs = Vec::new();
        for (i, (url, stem, input)) in list.iter().enumerate() {
            cx.message(url.clone());
            b.goto(url, Duration::from_secs_f64(v.float("delay").clamp(0.0, 30.0)))?;
            let png = b.screenshot(w, h, scale, v.bool("full_page"))?;
            let img = image::load_from_memory(&png)?;
            let out = match input {
                Some(f) => cx.output_path(Some(f), fmt),
                None => cx.output_named(&format!("{stem}.{fmt}")),
            };
            match fmt {
                "png" => common::write(&out, &png)?,
                _ => crate::gradient::save_rgb(&img.to_rgb8(), &out, fmt)?,
            }
            outs.push(Output::new(out).with_note(common::px(img.width(), img.height())));
            cx.progress((i + 1) as f32 / list.len() as f32);
            // Back to the viewport height for the next page.
            if v.bool("full_page") && i + 1 < list.len() {
                b.viewport(w, h, scale, v.bool("dark"), false)?;
            }
        }
        Ok(outs)
    }
}

const PAPERS: &[(&str, f64, f64)] = &[
    ("a4", 8.27, 11.69),
    ("letter", 8.5, 11.0),
    ("a3", 11.69, 16.54),
    ("a5", 5.83, 8.27),
    ("legal", 8.5, 14.0),
    ("tabloid", 11.0, 17.0),
];

impl Tool for HtmlToPdf {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn unavailable(&self) -> Option<String> {
        unavailable()
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = source_options();
        let mut papers: Vec<Choice> = vec![
            Choice::lit("a4", "A4"),
            Choice::lit("letter", "US Letter"),
            Choice::lit("a3", "A3"),
            Choice::lit("a5", "A5"),
            Choice::lit("legal", "US Legal"),
            Choice::lit("tabloid", "Tabloid"),
        ];
        papers.push(Choice::key("css"));
        o.extend([
            OptionSpec::choice("paper", papers, "a4"),
            OptionSpec::toggle("landscape", false),
            OptionSpec::float("margin", 0.0, 50.0, 12.0).unit("mm"),
            OptionSpec::toggle("backgrounds", true),
            OptionSpec::toggle("header_footer", false),
            OptionSpec::int("scale", 10, 200, 100).slider().unit("%").advanced(),
            OptionSpec::toggle("screen_media", false).advanced(),
        ]);
        o
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        Ok(if inputs.is_empty() { vec![Job::empty()] } else { inputs.iter().cloned().map(Job::single).collect() })
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let list = targets(job, v, cx)?;
        cx.message(tr("create.html.msg.starting"));
        let mut b = Browser::launch(Some(cx))?;
        b.new_page()?;
        if v.bool("screen_media") {
            b.call("Emulation.setEmulatedMedia", json!({"media": "screen"}), true, Duration::from_secs(10))?;
        }
        let paper = v.str("paper");
        let (pw, ph) = PAPERS.iter().find(|p| p.0 == paper).map(|p| (p.1, p.2)).unwrap_or((8.27, 11.69));
        let m = v.float("margin").clamp(0.0, 50.0) / 25.4;
        let hf = v.bool("header_footer");
        let small = "font-size:8px;font-family:'Latin Modern Roman','Times New Roman',serif;color:#5c5a55;width:100%;padding:0 12mm;display:flex;justify-content:space-between";
        let mut params = json!({
            "landscape": v.bool("landscape"),
            "printBackground": v.bool("backgrounds"),
            "scale": (v.int("scale").clamp(10, 200) as f64 / 100.0),
            "marginTop": if hf { m.max(0.5) } else { m },
            "marginBottom": if hf { m.max(0.5) } else { m },
            "marginLeft": m, "marginRight": m,
            "displayHeaderFooter": hf,
            "headerTemplate": format!("<div style=\"{small}\"><span class=\"title\"></span><span class=\"date\"></span></div>"),
            "footerTemplate": format!("<div style=\"{small};justify-content:center\"><span><span class=\"pageNumber\"></span> / <span class=\"totalPages\"></span></span></div>"),
            "preferCSSPageSize": paper == "css",
            "generateDocumentOutline": true,
            "generateTaggedPDF": true,
        });
        if paper != "css" {
            params["paperWidth"] = json!(pw);
            params["paperHeight"] = json!(ph);
        }
        let mut outs = Vec::new();
        for (i, (url, stem, input)) in list.iter().enumerate() {
            cx.message(url.clone());
            b.goto(url, Duration::from_secs_f64(v.float("delay").clamp(0.0, 30.0)))?;
            let pdf = b.pdf(params.clone())?;
            anyhow::ensure!(pdf.starts_with(b"%PDF"), "the browser returned no PDF");
            let out = match input {
                Some(f) => cx.output_path(Some(f), "pdf"),
                None => cx.output_named(&format!("{stem}.pdf")),
            };
            common::write(&out, &pdf)?;
            outs.push(Output::new(out));
            cx.progress((i + 1) as f32 / list.len() as f32);
        }
        Ok(outs)
    }

    fn preview(&self, _input: Option<&FileInfo>, _v: &Values, _max_side: u32) -> Result<Option<Preview>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stems() {
        assert_eq!(url_stem("https://www.example.com/blog/post?x=1"), "example.com-blog-post");
        assert_eq!(normalize_url("example.com"), "https://example.com");
    }
}
