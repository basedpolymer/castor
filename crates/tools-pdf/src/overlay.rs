//! Tools that draw on pages: page numbers, stamp/watermark, signature. All three have a live
//! preview (page 1 with the overlay).

use std::path::Path;

use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, Job, Output, Preview, Result, RunCx, Tool, ToolMeta};
use engine_pdf::draw::{self, Canvas};
use engine_pdf::fonts::{Face, FACE_CHOICES};
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::mupdf::{Matrix, Rect};

use crate::common::*;

fn font_choices() -> Vec<Choice> {
    FACE_CHOICES
        .iter()
        .map(|c| {
            Choice::lit(
                c,
                match *c {
                    "helvetica" => "Helvetica",
                    "helvetica-bold" => "Helvetica Bold",
                    "times" => "Times",
                    "times-bold" => "Times Bold",
                    "courier" => "Courier",
                    "latin-modern" => "Latin Modern",
                    "latin-modern-bold" => "Latin Modern Bold",
                    "latin-modern-sans" => "Latin Modern Sans",
                    _ => "TeX Gyre Chorus",
                },
            )
        })
        .collect()
}

/// Apply `f` to a copy of the first page of `input` and render it as a PNG preview.
fn preview_page(
    input: Option<&FileInfo>,
    v: &Values,
    max_side: u32,
    f: &dyn Fn(&mut PdfDocument, &[usize], usize, &Values) -> Result<()>,
) -> Result<Option<Preview>> {
    let Some(input) = input else { return Ok(None) };
    let mut src = engine_pdf::open_pdf(&input.path, password(v))?;
    let n = src.page_count()?.max(0) as usize;
    if n == 0 {
        return Ok(None);
    }
    let sel = pages(v, "pages", n).unwrap_or_else(|_| vec![0]);
    let first = sel.first().copied().unwrap_or(0);
    let mut doc = PdfDocument::new();
    engine_pdf::pages::copy_pages(&mut doc, &mut src, &[first], None)?;
    // Pretend the page keeps its number in the original document.
    f(&mut doc, &[0], n, &with_offset(v, first))?;
    let page = doc.load_page(0)?;
    let b = page.bounds()?;
    let s = max_side as f32 / b.width().max(b.height()).max(1.0);
    let pix = page.to_pixmap(&Matrix::new_scale(s, s), &mupdf::Colorspace::device_rgb(), false, true)?;
    let mut png = Vec::new();
    pix.write_to(&mut png, mupdf::ImageFormat::PNG)?;
    Ok(Some(Preview { png, caption: None }))
}

/// Values with a hidden "_first" = index of the previewed page in the original document.
fn with_offset(v: &Values, first: usize) -> Values {
    let mut v = v.clone();
    v.set("_first", castor_core::Value::Int(first as i64));
    v
}

fn save_with(doc: &mut PdfDocument, out: &Path, v: &Values) -> Result<()> {
    engine_pdf::stamp_producer(doc)?;
    engine_pdf::save_pdf(doc, out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })
}

// ------------------------------------------------------------------------------------------

pub struct PageNumbers {
    meta: ToolMeta,
}

impl PageNumbers {
    pub fn new() -> Self {
        PageNumbers {
            meta: meta("pdf.page-numbers", Family::Pdf, PDF_IN, "PDF", &["numbering", "paginate", "folio", "bates", "footer"], "numbered"),
        }
    }
}

fn number_label(v: &Values, num: i64, total: i64) -> String {
    let n = num.to_string();
    let t = total.to_string();
    match v.str("format") {
        "page_n" => tr_args("pdf.page-numbers.fmt.page_n", &[("n", &n)]),
        "n_of_total" => format!("{n} / {t}"),
        "page_n_of_total" => tr_args("pdf.page-numbers.fmt.page_n_of_total", &[("n", &n), ("total", &t)]),
        "roman" => roman(num.max(0) as usize, false),
        "roman_upper" => roman(num.max(0) as usize, true),
        "custom" => v.str("custom").replace("{n}", &n).replace("{total}", &t).replace("{roman}", &roman(num.max(0) as usize, false)),
        _ => n,
    }
}

fn apply_numbers(doc: &mut PdfDocument, sel: &[usize], _n: usize, v: &Values) -> Result<()> {
    let face = Face::parse(v.str("font"));
    let size = v.int("size").clamp(4, 96) as f32;
    let (rgb, alpha) = color(v, "color");
    let margin = mm(v.float("margin"));
    let skip = v.int("skip").max(0) as usize;
    let start = v.int("start");
    let first_offset = v.int("_first").max(0) as usize; // only set in previews
    let numbered: Vec<usize> = sel.iter().copied().skip(if first_offset > 0 { 0 } else { skip }).collect();
    let total = start + (sel.len().saturating_sub(skip)) as i64 - 1;
    let anchor = v.anchor("position");
    for (k, &p) in numbered.iter().enumerate() {
        let idx_in_sel = if first_offset > 0 { first_offset.saturating_sub(skip) } else { k };
        let num = start + idx_in_sel as i64;
        let label = number_label(v, num, total.max(num));
        let mut page = doc.load_pdf_page(p as i32)?;
        let mut canvas = Canvas::new(&page)?;
        let w = canvas.measure(doc, &mut page, face, size, &label)?;
        let (asc, desc) = canvas.metrics(doc, &mut page, face)?;
        let h = (asc - desc) * size;
        let mut a = anchor;
        let real_page = if first_offset > 0 { first_offset } else { p };
        if v.bool("mirror") && real_page % 2 == 1 {
            a = match a {
                Anchor::TopLeft => Anchor::TopRight,
                Anchor::TopRight => Anchor::TopLeft,
                Anchor::Left => Anchor::Right,
                Anchor::Right => Anchor::Left,
                Anchor::BottomLeft => Anchor::BottomRight,
                Anchor::BottomRight => Anchor::BottomLeft,
                other => other,
            };
        }
        let (x, y) = anchor_pos(a, canvas.bounds, w, h, margin);
        canvas.text(doc, &mut page, face, size, &label, x, y + asc * size, 0.0, rgb, alpha)?;
        canvas.commit(doc, &mut page, true)?;
    }
    Ok(())
}

impl Tool for PageNumbers {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![
                    Choice::lit("n", "1"),
                    Choice::key("page_n"),
                    Choice::lit("n_of_total", "1 / N"),
                    Choice::key("page_n_of_total"),
                    Choice::lit("roman", "i, ii, iii"),
                    Choice::lit("roman_upper", "I, II, III"),
                    Choice::key("custom"),
                ],
                "n",
            ),
            OptionSpec::text("custom", "{n}").when(Cond::Eq("format", "custom")),
            OptionSpec::anchor("position", Anchor::Bottom),
            OptionSpec::choice("font", font_choices(), "helvetica"),
            OptionSpec::int("size", 4, 96, 10).unit("pt"),
            OptionSpec::color("color", "#000000ff"),
            OptionSpec::int("start", -100000, 100000, 1),
            OptionSpec::int("skip", 0, 10000, 0),
            OptionSpec::float("margin", 0.0, 100.0, 10.0).unit("mm").advanced(),
            OptionSpec::toggle("mirror", false).advanced(),
            OptionSpec::pages("pages").common().advanced(),
            password_opt(),
        ]
    }
    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        preview_page(input, v, max_side, &apply_numbers)
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let mut v = v.clone();
        v.set("_first", castor_core::Value::Int(0));
        apply_numbers(&mut doc, &sel, n, &v)?;
        cx.progress(0.8);
        let out = out_for(cx, input, "pdf");
        save_with(&mut doc, &out, &v)?;
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &sel.len().saturating_sub(v.int("skip") as usize).to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Stamp {
    meta: ToolMeta,
}

impl Stamp {
    pub fn new() -> Self {
        Stamp {
            meta: meta("pdf.stamp", Family::Pdf, PDF_IN, "PDF", &["watermark", "confidential", "draft", "logo", "overlay", "brand"], "stamped"),
        }
    }
}

/// Load an image file for drawing; `knockout_white` makes near-white pixels transparent.
fn load_image(path: &Path, knockout_white: bool) -> Result<mupdf::Image> {
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    if !knockout_white {
        return Ok(mupdf::Image::from_bytes(&bytes)?);
    }
    let img = image::load_from_memory(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut rgba = img.to_rgba8();
    for p in rgba.pixels_mut() {
        let l = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
        // Soft knock-out: white → transparent, ink stays opaque.
        let a = if l >= 235 { 0 } else if l <= 170 { 255 } else { (235 - l) * 255 / 65 };
        p[3] = (p[3] as u32).min(a) as u8;
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(mupdf::Image::from_bytes(&png)?)
}

/// Size of the bounding box of a w×h box rotated by `angle` degrees.
fn rotated_extent(w: f32, h: f32, angle: f32) -> (f32, f32) {
    let (s, c) = angle.to_radians().sin_cos();
    (w * c.abs() + h * s.abs(), w * s.abs() + h * c.abs())
}

fn apply_stamp(doc: &mut PdfDocument, sel: &[usize], _n: usize, v: &Values) -> Result<()> {
    let opacity = v.float("opacity").clamp(0.02, 1.0) as f32;
    let angle = v.int("angle") as f32;
    let anchor = v.anchor("position");
    let margin = mm(v.float("margin"));
    let over = v.str("layer") != "under";
    let image_mode = v.str("kind") == "image";
    let img = if image_mode {
        let p = v.path("image").ok_or_else(|| anyhow::anyhow!("{}", tr("pdf.stamp.err.image")))?;
        Some(load_image(p, false)?)
    } else {
        None
    };
    let img_obj = match &img {
        Some(i) => Some(doc.add_image(i)?),
        None => None,
    };
    let text = v.str("text").replace("\\n", "\n");
    anyhow::ensure!(image_mode || !text.trim().is_empty(), "{}", tr("pdf.stamp.err.text"));
    let face = Face::parse(v.str("font"));
    let (rgb, alpha) = color(v, "color");
    for &p in sel {
        let mut page = doc.load_pdf_page(p as i32)?;
        let mut canvas = Canvas::new(&page)?;
        let b = canvas.bounds;
        if let (Some(i), Some(obj)) = (&img, &img_obj) {
            let w = b.width() * (v.float("image_width").clamp(1.0, 100.0) as f32 / 100.0);
            let h = w * i.height() as f32 / i.width().max(1) as f32;
            let (ew, eh) = rotated_extent(w, h, angle);
            let (x, y) = anchor_pos(anchor, b, ew, eh, margin);
            let (cx, cy) = (x + ew / 2.0, y + eh / 2.0);
            let r = Rect { x0: cx - w / 2.0, y0: cy - h / 2.0, x1: cx + w / 2.0, y1: cy + h / 2.0 };
            canvas.image_obj(doc, &mut page, obj, r, angle, opacity)?;
        } else {
            let mut size = v.int("size").clamp(4, 400) as f32;
            let lines: Vec<&str> = text.lines().collect();
            let (asc, desc) = canvas.metrics(doc, &mut page, face)?;
            let lh = (asc - desc) * 1.15;
            let mut widths: Vec<f32> = Vec::new();
            for l in &lines {
                widths.push(canvas.measure(doc, &mut page, face, 1.0, l)?);
            }
            let wmax = widths.iter().cloned().fold(0.0, f32::max).max(0.01);
            let block_h = lh * lines.len() as f32;
            // Shrink to fit the page if needed.
            let (ew, eh) = rotated_extent(wmax * size, block_h * size, angle);
            let fit = ((b.width() - 2.0 * margin) / ew).min((b.height() - 2.0 * margin) / eh);
            if fit < 1.0 {
                size *= fit.max(0.05);
            }
            let (bw, bh) = (wmax * size, block_h * size);
            let (ew, eh) = rotated_extent(bw, bh, angle);
            let (x, y) = anchor_pos(anchor, b, ew, eh, margin);
            let (cx, cy) = (x + ew / 2.0, y + eh / 2.0);
            let rot = draw::rotate_ccw(angle);
            for (i, l) in lines.iter().enumerate() {
                // Line origin relative to the block centre, unrotated, centred lines.
                let lx = -widths[i] * size / 2.0;
                let ly = -bh / 2.0 + (i as f32 * lh + asc) * size;
                let (ox, oy) = draw::apply(rot, lx, ly);
                canvas.text(doc, &mut page, face, size, l, cx + ox, cy + oy, angle, rgb, alpha * opacity)?;
            }
        }
        canvas.commit(doc, &mut page, over)?;
    }
    Ok(())
}

impl Tool for Stamp {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("kind", vec![Choice::key("text"), Choice::key("image")], "text"),
            OptionSpec::multiline("text", "CONFIDENTIAL").when(Cond::Eq("kind", "text")),
            OptionSpec::choice("font", font_choices(), "helvetica-bold").when(Cond::Eq("kind", "text")),
            OptionSpec::int("size", 4, 400, 60).unit("pt").when(Cond::Eq("kind", "text")),
            OptionSpec::color("color", "#c0392bff").when(Cond::Eq("kind", "text")),
            OptionSpec::file("image", &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"]).when(Cond::Eq("kind", "image")),
            OptionSpec::float("image_width", 1.0, 100.0, 30.0).slider().unit("%").when(Cond::Eq("kind", "image")),
            OptionSpec::float("opacity", 0.02, 1.0, 0.3).slider(),
            OptionSpec::int("angle", -180, 180, 45).slider().unit("°"),
            OptionSpec::anchor("position", Anchor::Center),
            OptionSpec::choice("layer", vec![Choice::key("over"), Choice::key("under")], "over"),
            OptionSpec::float("margin", 0.0, 100.0, 10.0).unit("mm").advanced(),
            OptionSpec::pages("pages").common().advanced(),
            password_opt(),
        ]
    }
    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        preview_page(input, v, max_side, &apply_stamp)
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        apply_stamp(&mut doc, &sel, n, v)?;
        cx.progress(0.8);
        let out = out_for(cx, input, "pdf");
        save_with(&mut doc, &out, v)?;
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &sel.len().to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Sign {
    meta: ToolMeta,
}

impl Sign {
    pub fn new() -> Self {
        Sign { meta: meta("pdf.sign", Family::Pdf, PDF_IN, "PDF", &["signature", "autograph", "initials", "esign", "contract"], "signed") }
    }
}

fn signature_dir() -> std::path::PathBuf {
    castor_core::paths::data_dir().join("signatures")
}

fn today() -> String {
    // Local date when the platform allows it, UTC otherwise.
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    tr_args(
        "pdf.sign.date_fmt",
        &[("y", &format!("{:04}", now.year())), ("m", &format!("{:02}", now.month() as u8)), ("d", &format!("{:02}", now.day()))],
    )
}

fn sign_pages(v: &Values, n: usize) -> Result<Vec<usize>> {
    Ok(match v.str("page") {
        "first" => vec![0],
        "all" => (0..n).collect(),
        "custom" => pages(v, "pages", n)?,
        _ => vec![n.saturating_sub(1)],
    })
}

fn apply_sign(doc: &mut PdfDocument, sel: &[usize], _n: usize, v: &Values) -> Result<()> {
    let width = mm(v.float("width"));
    let margin = mm(v.float("margin"));
    let anchor = v.anchor("position");
    let (ink, _) = color(v, "color");
    let typed = v.str("kind") != "image";
    let name = v.str("name").trim().to_string();
    anyhow::ensure!(!typed || !name.is_empty(), "{}", tr("pdf.sign.err.name"));
    let img = if typed {
        None
    } else {
        let p = v.path("image").ok_or_else(|| anyhow::anyhow!("{}", tr("pdf.sign.err.image")))?;
        Some(load_image(p, v.bool("remove_background"))?)
    };
    let img_obj = match &img {
        Some(i) => Some(doc.add_image(i)?),
        None => None,
    };
    let date = v.bool("date").then(today);
    for &p in sel {
        let mut page = doc.load_pdf_page(p as i32)?;
        let mut canvas = Canvas::new(&page)?;
        let b = canvas.bounds;
        let date_size = (width / 14.0).clamp(6.0, 11.0);
        let date_h = if date.is_some() { date_size * 1.6 } else { 0.0 };
        if let (Some(i), Some(obj)) = (&img, &img_obj) {
            let h = width * i.height() as f32 / i.width().max(1) as f32;
            let (x, y) = anchor_pos(anchor, b, width, h + date_h, margin);
            canvas.image_obj(doc, &mut page, obj, Rect { x0: x, y0: y, x1: x + width, y1: y + h }, 0.0, 1.0)?;
            if let Some(d) = &date {
                canvas.text(doc, &mut page, Face::Helvetica, date_size, d, x, y + h + date_size * 1.2, 0.0, [0.25, 0.25, 0.25], 1.0)?;
            }
        } else {
            let w1 = canvas.measure(doc, &mut page, Face::Chorus, 1.0, &name)?.max(0.01);
            let size = (width / w1).min(width * 0.6);
            let (asc, desc) = canvas.metrics(doc, &mut page, Face::Chorus)?;
            let h = (asc - desc) * size;
            let tw = w1 * size;
            let (x, y) = anchor_pos(anchor, b, tw, h + date_h, margin);
            canvas.text(doc, &mut page, Face::Chorus, size, &name, x, y + asc * size, 0.0, ink, 1.0)?;
            if let Some(d) = &date {
                canvas.text(doc, &mut page, Face::Helvetica, date_size, d, x, y + h + date_size * 1.2, 0.0, [0.25, 0.25, 0.25], 1.0)?;
            }
        }
        canvas.commit(doc, &mut page, true)?;
    }
    Ok(())
}

impl Tool for Sign {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("kind", vec![Choice::key("typed"), Choice::key("image")], "typed"),
            OptionSpec::text("name", "").when(Cond::Eq("kind", "typed")),
            OptionSpec::color("color", "#1b2a6bff").when(Cond::Eq("kind", "typed")),
            OptionSpec::file("image", &["png", "jpg", "jpeg", "webp"]).when(Cond::Eq("kind", "image")),
            OptionSpec::toggle("remove_background", true).when(Cond::Eq("kind", "image")),
            OptionSpec::choice("page", vec![Choice::key("last"), Choice::key("first"), Choice::key("all"), Choice::key("custom")], "last"),
            OptionSpec::pages("pages").common().when(Cond::Eq("page", "custom")),
            OptionSpec::anchor("position", Anchor::BottomRight),
            OptionSpec::float("width", 10.0, 200.0, 50.0).unit("mm"),
            OptionSpec::toggle("date", false),
            OptionSpec::float("margin", 0.0, 100.0, 20.0).unit("mm").advanced(),
            OptionSpec::toggle("save", false).advanced(),
            password_opt(),
        ]
    }
    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(input) = input else { return Ok(None) };
        let n = engine_pdf::page_count(&input.path).unwrap_or(1);
        let first = sign_pages(v, n).ok().and_then(|s| s.first().copied()).unwrap_or(0);
        let mut v2 = v.clone();
        v2.set("pages", castor_core::Value::Text((first + 1).to_string()));
        preview_page(Some(input), &v2, max_side, &apply_sign)
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = sign_pages(v, n)?;
        apply_sign(&mut doc, &sel, n, v)?;
        if v.bool("save") {
            let dir = signature_dir();
            std::fs::create_dir_all(&dir)?;
            if v.str("kind") == "image" {
                if let Some(p) = v.path("image") {
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "signature.png".into());
                    let dest = castor_core::job::unique_path(&dir.join(name));
                    std::fs::copy(p, dest)?;
                }
            } else {
                let dest = castor_core::job::unique_path(&dir.join(format!("{}.txt", castor_core::job::sanitize_file_name(v.str("name")))));
                std::fs::write(dest, v.str("name"))?;
            }
        }
        cx.progress(0.8);
        let out = out_for(cx, input, "pdf");
        save_with(&mut doc, &out, v)?;
        Ok(vec![Output::new(out).with_note(tr("pdf.sign.note"))])
    }
}
