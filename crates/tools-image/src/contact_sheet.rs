//! image.contact-sheet — many images → grid pages with header and captions, as PNG, JPEG or PDF.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::{common, ops};

pub struct ContactSheet {
    meta: ToolMeta,
}

impl ContactSheet {
    pub fn new() -> Self {
        ContactSheet {
            meta: common::meta(
                "image.contact-sheet",
                "PDF · PNG · JPG",
                &["contact sheet", "grid", "thumbnails", "overview", "proof", "index", "catalog", "collage"],
                "contact-sheet",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for ContactSheet {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("output", vec![Choice::lit("pdf", "PDF"), Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG")], "pdf"),
            OptionSpec::choice(
                "page",
                vec![Choice::lit("a4", "A4"), Choice::lit("a4-landscape", "A4 ↔"), Choice::lit("letter", "Letter"), Choice::lit("letter-landscape", "Letter ↔")],
                "a4",
            ),
            OptionSpec::int("columns", 1, 12, 4).slider(),
            OptionSpec::text("title", ""),
            OptionSpec::toggle("date", true),
            OptionSpec::choice("captions", vec![Choice::key("none"), Choice::key("name"), Choice::key("details")], "name"),
            OptionSpec::color("background", "#ffffff"),
            OptionSpec::text("watermark", "").advanced(),
        ]
    }

    /// All inputs make one sheet.
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", castor_core::tr("image.contact-sheet.empty"));
        Ok(vec![Job::many(inputs.to_vec())])
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let pages = render(&job.inputs, v, 300.0, cx)?;
        let first = job.inputs.first();
        let name = |ext: &str| match first {
            Some(f) if job.inputs.len() == 1 => cx.output_path(Some(f), ext),
            _ => cx.output_named(&format!("{}.{ext}", castor_core::tr("image.contact-sheet.file"))),
        };
        let mut outs = Vec::new();
        match v.str("output") {
            "pdf" => {
                let (pw, ph) = page_pt(v.str("page"));
                let mut jpegs = Vec::new();
                for p in &pages {
                    jpegs.push((codec::encode(&DynamicImage::ImageRgba8(p.clone()), Fmt::Jpeg, &EncodeOpts { quality: 88, ..Default::default() })?, p.width(), p.height()));
                }
                let out = name("pdf");
                common::write(&out, &pdf_from_jpegs(&jpegs, pw, ph))?;
                outs.push(Output::new(out).with_note(castor_core::tr_args("image.contact-sheet.pages", &[("count", &pages.len().to_string())])));
            }
            fmt => {
                let f = if fmt == "jpg" { Fmt::Jpeg } else { Fmt::Png };
                for (i, p) in pages.iter().enumerate() {
                    let ext = f.ext();
                    let out = if pages.len() == 1 {
                        name(ext)
                    } else {
                        cx.output_named(&format!("{}-{}.{ext}", castor_core::tr("image.contact-sheet.file"), i + 1))
                    };
                    let b = codec::encode(&DynamicImage::ImageRgba8(p.clone()), f, &EncodeOpts { quality: 90, png_effort: 1, ..Default::default() })?;
                    common::write(&out, &b)?;
                    outs.push(Output::new(out));
                }
            }
        }
        Ok(outs)
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        // Preview: the selected file repeated to fill the first page, at screen resolution.
        let files: Vec<FileInfo> = std::iter::repeat_n(f.clone(), (v.int("columns").clamp(1, 12) * 3) as usize).collect();
        let cx = RunCx::new(std::env::temp_dir(), std::env::temp_dir(), "", castor_core::CancelToken::new());
        let pages = render(&files, v, 72.0, &cx)?;
        common::preview(&pages[0], max_side, None)
    }
}

/// Page size in PDF points.
fn page_pt(page: &str) -> (f32, f32) {
    match page {
        "a4-landscape" => (841.89, 595.28),
        "letter" => (612.0, 792.0),
        "letter-landscape" => (792.0, 612.0),
        _ => (595.28, 841.89),
    }
}

struct Thumb {
    img: RgbaImage,
    caption: Vec<String>,
}

fn caption(f: &FileInfo, w: u32, h: u32, mode: &str) -> Vec<String> {
    match mode {
        "none" => vec![],
        "details" => {
            let mut l = vec![f.name(), format!("{} · {}", common::dims(w, h), castor_core::units::bytes(f.size))];
            if let Some(d) = exif_date(&f.path) {
                l.push(d);
            }
            l
        }
        _ => vec![f.name()],
    }
}

fn exif_date(p: &std::path::Path) -> Option<String> {
    let file = std::fs::File::open(p).ok()?;
    let r = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    let f = r.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)?;
    Some(f.display_value().to_string())
}

/// Render pages at `dpi`.
fn render(files: &[FileInfo], v: &Values, dpi: f32, cx: &RunCx) -> Result<Vec<RgbaImage>> {
    let (pw, ph) = page_pt(v.str("page"));
    let (w, h) = ((pw / 72.0 * dpi) as u32, (ph / 72.0 * dpi) as u32);
    let u = dpi / 72.0; // pixels per point
    let margin = 36.0 * u;
    let cols = v.int("columns").clamp(1, 12) as u32;
    let gap = 12.0 * u;
    let cell_w = (w as f32 - 2.0 * margin - gap * (cols - 1) as f32) / cols as f32;
    let cap_lines = match v.str("captions") {
        "none" => 0,
        "details" => 3,
        _ => 1,
    };
    let cap_size = 7.5 * u;
    let cap_h = if cap_lines > 0 { cap_lines as f32 * cap_size * 1.3 + 4.0 * u } else { 0.0 };
    let cell_h = cell_w * 0.75 + cap_h;
    let bg = v.color("background");
    let dark_bg = (bg[0] as u32 + bg[1] as u32 + bg[2] as u32) < 384;
    let fg = if dark_bg { [235, 235, 235, 255] } else { [30, 30, 30, 255] };
    let sub = if dark_bg { [170, 170, 170, 255] } else { [110, 110, 110, 255] };

    // Header
    let title = v.str("title").trim().to_string();
    let date = if v.bool("date") {
        let fmt = time::macros::format_description!("[year]-[month]-[day]");
        time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc()).format(fmt).unwrap_or_default()
    } else {
        String::new()
    };
    let header_h = if title.is_empty() && date.is_empty() { 0.0 } else { 34.0 * u };
    let rows_per_page = (((h as f32 - 2.0 * margin - header_h + gap) / (cell_h + gap)).floor() as u32).max(1);
    let per_page = (rows_per_page * cols) as usize;

    let mut pages = Vec::new();
    let n_pages = files.len().div_ceil(per_page);
    for (pi, chunk) in files.chunks(per_page).enumerate() {
        let mut page = RgbaImage::from_pixel(w, h, Rgba(bg));
        if header_h > 0.0 {
            if let Some(t) = ops::render_text(if title.is_empty() { &date } else { &title }, "", 16.0 * u, fg, true) {
                ops::overlay(&mut page, &t, margin as i64, margin as i64, 1.0);
            }
            let right = if n_pages > 1 { format!("{}  {}/{}", if title.is_empty() { "" } else { &date }, pi + 1, n_pages) } else if title.is_empty() { String::new() } else { date.clone() };
            if let Some(t) = ops::render_text(right.trim(), "", 9.0 * u, sub, false) {
                ops::overlay(&mut page, &t, (w as f32 - margin) as i64 - t.width() as i64, (margin + 6.0 * u) as i64, 1.0);
            }
        }
        let thumbs: Vec<Option<Thumb>> = {
            use rayon::prelude::*;
            chunk
                .par_iter()
                .map(|f| {
                    let d = codec::decode(&f.path, DecodeOpts { svg_side: cell_w as u32, ..Default::default() }).ok()?;
                    let (iw, ih) = (d.image.width(), d.image.height());
                    let (tw, th) = ops::fit_size(iw, ih, cell_w as u32, (cell_h - cap_h) as u32);
                    let small = DynamicImage::ImageRgba8(ops::resize_rgba(&d.image.thumbnail(tw * 2, th * 2).to_rgba8(), tw, th));
                    Some(Thumb { img: small.to_rgba8(), caption: caption(f, iw, ih, v.str("captions")) })
                })
                .collect()
        };
        for (i, t) in thumbs.into_iter().enumerate() {
            cx.check_cancel()?;
            let (c, r) = (i as u32 % cols, i as u32 / cols);
            let x = margin + c as f32 * (cell_w + gap);
            let y = margin + header_h + r as f32 * (cell_h + gap);
            match t {
                Some(t) => {
                    let ix = x + (cell_w - t.img.width() as f32) / 2.0;
                    let iy = y + (cell_h - cap_h - t.img.height() as f32) / 2.0;
                    ops::overlay(&mut page, &t.img, ix as i64, iy as i64, 1.0);
                    for (li, line) in t.caption.iter().enumerate() {
                        let col = if li == 0 { fg } else { sub };
                        let max_chars = (cell_w / (cap_size * 0.55)) as usize;
                        let text: String = if line.chars().count() > max_chars { line.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…" } else { line.clone() };
                        if let Some(ti) = ops::render_text(&text, "", cap_size, col, li == 0) {
                            let tx = x + (cell_w - ti.width() as f32) / 2.0;
                            let ty = y + cell_h - cap_h + 2.0 * u + li as f32 * cap_size * 1.3;
                            ops::overlay(&mut page, &ti, tx as i64, ty as i64, 1.0);
                        }
                    }
                }
                None => {
                    ops::fill_rounded_rect(&mut page, x, y, cell_w, cell_h - cap_h, 4.0 * u, [128, 128, 128, 60]);
                }
            }
        }
        let wm = v.str("watermark").trim();
        if !wm.is_empty() {
            if let Some(t) = ops::render_text(wm, "", 60.0 * u, [128, 128, 128, 255], true) {
                let t = ops::rotate_any(&t, -30.0, [0, 0, 0, 0], true);
                ops::overlay(&mut page, &t, (w as i64 - t.width() as i64) / 2, (h as i64 - t.height() as i64) / 2, 0.15);
            }
        }
        pages.push(page);
        cx.progress((pi + 1) as f32 / n_pages as f32);
    }
    Ok(pages)
}

/// Minimal PDF: one full-page JPEG per page (DCTDecode, no re-encoding by readers).
pub fn pdf_from_jpegs(pages: &[(Vec<u8>, u32, u32)], pw: f32, ph: f32) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let n = pages.len();
    // Object numbers: 1 catalog, 2 pages, 3 info, then per page: page, content, image.
    let page_id = |i: usize| 4 + i * 3;
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(body);
    };
    obj(&mut out, &mut offsets, b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_id(i))).collect();
    obj(&mut out, &mut offsets, format!("2 0 obj\n<< /Type /Pages /Kids [{}] /Count {n} >>\nendobj\n", kids.join(" ")).as_bytes());
    obj(&mut out, &mut offsets, b"3 0 obj\n<< /Producer (Castor) /Creator (Castor) >>\nendobj\n");
    for (i, (jpg, iw, ih)) in pages.iter().enumerate() {
        let (p, c, im) = (page_id(i), page_id(i) + 1, page_id(i) + 2);
        obj(
            &mut out,
            &mut offsets,
            format!("{p} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {im} 0 R >> >> /Contents {c} 0 R >>\nendobj\n").as_bytes(),
        );
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q\n");
        obj(&mut out, &mut offsets, format!("{c} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n", content.len()).as_bytes());
        let mut body = format!(
            "{im} 0 obj\n<< /Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            jpg.len()
        )
        .into_bytes();
        body.extend_from_slice(jpg);
        body.extend_from_slice(b"\nendstream\nendobj\n");
        obj(&mut out, &mut offsets, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}
