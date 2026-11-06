//! Transcript outputs: TXT, SRT, VTT, JSON, DOCX (hand-written OOXML) and PDF (krilla, Latin
//! Modern, justified paragraphs).

use std::io::Write;

use castor_core::units;
use engine_media::subs::{self, Layout};
use engine_media::whisper::Transcript;

/// One paragraph: start time and text.
pub struct Paragraph {
    pub start: f64,
    pub text: String,
}

/// Paragraphs: a new one after a pause of `pause` seconds, or after ~`max_chars` characters at
/// the end of a sentence.
pub fn paragraphs(t: &Transcript, pause: f64, max_chars: usize) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut last_end: Option<f64> = None;
    for s in &t.segments {
        let text = s.text.trim();
        if text.is_empty() {
            continue;
        }
        let new = match (out.last(), last_end) {
            (None, _) => true,
            (Some(p), Some(e)) => {
                s.start - e > pause || (p.text.chars().count() > max_chars && p.text.ends_with(['.', '!', '?', '…', '。']))
            }
            _ => false,
        };
        if new {
            out.push(Paragraph { start: s.start, text: text.to_string() });
        } else {
            let p = out.last_mut().unwrap();
            p.text.push(' ');
            p.text.push_str(text);
        }
        last_end = Some(s.end);
    }
    out
}

fn stamp(t: f64) -> String {
    let full = units::timecode(t);
    full[..8].to_string()
}

pub fn to_txt(t: &Transcript, timestamps: bool) -> String {
    let mut s = String::new();
    for p in paragraphs(t, 1.5, 700) {
        if timestamps {
            s.push_str(&format!("[{}] ", stamp(p.start)));
        }
        s.push_str(&p.text);
        s.push_str("\n\n");
    }
    s
}

pub fn to_srt(t: &Transcript) -> String {
    subs::to_srt(&subs::cues_from_transcript(t, Layout::default()))
}

pub fn to_vtt(t: &Transcript) -> String {
    subs::to_vtt(&subs::cues_from_transcript(t, Layout::default()))
}

pub fn to_json(t: &Transcript) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(t)?)
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // XML 1.0 forbids most control characters.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

/// Minimal, valid Word document (Title + subtitle + paragraphs).
pub fn to_docx(t: &Transcript, title: &str, subtitle: &str, timestamps: bool) -> anyhow::Result<Vec<u8>> {
    let mut body = String::new();
    body.push_str(&format!(r#"<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, xml_escape(title)));
    body.push_str(&format!(r#"<w:p><w:pPr><w:pStyle w:val="Subtitle"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, xml_escape(subtitle)));
    for p in paragraphs(t, 1.5, 700) {
        body.push_str("<w:p>");
        if timestamps {
            body.push_str(&format!(
                r#"<w:r><w:rPr><w:color w:val="808080"/><w:sz w:val="18"/></w:rPr><w:t xml:space="preserve">[{}] </w:t></w:r>"#,
                stamp(p.start)
            ));
        }
        body.push_str(&format!(r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, xml_escape(&p.text)));
    }
    let lang = if t.language.is_empty() { "en".to_string() } else { t.language.clone() };
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Cambria" w:hAnsi="Cambria" w:eastAsia="Cambria" w:cs="Cambria"/><w:sz w:val="22"/><w:lang w:val="{lang}"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="300" w:lineRule="auto"/><w:jc w:val="both"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:spacing w:after="80"/><w:jc w:val="left"/></w:pPr><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:spacing w:after="360"/><w:jc w:val="left"/></w:pPr><w:rPr><w:color w:val="666666"/><w:sz w:val="20"/></w:rPr></w:style>
</w:styles>"#
    );
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>Castor</dc:creator><dc:language>{lang}</dc:language><dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created></cp:coreProperties>"#,
        xml_escape(title)
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("[Content_Types].xml", content_types.to_string()),
            ("_rels/.rels", rels.to_string()),
            ("word/document.xml", document),
            ("word/styles.xml", styles),
            ("word/_rels/document.xml.rels", doc_rels.to_string()),
            ("docProps/core.xml", core),
        ] {
            z.start_file(name, opt)?;
            z.write_all(data.as_bytes())?;
        }
        z.finish()?;
    }
    Ok(buf.into_inner())
}

// ---------------------------------------------------------------------------------------------
// PDF

struct Face {
    data: Vec<u8>,
    font: krilla::text::Font,
}

impl Face {
    fn new(data: Vec<u8>) -> Option<Face> {
        let font = krilla::text::Font::new(data.clone().into(), 0)?;
        Some(Face { data, font })
    }
    fn width(&self, s: &str, size: f32) -> f32 {
        let Ok(f) = ttf_parser::Face::parse(&self.data, 0) else { return s.chars().count() as f32 * size * 0.5 };
        let upm = f.units_per_em() as f32;
        s.chars()
            .map(|c| f.glyph_index(c).and_then(|g| f.glyph_hor_advance(g)).map(|a| a as f32).unwrap_or(upm * 0.5))
            .sum::<f32>()
            * size
            / upm
    }
    fn covers(&self, s: &str) -> bool {
        let Ok(f) = ttf_parser::Face::parse(&self.data, 0) else { return false };
        s.chars().filter(|c| !c.is_whitespace() && !c.is_control()).all(|c| f.glyph_index(c).is_some())
    }
}

/// Fonts for the PDF: Latin Modern when it covers the text, else a system font that does.
fn pick_faces(text: &str) -> (Face, Face) {
    let (reg, bold) = crate::fonts::lm_roman();
    let lm = Face::new(reg.to_vec()).expect("bundled font");
    if lm.covers(text) {
        return (lm, Face::new(bold.to_vec()).expect("bundled font"));
    }
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    for fam in [
        "Segoe UI", "Arial", "Helvetica Neue", "Noto Sans", "Microsoft YaHei", "PingFang SC", "Hiragino Sans", "Malgun Gothic",
        "Noto Sans CJK SC", "Arial Unicode MS", "Nirmala UI", "Leelawadee UI", "Tahoma",
    ] {
        let q = fontdb::Query { families: &[fontdb::Family::Name(fam)], ..Default::default() };
        if let Some(id) = db.query(&q) {
            if let Some(data) = db.with_face_data(id, |d, _| d.to_vec()) {
                if let Some(f) = Face::new(data) {
                    if f.covers(text) {
                        let qb = fontdb::Query { families: &[fontdb::Family::Name(fam)], weight: fontdb::Weight::BOLD, ..Default::default() };
                        let b = db.query(&qb).and_then(|id| db.with_face_data(id, |d, _| d.to_vec())).and_then(Face::new);
                        let b = b.unwrap_or_else(|| Face::new(f.data.clone()).unwrap());
                        return (f, b);
                    }
                }
            }
        }
    }
    (lm, Face::new(bold.to_vec()).expect("bundled font"))
}

/// Break `text` into lines no wider than `width` (words; long words by characters).
fn break_lines(face: &Face, text: &str, size: f32, width: f32) -> Vec<Vec<String>> {
    let mut lines: Vec<Vec<String>> = vec![vec![]];
    let mut cur_w = 0.0;
    let space = face.width(" ", size);
    for word in text.split_whitespace() {
        let mut pieces = vec![word.to_string()];
        if face.width(word, size) > width {
            pieces.clear();
            let mut p = String::new();
            for c in word.chars() {
                if face.width(&format!("{p}{c}"), size) > width && !p.is_empty() {
                    pieces.push(std::mem::take(&mut p));
                }
                p.push(c);
            }
            pieces.push(p);
        }
        for w in pieces {
            let ww = face.width(&w, size);
            let line = lines.last_mut().unwrap();
            if !line.is_empty() && cur_w + space + ww > width {
                lines.push(vec![w]);
                cur_w = ww;
            } else {
                cur_w += if line.is_empty() { ww } else { space + ww };
                line.push(w);
            }
        }
    }
    lines
}

/// A clean A4 transcript in the house style.
pub fn to_pdf(t: &Transcript, title: &str, subtitle: &str, timestamps: bool) -> anyhow::Result<Vec<u8>> {
    use krilla::color::rgb;
    use krilla::geom::Point;
    use krilla::num::NormalizedF32;
    use krilla::page::PageSettings;
    use krilla::paint::Fill;
    use krilla::text::TextDirection;

    let paras = paragraphs(t, 1.5, 700);
    let all: String = std::iter::once(title.to_string()).chain(paras.iter().map(|p| p.text.clone())).collect::<Vec<_>>().join(" ");
    let (face, bold) = pick_faces(&all);
    let (pw, ph) = (595.0_f32, 842.0_f32);
    let (ml, mr, mt, mb) = (72.0_f32, 72.0_f32, 80.0_f32, 80.0_f32);
    let width = pw - ml - mr;
    let (size, lead) = (11.0_f32, 15.0_f32);
    let grey = |v: u8| Fill { paint: rgb::Color::new(v, v, v).into(), opacity: NormalizedF32::ONE, rule: Default::default() };

    // Lay out first: (page, y, x, text, font size, bold, grey level).
    struct Run {
        page: usize,
        x: f32,
        y: f32,
        text: String,
        size: f32,
        bold: bool,
        grey: u8,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut page = 0;
    let mut y = mt;
    let newline = |y: &mut f32, page: &mut usize, h: f32| {
        if *y + h > ph - mb {
            *page += 1;
            *y = mt;
        }
        *y += h;
    };
    for line in break_lines(&bold, title, 20.0, width) {
        newline(&mut y, &mut page, 26.0);
        runs.push(Run { page, x: ml, y, text: line.join(" "), size: 20.0, bold: true, grey: 0 });
    }
    newline(&mut y, &mut page, 16.0);
    runs.push(Run { page, x: ml, y, text: subtitle.to_string(), size: 9.0, bold: false, grey: 110 });
    y += 18.0;
    for p in &paras {
        if timestamps {
            newline(&mut y, &mut page, 13.0);
            runs.push(Run { page, x: ml, y, text: stamp(p.start), size: 8.0, bold: false, grey: 130 });
        }
        let lines = break_lines(&face, &p.text, size, width);
        let n = lines.len();
        for (i, words) in lines.into_iter().enumerate() {
            newline(&mut y, &mut page, lead);
            if words.is_empty() {
                continue;
            }
            // Justify every line but the last of a paragraph.
            let natural: f32 = words.iter().map(|w| face.width(w, size)).sum();
            let gaps = words.len().saturating_sub(1) as f32;
            let space = face.width(" ", size);
            let extra = if i + 1 < n && gaps > 0.0 { ((width - natural) / gaps).clamp(space * 0.6, space * 3.0) } else { space };
            let mut x = ml;
            for w in words {
                let ww = face.width(&w, size);
                runs.push(Run { page, x, y, text: w, size, bold: false, grey: 20 });
                x += ww + extra;
            }
        }
        y += 7.0;
    }

    let mut doc = krilla::Document::new();
    doc.set_metadata(
        krilla::metadata::Metadata::new()
            .title(title.to_string())
            .creator("Castor".to_string())
            .language(if t.language.is_empty() { "en".into() } else { t.language.clone() }),
    );
    let pages = page + 1;
    for pg in 0..pages {
        let mut page = doc.start_page_with(PageSettings::from_wh(pw, ph).ok_or_else(|| anyhow::anyhow!("page size"))?);
        let mut s = page.surface();
        for r in runs.iter().filter(|r| r.page == pg) {
            s.set_fill(Some(grey(r.grey)));
            let f = if r.bold { bold.font.clone() } else { face.font.clone() };
            s.draw_text(Point::from_xy(r.x, r.y), f, r.size, &r.text, false, TextDirection::Auto);
        }
        // Folio
        s.set_fill(Some(grey(120)));
        let folio = format!("{} / {}", pg + 1, pages);
        let fw = face.width(&folio, 8.0);
        s.draw_text(Point::from_xy((pw - fw) / 2.0, ph - 40.0), face.font.clone(), 8.0, &folio, false, TextDirection::Auto);
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| anyhow::anyhow!("PDF: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_media::whisper::{Segment, Word};

    fn sample() -> Transcript {
        let seg = |s: f64, e: f64, t: &str| Segment {
            start: s,
            end: e,
            text: t.into(),
            words: t.split(' ').enumerate().map(|(i, w)| Word { start: s + i as f64 * 0.3, end: s + i as f64 * 0.3 + 0.25, text: w.into(), p: 0.9 }).collect(),
        };
        Transcript {
            language: "fr".into(),
            duration: 20.0,
            segments: vec![seg(0.0, 3.0, "Bonjour à tous & bienvenue."), seg(3.1, 6.0, "Voici l'été."), seg(10.0, 12.0, "Nouveau paragraphe <ici>.")],
        }
    }

    #[test]
    fn paragraphs_split_on_pause() {
        let p = paragraphs(&sample(), 1.5, 700);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].text, "Bonjour à tous & bienvenue. Voici l'été.");
        assert!(to_txt(&sample(), true).starts_with("[00:00:00] Bonjour"));
    }

    #[test]
    fn docx_is_a_zip_with_escaped_xml() {
        let d = to_docx(&sample(), "Test", "fr · 20 s", true).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(d)).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
        assert!(s.contains("&amp; bienvenue") && s.contains("&lt;ici&gt;"));
    }

    #[test]
    fn pdf_renders() {
        let p = to_pdf(&sample(), "Interview", "French · 20 s", true).unwrap();
        assert!(p.starts_with(b"%PDF"));
        assert!(p.len() > 1000);
    }
}
