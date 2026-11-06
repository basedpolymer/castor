//! image.redact — blur / pixelate / box over faces (YuNet), text found by OCR (terms, emails,
//! phone numbers) and manual regions. Metadata is always removed from the result.

use anyhow::Result;
use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, Requirement, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, RgbaImage};

use crate::codec::{DecodeOpts, EncodeOpts};
use crate::{common, ops};

pub struct Redact {
    meta: ToolMeta,
}

impl Redact {
    pub fn new() -> Self {
        Redact {
            meta: common::meta(
                "image.redact",
                "JPG · PNG · WEBP",
                &["redact", "blur", "faces", "anonymize", "privacy", "pixelate", "hide", "censor", "gdpr", "license plate"],
                "redacted",
                ToolStatus::Ready,
                &[Requirement::Model("yunet")],
                true,
            ),
        }
    }
}

impl Tool for Redact {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::toggle("faces", true),
            OptionSpec::multiline("terms", ""),
            OptionSpec::toggle("contacts", false),
            OptionSpec::language("language", engine_pdf::ocr::languages(), true, "eng"),
            OptionSpec::text("regions", "").advanced(),
            OptionSpec::choice("style", vec![Choice::key("blur"), Choice::key("pixelate"), Choice::key("box")], "blur"),
            OptionSpec::color("color", "#000000").when(Cond::Eq("style", "box")),
            OptionSpec::int("strength", 1, 100, 60).slider().unit("%").when(Cond::Ne("style", "box")),
            OptionSpec::int("padding", 0, 100, 15).unit("%"),
            common::format_option("keep"),
        ]
    }

    fn downloads(&self, v: &Values) -> Vec<Asset> {
        let mut a = Vec::new();
        if v.bool("faces") {
            a.extend(engine_ml::models::asset("yunet"));
        }
        if needs_ocr(v) {
            a.extend(engine_pdf::ocr::assets(v.str("language")));
        }
        a
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        let mut img = d.image.to_rgba8();
        let (w, h) = img.dimensions();
        let mut boxes: Vec<[f32; 4]> = Vec::new();
        let mut counts = (0usize, 0usize, 0usize);
        if v.bool("faces") {
            cx.message(castor_core::tr("image.redact.finding_faces"));
            let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let faces = engine_ml::detect_faces(&rgb, cx)?;
            counts.0 = faces.len();
            boxes.extend(faces.into_iter().map(|b| pad(b, v.int("padding") as f32 / 100.0)));
        }
        cx.progress(0.4);
        if needs_ocr(v) {
            cx.message(castor_core::tr("image.redact.reading_text"));
            let page = engine_pdf::ocr::recognize(img.as_raw(), w, h, v.str("language"), cx)?;
            let words: Vec<(String, [f32; 4])> = page.words.into_iter().map(|w| (w.text, w.bbox)).collect();
            let found = match_words(&words, &terms(v), v.bool("contacts"));
            counts.1 = found.len();
            boxes.extend(found.into_iter().map(|b| pad(b, 0.08)));
        }
        let manual = parse_regions(v.str("regions"), w, h);
        counts.2 = manual.len();
        boxes.extend(manual);
        cx.progress(0.7);
        for b in &boxes {
            apply_box(&mut img, *b, v);
        }
        let alpha = crate::codec::uses_alpha(&d.image);
        let fmt = common::out_format(v, d.format, alpha);
        // Never carry metadata over: GPS, names and thumbnails could undo the redaction.
        let o = EncodeOpts { quality: 92, icc: d.icc.clone(), ..Default::default() };
        let out = common::save(&DynamicImage::ImageRgba8(img), fmt, &o, Some(input), cx)?;
        let note = castor_core::tr_args(
            "image.redact.note",
            &[("faces", &counts.0.to_string()), ("text", &counts.1.to_string()), ("regions", &counts.2.to_string())],
        );
        Ok(vec![Output::new(out).with_note(note)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        // Manual regions and faces (when the model is present); OCR is left to the run.
        let Some(f) = input else { return Ok(None) };
        let mut img = common::load_preview(f, max_side)?.to_rgba8();
        let (w, h) = img.dimensions();
        let mut boxes = parse_regions(v.str("regions"), w, h);
        if v.bool("faces") && engine_ml::models::asset("yunet").is_some_and(|a| a.is_present()) {
            let cx = RunCx::new(std::env::temp_dir(), std::env::temp_dir(), "", castor_core::CancelToken::new());
            let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            boxes.extend(engine_ml::detect_faces(&rgb, &cx)?.into_iter().map(|b| pad(b, v.int("padding") as f32 / 100.0)));
        }
        for b in &boxes {
            apply_box(&mut img, *b, v);
        }
        common::preview(&img, max_side, None)
    }
}

fn terms(v: &Values) -> Vec<String> {
    v.str("terms").lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect()
}

fn needs_ocr(v: &Values) -> bool {
    !terms(v).is_empty() || v.bool("contacts")
}

fn pad(b: [f32; 4], p: f32) -> [f32; 4] {
    let (dw, dh) = ((b[2] - b[0]) * p, (b[3] - b[1]) * p);
    [b[0] - dw, b[1] - dh, b[2] + dw, b[3] + dh]
}

/// "x,y,w,h; …" in pixels, or with % ("10%,20%,30%,5%").
pub fn parse_regions(s: &str, w: u32, h: u32) -> Vec<[f32; 4]> {
    s.split([';', '\n'])
        .filter_map(|r| {
            let parts: Vec<&str> = r.split(',').map(str::trim).collect();
            if parts.len() != 4 {
                return None;
            }
            let val = |t: &str, total: u32| -> Option<f32> {
                match t.strip_suffix('%') {
                    Some(p) => p.trim().parse::<f32>().ok().map(|v| v / 100.0 * total as f32),
                    None => t.parse::<f32>().ok(),
                }
            };
            let (x, y, rw, rh) = (val(parts[0], w)?, val(parts[1], h)?, val(parts[2], w)?, val(parts[3], h)?);
            (rw > 0.0 && rh > 0.0).then_some([x, y, x + rw, y + rh])
        })
        .collect()
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.' || *c == '+').flat_map(|c| c.to_lowercase()).collect::<String>().trim_matches('.').to_string()
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Boxes of words matching the terms (single words: substring; phrases: consecutive words),
/// plus e-mail addresses and phone numbers when `contacts`.
pub fn match_words(words: &[(String, [f32; 4])], terms: &[String], contacts: bool) -> Vec<[f32; 4]> {
    let normed: Vec<String> = words.iter().map(|(t, _)| norm(t)).collect();
    let mut out = Vec::new();
    for term in terms {
        let toks: Vec<String> = term.split_whitespace().map(norm).filter(|t| !t.is_empty()).collect();
        if toks.is_empty() {
            continue;
        }
        if toks.len() == 1 {
            for (i, n) in normed.iter().enumerate() {
                if n.contains(&toks[0]) {
                    out.push(words[i].1);
                }
            }
        } else {
            for i in 0..normed.len().saturating_sub(toks.len() - 1) {
                if toks.iter().enumerate().all(|(k, t)| normed[i + k] == *t || (k == toks.len() - 1 && normed[i + k].starts_with(t.as_str()))) {
                    out.push((i..i + toks.len()).map(|k| words[k].1).reduce(union).unwrap_or(words[i].1));
                }
            }
        }
    }
    if contacts {
        let mut i = 0;
        while i < words.len() {
            let t = &words[i].0;
            if t.contains('@') && t.split('@').nth(1).is_some_and(|d| d.contains('.')) {
                out.push(words[i].1);
                i += 1;
                continue;
            }
            // Phone numbers, possibly split over several words: "+33 6 12 34 56 78".
            let is_num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || "+-().".contains(c)) && s.chars().any(|c| c.is_ascii_digit());
            if is_num(t) {
                let mut j = i;
                let mut digits = 0;
                let mut b = words[i].1;
                while j < words.len() && is_num(&words[j].0) {
                    digits += words[j].0.chars().filter(|c| c.is_ascii_digit()).count();
                    b = union(b, words[j].1);
                    j += 1;
                }
                if digits >= 8 && digits <= 15 {
                    out.push(b);
                }
                i = j;
                continue;
            }
            i += 1;
        }
    }
    out
}

fn apply_box(img: &mut RgbaImage, b: [f32; 4], v: &Values) {
    let (w, h) = img.dimensions();
    let x0 = b[0].floor().clamp(0.0, w as f32) as u32;
    let y0 = b[1].floor().clamp(0.0, h as f32) as u32;
    let x1 = b[2].ceil().clamp(0.0, w as f32) as u32;
    let y1 = b[3].ceil().clamp(0.0, h as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    let strength = v.int("strength").clamp(1, 100) as f32 / 100.0;
    match v.str("style") {
        "box" => {
            let c = v.color("color");
            ops::fill_rounded_rect(img, x0 as f32, y0 as f32, bw as f32, bh as f32, 0.0, [c[0], c[1], c[2], 255]);
        }
        "pixelate" => {
            let block = ((bw.min(bh) as f32) * (0.05 + 0.2 * strength)).max(4.0) as u32;
            ops::pixelate_region(img, x0, y0, x1, y1, block);
        }
        _ => {
            // Heavy blur on a margin-extended crop, then pasted back with feathered edges.
            let m = (bw.max(bh) / 4).max(4);
            let (cx0, cy0) = (x0.saturating_sub(m), y0.saturating_sub(m));
            let (cx1, cy1) = ((x1 + m).min(w), (y1 + m).min(h));
            let crop = image::imageops::crop_imm(img, cx0, cy0, cx1 - cx0, cy1 - cy0).to_image();
            let sigma = bw.min(bh) as f32 * (0.05 + 0.25 * strength);
            // Two passes: no recognisable detail survives, even at low strength.
            let blurred = ops::blur_rgba(&ops::blur_rgba(&crop, sigma), sigma * 0.5);
            let feather = (bw.min(bh) as f32 * 0.08).max(1.0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = ((x - x0) as f32).min((x1 - 1 - x) as f32);
                    let dy = ((y - y0) as f32).min((y1 - 1 - y) as f32);
                    let a = (dx.min(dy) / feather).clamp(0.0, 1.0);
                    let s = blurred.get_pixel(x - cx0, y - cy0);
                    let d = img.get_pixel_mut(x, y);
                    for c in 0..4 {
                        d[c] = (d[c] as f32 * (1.0 - a) + s[c] as f32 * a).round() as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regions() {
        let r = parse_regions("10,20,30,40; 10%,50%,20%,10%", 200, 100);
        assert_eq!(r, vec![[10.0, 20.0, 40.0, 60.0], [20.0, 50.0, 60.0, 60.0]]);
    }
    #[test]
    fn words() {
        let w = |t: &str, x: f32| (t.to_string(), [x, 0.0, x + 10.0, 10.0]);
        let words = vec![w("Call", 0.0), w("+33", 20.0), w("6", 40.0), w("12", 60.0), w("34", 80.0), w("56", 100.0), w("78", 120.0), w("Secret", 140.0), w("Project,", 160.0), w("me@x.org", 180.0)];
        let found = match_words(&words, &["secret project".into()], true);
        assert!(found.contains(&[140.0, 0.0, 170.0, 10.0]));
        assert!(found.contains(&[20.0, 0.0, 130.0, 10.0]));
        assert!(found.contains(&[180.0, 0.0, 190.0, 10.0]));
    }
}
