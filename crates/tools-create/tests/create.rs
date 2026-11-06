//! Integration tests: every create tool through the real runner, small outputs in a temp dir.
//! Set CASTOR_KEEP=1 to keep the outputs in tests/fixtures/generated/create for a visual check.

use std::path::{Path, PathBuf};

use castor_core::options::{Value, Values};
use castor_core::runner::{self, OutputPolicy, RunRequest, Step};
use castor_core::{CancelToken, FileInfo, Tool};

fn out_dir(name: &str) -> (PathBuf, Option<tempfile::TempDir>) {
    if std::env::var_os("CASTOR_KEEP").is_some() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/generated/create").join(name);
        std::fs::create_dir_all(&p).unwrap();
        (p, None)
    } else {
        let d = tempfile::tempdir().unwrap();
        (d.path().to_path_buf(), Some(d))
    }
}

fn run(id: &str, inputs: &[PathBuf], set: &[(&str, &str)], out: &Path) -> castor_core::history::RunRecord {
    let t: std::sync::Arc<dyn Tool> = tools_create::tools().into_iter().find(|t| t.meta().id == id).unwrap();
    let specs = t.options();
    let mut v = Values::defaults(&specs);
    for (k, val) in set {
        let s = specs.iter().find(|s| s.key == *k).unwrap_or_else(|| panic!("{id} has no option {k}"));
        v.set(k, s.parse(val).unwrap());
    }
    let req = RunRequest {
        steps: vec![Step { tool: t, values: v }],
        inputs: inputs.iter().map(FileInfo::from_path).collect(),
        output: OutputPolicy::Folder(out.to_path_buf()),
        chain_name: None,
        max_parallel: 0,
        record: false,
    };
    let r = runner::run_blocking(req, CancelToken::new(), |_| {});
    assert!(r.errors.is_empty(), "{id} {set:?}: {:?}", r.errors);
    assert!(!r.outputs.is_empty(), "{id}: no output");
    for o in &r.outputs {
        assert!(std::fs::metadata(o).map(|m| m.len() > 0).unwrap_or(false), "{id}: empty {}", o.display());
    }
    r
}

fn decode_qr(p: &Path) -> Vec<String> {
    let img = image::open(p).unwrap().to_rgba8();
    tools_create::render::png::decode_qr(&img)
}

#[test]
fn qr_codes_scan_in_every_format_and_card() {
    let (d, _g) = out_dir("qr");
    let r = run("create.qr-code", &[], &[("url", "https://basedpolymer.eu/castor"), ("style", "dots"), ("size", "800")], &d);
    assert_eq!(decode_qr(&r.outputs[0]), vec!["https://basedpolymer.eu/castor".to_string()]);
    run("create.qr-code", &[], &[("format", "svg"), ("style", "diamond"), ("gradient", "linear")], &d);
    run("create.qr-code", &[], &[("format", "pdf"), ("style", "classy")], &d);
    // Logo in the middle: EC goes to H and it still scans.
    let logo = d.join("logo.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([47, 109, 181, 255])).save(&logo).unwrap();
    let r = run("create.qr-code", &[], &[("logo", logo.to_str().unwrap()), ("style", "rounded"), ("file_name", "with-logo")], &d);
    assert_eq!(decode_qr(&r.outputs[0]).len(), 1);
    for card in ["wifi", "table-tent", "review", "business", "ticket", "poster", "tag", "social", "invitation"] {
        let mut set = vec![("card", card), ("format", "png"), ("file_name", card)];
        if card == "wifi" {
            set.extend([("content", "wifi"), ("ssid", "Atelier"), ("wifi_password", "castor-2026")]);
        }
        run("create.qr-code", &[], &set, &d);
    }
    run("create.qr-code", &[], &[("card", "wifi"), ("content", "wifi"), ("ssid", "Atelier"), ("format", "pdf")], &d);
}

#[test]
fn barcodes() {
    let (d, _g) = out_dir("barcode");
    run("create.barcode", &[], &[("format", "ean13"), ("data", "590123412345")], &d);
    run("create.barcode", &[], &[("format", "code128"), ("data", "CASTOR-1\nCASTOR-2"), ("batch", "true"), ("output", "svg")], &d);
    run("create.barcode", &[], &[("format", "datamatrix"), ("data", "Castor"), ("output", "pdf")], &d);
}

#[test]
fn images_and_patterns() {
    let (d, _g) = out_dir("images");
    let r = run("create.placeholder", &[], &[("preset", "social"), ("label", "Hero")], &d);
    assert_eq!(r.outputs.len(), 6);
    assert_eq!(image::image_dimensions(&r.outputs[0]).unwrap(), (1200, 630));
    let r = run("create.favicon", &[], &[("text", "C"), ("shape", "diamond")], &d);
    assert!(r.outputs.iter().any(|p| p.ends_with("favicon.ico")));
    run("create.gradient", &[], &[("size", "640x360"), ("palette", "sunset")], &d);
    for s in ["silk", "aurora", "topography"] {
        run("create.shader", &[], &[("size", "480x270"), ("preset", s), ("file_name", s)], &d);
    }
    run("create.pattern", &[], &[("kind", "terrazzo"), ("output", "tile"), ("format", "svg")], &d);
    run("create.pattern", &[], &[("kind", "hexagons"), ("size", "600x400")], &d);
    run("create.watermark-tile", &[], &[("text", "DRAFT"), ("size", "800x600")], &d);
    run("create.mockup", &[], &[("device", "phone"), ("size", "800x600"), ("angle", "left")], &d);
}

#[test]
fn documents() {
    let (d, _g) = out_dir("docs");
    run("create.social-card", &[], &[("template", "classic")], &d);
    let csv = d.join("people.csv");
    std::fs::write(&csv, "name;course;date\nZoé Martin;Typography;6 Nov 2026\nAda Example;Kerning;7 Nov 2026\n").unwrap();
    let r = run("create.data-merge", &[csv.clone()], &[("layout", "certificate")], &d);
    assert_eq!(r.outputs.len(), 1);
    let r = run("create.data-merge", &[csv.clone()], &[("layout", "letter"), ("output", "each"), ("name_pattern", "letter-{name}")], &d);
    assert_eq!(r.outputs.len(), 2);
    let r = run("create.social-card", &[csv], &[("template", "minimal")], &d);
    assert_eq!(r.outputs.len(), 2);
    run("create.brand-sheet", &[], &[], &d);
    let r = run("create.carousel", &[], &[], &d);
    assert_eq!(r.outputs.len(), 6);
    run("create.formula", &[], &[("format", "svg")], &d);
    run("create.formula", &[], &[("format", "png"), ("formula", "\\sum_{k=1}^n k = \\frac{n(n+1)}{2}")], &d);
}

#[test]
fn fonts_convert_and_specimen() {
    let (d, _g) = out_dir("fonts");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/lmroman10-regular.otf");
    let r = run("create.fonts", &[src.clone()], &[("action", "convert"), ("to", "woff2")], &d);
    let w2 = r.outputs[0].clone();
    let r = run("create.fonts", &[w2], &[("action", "convert"), ("to", "sfnt")], &d);
    assert!(r.outputs[0].extension().unwrap() == "otf");
    run("create.fonts", &[src.clone()], &[("action", "convert"), ("to", "woff")], &d);
    run("create.fonts", &[src.clone()], &[("action", "specimen")], &d);
    run("create.fonts", &[src], &[("action", "info"), ("info_format", "json")], &d);
}

#[test]
fn videos() {
    if engine_media::ffmpeg().is_err() {
        eprintln!("ffmpeg missing: video tests skipped");
        return;
    }
    let (d, _g) = out_dir("video");
    let r = run("create.gradient", &[], &[("size", "320x180"), ("format", "mp4"), ("duration", "2"), ("fps", "12")], &d);
    let p = engine_media::probe(&r.outputs[0]).unwrap();
    assert!((p.duration - 2.0).abs() < 0.3, "{}", p.duration);
    run("create.motion", &[], &[("size", "320x180"), ("duration", "1.5"), ("format", "webm"), ("fps", "24")], &d);
    run("create.motion", &[], &[("size", "320x180"), ("duration", "1.5"), ("format", "mov"), ("preset", "sting"), ("fps", "24")], &d);
    // A 3-second tone for the audiogram.
    let tone = d.join("tone.wav");
    let st = std::process::Command::new(engine_media::ffmpeg().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=f=330:d=3"])
        .arg(&tone)
        .status()
        .unwrap();
    assert!(st.success());
    let srt = d.join("cues.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,500\nHello from Castor\n").unwrap();
    let r = run("create.audiogram", &[tone.clone()], &[("style", "circle"), ("aspect", "square"), ("captions", "file"), ("subtitles", srt.to_str().unwrap()), ("fps", "15")], &d);
    let p = engine_media::probe(&r.outputs[0]).unwrap();
    assert!(p.has_audio && (p.duration - 3.0).abs() < 0.4, "{p:?}");
    // Screen recording into a mockup.
    let clip = d.join("screen.mp4");
    let st = std::process::Command::new(engine_media::ffmpeg().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x400:d=1:r=15", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(st.success());
    run("create.mockup", &[clip], &[("device", "laptop"), ("size", "640x480"), ("angle", "right")], &d);
}

#[test]
fn html_with_installed_browser() {
    if tools_create::render::chrome::find_browser().is_none() {
        eprintln!("no Chromium-based browser: HTML tests skipped");
        return;
    }
    let (d, _g) = out_dir("html");
    let page = d.join("page.html");
    std::fs::write(&page, "<!doctype html><meta charset=utf-8><style>body{font:24px serif;background:#ffeca4}</style><h1>Castor ◇ é</h1><p>Hello.</p>").unwrap();
    let r = run("create.html-to-image", &[page.clone()], &[("viewport", "800x600"), ("scale", "1")], &d);
    assert_eq!(image::image_dimensions(&r.outputs[0]).unwrap().0, 800);
    let r = run("create.html-to-pdf", &[page], &[("header_footer", "true")], &d);
    assert!(std::fs::read(&r.outputs[0]).unwrap().starts_with(b"%PDF"));
    run("create.html-to-pdf", &[], &[("source", "html"), ("html", "<h1>Pasted</h1>")], &d);
}

#[test]
#[ignore = "network: downloads yt-dlp and a public-domain clip"]
fn video_download() {
    let (d, _g) = out_dir("download");
    run("create.video-download", &[], &[("urls", "https://archive.org/details/Popeye_forPresident"), ("quality", "audio")], &d);
}

#[allow(dead_code)]
fn unused(_: Value) {}
