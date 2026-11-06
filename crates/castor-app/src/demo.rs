//! Demo tools (`--features demo-tools`, development only): they exercise every UI path —
//! all option kinds, a slow job with progress, a live preview with no input, a failure, the
//! downloads consent, an unavailable tool, N → 1, and an in-place tool with undo.

use std::sync::Arc;
use std::time::Duration;

use castor_core::download::Asset;
use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};

const LANGS: &[(&str, &str)] = &[("eng", "English"), ("fra", "Français"), ("deu", "Deutsch"), ("spa", "Español"), ("ita", "Italiano")];

fn meta(id: &'static str, family: Family, inputs: InputSpec, output: &'static str, suffix: &'static str) -> ToolMeta {
    ToolMeta {
        id,
        family,
        inputs,
        output,
        keywords: &["demo"],
        status: ToolStatus::Ready,
        requires: &[],
        suffix,
        in_place: false,
        serial: false,
    }
}

fn copy_out(job: &Job, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
    let mut out = Vec::new();
    for f in &job.inputs {
        cx.check_cancel()?;
        let ext = f.extension();
        let p = cx.output_path(Some(f), if ext.is_empty() { "bin" } else { &ext });
        std::fs::copy(&f.path, &p)?;
        out.push(Output::new(p));
    }
    Ok(out)
}

/// Every option kind.
struct AllOptions(ToolMeta);
impl Tool for AllOptions {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("preset", vec![Choice::key("light"), Choice::key("balanced"), Choice::key("strong")], "balanced"),
            OptionSpec::choice(
                "format",
                vec![Choice::lit("jpg", "JPG"), Choice::lit("png", "PNG"), Choice::lit("webp", "WebP"), Choice::lit("avif", "AVIF"), Choice::lit("tiff", "TIFF"), Choice::lit("bmp", "BMP")],
                "webp",
            )
            .common(),
            OptionSpec::int("quality", 1, 100, 82).slider().unit("%").common(),
            OptionSpec::float("scale", 0.1, 4.0, 1.0).step(0.1).unit("×"),
            OptionSpec::toggle("strip", true),
            OptionSpec::text("title", "").placeholder("demo.all-options.opt.title.placeholder"),
            OptionSpec::color("color", "#2f6db5ff").common(),
            OptionSpec::anchor("position", Anchor::BottomRight).common().when(Cond::True("strip")),
            OptionSpec::size("size", 1920, 1080),
            OptionSpec::pages("pages").common().advanced(),
            OptionSpec::password("password").common().advanced(),
            OptionSpec::multiline("notes", "").advanced(),
            OptionSpec::file("logo", &["png", "svg"]).advanced(),
            OptionSpec::font("font", "LM Roman 10").advanced(),
            OptionSpec::language("language", LANGS, true, "eng+fra").common().advanced(),
        ]
    }
    fn run(&self, job: &Job, _v: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        std::thread::sleep(Duration::from_millis(300));
        cx.progress(1.0);
        copy_out(job, cx)
    }
}

/// A slow job with progress and messages; cancellable.
struct Slow(ToolMeta);
impl Tool for Slow {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::int("seconds", 1, 60, 6).slider().unit("s")]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        let steps = (v.int("seconds").max(1) * 10) as usize;
        for i in 0..steps {
            cx.check_cancel()?;
            std::thread::sleep(Duration::from_millis(100));
            cx.progress(i as f32 / steps as f32);
            if i % 10 == 0 {
                cx.message(format!("Step {} of {}", i / 10 + 1, steps / 10));
            }
        }
        copy_out(job, cx)
    }
}

/// No input, live preview: a gradient card.
struct Gradient(ToolMeta);
impl Gradient {
    fn render(v: &Values, max_side: u32) -> image::RgbaImage {
        let (w, h) = v.size("size");
        let (w, h) = (w.max(1), h.max(1));
        let s = (max_side as f32 / w.max(h) as f32).min(1.0);
        let (pw, ph) = (((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1));
        let a = v.color("from");
        let b = v.color("to");
        let diag = v.str("direction") == "diagonal";
        image::RgbaImage::from_fn(pw, ph, |x, y| {
            let t = if diag { (x as f32 / pw as f32 + y as f32 / ph as f32) / 2.0 } else { y as f32 / ph as f32 };
            let m = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8;
            image::Rgba([m(0), m(1), m(2), m(3)])
        })
    }
}
impl Tool for Gradient {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::color("from", "#fbe2ffff"),
            OptionSpec::color("to", "#ade1ffff"),
            OptionSpec::choice("direction", vec![Choice::key("vertical"), Choice::key("diagonal")], "diagonal"),
            OptionSpec::size("size", 1600, 900),
        ]
    }
    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        let img = Self::render(v, 8192);
        let p = cx.output_path(None, "png");
        img.save(&p)?;
        Ok(vec![Output::new(p)])
    }
    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> castor_core::Result<Option<Preview>> {
        let img = Self::render(v, max_side.min(640));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png)?;
        let (w, h) = v.size("size");
        Ok(Some(Preview { png: png.into_inner(), caption: Some(format!("{w} × {h} px")) }))
    }
}

/// Always fails, with a multi-line error.
struct Fail(ToolMeta);
impl Tool for Fail {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn run(&self, job: &Job, _v: &Values, _cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        std::thread::sleep(Duration::from_millis(200));
        let name = job.inputs.first().map(|f| f.name()).unwrap_or_default();
        anyhow::bail!("could not read {name}: the file looks damaged\nat byte 1024: unexpected end of stream\n(demo failure)")
    }
}

/// Needs a (fake) download: exercises the consent dialog. Runs without fetching anything.
struct NeedsModel(ToolMeta);
impl Tool for NeedsModel {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::choice("model", vec![Choice::key("small"), Choice::key("large")], "small")]
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        let big = v.str("model") == "large";
        vec![Asset {
            id: if big { "models/demo-large.onnx" } else { "models/demo-small.onnx" },
            url: "https://example.invalid/demo.onnx",
            sha256: "",
            size: if big { 176_000_000 } else { 44_000_000 },
            license: "Apache-2.0",
        }]
    }
    fn run(&self, job: &Job, _v: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        cx.message("Demo: the model would load here");
        std::thread::sleep(Duration::from_millis(600));
        copy_out(job, cx)
    }
}

/// Unavailable on this machine.
struct Unavailable(ToolMeta);
impl Tool for Unavailable {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn unavailable(&self) -> Option<String> {
        Some(castor_core::tr("demo.unavailable.why"))
    }
    fn run(&self, _job: &Job, _v: &Values, _cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        anyhow::bail!("unavailable")
    }
}

/// N → 1: concatenates the files into one text report.
struct Merge(ToolMeta);
impl Tool for Merge {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> castor_core::Result<Vec<Job>> {
        Ok(vec![Job::many(inputs.to_vec())])
    }
    fn run(&self, job: &Job, _v: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        let p = cx.output_named("demo-report.txt");
        let mut s = String::new();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            cx.progress(i as f32 / job.inputs.len() as f32);
            s.push_str(&format!("{}\t{}\n", f.name(), f.size));
            std::thread::sleep(Duration::from_millis(150));
        }
        std::fs::write(&p, s)?;
        Ok(vec![Output::new(p).with_note(format!("{} files listed", job.inputs.len()))])
    }
}

/// In place: renames with a prefix, undo journal restores the name.
struct Rename(ToolMeta);
impl Tool for Rename {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::text("prefix", "demo-")]
    }
    fn run(&self, job: &Job, v: &Values, _cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        let f = job.input()?;
        let new = f.path.with_file_name(format!("{}{}", v.str("prefix"), f.name()));
        anyhow::ensure!(!new.exists(), "{} already exists", new.display());
        std::fs::rename(&f.path, &new)?;
        Ok(vec![Output::new(&new).with_undo(serde_json::json!({ "from": new, "to": f.path }))])
    }
    fn undo(&self, j: &serde_json::Value) -> castor_core::Result<()> {
        let from = j["from"].as_str().ok_or_else(|| anyhow::anyhow!("bad journal"))?;
        let to = j["to"].as_str().ok_or_else(|| anyhow::anyhow!("bad journal"))?;
        std::fs::rename(from, to)?;
        Ok(())
    }
}

pub fn tools() -> Vec<Arc<dyn Tool>> {
    const IMG: &[FileKind] = &[FileKind::Image];
    const ANY: &[FileKind] = &[FileKind::Any];
    let mut rename = meta("demo.rename", Family::Files, InputSpec::kinds(ANY), "same", "");
    rename.in_place = true;
    let mut soon = meta("demo.soon", Family::Audio, InputSpec::kinds(&[FileKind::Audio]), "MP3", "soon");
    soon.status = ToolStatus::Soon;
    vec![
        Arc::new(AllOptions(meta("demo.all-options", Family::Image, InputSpec::kinds(IMG), "JPG · PNG · WEBP", "demo"))),
        Arc::new(Slow(meta("demo.slow", Family::Video, InputSpec::kinds(ANY), "same", "slow"))),
        Arc::new(Gradient(meta("demo.gradient", Family::Create, InputSpec::none(), "PNG", "gradient"))),
        Arc::new(Fail(meta("demo.fail", Family::Pdf, InputSpec::kinds(ANY), "PDF", "fail"))),
        Arc::new(NeedsModel(meta("demo.model", Family::Image, InputSpec::kinds(IMG), "PNG", "model"))),
        Arc::new(Unavailable(meta("demo.unavailable", Family::Pdf, InputSpec::kinds(&[FileKind::Office]), "PDF", "pdf"))),
        Arc::new(Merge(meta("demo.merge", Family::Files, InputSpec::kinds(ANY).min(2), "TXT", "merged"))),
        Arc::new(Rename(rename)),
        Arc::new(Unavailable(soon)),
    ]
}
