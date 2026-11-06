//! `create.qr-code`: QR codes in 12 module styles, with colours, gradients, a centred logo and
//! nine printable cards. Every output is rendered back to pixels and decoded with `rqrr`: a code
//! that does not scan is never written.

use anyhow::{Result, bail};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};
use qrcode::{EcLevel, QrCode};

use crate::common;
use crate::render::{png, style, typ};

pub struct QrCodeTool {
    meta: ToolMeta,
}

impl QrCodeTool {
    pub fn new() -> Self {
        QrCodeTool {
            meta: common::meta(
                "create.qr-code",
                InputSpec::none(),
                "PNG · SVG · PDF",
                &["qr", "qrcode", "wifi", "vcard", "link", "scan", "menu", "card"],
                "qr-code",
            ),
        }
    }
}

pub const STYLES: &[&str] =
    &["square", "gapped", "dots", "rounded", "fluid", "classy", "leaf", "diamond", "star", "vbars", "hbars", "mosaic"];
pub const CARDS: &[&str] =
    &["none", "wifi", "table-tent", "review", "business", "ticket", "poster", "tag", "social", "invitation"];

impl Tool for QrCodeTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let is = |v: &'static str| Cond::Eq("content", v);
        vec![
            OptionSpec::choice(
                "content",
                ["url", "text", "wifi", "contact", "email", "phone", "sms", "geo", "event"].map(Choice::key).to_vec(),
                "url",
            ),
            OptionSpec::text("url", "https://basedpolymer.eu/castor").when(is("url")),
            OptionSpec::multiline("text", "").when(is("text")),
            OptionSpec::text("ssid", "").when(is("wifi")),
            OptionSpec::password("wifi_password").when(is("wifi")),
            OptionSpec::choice("security", vec![Choice::lit("wpa", "WPA/WPA2/WPA3"), Choice::lit("wep", "WEP"), Choice::key("open")], "wpa")
                .when(is("wifi")),
            OptionSpec::toggle("hidden", false).when(is("wifi")).advanced(),
            OptionSpec::text("name", "").when(is("contact")),
            OptionSpec::text("org", "").when(is("contact")),
            OptionSpec::text("job_title", "").when(is("contact")),
            OptionSpec::text("phone", "").when(Cond::Ne("content", "url")),
            OptionSpec::text("email", "").when(Cond::Ne("content", "url")),
            OptionSpec::text("website", "").when(is("contact")),
            OptionSpec::text("address", "").when(is("contact")),
            OptionSpec::text("subject", "").when(is("email")),
            OptionSpec::multiline("message", "").when(Cond::Ne("content", "url")),
            OptionSpec::text("latitude", "").when(is("geo")),
            OptionSpec::text("longitude", "").when(is("geo")),
            OptionSpec::text("event_title", "").when(is("event")),
            OptionSpec::text("start", "").when(is("event")),
            OptionSpec::text("end", "").when(is("event")),
            OptionSpec::text("location", "").when(is("event")),
            // Look.
            OptionSpec::choice("style", STYLES.iter().map(|s| Choice::key(s)).collect(), "rounded"),
            OptionSpec::choice("eyes", ["auto", "square", "rounded", "circle", "leaf", "diamond"].map(Choice::key).to_vec(), "auto"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("background", "#ffffff"),
            OptionSpec::choice("gradient", ["none", "linear", "radial"].map(Choice::key).to_vec(), "none"),
            OptionSpec::color("color2", "#2f6db5").when(Cond::Ne("gradient", "none")),
            OptionSpec::color("eye_color", "").advanced(),
            OptionSpec::file("logo", &["png", "jpg", "jpeg", "webp", "svg", "gif", "bmp"]),
            OptionSpec::int("logo_size", 10, 30, 20).slider().unit("%").when(Cond::Ne("logo", "")),
            OptionSpec::choice("ec", ["auto", "L", "M", "Q", "H"].map(Choice::key).to_vec(), "auto").advanced(),
            OptionSpec::int("margin", 0, 10, 4).unit("modules").advanced(),
            // Output.
            OptionSpec::choice("card", CARDS.iter().map(|s| Choice::key(s)).collect(), "none"),
            OptionSpec::text("card_title", "").when(Cond::Ne("card", "none")),
            OptionSpec::text("card_subtitle", "").when(Cond::Ne("card", "none")),
            OptionSpec::multiline("card_text", "").when(Cond::Ne("card", "none")),
            OptionSpec::color("card_tint", "#ffeca4").when(Cond::Ne("card", "none")),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("svg", "SVG"), Choice::lit("pdf", "PDF")], "png"),
            OptionSpec::int("size", 128, 8192, 1024).unit("px").when(Cond::Ne("format", "pdf")),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let spec = Spec::from_values(v)?;
        let code = spec.code()?;
        let svg = svg(&code, &spec)?;
        let card = v.str("card");
        let (img, extra) = if card != "none" && !card.is_empty() {
            let doc = card_doc(&code, &spec, v)?.compile()?;
            let pm = typ::pixmap(&doc, 0, (max_side as f64 / typ::page_size(&doc, 0).0.max(typ::page_size(&doc, 0).1)) as f32)?;
            let (w, h) = typ::page_size(&doc, 0);
            (png::pixmap_to_rgba(&pm), format!(" · {:.0} × {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4))
        } else {
            (png::pixmap_to_rgba(&png::render_svg_fit(svg.as_bytes(), max_side, false)?), String::new())
        };
        let ok = scans(&code_payload(&spec), &svg, &code);
        let caption = common::figure(&format!("{}{} · {}", describe(&code), extra, common::scans_label(ok)));
        Ok(Some(common::preview(png::encode_rgba(&img)?, caption)))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let spec = Spec::from_values(v)?;
        let code = spec.code()?;
        let svg = svg(&code, &spec)?;
        cx.progress(0.3);
        let payload = code_payload(&spec);
        if !scans(&payload, &svg, &code) {
            bail!("{}", tr("create.qr-code.err.no_scan"));
        }
        let format = v.str("format");
        let card = v.str("card");
        let name = v.str("file_name");
        let size = v.int("size").clamp(64, 8192) as u32;
        let mut note = describe(&code);
        let out = if card != "none" && !card.is_empty() {
            let doc = card_doc(&code, &spec, v)?.compile()?;
            // The printed card must scan too: render it at 200 dpi and decode.
            let pm = typ::pixmap(&doc, 0, 200.0 / 72.0)?;
            let found = png::decode_qr(&png::pixmap_to_rgba(&pm));
            anyhow::ensure!(found.iter().any(|s| *s == payload), "{}", tr("create.qr-code.err.no_scan"));
            cx.progress(0.7);
            let (w, h) = typ::page_size(&doc, 0);
            note = format!("{note} · {:.0} × {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4);
            match format {
                "png" => {
                    let p = common::out_named(cx, name, "png");
                    let pm = typ::pixmap(&doc, 0, 300.0 / 72.0)?;
                    common::write(&p, &png::encode_pixmap(&pm)?)?;
                    p
                }
                "svg" => {
                    let p = common::out_named(cx, name, "svg");
                    common::write(&p, typ::svg(&doc, 0)?.as_bytes())?;
                    p
                }
                _ => {
                    let p = common::out_named(cx, name, "pdf");
                    common::write(&p, &typ::pdf(&doc)?)?;
                    p
                }
            }
        } else {
            match format {
                "svg" => {
                    let p = common::out_named(cx, name, "svg");
                    common::write(&p, svg.as_bytes())?;
                    p
                }
                "pdf" => {
                    let p = common::out_named(cx, name, "pdf");
                    let src = format!(
                        "#set page(width: 60mm, height: 60mm, margin: 0pt)\n#image(\"/qr.svg\", width: 100%, height: 100%)\n"
                    );
                    let doc = typ::Doc::new(src).file("/qr.svg", svg.as_bytes().to_vec()).compile()?;
                    common::write(&p, &typ::pdf(&doc)?)?;
                    p
                }
                _ => {
                    let p = common::out_named(cx, name, "png");
                    let pm = png::render_svg(svg.as_bytes(), size, size, false)?;
                    common::write(&p, &png::encode_pixmap(&pm)?)?;
                    note = format!("{note} · {}", common::px(size, size));
                    p
                }
            }
        };
        cx.progress(1.0);
        Ok(vec![Output::new(out).with_note(note)])
    }
}

/// Everything that decides the picture.
pub struct Spec {
    pub payload: String,
    pub style: String,
    pub eyes: String,
    pub fg: [u8; 4],
    pub fg2: [u8; 4],
    pub bg: [u8; 4],
    pub eye: [u8; 4],
    pub gradient: String,
    pub logo: Option<Vec<u8>>,
    pub logo_pct: f32,
    pub ec: String,
    pub margin: usize,
}

impl Spec {
    pub fn from_values(v: &Values) -> Result<Spec> {
        let payload = payload(v)?;
        anyhow::ensure!(!payload.trim().is_empty(), "{}", tr("create.qr-code.err.empty"));
        let logo = match v.path("logo") {
            Some(p) if !p.as_os_str().is_empty() => {
                let img = png::load_image(p, 512)?;
                Some(png::encode_rgba(&img)?)
            }
            _ => None,
        };
        let fg = v.color("color");
        let eye = castor_core::options::parse_hex_color(v.str("eye_color")).unwrap_or(fg);
        Ok(Spec {
            payload,
            style: v.str("style").to_string(),
            eyes: v.str("eyes").to_string(),
            fg,
            fg2: v.color("color2"),
            bg: castor_core::options::parse_hex_color(v.str("background")).unwrap_or([255, 255, 255, 255]),
            eye,
            gradient: v.str("gradient").to_string(),
            logo,
            logo_pct: v.int("logo_size").clamp(10, 30) as f32 / 100.0,
            ec: v.str("ec").to_string(),
            margin: v.int("margin").clamp(0, 10) as usize,
        })
    }

    pub fn simple(payload: &str) -> Spec {
        Spec {
            payload: payload.to_string(),
            style: "square".into(),
            eyes: "auto".into(),
            fg: style::INK,
            fg2: style::INK,
            bg: [255, 255, 255, 255],
            eye: style::INK,
            gradient: "none".into(),
            logo: None,
            logo_pct: 0.2,
            ec: "M".into(),
            margin: 4,
        }
    }

    pub fn code(&self) -> Result<QrCode> {
        let ec = match self.ec.as_str() {
            "L" => EcLevel::L,
            "Q" => EcLevel::Q,
            "H" => EcLevel::H,
            "M" => EcLevel::M,
            // Auto: a logo hides modules, so it needs the highest level; otherwise Q tolerates
            // the stylised modules and print wear.
            _ if self.logo.is_some() => EcLevel::H,
            _ => EcLevel::Q,
        };
        let ec = if self.logo.is_some() { EcLevel::H } else { ec };
        QrCode::with_error_correction_level(self.payload.as_bytes(), ec)
            .or_else(|_| QrCode::with_error_correction_level(self.payload.as_bytes(), EcLevel::L))
            .map_err(|_| anyhow::anyhow!("{}", tr("create.qr-code.err.too_long")))
    }
}

fn code_payload(s: &Spec) -> String {
    s.payload.clone()
}

/// "Version 3 · EC Q · 29 × 29 modules".
pub fn describe(code: &QrCode) -> String {
    let v = match code.version() {
        qrcode::Version::Normal(n) => n,
        qrcode::Version::Micro(n) => n,
    };
    let ec = match code.error_correction_level() {
        EcLevel::L => "L",
        EcLevel::M => "M",
        EcLevel::Q => "Q",
        EcLevel::H => "H",
    };
    tr_args(
        "create.qr-code.describe",
        &[("version", &v.to_string()), ("ec", ec), ("n", &code.width().to_string())],
    )
}

// ---------------------------------------------------------------- payloads

fn esc_wifi(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

fn esc_vcard(s: &str) -> String {
    s.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace('\n', "\\n")
}

fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(b) {
            o.push(*b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// "2026-11-06 19:30" / "2026-11-06T19:30" / "2026-11-06" → "20261106T193000" / "20261106".
fn ical_date(s: &str) -> Result<String> {
    let s = s.trim();
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    match digits.len() {
        8 => Ok(digits),
        12 => Ok(format!("{}T{}00", &digits[..8], &digits[8..])),
        14 => Ok(format!("{}T{}", &digits[..8], &digits[8..])),
        _ => bail!("{}", tr_args("create.qr-code.err.date", &[("value", s)])),
    }
}

/// The text encoded in the code for the chosen content type.
pub fn payload(v: &Values) -> Result<String> {
    let t = |k: &str| v.str(k).trim().to_string();
    Ok(match v.str("content") {
        "text" => v.str("text").to_string(),
        "wifi" => {
            let ssid = t("ssid");
            anyhow::ensure!(!ssid.is_empty(), "{}", tr("create.qr-code.err.ssid"));
            let sec = match v.str("security") {
                "wep" => "WEP",
                "open" => "nopass",
                _ => "WPA",
            };
            let mut s = format!("WIFI:T:{sec};S:{};", esc_wifi(&ssid));
            if sec != "nopass" {
                s.push_str(&format!("P:{};", esc_wifi(v.str("wifi_password"))));
            }
            if v.bool("hidden") {
                s.push_str("H:true;");
            }
            s.push(';');
            s
        }
        "contact" => {
            let name = t("name");
            let (first, last) = match name.rsplit_once(' ') {
                Some((f, l)) => (f.to_string(), l.to_string()),
                None => (String::new(), name.clone()),
            };
            let mut s = String::from("BEGIN:VCARD\nVERSION:3.0\n");
            s.push_str(&format!("N:{};{};;;\nFN:{}\n", esc_vcard(&last), esc_vcard(&first), esc_vcard(&name)));
            for (k, key) in [("ORG", "org"), ("TITLE", "job_title"), ("TEL", "phone"), ("EMAIL", "email"), ("URL", "website")] {
                if !t(key).is_empty() {
                    s.push_str(&format!("{k}:{}\n", esc_vcard(&t(key))));
                }
            }
            if !t("address").is_empty() {
                s.push_str(&format!("ADR:;;{};;;;\n", esc_vcard(&t("address"))));
            }
            if !t("message").is_empty() {
                s.push_str(&format!("NOTE:{}\n", esc_vcard(&t("message"))));
            }
            s.push_str("END:VCARD");
            s
        }
        "email" => {
            let to = t("email");
            anyhow::ensure!(!to.is_empty(), "{}", tr("create.qr-code.err.email"));
            let mut q = Vec::new();
            if !t("subject").is_empty() {
                q.push(format!("subject={}", pct(&t("subject"))));
            }
            if !v.str("message").is_empty() {
                q.push(format!("body={}", pct(v.str("message"))));
            }
            if q.is_empty() { format!("mailto:{to}") } else { format!("mailto:{to}?{}", q.join("&")) }
        }
        "phone" => {
            let n: String = t("phone").chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
            anyhow::ensure!(!n.is_empty(), "{}", tr("create.qr-code.err.phone"));
            format!("tel:{n}")
        }
        "sms" => {
            let n: String = t("phone").chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
            anyhow::ensure!(!n.is_empty(), "{}", tr("create.qr-code.err.phone"));
            format!("SMSTO:{n}:{}", v.str("message"))
        }
        "geo" => {
            let lat: f64 = t("latitude").replace(',', ".").parse().map_err(|_| anyhow::anyhow!("{}", tr("create.qr-code.err.geo")))?;
            let lon: f64 = t("longitude").replace(',', ".").parse().map_err(|_| anyhow::anyhow!("{}", tr("create.qr-code.err.geo")))?;
            anyhow::ensure!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon), "{}", tr("create.qr-code.err.geo"));
            format!("geo:{lat},{lon}")
        }
        "event" => {
            let mut s = String::from("BEGIN:VEVENT\n");
            s.push_str(&format!("SUMMARY:{}\n", esc_vcard(&t("event_title"))));
            if !t("start").is_empty() {
                s.push_str(&format!("DTSTART:{}\n", ical_date(&t("start"))?));
            }
            if !t("end").is_empty() {
                s.push_str(&format!("DTEND:{}\n", ical_date(&t("end"))?));
            }
            if !t("location").is_empty() {
                s.push_str(&format!("LOCATION:{}\n", esc_vcard(&t("location"))));
            }
            if !t("message").is_empty() {
                s.push_str(&format!("DESCRIPTION:{}\n", esc_vcard(&t("message"))));
            }
            s.push_str("END:VEVENT");
            s
        }
        _ => {
            let u = t("url");
            if !u.is_empty() && !u.contains("://") && !u.starts_with("mailto:") && u.contains('.') && !u.contains(' ') {
                format!("https://{u}")
            } else {
                u
            }
        }
    })
}

// ---------------------------------------------------------------- drawing

/// Is module (x, y) inside one of the three finder patterns (7 × 7)?
fn in_finder(n: usize, x: usize, y: usize) -> bool {
    (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7)
}

fn f(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// A rectangle with individually rounded corners (tl, tr, br, bl radii).
fn rrect(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> String {
    let [tl, tr, br, bl] = r;
    format!(
        "M{} {}H{}{}V{}{}H{}{}V{}{}Z",
        f(x + tl),
        f(y),
        f(x + w - tr),
        if tr > 0.0 { format!("A{} {} 0 0 1 {} {}", f(tr), f(tr), f(x + w), f(y + tr)) } else { String::new() },
        f(y + h - br),
        if br > 0.0 { format!("A{} {} 0 0 1 {} {}", f(br), f(br), f(x + w - br), f(y + h)) } else { String::new() },
        f(x + bl),
        if bl > 0.0 { format!("A{} {} 0 0 1 {} {}", f(bl), f(bl), f(x), f(y + h - bl)) } else { String::new() },
        f(y + tl),
        if tl > 0.0 { format!("A{} {} 0 0 1 {} {}", f(tl), f(tl), f(x + tl), f(y)) } else { String::new() },
    )
}

fn circle(cx: f32, cy: f32, r: f32) -> String {
    format!("M{} {}a{} {} 0 1 0 {} 0a{} {} 0 1 0 {} 0Z", f(cx - r), f(cy), f(r), f(r), f(2.0 * r), f(r), f(r), f(-2.0 * r))
}

fn diamond(cx: f32, cy: f32, r: f32) -> String {
    format!("M{} {}L{} {}L{} {}L{} {}Z", f(cx), f(cy - r), f(cx + r), f(cy), f(cx), f(cy + r), f(cx - r), f(cy))
}

fn star(cx: f32, cy: f32, r: f32) -> String {
    // Four-pointed star (a pinched diamond): the house motif, sparkling.
    let k = r * 0.2;
    format!(
        "M{} {}Q{} {} {} {}Q{} {} {} {}Q{} {} {} {}Q{} {} {} {}Z",
        f(cx),
        f(cy - r),
        f(cx + k),
        f(cy - k),
        f(cx + r),
        f(cy),
        f(cx + k),
        f(cy + k),
        f(cx),
        f(cy + r),
        f(cx - k),
        f(cy + k),
        f(cx - r),
        f(cy),
        f(cx - k),
        f(cy - k),
        f(cx),
        f(cy - r)
    )
}

/// Module path data for one style. `dark(x, y)` is false outside the grid.
fn modules_path(n: usize, style: &str, dark: &dyn Fn(i64, i64) -> bool, skip: &dyn Fn(usize, usize) -> bool, m: f32) -> String {
    let mut d = String::new();
    let on = |x: usize, y: usize| dark(x as i64, y as i64) && !skip(x, y);
    match style {
        "vbars" | "hbars" => {
            let vertical = style == "vbars";
            for a in 0..n {
                let mut b = 0;
                while b < n {
                    let (x, y) = if vertical { (a, b) } else { (b, a) };
                    if !on(x, y) {
                        b += 1;
                        continue;
                    }
                    let start = b;
                    while b < n && { let (x, y) = if vertical { (a, b) } else { (b, a) }; on(x, y) } {
                        b += 1;
                    }
                    let len = (b - start) as f32;
                    let w = 0.82;
                    let r = w / 2.0;
                    let s = start as f32;
                    let a = a as f32;
                    if vertical {
                        d.push_str(&rrect(m + a + (1.0 - w) / 2.0, m + s + 0.06, w, len - 0.12, [r; 4]));
                    } else {
                        d.push_str(&rrect(m + s + 0.06, m + a + (1.0 - w) / 2.0, len - 0.12, w, [r; 4]));
                    }
                }
            }
        }
        "square" => {
            // Merge horizontal runs: smaller files, no hairline seams between modules.
            for y in 0..n {
                let mut x = 0;
                while x < n {
                    if !on(x, y) {
                        x += 1;
                        continue;
                    }
                    let s = x;
                    while x < n && on(x, y) {
                        x += 1;
                    }
                    d.push_str(&format!("M{} {}h{}v1h-{}Z", f(m + s as f32), f(m + y as f32), x - s, x - s));
                }
            }
        }
        _ => {
            for y in 0..n {
                for x in 0..n {
                    if !on(x, y) {
                        continue;
                    }
                    let (fx, fy) = (m + x as f32, m + y as f32);
                    let (cx, cy) = (fx + 0.5, fy + 0.5);
                    let (xi, yi) = (x as i64, y as i64);
                    let (u, r_, dn, l) = (dark(xi, yi - 1), dark(xi + 1, yi), dark(xi, yi + 1), dark(xi - 1, yi));
                    match style {
                        "gapped" => d.push_str(&rrect(fx + 0.1, fy + 0.1, 0.8, 0.8, [0.08; 4])),
                        "dots" => d.push_str(&circle(cx, cy, 0.46)),
                        "diamond" => d.push_str(&diamond(cx, cy, 0.62)),
                        "star" => d.push_str(&star(cx, cy, 0.68)),
                        "leaf" => d.push_str(&rrect(fx + 0.04, fy + 0.04, 0.92, 0.92, [0.46, 0.0, 0.46, 0.0])),
                        "mosaic" => {
                            let h = (x * 7349 + y * 1973 + x * y * 31) % 5;
                            let s = 0.74 + h as f32 * 0.065;
                            d.push_str(&rrect(cx - s / 2.0, cy - s / 2.0, s, s, [0.05; 4]));
                        }
                        "classy" => {
                            let r = 0.5;
                            let tl = if !u && !l { r } else { 0.0 };
                            let br = if !dn && !r_ { r } else { 0.0 };
                            d.push_str(&rrect(fx, fy, 1.0, 1.0, [tl, 0.0, br, 0.0]));
                        }
                        // rounded / fluid: connected shapes, outer corners rounded.
                        _ => {
                            let r = if style == "fluid" { 0.5 } else { 0.32 };
                            let tl = if !u && !l { r } else { 0.0 };
                            let tr = if !u && !r_ { r } else { 0.0 };
                            let br = if !dn && !r_ { r } else { 0.0 };
                            let bl = if !dn && !l { r } else { 0.0 };
                            // Tiny overlap so neighbours fuse without anti-aliasing seams.
                            let ex = |b: bool| if b { 0.02 } else { 0.0 };
                            d.push_str(&rrect(fx - ex(l), fy - ex(u), 1.0 + ex(l) + ex(r_), 1.0 + ex(u) + ex(dn), [tl, tr, br, bl]));
                        }
                    }
                }
            }
        }
    }
    d
}

/// Path data of one finder pattern ("eye") at (ox, oy), outer ring and ball as separate paths.
fn eye_paths(kind: &str, ox: f32, oy: f32, corner: usize) -> (String, String) {
    let ring = |r_out: [f32; 4], r_in: [f32; 4]| {
        // Outer 7×7 minus inner 5×5 (even-odd).
        format!("{}{}", rrect(ox, oy, 7.0, 7.0, r_out), rrect(ox + 1.0, oy + 1.0, 5.0, 5.0, r_in))
    };
    match kind {
        "rounded" => (ring([2.0; 4], [1.2; 4]), rrect(ox + 2.0, oy + 2.0, 3.0, 3.0, [0.9; 4])),
        "circle" => (
            format!("{}{}", circle(ox + 3.5, oy + 3.5, 3.5), circle(ox + 3.5, oy + 3.5, 2.5)),
            circle(ox + 3.5, oy + 3.5, 1.5),
        ),
        "leaf" => {
            // Pointed corner towards the centre of the code.
            let (a, b) = (3.0, 0.0);
            let rad = match corner {
                0 => [a, a, b, a],
                1 => [a, a, a, b],
                _ => [a, b, a, a],
            };
            let rin = rad.map(|r| if r > 0.0 { 2.0 } else { 0.0 });
            let rball = rad.map(|r| if r > 0.0 { 1.3 } else { 0.0 });
            (ring(rad, rin), rrect(ox + 2.0, oy + 2.0, 3.0, 3.0, rball))
        }
        "diamond" => (ring([0.0; 4], [0.0; 4]), diamond(ox + 3.5, oy + 3.5, 2.05)),
        _ => (ring([0.0; 4], [0.0; 4]), rrect(ox + 2.0, oy + 2.0, 3.0, 3.0, [0.0; 4])),
    }
}

fn default_eyes(style: &str) -> &'static str {
    match style {
        "dots" | "fluid" => "circle",
        "rounded" | "gapped" | "vbars" | "hbars" => "rounded",
        "classy" | "leaf" => "leaf",
        "diamond" | "star" => "diamond",
        _ => "square",
    }
}

/// The QR code as an SVG document (unit = one module).
pub fn svg(code: &QrCode, s: &Spec) -> Result<String> {
    let n = code.width();
    let m = s.margin as f32;
    let total = n as f32 + 2.0 * m;
    let colors = code.to_colors();
    let dark = |x: i64, y: i64| {
        x >= 0 && y >= 0 && (x as usize) < n && (y as usize) < n && colors[y as usize * n + x as usize] == qrcode::Color::Dark
    };
    // Logo: clear a centred square of whole modules (+ a quiet ring) so nothing touches it.
    let (logo_box, logo_mods) = if s.logo.is_some() {
        let side = (n as f32 * s.logo_pct).round().max(3.0);
        let side = if (side as usize) % 2 != n % 2 { side + 1.0 } else { side };
        let start = (n as f32 - side) / 2.0;
        (Some((start, side)), Some((start.floor() as usize, (start + side).ceil() as usize)))
    } else {
        (None, None)
    };
    let skip = |x: usize, y: usize| {
        if in_finder(n, x, y) {
            return true;
        }
        if let Some((a, b)) = logo_mods {
            if x >= a && x < b && y >= a && y < b {
                return true;
            }
        }
        false
    };
    let data = modules_path(n, &s.style, &dark, &skip, m);
    let eyes = if s.eyes == "auto" || s.eyes.is_empty() { default_eyes(&s.style) } else { s.eyes.as_str() };
    let mut rings = String::new();
    let mut balls = String::new();
    for (i, (ex, ey)) in [(0usize, 0usize), (n - 7, 0), (0, n - 7)].into_iter().enumerate() {
        let (r, b) = eye_paths(eyes, m + ex as f32, m + ey as f32, i);
        rings.push_str(&r);
        balls.push_str(&b);
    }

    let mut o = String::new();
    o.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {t} {t}\" width=\"{t}\" height=\"{t}\" shape-rendering=\"geometricPrecision\">",
        t = f(total)
    ));
    let (fg, fga) = style::svg_paint(s.fg);
    let (fg2, fg2a) = style::svg_paint(s.fg2);
    let fill = match s.gradient.as_str() {
        "linear" => {
            o.push_str(&format!(
                "<defs><linearGradient id=\"g\" gradientUnits=\"userSpaceOnUse\" x1=\"{m}\" y1=\"{m}\" x2=\"{e}\" y2=\"{e}\"><stop offset=\"0\" stop-color=\"{fg}\" stop-opacity=\"{fga}\"/><stop offset=\"1\" stop-color=\"{fg2}\" stop-opacity=\"{fg2a}\"/></linearGradient></defs>",
                m = f(m),
                e = f(m + n as f32)
            ));
            "url(#g)".to_string()
        }
        "radial" => {
            o.push_str(&format!(
                "<defs><radialGradient id=\"g\" gradientUnits=\"userSpaceOnUse\" cx=\"{c}\" cy=\"{c}\" r=\"{r}\"><stop offset=\"0\" stop-color=\"{fg2}\" stop-opacity=\"{fg2a}\"/><stop offset=\"1\" stop-color=\"{fg}\" stop-opacity=\"{fga}\"/></radialGradient></defs>",
                c = f(total / 2.0),
                r = f(n as f32 * 0.72)
            ));
            "url(#g)".to_string()
        }
        _ => format!("{fg}\" fill-opacity=\"{fga}"),
    };
    if s.bg[3] > 0 {
        let (bg, bga) = style::svg_paint(s.bg);
        o.push_str(&format!("<rect width=\"{t}\" height=\"{t}\" fill=\"{bg}\" fill-opacity=\"{bga}\"/>", t = f(total)));
    }
    o.push_str(&format!("<path fill=\"{fill}\" d=\"{data}\"/>"));
    let eye_fill = if s.eye == s.fg { fill.clone() } else { let (e, ea) = style::svg_paint(s.eye); format!("{e}\" fill-opacity=\"{ea}") };
    o.push_str(&format!("<path fill=\"{eye_fill}\" fill-rule=\"evenodd\" d=\"{rings}\"/>"));
    o.push_str(&format!("<path fill=\"{eye_fill}\" d=\"{balls}\"/>"));
    if let (Some((start, side)), Some(logo)) = (logo_box, &s.logo) {
        let pad = 0.35;
        let (x, w) = (m + start + pad, side - 2.0 * pad);
        o.push_str(&format!(
            "<image x=\"{x}\" y=\"{x}\" width=\"{w}\" height=\"{w}\" preserveAspectRatio=\"xMidYMid meet\" href=\"data:image/png;base64,{}\"/>",
            common::base64(logo),
            x = f(x),
            w = f(w)
        ));
    }
    o.push_str("</svg>");
    Ok(o)
}

/// Render the SVG at a comfortable resolution and decode it; also try inverted colours.
pub fn scans(payload: &str, svg: &str, code: &QrCode) -> bool {
    let total = code.width() + 8;
    let side = (total as u32 * 8).clamp(400, 1600);
    let Ok(pm) = png::render_svg(svg.as_bytes(), side, side, false) else { return false };
    let img = png::pixmap_to_rgba(&pm);
    if png::decode_qr(&img).iter().any(|s| s == payload) {
        return true;
    }
    let mut inv = img.clone();
    for p in inv.pixels_mut() {
        // Over white first, then invert (light-on-dark codes).
        let a = p[3] as u32;
        for c in 0..3 {
            let v = (p[c] as u32 * a + 255 * (255 - a)) / 255;
            p[c] = 255 - v as u8;
        }
        p[3] = 255;
    }
    png::decode_qr(&inv).iter().any(|s| s == payload)
}

// ---------------------------------------------------------------- cards

/// Typst document of a printable card holding the code.
pub fn card_doc(code: &QrCode, s: &Spec, v: &Values) -> Result<typ::Doc> {
    let card = v.str("card");
    // Cards print the code in ink on white for reliable scanning unless the user chose colours.
    let qr_svg = svg(code, s)?;
    let def = |k: &str| tr(&format!("create.qr-code.card.{card}.{k}"));
    let pick = |key: &str, d: String| {
        let x = v.str(key).trim();
        if x.is_empty() { d } else { x.to_string() }
    };
    let ssid = v.str("ssid").trim().to_string();
    let title = pick("card_title", def("title"));
    let subtitle = pick(
        "card_subtitle",
        match card {
            "wifi" => ssid.clone(),
            _ => def("subtitle"),
        },
    );
    let text = pick(
        "card_text",
        match card {
            "wifi" => {
                if v.str("security") == "open" {
                    tr("create.qr-code.card.wifi.open")
                } else {
                    format!("{}\n{}", tr("create.qr-code.card.wifi.password"), v.str("wifi_password"))
                }
            }
            "business" => [v.str("job_title"), v.str("org"), v.str("phone"), v.str("email"), v.str("website")]
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => def("text"),
        },
    );
    let title = if card == "business" && v.str("card_title").trim().is_empty() && !v.str("name").trim().is_empty() {
        v.str("name").trim().to_string()
    } else {
        title
    };
    let tint = v.color("card_tint");
    let data = serde_json::json!({
        "title": title, "subtitle": subtitle, "text": text,
        "tint": style::hex(tint), "card": card,
        "scan": tr("create.qr-code.card.scan"),
        "network": tr("create.qr-code.card.wifi.network"),
    });
    let body = match card {
        "wifi" => CARD_WIFI,
        "table-tent" => CARD_TENT,
        "review" => CARD_REVIEW,
        "business" => CARD_BUSINESS,
        "ticket" => CARD_TICKET,
        "poster" => CARD_POSTER,
        "tag" => CARD_TAG,
        "social" => CARD_SOCIAL,
        "invitation" => CARD_INVITATION,
        other => bail!("unknown card '{other}'"),
    };
    let src = format!("{}\n{CARD_COMMON}\n{body}", style::TYPST_PRELUDE);
    Ok(typ::Doc::new(src).file("/qr.svg", qr_svg.into_bytes()).json("/data.json", &data))
}

const CARD_COMMON: &str = r##"
#let d = json("/data.json")
#let t = rgb(d.tint)
#let qr(w) = image("/qr.svg", width: w)
#set text(font: serif, fill: ink, size: 10pt, lang: "en")
#set par(leading: 0.55em)
#let lines(s) = s.split("\n").map(l => [#l]).join(linebreak())
#let scanline = text(font: sans, size: 7pt, fill: muted, tracking: 0.08em, upper(d.scan))
"##;

const CARD_WIFI: &str = r##"
#set page(width: 148mm, height: 105mm, margin: 0pt, fill: paper)
#place(top + left, rect(width: 100%, height: 9mm, fill: t))
#place(top + left, dx: 12mm, dy: 2.6mm, text(font: sans, size: 9pt, tracking: 0.1em, weight: "bold", upper(d.title)))
#place(top + right, dx: -12mm, dy: 3mm, diamond(size: 5pt))
#pad(left: 12mm, right: 12mm, top: 19mm, bottom: 10mm, grid(columns: (1fr, 52mm), gutter: 8mm,
  align(left + horizon, {
    text(font: sans, size: 8pt, fill: muted, tracking: 0.06em, upper(d.network))
    v(1mm)
    text(font: sans, weight: "bold", size: 20pt, d.subtitle)
    v(5mm)
    line(length: 100%, stroke: 0.5pt + rulec)
    v(3mm)
    text(font: mono, size: 11pt, lines(d.text))
  }),
  align(center + horizon, { qr(50mm); v(1mm); scanline })
))
"##;

const CARD_TENT: &str = r##"
#set page(width: 105mm, height: 148mm, margin: 0pt, fill: paper)
#place(top + center, dy: 10mm, diamond(size: 6pt))
#pad(x: 12mm, top: 20mm, bottom: 12mm, align(center, {
  text(font: chorus, size: 30pt, d.title)
  v(1mm)
  text(font: sans, size: 9pt, fill: muted, tracking: 0.08em, upper(d.subtitle))
  v(7mm)
  box(fill: t, inset: 5mm, radius: 2mm, qr(58mm))
  v(5mm)
  text(size: 10pt, style: "italic", lines(d.text))
}))
#place(bottom + center, dy: -7mm, box(width: 70mm, pagerule(size: 4pt, stroke: 0.5pt + ink)))
"##;

const CARD_REVIEW: &str = r##"
#set page(width: 105mm, height: 148mm, margin: 0pt, fill: paper)
#place(top + left, rect(width: 100%, height: 52mm, fill: t))
#pad(x: 12mm, top: 14mm, bottom: 10mm, align(center, {
  text(font: sans, size: 8pt, tracking: 0.1em, upper(d.subtitle))
  v(2mm)
  text(font: serif, weight: "bold", size: 21pt, d.title)
  v(3mm)
  box(fill: white, inset: 4mm, radius: 2mm, stroke: 0.5pt + rulec, qr(54mm))
  v(5mm)
  text(size: 11pt, fill: ink, sym.star.filled + sym.star.filled + sym.star.filled + sym.star.filled + sym.star.filled)
  v(2mm)
  text(size: 10pt, style: "italic", fill: muted, lines(d.text))
}))
"##;

const CARD_BUSINESS: &str = r##"
#set page(width: 85mm, height: 55mm, margin: 0pt, fill: paper)
#place(left + top, rect(width: 2.2mm, height: 100%, fill: t))
#pad(left: 8mm, right: 5mm, y: 6mm, grid(columns: (1fr, 25mm), gutter: 4mm,
  {
    text(font: sans, weight: "bold", size: 12.5pt, d.title)
    v(0.5mm)
    text(font: sans, size: 7.5pt, fill: muted, tracking: 0.05em, upper(d.subtitle))
    v(2.5mm)
    box(width: 100%, pagerule(size: 3pt, stroke: 0.45pt + ink))
    v(2mm)
    text(size: 7.5pt, lines(d.text))
  },
  align(center + horizon, qr(25mm))
))
"##;

const CARD_TICKET: &str = r##"
#set page(width: 190mm, height: 70mm, margin: 0pt, fill: paper)
#place(left + top, rect(width: 140mm, height: 100%, fill: t))
#place(left + top, dx: 140mm, line(start: (0pt, 4mm), end: (0pt, 66mm), stroke: (paint: ink, thickness: 0.6pt, dash: "dashed")))
#place(left + top, dx: 140mm - 4mm, dy: -4mm, circle(radius: 4mm, fill: white, stroke: none))
#place(left + bottom, dx: 140mm - 4mm, dy: 4mm, circle(radius: 4mm, fill: white, stroke: none))
#place(left + top, dx: 12mm, dy: 10mm, box(width: 118mm, {
  text(font: sans, size: 8pt, tracking: 0.12em, upper(d.subtitle))
  v(1mm)
  text(font: serif, weight: "bold", size: 26pt, d.title)
  v(3mm)
  box(width: 60mm, pagerule(size: 4pt, stroke: 0.6pt + ink))
  v(3mm)
  text(size: 10pt, lines(d.text))
}))
#place(left + top, dx: 146mm, dy: 9mm, box(width: 38mm, align(center, { qr(36mm); v(1mm); scanline })))
"##;

const CARD_POSTER: &str = r##"
#set page(width: 210mm, height: 297mm, margin: 0pt, fill: paper)
#place(top + right, dx: -18mm, dy: 16mm, text(font: sans, size: 9pt, fill: muted, tracking: 0.1em, upper(d.subtitle)))
#place(top + left, dx: 18mm, dy: 14mm, box(width: 174mm, pagerule(size: 6pt, stroke: 0.8pt + ink)))
#place(top + left, dx: 10mm, dy: 18mm, text(font: chorus, size: 150pt, fill: ink.transparentize(90%), d.title.clusters().at(0, default: "")))
#pad(x: 18mm, top: 40mm, bottom: 20mm, {
  text(font: sans, weight: "bold", size: 44pt, d.title)
  v(4mm)
  text(size: 15pt, fill: muted, lines(d.text))
  v(1fr)
  align(center, box(fill: t, inset: 10mm, radius: 3mm, qr(110mm)))
  v(4mm)
  align(center, text(font: sans, size: 11pt, fill: muted, tracking: 0.12em, upper(d.scan)))
})
"##;

const CARD_TAG: &str = r##"
#set page(width: 55mm, height: 95mm, margin: 0pt, fill: paper)
#place(top + center, dy: 7mm, circle(radius: 2.6mm, stroke: 0.6pt + ink, fill: white))
#place(top + left, dy: 16mm, rect(width: 100%, height: 1.6mm, fill: t))
#pad(x: 6mm, top: 22mm, bottom: 6mm, align(center, {
  text(font: sans, weight: "bold", size: 12pt, d.title)
  v(1mm)
  text(font: sans, size: 7pt, fill: muted, tracking: 0.08em, upper(d.subtitle))
  v(3mm)
  qr(36mm)
  v(2mm)
  text(font: serif, weight: "bold", size: 14pt, lines(d.text))
}))
"##;

const CARD_SOCIAL: &str = r##"
#set page(width: 100mm, height: 100mm, margin: 0pt, fill: t)
#place(top + left, dx: 8mm, dy: 8mm, diamond(size: 5pt))
#place(top + right, dx: -8mm, dy: 8mm, diamond(size: 5pt))
#pad(x: 10mm, top: 11mm, bottom: 8mm, align(center, {
  text(font: chorus, size: 24pt, d.title)
  v(1mm)
  box(fill: white, inset: 3mm, radius: 2mm, qr(42mm))
  v(3mm)
  text(font: sans, weight: "bold", size: 12pt, d.subtitle)
  v(0.5mm)
  text(size: 8.5pt, fill: muted, lines(d.text))
}))
"##;

const CARD_INVITATION: &str = r##"
#set page(width: 148mm, height: 105mm, margin: 0pt, fill: paper)
#place(center + horizon, rect(width: 136mm, height: 93mm, stroke: 0.5pt + ink, fill: none))
#place(center + horizon, rect(width: 133mm, height: 90mm, stroke: 0.3pt + rulec, fill: none))
#place(top + center, dy: 6mm - 3pt, box(fill: paper, inset: (x: 3pt), diamond(size: 5pt)))
#pad(x: 16mm, y: 14mm, grid(columns: (1fr, 40mm), gutter: 8mm,
  align(left + horizon, {
    text(font: chorus, size: 30pt, d.title)
    v(2mm)
    text(font: sans, size: 8.5pt, fill: muted, tracking: 0.1em, upper(d.subtitle))
    v(4mm)
    text(size: 10.5pt, style: "italic", lines(d.text))
  }),
  align(center + horizon, { box(fill: t, inset: 2.5mm, radius: 1.5mm, qr(36mm)); v(1mm); scanline })
))
"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> Values {
        let t = QrCodeTool::new();
        let specs = t.options();
        let mut v = Values::defaults(&specs);
        for (k, val) in pairs {
            let s = specs.iter().find(|s| s.key == *k).unwrap();
            v.set(k, s.parse(val).unwrap());
        }
        v
    }

    #[test]
    fn every_style_scans() {
        for style in STYLES {
            for eyes in ["auto", "square", "rounded", "circle", "leaf", "diamond"] {
                let v = values(&[("style", style), ("eyes", eyes), ("url", "https://basedpolymer.eu/castor?x=1")]);
                let s = Spec::from_values(&v).unwrap();
                let c = s.code().unwrap();
                let svg = svg(&c, &s).unwrap();
                assert!(scans(&s.payload, &svg, &c), "style {style} eyes {eyes} does not scan");
            }
        }
    }

    #[test]
    fn payloads() {
        let v = values(&[("content", "wifi"), ("ssid", "Café; Wi:Fi"), ("security", "wpa")]);
        let mut v = v;
        v.set("wifi_password", castor_core::Value::Text("p\"w".into()));
        assert_eq!(payload(&v).unwrap(), "WIFI:T:WPA;S:Café\\; Wi\\:Fi;P:p\\\"w;;");
        let v = values(&[("content", "event"), ("event_title", "Launch"), ("start", "2026-11-06 19:30")]);
        assert!(payload(&v).unwrap().contains("DTSTART:20261106T193000"));
        let v = values(&[("url", "basedpolymer.eu")]);
        assert_eq!(payload(&v).unwrap(), "https://basedpolymer.eu");
    }
}
