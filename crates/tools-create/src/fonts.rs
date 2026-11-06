//! `create.fonts`: font specimens (PDF/PNG: names, info, waterfall, glyph grid), conversion
//! between TTF/OTF, WOFF and WOFF2, an info report, a catalogue of the installed families, and
//! (only when asked, Beta) fetching an open-licence family from the Google Fonts repository.

use std::io::Write as _;

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common;
use crate::render::{style, typ};

pub struct FontsTool {
    meta: ToolMeta,
}

impl FontsTool {
    pub fn new() -> Self {
        FontsTool {
            meta: common::meta(
                "create.fonts",
                InputSpec::kinds(&[FileKind::Font]).with_extensions(&["otc", "pfb"]).min(0),
                "PDF · PNG · WOFF2 · WOFF · TTF",
                &["font", "typeface", "specimen", "woff", "woff2", "ttf", "otf", "convert", "glyphs", "google fonts"],
                "specimen",
            ),
        }
    }
}

impl Tool for FontsTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("action", ["specimen", "convert", "info", "catalog", "google"].map(Choice::key).to_vec(), "specimen"),
            OptionSpec::text("sample", "Sphinx of black quartz, judge my vow. Voix ambiguë d’un cœur qui au zéphyr préfère les jattes de kiwis.")
                .when(Cond::Eq("action", "specimen")),
            OptionSpec::choice("specimen_format", vec![Choice::lit("pdf", "PDF"), Choice::lit("png", "PNG")], "pdf").when(Cond::Eq("action", "specimen")),
            OptionSpec::choice(
                "to",
                vec![Choice::lit("woff2", "WOFF2"), Choice::lit("woff", "WOFF"), Choice::lit("sfnt", "TTF / OTF")],
                "woff2",
            )
            .when(Cond::Eq("action", "convert")),
            OptionSpec::choice("info_format", vec![Choice::lit("txt", "TXT"), Choice::lit("json", "JSON")], "txt").when(Cond::Eq("action", "info")),
            OptionSpec::text("filter", "").when(Cond::Eq("action", "catalog")),
            OptionSpec::text("google_family", "").when(Cond::Eq("action", "google")),
            OptionSpec::toggle("allow_network", false).when(Cond::Eq("action", "google")),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], v: &Values) -> Result<Vec<Job>> {
        match v.str("action") {
            "catalog" | "google" => Ok(vec![Job::many(inputs.to_vec())]),
            _ => {
                anyhow::ensure!(!inputs.is_empty(), "{}", tr("create.fonts.err.no_font"));
                Ok(inputs.iter().cloned().map(Job::single).collect())
            }
        }
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let doc = match (v.str("action"), input) {
            ("catalog", _) => catalog_doc(v.str("filter"), 60)?,
            (_, Some(f)) => {
                let data = sfnt(&std::fs::read(&f.path)?)?;
                specimen_doc(&data, v.str("sample"), &f.name())?
            }
            _ => return Ok(None),
        };
        Ok(Some(common::preview(typ::png_fit(&doc, 0, max_side)?, common::figure(&tr("create.fonts.caption")))))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        match v.str("action") {
            "catalog" => {
                cx.message(tr("create.fonts.msg.scanning"));
                let doc = catalog_doc(v.str("filter"), usize::MAX)?;
                let out = cx.output_named("installed-fonts.pdf");
                common::write(&out, &typ::pdf(&doc)?)?;
                Ok(vec![Output::new(out).with_note(tr_args("create.fonts.note_pages", &[("pages", &typ::page_count(&doc).to_string())]))])
            }
            "google" => google(v, cx),
            action => {
                let input = job.input()?;
                let raw = std::fs::read(&input.path)?;
                let data = sfnt(&raw).map_err(|e| anyhow!("{}: {e}", input.name()))?;
                match action {
                    "convert" => {
                        let (bytes, ext) = match v.str("to") {
                            "woff" => (woff1(&data)?, "woff"),
                            "sfnt" => {
                                let ext = if data.starts_with(b"OTTO") { "otf" } else { "ttf" };
                                (data.clone(), ext)
                            }
                            _ => (
                                ttf2woff2::encode(&data, ttf2woff2::BrotliQuality::default()).map_err(|e| anyhow!("WOFF2: {e}"))?,
                                "woff2",
                            ),
                        };
                        let out = cx.output_named(&format!("{}.{ext}", input.stem()));
                        common::write(&out, &bytes)?;
                        Ok(vec![Output::new(out).with_note(format!(
                            "{} → {} ({})",
                            castor_core::units::bytes(raw.len() as u64),
                            castor_core::units::bytes(bytes.len() as u64),
                            castor_core::units::change(raw.len() as u64, bytes.len() as u64)
                        ))])
                    }
                    "info" => {
                        let info = font_info(&data)?;
                        let (text, ext) = if v.str("info_format") == "json" {
                            (serde_json::to_string_pretty(&info)?, "json")
                        } else {
                            let mut s = String::new();
                            for (k, val) in info.as_object().into_iter().flatten() {
                                s.push_str(&format!("{k:<14} {}\n", val.as_str().map(String::from).unwrap_or_else(|| val.to_string())));
                            }
                            (s, "txt")
                        };
                        let out = cx.output_named(&format!("{}-info.{ext}", input.stem()));
                        common::write(&out, text.as_bytes())?;
                        Ok(vec![Output::new(out)])
                    }
                    _ => {
                        let doc = specimen_doc(&data, v.str("sample"), &input.name())?;
                        if v.str("specimen_format") == "png" {
                            let mut outs = Vec::new();
                            for i in 0..typ::page_count(&doc) {
                                let out = cx.output_named(&format!("{}-specimen-{}.png", input.stem(), i + 1));
                                common::write(&out, &typ::png_width(&doc, i, 1654)?)?;
                                outs.push(Output::new(out));
                            }
                            Ok(outs)
                        } else {
                            let out = cx.output_path(Some(input), "pdf");
                            common::write(&out, &typ::pdf(&doc)?)?;
                            Ok(vec![Output::new(out)])
                        }
                    }
                }
            }
        }
    }
}

/// Any font container → plain sfnt (TTF/OTF/TTC) bytes.
pub fn sfnt(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.starts_with(b"wOF2") {
        return wuff::decompress_woff2(raw).map_err(|e| anyhow!("WOFF2: {e:?}"));
    }
    if raw.starts_with(b"wOFF") {
        return wuff::decompress_woff1(raw).map_err(|e| anyhow!("WOFF: {e:?}"));
    }
    let ok = raw.starts_with(&[0, 1, 0, 0]) || raw.starts_with(b"OTTO") || raw.starts_with(b"true") || raw.starts_with(b"ttcf");
    anyhow::ensure!(ok, "{}", tr("create.fonts.err.not_font"));
    Ok(raw.to_vec())
}

/// WOFF 1.0 encoder (zlib per table, original checksums kept).
pub fn woff1(data: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(!data.starts_with(b"ttcf"), "{}", tr("create.fonts.err.collection"));
    let rd16 = |o: usize| -> Result<u16> { Ok(u16::from_be_bytes(data.get(o..o + 2).ok_or_else(|| anyhow!("truncated font"))?.try_into()?)) };
    let rd32 = |o: usize| -> Result<u32> { Ok(u32::from_be_bytes(data.get(o..o + 4).ok_or_else(|| anyhow!("truncated font"))?.try_into()?)) };
    let flavor = rd32(0)?;
    let n = rd16(4)? as usize;
    let mut tables = Vec::with_capacity(n);
    for i in 0..n {
        let e = 12 + i * 16;
        let (tag, csum, off, len) = (rd32(e)?, rd32(e + 4)?, rd32(e + 8)? as usize, rd32(e + 12)? as usize);
        let raw = data.get(off..off + len).ok_or_else(|| anyhow!("truncated table"))?;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(raw)?;
        let comp = z.finish()?;
        let stored = if comp.len() < raw.len() { comp } else { raw.to_vec() };
        tables.push((tag, csum, len as u32, stored));
    }
    tables.sort_by_key(|t| t.0);
    let header = 44 + 20 * n;
    let mut body = Vec::new();
    let mut dir = Vec::new();
    let mut total_sfnt = 12 + 16 * n as u32;
    for (tag, csum, orig, stored) in &tables {
        let off = header + body.len();
        dir.extend_from_slice(&tag.to_be_bytes());
        dir.extend_from_slice(&(off as u32).to_be_bytes());
        dir.extend_from_slice(&(stored.len() as u32).to_be_bytes());
        dir.extend_from_slice(&orig.to_be_bytes());
        dir.extend_from_slice(&csum.to_be_bytes());
        body.extend_from_slice(stored);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        total_sfnt += (orig + 3) & !3;
    }
    let mut o = Vec::with_capacity(header + body.len());
    o.extend_from_slice(b"wOFF");
    o.extend_from_slice(&flavor.to_be_bytes());
    o.extend_from_slice(&((header + body.len()) as u32).to_be_bytes());
    o.extend_from_slice(&(n as u16).to_be_bytes());
    o.extend_from_slice(&0u16.to_be_bytes());
    o.extend_from_slice(&total_sfnt.to_be_bytes());
    o.extend_from_slice(&1u16.to_be_bytes()); // major version
    o.extend_from_slice(&0u16.to_be_bytes());
    o.extend_from_slice(&[0; 20]); // no metadata, no private block
    o.extend_from_slice(&dir);
    o.extend_from_slice(&body);
    Ok(o)
}

/// Names and facts about the first face.
pub fn font_info(data: &[u8]) -> Result<serde_json::Value> {
    let face = ttf_parser::Face::parse(data, 0).map_err(|e| anyhow!("{}: {e}", tr("create.fonts.err.not_font")))?;
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|n| n.name_id == id)
            .filter_map(|n| n.to_string())
            .next()
            .unwrap_or_default()
    };
    let family = { let t = name(16); if t.is_empty() { name(1) } else { t } };
    let sub = { let t = name(17); if t.is_empty() { name(2) } else { t } };
    let mut chars = 0usize;
    if let Some(cmap) = face.tables().cmap {
        let mut set = std::collections::HashSet::new();
        for st in cmap.subtables {
            if st.is_unicode() {
                st.codepoints(|c| {
                    set.insert(c);
                });
            }
        }
        chars = set.len();
    }
    let axes: Vec<String> = face
        .variation_axes()
        .into_iter()
        .map(|a| format!("{} {}–{}", a.tag, a.min_value, a.max_value))
        .collect();
    Ok(serde_json::json!({
        "family": family,
        "style": sub,
        "full_name": name(4),
        "version": name(5),
        "weight": face.weight().to_number(),
        "italic": face.is_italic(),
        "outlines": if data.starts_with(b"OTTO") { "CFF (PostScript)" } else { "TrueType" },
        "glyphs": face.number_of_glyphs(),
        "characters": chars,
        "variable_axes": axes.join(", "),
        "faces_in_file": ttf_parser::fonts_in_collection(data).unwrap_or(1),
        "designer": name(9),
        "manufacturer": name(8),
        "copyright": name(0),
        "license": name(13),
        "license_url": name(14),
    }))
}

/// Up to `max` characters the font covers (for the glyph grid).
fn covered_chars(data: &[u8], max: usize) -> Vec<char> {
    let Ok(face) = ttf_parser::Face::parse(data, 0) else { return Vec::new() };
    let mut set = std::collections::BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for st in cmap.subtables {
            if st.is_unicode() {
                st.codepoints(|c| {
                    if let Some(ch) = char::from_u32(c) {
                        if !ch.is_control() && !ch.is_whitespace() && c >= 0x21 {
                            set.insert(ch);
                        }
                    }
                });
            }
        }
    }
    set.into_iter().take(max).collect()
}

/// A specimen document: name in the font, info table, waterfall, glyph grid.
pub fn specimen_doc(data: &[u8], sample: &str, file_name: &str) -> Result<typ::PagedDocument> {
    let info = font_info(data)?;
    // Typst knows the face by its family as Typst reads it.
    let face = typst::text::Font::iter(typst::foundations::Bytes::new(data.to_vec()))
        .next()
        .ok_or_else(|| anyhow!("{}", tr("create.fonts.err.not_font")))?;
    let fam = face.info().family.to_string();
    // Select exactly this face (a slanted or bold file shares its family with others).
    let fstyle = format!("{:?}", face.info().variant.style).to_lowercase();
    let fweight = face.info().variant.weight.to_number();
    let glyphs: String = covered_chars(data, 320).into_iter().collect();
    let rows: Vec<(String, String)> = [
        ("family", "create.fonts.k.family"),
        ("style", "create.fonts.k.style"),
        ("version", "create.fonts.k.version"),
        ("weight", "create.fonts.k.weight"),
        ("outlines", "create.fonts.k.outlines"),
        ("glyphs", "create.fonts.k.glyphs"),
        ("characters", "create.fonts.k.characters"),
        ("variable_axes", "create.fonts.k.axes"),
        ("designer", "create.fonts.k.designer"),
        ("license", "create.fonts.k.license"),
    ]
    .iter()
    .filter_map(|(k, label)| {
        let v = &info[*k];
        let s = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
        let s: String = s.chars().take(160).collect();
        (!s.is_empty() && s != "null").then(|| (tr(label), s))
    })
    .collect();
    let d = serde_json::json!({
        "font": fam, "display": info["family"].as_str().unwrap_or(&fam), "style": fstyle, "weight": fweight, "file": file_name, "sample": sample, "glyphs": glyphs,
        "rows": rows, "title": tr("create.fonts.specimen"), "glyph_title": tr("create.fonts.glyphs"),
        "table": tr("create.fonts.table"),
    });
    let src = format!("{}\n{SPECIMEN}", style::TYPST_PRELUDE);
    let mut doc = typ::Doc::new(src).json("/data.json", &d);
    doc.font_data.push(data.to_vec());
    doc.compile()
}

const SPECIMEN: &str = r##"
#let d = json("/data.json")
#set page(paper: "a4", margin: (x: 20mm, top: 22mm, bottom: 20mm), fill: paper,
  footer: context { set text(font: serif, size: 8.5pt, fill: muted); d.file; h(1fr); counter(page).display() })
#set text(font: serif, size: 10pt, fill: ink, fallback: true)
#let spec(size, body, fallback: true) = text(font: d.font, style: d.style, weight: d.weight, size: size, fallback: fallback, body)
#place(top + right, dy: -8mm, box(width: 50mm, { align(right, text(font: sans, size: 9pt, d.title)); v(-2mm); pagerule(stroke: 0.5pt + ink, size: 3.5pt) }))
#v(6mm)
#spec(46pt, d.display, fallback: false)
#v(2mm)
#spec(13pt, d.sample)
#v(6mm)
#text(font: serif, size: 9.5pt, fill: muted, [*Table 1* — ] + d.table)
#v(-1mm)
#line(length: 100%, stroke: 1.1pt + ink)
#v(-2mm)
#table(columns: (38mm, 1fr), stroke: none, inset: (x: 0pt, y: 4.5pt),
  ..d.rows.map(r => (text(font: sans, size: 9pt, fill: muted, r.at(0)), text(size: 9.5pt, r.at(1)))).flatten())
#v(-2mm)
#line(length: 100%, stroke: 1.1pt + ink)
#v(6mm)
#for s in (9, 11, 14, 18, 24, 32, 44) {
  grid(columns: (12mm, 1fr), align: (left + top, left + top),
    pad(top: s * 0.35pt, text(font: mono, size: 7.5pt, fill: faint, str(s) + " pt")),
    block(height: s * 1.3pt, clip: true, spec(s * 1pt, d.sample)))
  v(1mm)
}
#pagebreak()
#text(font: sans, weight: "bold", size: 14pt, d.glyph_title)
#v(3mm)
#grid(columns: (1fr,) * 12, gutter: 0pt,
  ..d.glyphs.clusters().map(c => box(width: 100%, height: 13.5mm, stroke: 0.3pt + rulec, inset: 0pt,
    align(center + horizon, spec(17pt, c, fallback: false)))))
"##;

/// A catalogue of installed families, each line set in its own font.
pub fn catalog_doc(filter: &str, max: usize) -> Result<typ::PagedDocument> {
    let db = typ::system_db();
    let f = filter.trim().to_lowercase();
    let mut fams: Vec<String> = db.faces().flat_map(|face| face.families.first().map(|(n, _)| n.clone())).collect();
    fams.sort_by(|a, b| castor_core::files::natural_cmp(a, b));
    fams.dedup();
    let fams: Vec<String> = fams.into_iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)).take(max).collect();
    anyhow::ensure!(!fams.is_empty(), "{}", tr("create.fonts.err.none_found"));
    let mut doc = typ::Doc::new(String::new());
    // One face per family (the most regular one), each file read once.
    let mut seen = std::collections::HashSet::new();
    for fam in &fams {
        let best = db
            .faces()
            .filter(|face| face.families.first().is_some_and(|(n, _)| n == fam))
            .min_by_key(|face| (face.style != resvg::usvg::fontdb::Style::Normal, (face.weight.0 as i32 - 400).abs()));
        if let Some(face) = best {
            if let resvg::usvg::fontdb::Source::File(p) = &face.source {
                if seen.insert(p.clone()) {
                    if let Ok(data) = std::fs::read(p) {
                        doc.font_data.push(data);
                    }
                }
            }
        }
    }
    let d = serde_json::json!({"fams": fams, "title": tr("create.fonts.catalog"), "count": fams.len()});
    doc.source = format!(
        "{}\n{}",
        style::TYPST_PRELUDE,
        r##"
#let d = json("/data.json")
#set page(paper: "a4", margin: (x: 18mm, y: 18mm), fill: paper, footer: context align(right, text(font: serif, size: 8pt, fill: muted, counter(page).display())))
#set text(font: serif, size: 10pt, fill: ink)
#text(font: sans, weight: "bold", size: 20pt, d.title)
#h(1fr)
#text(font: sans, size: 10pt, fill: muted, str(d.count))
#v(-2mm)
#pagerule(stroke: 0.6pt + ink, size: 4pt)
#v(3mm)
#for f in d.fams {
  grid(columns: (55mm, 1fr), align: horizon,
    text(font: mono, size: 7.5pt, fill: muted, f),
    block(height: 15pt, clip: true, text(font: (f, "Latin Modern Roman"), size: 14pt, "Castor — Hamburgefonstiv 0123")))
  line(length: 100%, stroke: 0.3pt + rulec)
}
"##
    );
    doc.json("/data.json", &d).compile()
}

/// Fetch a family from github.com/google/fonts (open licences only), with explicit consent.
fn google(v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
    anyhow::ensure!(v.bool("allow_network"), "{}", tr("create.fonts.err.consent"));
    let fam = v.str("google_family").trim().to_string();
    anyhow::ensure!(!fam.is_empty(), "{}", tr("create.fonts.err.family"));
    let dir_name: String = fam.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    anyhow::ensure!(!dir_name.is_empty(), "{}", tr("create.fonts.err.family"));
    let cancel = cx.cancel_token();
    let base = "https://raw.githubusercontent.com/google/fonts/main";
    let tmp = cx.tmp_dir().join("gf");
    std::fs::create_dir_all(&tmp)?;
    let mut found = None;
    for lic in ["ofl", "apache", "ufl"] {
        let meta = tmp.join(format!("{lic}-METADATA.pb"));
        if castor_core::download::fetch(&format!("{base}/{lic}/{dir_name}/METADATA.pb"), &meta, "", 0, &cancel, &|_, _| {}).is_ok() {
            found = Some((lic, std::fs::read_to_string(&meta)?));
            break;
        }
    }
    let (lic, meta) = found.ok_or_else(|| anyhow!("{}", tr_args("create.fonts.err.not_on_google", &[("family", &fam)])))?;
    let files: Vec<String> = meta
        .lines()
        .filter_map(|l| l.trim().strip_prefix("filename:"))
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|f| !f.contains('/') && !f.contains('\\') && !f.contains("..") && (f.ends_with(".ttf") || f.ends_with(".otf")))
        .collect();
    anyhow::ensure!(!files.is_empty(), "{}", tr_args("create.fonts.err.not_on_google", &[("family", &fam)]));
    let mut outs = Vec::new();
    for (i, f) in files.iter().enumerate() {
        cx.check_cancel()?;
        cx.message(f.clone());
        let enc: String = f.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        let dl = tmp.join(f);
        castor_core::download::fetch(&format!("{base}/{lic}/{dir_name}/{enc}"), &dl, "", 0, &cancel, &|_, _| {})?;
        // Untrusted bytes: keep them only if they parse as a font.
        let data = std::fs::read(&dl)?;
        ttf_parser::Face::parse(&data, 0).map_err(|_| anyhow!("{f}: {}", tr("create.fonts.err.not_font")))?;
        let out = cx.output_named(f);
        common::write(&out, &data)?;
        outs.push(Output::new(out).with_note(lic.to_uppercase()));
        cx.progress((i + 1) as f32 / files.len() as f32);
    }
    Ok(outs)
}

#[cfg(test)]
mod tests {
    use super::*;
    const LM: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-regular.otf");

    #[test]
    fn woff_round_trips() {
        let w1 = woff1(LM).unwrap();
        assert!(w1.len() < LM.len());
        assert_eq!(sfnt(&w1).unwrap().len() / 4, sfnt(&w1).unwrap().len() / 4);
        let back = sfnt(&w1).unwrap();
        ttf_parser::Face::parse(&back, 0).unwrap();
        let w2 = ttf2woff2::encode(LM, ttf2woff2::BrotliQuality::default()).unwrap();
        let back2 = sfnt(&w2).unwrap();
        assert_eq!(ttf_parser::Face::parse(&back2, 0).unwrap().number_of_glyphs(), ttf_parser::Face::parse(LM, 0).unwrap().number_of_glyphs());
    }

    #[test]
    fn specimen_compiles() {
        let d = specimen_doc(LM, "Hello", "lmsans10-regular.otf").unwrap();
        assert!(typ::page_count(&d) >= 2);
        assert_eq!(font_info(LM).unwrap()["family"], "Latin Modern Sans");
    }
}
