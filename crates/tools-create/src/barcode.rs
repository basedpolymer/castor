//! `create.barcode`: retail and logistics barcodes (EAN-13, EAN-8, UPC-A, Code 128, Code 39,
//! ITF, Codabar) and 2D codes (Data Matrix, PDF417, Aztec), encoded and verified with `rxing`
//! (Apache-2.0). Retail codes get their check digit computed and the classic printed layout.

use anyhow::{Result, bail};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};
use rxing::{BarcodeFormat, EncodeHints, Writer};

use crate::common::{self, xml};
use crate::render::{png, style, typ};

pub struct BarcodeTool {
    meta: ToolMeta,
}

impl BarcodeTool {
    pub fn new() -> Self {
        BarcodeTool {
            meta: common::meta(
                "create.barcode",
                InputSpec::none(),
                "PNG · SVG · PDF",
                &["barcode", "ean", "upc", "code128", "code39", "datamatrix", "pdf417", "gs1", "isbn", "label"],
                "barcode",
            ),
        }
    }
}

const FORMATS: &[&str] = &["ean13", "ean8", "upca", "code128", "code39", "itf", "codabar", "datamatrix", "pdf417", "aztec"];

fn format_of(s: &str) -> BarcodeFormat {
    match s {
        "ean8" => BarcodeFormat::EAN_8,
        "upca" => BarcodeFormat::UPC_A,
        "code128" => BarcodeFormat::CODE_128,
        "code39" => BarcodeFormat::CODE_39,
        "itf" => BarcodeFormat::ITF,
        "codabar" => BarcodeFormat::CODABAR,
        "datamatrix" => BarcodeFormat::DATA_MATRIX,
        "pdf417" => BarcodeFormat::PDF_417,
        "aztec" => BarcodeFormat::AZTEC,
        _ => BarcodeFormat::EAN_13,
    }
}

fn is_2d(f: &str) -> bool {
    matches!(f, "datamatrix" | "pdf417" | "aztec")
}

impl Tool for BarcodeTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![
                    Choice::lit("ean13", "EAN-13"),
                    Choice::lit("ean8", "EAN-8"),
                    Choice::lit("upca", "UPC-A"),
                    Choice::lit("code128", "Code 128"),
                    Choice::lit("code39", "Code 39"),
                    Choice::lit("itf", "ITF (Interleaved 2 of 5)"),
                    Choice::lit("codabar", "Codabar"),
                    Choice::lit("datamatrix", "Data Matrix"),
                    Choice::lit("pdf417", "PDF417"),
                    Choice::lit("aztec", "Aztec"),
                ],
                "ean13",
            ),
            OptionSpec::multiline("data", "590123412345"),
            OptionSpec::toggle("batch", false),
            OptionSpec::toggle("show_text", true),
            OptionSpec::int("height", 5, 200, 22).slider().unit("mm").when(Cond::Ne("format", "datamatrix")),
            OptionSpec::color("color", "#000000"),
            OptionSpec::color("background", "#ffffff"),
            OptionSpec::int("margin", 0, 40, 10).unit("modules").advanced(),
            OptionSpec::choice("output", vec![Choice::lit("png", "PNG"), Choice::lit("svg", "SVG"), Choice::lit("pdf", "PDF")], "png"),
            OptionSpec::int("scale", 1, 40, 6).unit("px").when(Cond::Eq("output", "png")),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let line = lines(v).into_iter().next().unwrap_or_default();
        let b = build(&line, v)?;
        let pm = png::render_svg_fit(b.svg.as_bytes(), max_side, false)?;
        let ok = verify(&b, v).is_ok();
        let cap = format!("{} · {} · {}", b.label, b.text, common::scans_label(ok));
        Ok(Some(common::preview(png::encode_pixmap(&pm)?, common::figure(&cap))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let all = lines(v);
        anyhow::ensure!(!all.is_empty(), "{}", tr("create.barcode.err.empty"));
        let mut outs = Vec::new();
        for (i, line) in all.iter().enumerate() {
            cx.check_cancel()?;
            cx.progress(i as f32 / all.len() as f32);
            let b = build(line, v).map_err(|e| anyhow::anyhow!("“{line}”: {e:#}"))?;
            verify(&b, v).map_err(|e| anyhow::anyhow!("“{line}”: {e:#}"))?;
            let stem = format!("{}-{}", v.str("format"), common::slug_keep_case(&b.text.chars().take(40).collect::<String>()));
            let path = match v.str("output") {
                "svg" => {
                    let p = cx.output_named(&format!("{stem}.svg"));
                    // Text converted to paths: the file looks the same everywhere.
                    let tree = png::parse_svg(b.svg.as_bytes(), false)?;
                    common::write(&p, tree.to_string(&resvg::usvg::WriteOptions::default()).as_bytes())?;
                    p
                }
                "pdf" => {
                    let p = cx.output_named(&format!("{stem}.pdf"));
                    let (wmm, hmm) = b.size_mm;
                    let src = format!(
                        "#set page(width: {wmm:.3}mm, height: {hmm:.3}mm, margin: 0pt)\n#image(\"/b.svg\", width: 100%, height: 100%)\n"
                    );
                    let doc = typ::Doc::new(src).file("/b.svg", b.svg.clone().into_bytes()).compile()?;
                    common::write(&p, &typ::pdf(&doc)?)?;
                    p
                }
                _ => {
                    let p = cx.output_named(&format!("{stem}.png"));
                    let scale = v.int("scale").clamp(1, 40) as f32;
                    let (w, h) = ((b.units.0 * scale).round() as u32, (b.units.1 * scale).round() as u32);
                    let pm = png::render_svg(b.svg.as_bytes(), w, h, false)?;
                    common::write(&p, &png::encode_pixmap(&pm)?)?;
                    p
                }
            };
            outs.push(Output::new(path).with_note(format!("{} · {}", b.label, b.text)));
        }
        cx.progress(1.0);
        Ok(outs)
    }
}

fn lines(v: &Values) -> Vec<String> {
    let data = v.str("data");
    if v.bool("batch") {
        data.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
    } else if data.trim().is_empty() {
        Vec::new()
    } else {
        vec![data.trim_end_matches(['\r', '\n']).to_string()]
    }
}

/// A built barcode: SVG in module units, the text it encodes, its size.
pub struct Built {
    pub svg: String,
    /// What a scanner should read back.
    pub text: String,
    pub label: String,
    /// Width, height in modules (SVG user units).
    pub units: (f32, f32),
    /// Printed size in millimetres (0.33 mm module for 1D retail codes, 0.5 mm for 2D).
    pub size_mm: (f32, f32),
}

/// GS1 check digit for the given digits (without the check digit).
pub fn gs1_check(d: &str) -> u32 {
    let sum: u32 = d
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| c.to_digit(10).unwrap_or(0) * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    (10 - sum % 10) % 10
}

/// Normalise retail numbers: add the check digit, or verify it.
fn retail(data: &str, len: usize) -> Result<String> {
    let d: String = data.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    anyhow::ensure!(d.chars().all(|c| c.is_ascii_digit()), "{}", tr("create.barcode.err.digits"));
    if d.len() == len - 1 {
        return Ok(format!("{d}{}", gs1_check(&d)));
    }
    anyhow::ensure!(d.len() == len, "{}", tr_args("create.barcode.err.length", &[("n", &(len - 1).to_string()), ("m", &len.to_string())]));
    let (body, check) = d.split_at(len - 1);
    anyhow::ensure!(
        gs1_check(body).to_string() == check,
        "{}",
        tr_args("create.barcode.err.check", &[("digit", &gs1_check(body).to_string())])
    );
    Ok(d)
}

fn encode(text: &str, fmt: BarcodeFormat) -> Result<rxing::common::BitMatrix> {
    // Scanners read these symbologies as ISO-8859-1: other characters would come out garbled.
    if !text.chars().all(|c| (c as u32) < 256) && fmt != BarcodeFormat::AZTEC {
        anyhow::bail!("{}", tr("create.barcode.err.latin1"));
    }
    let mut hints = EncodeHints { Margin: Some("0".into()), ..Default::default() };
    if fmt == BarcodeFormat::AZTEC {
        hints.CharacterSet = Some("UTF-8".into());
    }
    rxing::MultiFormatWriter
        .encode_with_hints(text, &fmt, 0, 0, &hints)
        .map_err(|e| anyhow::anyhow!("{}: {e}", tr("create.barcode.err.encode")))
}

pub fn build(data: &str, v: &Values) -> Result<Built> {
    let f = v.str("format");
    let fg = v.color("color");
    let bg = castor_core::options::parse_hex_color(v.str("background")).unwrap_or([255, 255, 255, 255]);
    let q = v.int("margin").clamp(0, 40) as f32;
    let show = v.bool("show_text");
    let label = FORMATS.iter().position(|x| *x == f).map(|_| f.to_string()).unwrap_or_default();
    let text = match f {
        "ean13" => retail(data, 13)?,
        "ean8" => retail(data, 8)?,
        "upca" => retail(data, 12)?,
        "itf" => {
            let d: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            anyhow::ensure!(d.chars().all(|c| c.is_ascii_digit()) && !d.is_empty(), "{}", tr("create.barcode.err.digits"));
            anyhow::ensure!(d.len() % 2 == 0, "{}", tr("create.barcode.err.itf_even"));
            d
        }
        "code39" => {
            let d = data.trim().to_uppercase();
            anyhow::ensure!(
                d.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || " -.$/+%".contains(c)),
                "{}",
                tr("create.barcode.err.code39")
            );
            d
        }
        "codabar" => {
            let d = data.trim().to_uppercase();
            anyhow::ensure!(d.chars().all(|c| c.is_ascii_digit() || "-$:/.+ABCD".contains(c)), "{}", tr("create.barcode.err.codabar"));
            d
        }
        "code128" => {
            anyhow::ensure!(data.chars().all(|c| (c as u32) < 128), "{}", tr("create.barcode.err.ascii"));
            data.to_string()
        }
        _ => data.to_string(),
    };
    anyhow::ensure!(!text.is_empty(), "{}", tr("create.barcode.err.empty"));
    let m = encode(&text, format_of(f))?;
    let (fgc, fga) = style::svg_paint(fg);
    let (bgc, bga) = style::svg_paint(bg);
    let label = match f {
        "ean13" => "EAN-13",
        "ean8" => "EAN-8",
        "upca" => "UPC-A",
        "code128" => "Code 128",
        "code39" => "Code 39",
        "itf" => "ITF",
        "codabar" => "Codabar",
        "datamatrix" => "Data Matrix",
        "pdf417" => "PDF417",
        "aztec" => "Aztec",
        _ => &label,
    }
    .to_string();

    if is_2d(f) {
        let (w, h) = (m.width() as f32, m.height() as f32);
        // PDF417 rows are drawn three times taller than a module is wide.
        let row_h = if f == "pdf417" && h < w / 2.0 { 3.0 } else { 1.0 };
        let q2 = if f == "pdf417" { q.max(2.0) } else { q.max(1.0).min(4.0) };
        let (tw, th) = (w + 2.0 * q2, h * row_h + 2.0 * q2);
        let mut o = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {tw} {th}\" width=\"{tw}\" height=\"{th}\" shape-rendering=\"crispEdges\"><rect width=\"{tw}\" height=\"{th}\" fill=\"{bgc}\" fill-opacity=\"{bga}\"/><path fill=\"{fgc}\" fill-opacity=\"{fga}\" d=\""
        );
        for y in 0..m.height() {
            let mut x = 0;
            while x < m.width() {
                if !m.get(x, y) {
                    x += 1;
                    continue;
                }
                let s = x;
                while x < m.width() && m.get(x, y) {
                    x += 1;
                }
                o.push_str(&format!("M{} {}h{}v{}h-{}Z", s as f32 + q2, y as f32 * row_h + q2, x - s, row_h, x - s));
            }
        }
        o.push_str("\"/></svg>");
        let module_mm = 0.5;
        return Ok(Built { svg: o, text, label, units: (tw, th), size_mm: (tw * module_mm, th * module_mm) });
    }

    // 1D: one row of modules.
    let bars: Vec<bool> = (0..m.width()).map(|x| m.get(x, m.height() / 2)).collect();
    let n = bars.len() as f32;
    let module_mm = 0.33;
    let bar_h = (v.int("height").clamp(5, 200) as f32 / module_mm).round();
    let retail_layout = show && matches!(f, "ean13" | "ean8" | "upca");
    let font = 11.0; // digit height in modules (≈ 3.6 mm at 0.33 mm per module)
    let left_extra = if retail_layout && f != "ean8" { 9.0 } else { 0.0 };
    let right_extra = if retail_layout && f == "upca" { 9.0 } else { 0.0 };
    let q = q.max(if retail_layout { 2.0 } else { 0.0 });
    let text_h = if show { font + 3.0 } else { 0.0 };
    let tw = n + 2.0 * q + left_extra + right_extra;
    let th = bar_h + text_h + 2.0;
    let x0 = q + left_extra;
    // Guard bars run down into the digit line on retail codes.
    let guards: Vec<(usize, usize)> = match f {
        "ean13" => vec![(0, 3), (45, 50), (92, 95)],
        "upca" => vec![(0, 10), (45, 50), (85, 95)],
        "ean8" => vec![(0, 3), (31, 36), (64, 67)],
        _ => vec![],
    };
    let tall = |i: usize| retail_layout && guards.iter().any(|(a, b)| i >= *a && i < *b);
    let mut o = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {tw} {th}\" width=\"{tw}\" height=\"{th}\" shape-rendering=\"crispEdges\"><rect width=\"{tw}\" height=\"{th}\" fill=\"{bgc}\" fill-opacity=\"{bga}\"/><path fill=\"{fgc}\" fill-opacity=\"{fga}\" d=\""
    );
    let mut i = 0;
    while i < bars.len() {
        if !bars[i] {
            i += 1;
            continue;
        }
        let s = i;
        while i < bars.len() && bars[i] && tall(i) == tall(s) {
            i += 1;
        }
        let h = if tall(s) { bar_h + font * 0.55 } else { bar_h };
        o.push_str(&format!("M{} 1h{}v{}h-{}Z", x0 + s as f32, i - s, h, i - s));
    }
    o.push_str("\"/>");
    if show {
        let ty = 1.0 + bar_h + font * 0.92;
        let t = |x: f32, s: &str, size: f32, anchor: &str| {
            format!(
                "<text x=\"{x}\" y=\"{ty}\" font-family=\"Latin Modern Mono\" font-size=\"{size}\" text-anchor=\"{anchor}\" fill=\"{fgc}\">{}</text>",
                xml(s)
            )
        };
        match f {
            "ean13" if retail_layout => {
                o.push_str(&t(x0 - 1.5, &text[..1], font, "end"));
                o.push_str(&t(x0 + 24.0, &text[1..7], font, "middle"));
                o.push_str(&t(x0 + 71.0, &text[7..], font, "middle"));
            }
            "ean8" if retail_layout => {
                o.push_str(&t(x0 + 17.0, &text[..4], font, "middle"));
                o.push_str(&t(x0 + 50.0, &text[4..], font, "middle"));
            }
            "upca" if retail_layout => {
                o.push_str(&t(x0 - 1.5, &text[..1], font * 0.75, "end"));
                o.push_str(&t(x0 + 27.5, &text[1..6], font, "middle"));
                o.push_str(&t(x0 + 67.5, &text[6..11], font, "middle"));
                o.push_str(&t(x0 + n + 1.5, &text[11..], font * 0.75, "start"));
            }
            _ => o.push_str(&t(tw / 2.0, &text, font, "middle")),
        }
    }
    o.push_str("</svg>");
    Ok(Built { svg: o, text, label, units: (tw, th), size_mm: (tw * module_mm, th * module_mm) })
}

/// Render at 3 px per module and decode with rxing; the read text must match.
pub fn verify(b: &Built, v: &Values) -> Result<()> {
    let f = v.str("format");
    let k = 3.0;
    let (w, h) = ((b.units.0 * k).round() as u32, (b.units.1 * k).round() as u32);
    let pm = png::render_svg(b.svg.as_bytes(), w, h, false)?;
    let rgb = png::flatten(&png::pixmap_to_rgba(&pm), [255, 255, 255]);
    let luma: Vec<u8> = rgb.pixels().map(|p| ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8).collect();
    let try_decode = |l: Vec<u8>| rxing::helpers::detect_in_luma(l, w, h, Some(format_of(f))).ok().map(|r| r.getText().to_string());
    let read = try_decode(luma.clone()).or_else(|| try_decode(luma.iter().map(|x| 255 - x).collect()));
    let expected = match f {
        // ZXing reads UPC-A as EAN-13 with a leading 0 in some paths; accept both.
        "upca" => vec![b.text.clone(), format!("0{}", b.text)],
        "codabar" => vec![b.text.clone(), b.text.trim_matches(|c| "ABCD".contains(c)).to_string()],
        _ => vec![b.text.clone()],
    };
    match read {
        Some(t) if expected.iter().any(|e| *e == t) => Ok(()),
        Some(t) => bail!("{}", tr_args("create.barcode.err.mismatch", &[("read", &t)])),
        None => bail!("{}", tr("create.barcode.err.no_scan")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(f: &str, data: &str) -> Values {
        let t = BarcodeTool::new();
        let mut v = Values::defaults(&t.options());
        v.set("format", castor_core::Value::Text(f.into()));
        v.set("data", castor_core::Value::Text(data.into()));
        v
    }

    #[test]
    fn check_digits() {
        assert_eq!(gs1_check("590123412345"), 7);
        assert_eq!(retail("590123412345", 13).unwrap(), "5901234123457");
        assert!(retail("5901234123450", 13).is_err());
        assert_eq!(retail("03600029145", 12).unwrap(), "036000291452");
    }

    #[test]
    fn every_format_round_trips() {
        for (f, d) in [
            ("ean13", "590123412345"),
            ("ean8", "9638507"),
            ("upca", "03600029145"),
            ("code128", "CASTOR-2026/11 ok"),
            ("code39", "CASTOR 42"),
            ("itf", "12345678901231"),
            ("codabar", "A40156B"),
            ("datamatrix", "Castor données 2026"),
            ("pdf417", "Castor PDF417 test 1234567890"),
            ("aztec", "https://basedpolymer.eu/castor"),
        ] {
            let v = values(f, d);
            let b = build(d, &v).unwrap_or_else(|e| panic!("{f}: {e:#}"));
            verify(&b, &v).unwrap_or_else(|e| panic!("{f}: {e:#}"));
        }
    }
}
