//! Integration tests: every image tool on small synthetic files (no network needed).
//! ML tools are exercised only when their model is already downloaded.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use castor_core::options::{Value, Values};
use castor_core::{CancelToken, FileInfo, Output, RunCx, Tool};
use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};

fn tool(id: &str) -> Arc<dyn Tool> {
    tools_image::tools().into_iter().find(|t| t.meta().id == id).unwrap_or_else(|| panic!("no tool {id}"))
}

fn run(id: &str, inputs: &[PathBuf], set: &[(&str, Value)], out: &Path) -> Vec<Output> {
    let t = tool(id);
    let mut v = Values::defaults(&t.options());
    for (k, x) in set {
        assert!(t.options().iter().any(|o| o.key == *k), "{id} has no option {k}");
        v.set(k, x.clone());
    }
    let files: Vec<FileInfo> = inputs.iter().map(FileInfo::from_path).collect();
    for f in &files {
        assert!(t.accepts(f) || t.meta().inputs.folders, "{id} refuses {}", f.name());
    }
    let jobs = t.plan(&files, &v).expect("plan");
    let mut outs = Vec::new();
    for j in jobs {
        let tmp = out.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let cx = RunCx::new(out.to_path_buf(), tmp, t.meta().suffix, CancelToken::new());
        outs.extend(t.run(&j, &v, &cx).unwrap_or_else(|e| panic!("{id}: {e:#}")));
    }
    for o in &outs {
        assert!(o.path.exists(), "{id}: missing output {}", o.path.display());
    }
    outs
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

/// A photo-like test image with gradients, edges and a "subject".
fn photo(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let d = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt();
        if d < 0.2 {
            Rgb([(200.0 + 50.0 * fx) as u8, (80.0 + 60.0 * fy) as u8, 60])
        } else {
            Rgb([(40.0 + 120.0 * fy) as u8, (90.0 + 100.0 * fx) as u8, (160.0 + 60.0 * (fx * 7.0).sin()) as u8])
        }
    })
}

struct Fx {
    dir: tempfile::TempDir,
}

impl Fx {
    fn new() -> Fx {
        Fx { dir: tempfile::tempdir().unwrap() }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn out(&self) -> PathBuf {
        let p = self.dir.path().join("out");
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn jpeg(&self, name: &str, w: u32, h: u32) -> PathBuf {
        let p = self.path(name);
        DynamicImage::ImageRgb8(photo(w, h)).save(&p).unwrap();
        p
    }
    fn png_alpha(&self, name: &str) -> PathBuf {
        let p = self.path(name);
        let img = RgbaImage::from_fn(200, 120, |x, y| {
            if (x as i32 - 100).abs() < 60 && (y as i32 - 60).abs() < 40 { Rgba([20, 120, 220, 255]) } else { Rgba([0, 0, 0, 0]) }
        });
        img.save(&p).unwrap();
        p
    }
}

fn dims(p: &Path) -> (u32, u32) {
    let d = tools_image::codec::decode(p, Default::default()).expect("decode output");
    (d.image.width(), d.image.height())
}

#[test]
fn convert_to_every_format() {
    let fx = Fx::new();
    let src = fx.jpeg("photo é.jpg", 320, 200); // unicode name
    for fmt in ["png", "webp", "avif", "gif", "tiff", "bmp", "ico", "jpg"] {
        let o = run("image.convert", &[src.clone()], &[("format", t(fmt))], &fx.out());
        assert_eq!(o[0].path.extension().unwrap().to_string_lossy(), if fmt == "tiff" { "tiff" } else { fmt });
        if fmt != "ico" {
            assert_eq!(dims(&o[0].path), (320, 200), "{fmt}");
        }
    }
    // Transparency onto JPEG uses the background colour.
    let a = fx.png_alpha("logo.png");
    let o = run("image.convert", &[a], &[("format", t("jpg")), ("background", t("#ff0000"))], &fx.out());
    let img = image::open(&o[0].path).unwrap().to_rgb8();
    let p = img.get_pixel(2, 2);
    assert!(p[0] > 200 && p[1] < 60, "{p:?}");
}

#[test]
fn convert_keeps_animation() {
    let fx = Fx::new();
    let p = fx.path("anim.gif");
    {
        let f = std::fs::File::create(&p).unwrap();
        let mut enc = image::codecs::gif::GifEncoder::new(f);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        for i in 0..6u32 {
            let img = RgbaImage::from_fn(64, 48, |x, _| if x / 8 == i { Rgba([255, 0, 0, 255]) } else { Rgba([255, 255, 255, 255]) });
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(80, 1))).unwrap();
        }
    }
    let o = run("image.convert", &[p], &[("format", t("webp"))], &fx.out());
    let data = std::fs::read(&o[0].path).unwrap();
    let frames = tools_image::codec::decode_frames(&data, tools_image::codec::Fmt::Webp).unwrap();
    assert_eq!(frames.map(|f| f.len()), Some(6));
}

#[test]
fn decodes_svg() {
    let fx = Fx::new();
    let p = fx.path("s.svg");
    std::fs::write(&p, r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="red"/></svg>"#).unwrap();
    let o = run("image.convert", &[p.clone()], &[("format", t("png"))], &fx.out());
    assert_eq!(dims(&o[0].path), (100, 50));
    let o = run("image.convert", &[p], &[("format", t("png")), ("svg_size", Value::Int(400))], &fx.out());
    assert_eq!(dims(&o[0].path), (400, 200));
}

#[test]
fn compress_never_bigger_and_shrinks() {
    let fx = Fx::new();
    let src = fx.path("big.jpg");
    // A high-quality JPEG compresses well.
    let img = photo(800, 600);
    let mut f = std::fs::File::create(&src).unwrap();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 98).encode_image(&img).unwrap();
    drop(f);
    let before = std::fs::metadata(&src).unwrap().len();
    for mode in ["quality", "identical", "size"] {
        let o = run("image.compress", &[src.clone()], &[("mode", t(mode)), ("target_kb", Value::Int(20))], &fx.out());
        let after = std::fs::metadata(&o[0].path).unwrap().len();
        assert!(after < before, "{mode}: {after} >= {before}");
    }
    // An already tiny file is never made bigger.
    let tiny = fx.path("tiny.png");
    RgbImage::from_pixel(8, 8, Rgb([1, 2, 3])).save(&tiny).unwrap();
    let before = std::fs::metadata(&tiny).unwrap().len();
    let o = run("image.compress", &[tiny], &[], &fx.out());
    assert!(std::fs::metadata(&o[0].path).unwrap().len() <= before);
}

#[test]
fn jpeg_outputs_decode_with_the_image_crate() {
    // Our own outputs must be readable by our own decoder (chains: compress → resize…).
    let fx = Fx::new();
    let src = fx.path("big.png");
    DynamicImage::ImageRgb8(photo(1900, 1300)).save(&src).unwrap();
    for mode in ["quality", "identical", "size"] {
        let o = run("image.compress", &[src.clone()], &[("mode", t(mode)), ("format", t("jpg")), ("target_kb", Value::Int(60))], &fx.out());
        let img = image::open(&o[0].path).unwrap_or_else(|e| panic!("{mode}: {e}"));
        assert_eq!((img.width(), img.height()), (1900, 1300));
    }
}

#[test]
fn compress_png_palette() {
    let fx = Fx::new();
    let p = fx.path("shot.png");
    // A photo with sensor-like noise: lossless PNG is big, a good palette is much smaller.
    let mut img = photo(400, 300);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let n = ((x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 7) % 9;
        for c in 0..3 {
            px[c] = px[c].saturating_add(n as u8);
        }
    }
    DynamicImage::ImageRgb8(img).save(&p).unwrap();
    let before = std::fs::metadata(&p).unwrap().len();
    let o = run("image.compress", &[p], &[], &fx.out());
    let after = std::fs::metadata(&o[0].path).unwrap().len();
    assert!(after < before, "{after} >= {before}");
    assert_eq!(dims(&o[0].path), (400, 300));
}

#[test]
fn resize_modes() {
    let fx = Fx::new();
    let src = fx.jpeg("r.jpg", 1200, 800);
    let o = run("image.resize", &[src.clone()], &[("size", Value::Size(600, 600))], &fx.out());
    assert_eq!(dims(&o[0].path), (600, 400));
    let o = run("image.resize", &[src.clone()], &[("preset", t("instagram-portrait")), ("mode", t("fill"))], &fx.out());
    assert_eq!(dims(&o[0].path), (1080, 1350));
    let o = run("image.resize", &[src.clone()], &[("mode", t("pad")), ("size", Value::Size(500, 500))], &fx.out());
    assert_eq!(dims(&o[0].path), (500, 500));
    let o = run("image.resize", &[src], &[("mode", t("percent")), ("percent", Value::Int(25)), ("format", t("webp"))], &fx.out());
    assert_eq!(dims(&o[0].path), (300, 200));
}

#[test]
fn crop_ratio_and_trim() {
    let fx = Fx::new();
    let src = fx.jpeg("c.jpg", 400, 300);
    let o = run("image.crop", &[src], &[("ratio", t("1-1"))], &fx.out());
    assert_eq!(dims(&o[0].path), (300, 300));
    let p = fx.path("bordered.png");
    let img = RgbImage::from_fn(300, 200, |x, y| if (50..250).contains(&x) && (40..160).contains(&y) { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) });
    img.save(&p).unwrap();
    let o = run("image.crop", &[p], &[("mode", t("trim"))], &fx.out());
    assert_eq!(dims(&o[0].path), (200, 120));
}

#[test]
fn rotate_and_flip() {
    let fx = Fx::new();
    let src = fx.jpeg("o.jpg", 300, 100);
    let o = run("image.rotate", &[src.clone()], &[], &fx.out());
    assert_eq!(dims(&o[0].path), (100, 300));
    let o = run("image.rotate", &[src], &[("angle", t("none")), ("straighten", Value::Float(5.0))], &fx.out());
    let (w, h) = dims(&o[0].path);
    assert!(w < 300 && h < 100, "{w}x{h}");
}

#[test]
fn strip_metadata_without_recompression() {
    let fx = Fx::new();
    let src = fx.jpeg("m.jpg", 64, 64);
    // Add EXIF (artist + orientation) and XMP.
    let bytes = std::fs::read(&src).unwrap();
    let exif = tools_image::meta::minimal_exif(Some(6), Some("basedpolymer"), Some("(c) Castor")).unwrap();
    let xmp = tools_image::meta::build_xmp(&tools_image::meta::XmpFields { title: Some("x".into()), ..Default::default() });
    let with = tools_image::meta::embed(bytes, tools_image::codec::Fmt::Jpeg, None, Some(&exif), Some(&xmp)).unwrap();
    std::fs::write(&src, &with).unwrap();
    let o = run("image.strip-metadata", &[src.clone()], &[], &fx.out());
    let out = std::fs::read(&o[0].path).unwrap();
    assert!(out.len() < with.len());
    let s = String::from_utf8_lossy(&out);
    assert!(!s.contains("basedpolymer") && !s.contains("xmpmeta"));
    // Orientation kept (minimal EXIF), pixels untouched: the scan data is byte-identical.
    assert_eq!(tools_image::codec::exif_orientation(&exif), Some(6));
    let tail = |b: &[u8]| b[b.windows(2).rposition(|w| w == [0xFF, 0xDA]).unwrap()..].to_vec();
    assert_eq!(tail(&out), tail(&with));
}

#[test]
fn watermark_text_and_tiled() {
    let fx = Fx::new();
    let src = fx.jpeg("w.jpg", 400, 300);
    for layout in ["single", "tiled"] {
        let o = run("image.watermark", &[src.clone()], &[("layout", t(layout)), ("text", t("CASTOR"))], &fx.out());
        let a = image::open(&src).unwrap().to_rgb8();
        let b = image::open(&o[0].path).unwrap().to_rgb8();
        let diff = a.pixels().zip(b.pixels()).filter(|(p, q)| (p[0] as i32 - q[0] as i32).abs() > 30).count();
        assert!(diff > 200, "{layout}: watermark not visible ({diff})");
    }
}

#[test]
fn beautify_stylize_colorblind() {
    let fx = Fx::new();
    let src = fx.jpeg("s.jpg", 320, 200);
    let o = run("image.beautify", &[src.clone()], &[("frame", t("macos")), ("ratio", t("1-1"))], &fx.out());
    let (w, h) = dims(&o[0].path);
    assert_eq!(w, h);
    for look in ["film", "halftone", "sketch", "duotone", "pixel"] {
        run("image.stylize", &[src.clone()], &[("look", t(look))], &fx.out());
    }
    let o = run("image.colorblind", &[src], &[], &fx.out());
    assert_eq!(o.len(), 2);
}

#[test]
fn previews_render() {
    let fx = Fx::new();
    let src = FileInfo::from_path(fx.jpeg("p.jpg", 640, 480));
    for id in ["image.watermark", "image.stylize", "image.crop", "image.palette", "image.beautify", "image.compare", "image.colorblind", "image.resize", "image.rotate"] {
        let tl = tool(id);
        let v = Values::defaults(&tl.options());
        let p = tl.preview(Some(&src), &v, 400).unwrap_or_else(|e| panic!("{id}: {e:#}"));
        let p = p.unwrap_or_else(|| panic!("{id}: no preview"));
        let img = image::load_from_memory(&p.png).unwrap();
        assert!(img.width().max(img.height()) <= 400, "{id}");
    }
    // Beautify and watermark preview without a file too.
    for id in ["image.beautify", "image.watermark"] {
        let tl = tool(id);
        assert!(tl.preview(None, &Values::defaults(&tl.options()), 300).unwrap().is_some());
    }
}

#[test]
fn palette_files() {
    let fx = Fx::new();
    let src = fx.jpeg("pal.jpg", 200, 200);
    let o = run("image.palette", &[src], &[], &fx.out());
    let exts: Vec<String> = o.iter().map(|x| x.path.extension().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(exts, ["png", "ase", "gpl", "css", "json"]);
    let ase = std::fs::read(&o[1].path).unwrap();
    assert!(ase.starts_with(b"ASEF"));
}

#[test]
fn icon_pack_all() {
    let fx = Fx::new();
    let src = fx.png_alpha("icon.png");
    let o = run("image.icon-pack", &[src], &[], &fx.out());
    let d = &o[0].path;
    for f in ["favicon.ico", "icon.ico", "icon.icns", "site.webmanifest", "apple-touch-icon.png", "AppIcon.appiconset/Contents.json", "AppIcon.appiconset/Icon-1024.png", "android/mipmap-xxxhdpi/ic_launcher.png"] {
        assert!(d.join(f).is_file(), "missing {f}");
    }
    let icns = std::fs::File::open(d.join("icon.icns")).unwrap();
    let fam = icns::IconFamily::read(icns).unwrap();
    assert!(fam.available_icons().len() >= 8);
    let ico = image::open(d.join("icon.ico")).unwrap();
    assert_eq!(ico.width(), 256);
}

#[test]
fn contact_sheet_pdf_and_png() {
    let fx = Fx::new();
    let files: Vec<PathBuf> = (0..7).map(|i| fx.jpeg(&format!("c{i}.jpg"), 300 + i * 10, 200)).collect();
    let o = run("image.contact-sheet", &files, &[("title", t("Trip"))], &fx.out());
    let pdf = std::fs::read(&o[0].path).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
    let o = run("image.contact-sheet", &files, &[("output", t("png")), ("columns", Value::Int(3))], &fx.out());
    assert_eq!(o.len(), 1);
}

#[test]
fn compare_outputs() {
    let fx = Fx::new();
    let a = fx.jpeg("before.jpg", 300, 200);
    let b = fx.png_alpha("after.png");
    let o = run("image.compare", &[a.clone(), b.clone()], &[], &fx.out());
    let html = std::fs::read_to_string(&o[0].path).unwrap();
    assert!(html.contains("data:image/jpeg;base64,") && html.contains("type=\"range\""));
    let o = run("image.compare", &[a.clone(), b.clone()], &[("output", t("gif"))], &fx.out());
    let frames = tools_image::codec::decode_frames(&std::fs::read(&o[0].path).unwrap(), tools_image::codec::Fmt::Gif).unwrap();
    assert!(frames.map(|f| f.len()).unwrap_or(0) > 10);
    let o = run("image.compare", &[a, b], &[("output", t("png")), ("layout", t("side"))], &fx.out());
    assert!(dims(&o[0].path).0 > 600);
}

#[test]
fn optimize_svg_and_gif() {
    let fx = Fx::new();
    let p = fx.path("v.svg");
    std::fs::write(&p, "<?xml version=\"1.0\"?>\n<!-- c -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" width=\"100.000\" height=\"100.000\" inkscape:version=\"1\">\n  <metadata>m</metadata>\n  <path d=\"M 10.123456 20.987654 L 90.000001 80.000001 Z\" fill=\"#123456\"/>\n</svg>\n").unwrap();
    let o = run("image.optimize-svg", &[p.clone()], &[], &fx.out());
    let s = std::fs::read_to_string(&o[0].path).unwrap();
    assert!(s.contains("d=\"M10.123 20.988L90 80Z\""), "{s}");
    assert!(!s.contains("inkscape") && !s.contains("metadata"));

    let g = fx.path("a.gif");
    {
        let f = std::fs::File::create(&g).unwrap();
        let mut enc = image::codecs::gif::GifEncoder::new(f);
        for i in 0..10u32 {
            let img = RgbaImage::from_fn(160, 120, |x, y| Rgba([((x + i * 7) % 256) as u8, (y * 2) as u8, 128, 255]));
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(30, 1))).unwrap();
        }
    }
    let o = run("image.optimize-gif", &[g.clone()], &[("fps", t("15"))], &fx.out());
    assert!(std::fs::metadata(&o[0].path).unwrap().len() <= std::fs::metadata(&g).unwrap().len());
}

#[test]
fn duplicates_finds_resized_copy_and_undoes_moves() {
    let fx = Fx::new();
    let dir = fx.path("set");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.jpg");
    DynamicImage::ImageRgb8(photo(400, 300)).save(&a).unwrap();
    let b = dir.join("b.jpg");
    DynamicImage::ImageRgb8(photo(400, 300)).resize(200, 150, image::imageops::FilterType::Lanczos3).save(&b).unwrap();
    let c = dir.join("c.png");
    RgbImage::from_fn(400, 300, |x, y| Rgb([(x ^ y) as u8, (x * 3) as u8, (y * 5) as u8])).save(&c).unwrap();
    let o = run("image.duplicates", &[dir.clone()], &[("action", t("move"))], &fx.out());
    assert!(a.exists() && !b.exists() && c.exists(), "the smaller copy is moved aside");
    let journal = o[0].undo.clone().unwrap();
    tool("image.duplicates").undo(&journal).unwrap();
    assert!(b.exists());
}

#[test]
fn vectorize_logo() {
    let fx = Fx::new();
    let src = fx.png_alpha("logo.png");
    let o = run("image.vectorize", &[src], &[], &fx.out());
    let svg = std::fs::read_to_string(&o[0].path).unwrap();
    assert!(svg.contains("<svg") && svg.contains("<path"));
    tools_image::codec::render_svg(svg.as_bytes(), 200, None).unwrap();
}

#[test]
fn seo_renames_and_writes_xmp() {
    let fx = Fx::new();
    let a = fx.jpeg("IMG_0001.jpg", 100, 80);
    let b = fx.jpeg("IMG_0002.jpg", 100, 80);
    let o = run(
        "image.seo",
        &[a, b],
        &[("title", t("Red bike in Paris")), ("alt", t("A red bike")), ("keywords", t("bike, paris")), ("gps", t("48.85, 2.35"))],
        &fx.out(),
    );
    let names: Vec<String> = o.iter().map(|x| x.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["red-bike-in-paris-1.jpg", "red-bike-in-paris-2.jpg"]);
    let s = String::from_utf8_lossy(&std::fs::read(&o[0].path).unwrap()).into_owned();
    assert!(s.contains("A red bike") && s.contains("Photoshop 3.0") && s.contains("GPSLatitude"));
}

#[test]
fn redact_regions_without_models() {
    let fx = Fx::new();
    let src = fx.jpeg("r.jpg", 300, 200);
    let o = run("image.redact", &[src.clone()], &[("faces", Value::Bool(false)), ("regions", t("0,0,100,100")), ("style", t("box"))], &fx.out());
    let img = image::open(&o[0].path).unwrap().to_rgb8();
    let p = img.get_pixel(50, 50);
    assert!(p[0] < 20 && p[1] < 20 && p[2] < 20, "{p:?}");
}

#[test]
fn corrupt_and_empty_inputs_fail_cleanly() {
    let fx = Fx::new();
    let empty = fx.path("empty.jpg");
    std::fs::write(&empty, b"").unwrap();
    let bad = fx.path("bad.png");
    std::fs::write(&bad, b"\x89PNG\r\n\x1a\nnot really").unwrap();
    let t = tool("image.convert");
    let v = Values::defaults(&t.options());
    for p in [empty, bad] {
        let cx = RunCx::new(fx.out(), fx.out(), "", CancelToken::new());
        let job = castor_core::Job::single(FileInfo::from_path(&p));
        let e = t.run(&job, &v, &cx).unwrap_err();
        assert!(!format!("{e:#}").is_empty());
    }
}

#[test]
fn decodes_real_samples_when_present() {
    // Fixtures fetched by hand into tests/fixtures/generated/image (gitignored).
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/generated/image");
    for name in ["sample.heic", "sample.avif", "sample.jxl"] {
        let p = dir.join(name);
        if p.exists() {
            let d = tools_image::codec::decode(&p, Default::default()).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert!(d.image.width() > 16, "{name}");
        }
    }
}

#[test]
fn remove_background_when_model_present() {
    let Some(a) = engine_ml::models::asset("isnet-general-use") else { return };
    if !a.is_present() {
        eprintln!("skipped: isnet model not downloaded");
        return;
    }
    let fx = Fx::new();
    let src = fx.jpeg("subject.jpg", 320, 240);
    let o = run("image.remove-background", &[src], &[("model", t("fast"))], &fx.out());
    let img = image::open(&o[0].path).unwrap().to_rgba8();
    assert!(img.pixels().any(|p| p[3] == 0) && img.pixels().any(|p| p[3] == 255));
}
