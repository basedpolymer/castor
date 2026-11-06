//! `files.metadata`: see, edit or strip the common metadata of documents and media — PDF info
//! (engine-pdf / MuPDF), audio tags (lofty: ID3, Vorbis comments, MP4, APE…), video tags
//! (ffmpeg, stream copy), photo EXIF/XMP (read; stripped losslessly for JPEG, PNG and WebP).
//! Edits and strips write a new file; the original is never touched.

use std::path::Path;

use anyhow::{Result, anyhow, bail};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};
use serde_json::{Map, Value as Json, json};

use crate::common::{self, Cell};

pub struct Metadata {
    meta: ToolMeta,
}

impl Metadata {
    pub fn new() -> Self {
        Metadata {
            meta: common::meta(
                "files.metadata",
                InputSpec::kinds(&[FileKind::Pdf, FileKind::Image, FileKind::Raw, FileKind::Audio, FileKind::Video]),
                "TXT · JSON · same format",
                &["metadata", "tags", "exif", "id3", "properties", "author", "title", "strip", "privacy", "gps"],
                "meta",
                false,
            ),
        }
    }
}

impl Tool for Metadata {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let edit = Cond::Eq("action", "edit");
        vec![
            OptionSpec::choice("action", ["view", "edit", "strip"].map(Choice::key).to_vec(), "view"),
            OptionSpec::choice("report", vec![Choice::lit("txt", "TXT"), Choice::lit("json", "JSON")], "txt").when(Cond::Eq("action", "view")),
            OptionSpec::text("title", "").when(edit.clone()),
            OptionSpec::text("author", "").when(edit.clone()),
            OptionSpec::text("subject", "").when(edit.clone()),
            OptionSpec::text("keywords", "").when(edit.clone()),
            OptionSpec::text("year", "").when(edit.clone()),
            OptionSpec::text("comment", "").when(edit.clone()),
            OptionSpec::text("copyright", "").when(edit),
        ]
    }

    fn preview(&self, input: Option<&FileInfo>, _v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let m = read(f)?;
        let rows: Vec<Vec<Cell>> = m.iter().map(|(k, v)| vec![(k.clone(), None), (v.as_str().map(String::from).unwrap_or_else(|| v.to_string()), None)]).collect();
        let png = common::table_png(&tr_args("files.metadata.caption", &[("name", &f.name())]), &[&tr("files.metadata.col.field"), &tr("files.metadata.col.value")], &rows, max_side, 0)?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let f = job.input()?;
        match v.str("action") {
            "edit" => {
                let fields = fields(v);
                anyhow::ensure!(!fields.is_empty(), "{}", tr("files.metadata.err.nothing"));
                let out = cx.output_path(Some(f), &f.extension());
                write(f, &out, Some(&fields), cx)?;
                Ok(vec![Output::new(out).with_note(fields.iter().map(|(k, _)| tr(&format!("files.metadata.opt.{k}"))).collect::<Vec<_>>().join(", "))])
            }
            "strip" => {
                let out = cx.output_path(Some(f), &f.extension());
                let before = read(f).map(|m| m.len()).unwrap_or(0);
                write(f, &out, None, cx)?;
                let after = read(&FileInfo::from_path(&out)).map(|m| m.len()).unwrap_or(0);
                Ok(vec![Output::new(out).with_note(tr_args("files.metadata.stripped", &[("before", &before.to_string()), ("after", &after.to_string())]))])
            }
            _ => {
                let m = read(f)?;
                let fmt = v.str("report");
                let out = cx.output_named(&format!("{}-metadata.{fmt}", f.stem()));
                let count = m.len();
                let text = if fmt == "json" {
                    serde_json::to_string_pretty(&Json::Object(m))?
                } else {
                    let w = m.keys().map(|k| k.chars().count()).max().unwrap_or(0);
                    m.iter().map(|(k, v)| format!("{k:<w$}  {}\n", v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect()
                };
                std::fs::write(&out, text)?;
                Ok(vec![Output::new(out).with_note(tr_args("files.metadata.fields", &[("count", &count.to_string())]))])
            }
        }
    }
}

fn fields(v: &Values) -> Vec<(&'static str, String)> {
    ["title", "author", "subject", "keywords", "year", "comment", "copyright"]
        .into_iter()
        .filter_map(|k| {
            let s = v.str(k).trim();
            (!s.is_empty()).then(|| (k, s.to_string()))
        })
        .collect()
}

fn kind_of(f: &FileInfo) -> FileKind {
    match f.kind {
        FileKind::Other => FileKind::from_extension(&f.extension()),
        k => k,
    }
}

/// Every metadata field we can read, as "Group: Name" → value.
pub fn read(f: &FileInfo) -> Result<Map<String, Json>> {
    let mut m = Map::new();
    match kind_of(f) {
        FileKind::Pdf => {
            let doc = engine_pdf::open_pdf(&f.path, "")?;
            for k in ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate"] {
                let v = engine_pdf::get_info(&doc, k);
                if !v.is_empty() {
                    m.insert(format!("PDF: {k}"), json!(v));
                }
            }
            m.insert("PDF: Pages".into(), json!(doc.page_count()?));
        }
        FileKind::Audio => audio_read(&f.path, &mut m)?,
        FileKind::Video => video_read(&f.path, &mut m)?,
        _ => image_read(&f.path, &mut m)?,
    }
    Ok(m)
}

fn image_read(p: &Path, m: &mut Map<String, Json>) -> Result<()> {
    let file = std::fs::File::open(p)?;
    match exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
        Ok(ex) => {
            for field in ex.fields() {
                let name = format!("EXIF: {}", field.tag);
                let mut val = field.display_value().with_unit(&ex).to_string();
                val.truncate(300);
                m.entry(name).or_insert(json!(val));
            }
        }
        Err(exif::Error::NotFound(_)) => {}
        Err(e) => m.insert("EXIF".into(), json!(format!("{e}"))).map(|_| ()).unwrap_or(()),
    }
    // XMP packet, if any (shown raw, trimmed).
    let data = std::fs::read(p)?;
    if let Some(i) = find(&data, b"<x:xmpmeta") {
        let end = find(&data[i..], b"</x:xmpmeta>").map(|e| i + e + 12).unwrap_or((i + 2000).min(data.len()));
        let xmp = String::from_utf8_lossy(&data[i..end]).chars().take(2000).collect::<String>();
        m.insert("XMP".into(), json!(xmp));
    }
    Ok(())
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn audio_read(p: &Path, m: &mut Map<String, Json>) -> Result<()> {
    use lofty::prelude::*;
    let tf = lofty::read_from_path(p).map_err(|e| anyhow!("{}: {e}", tr("files.metadata.err.read")))?;
    let pr = tf.properties();
    m.insert("Audio: Duration".into(), json!(castor_core::units::duration(pr.duration().as_secs_f64())));
    if let Some(b) = pr.overall_bitrate() {
        m.insert("Audio: Bitrate".into(), json!(format!("{b} kbps")));
    }
    if let Some(r) = pr.sample_rate() {
        m.insert("Audio: Sample rate".into(), json!(format!("{r} Hz")));
    }
    for tag in tf.tags() {
        for item in tag.items() {
            if let Some(t) = item.value().text() {
                m.insert(format!("{:?}: {:?}", tag.tag_type(), item.key()), json!(t.chars().take(300).collect::<String>()));
            }
        }
        if !tag.pictures().is_empty() {
            m.insert(format!("{:?}: Pictures", tag.tag_type()), json!(tag.pictures().len()));
        }
    }
    Ok(())
}

fn video_read(p: &Path, m: &mut Map<String, Json>) -> Result<()> {
    let probe = engine_media::ffprobe()?;
    let mut cmd = castor_core::process::command(&probe);
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(p);
    let out = cmd.output().map_err(|e| anyhow!("ffprobe: {e}"))?;
    anyhow::ensure!(out.status.success(), "{}", tr("files.metadata.err.read"));
    let j: Json = serde_json::from_slice(&out.stdout)?;
    if let Some(fmt) = j.get("format") {
        if let Some(d) = fmt.get("duration").and_then(Json::as_str).and_then(|s| s.parse::<f64>().ok()) {
            m.insert("Video: Duration".into(), json!(castor_core::units::duration(d)));
        }
        if let Some(t) = fmt.get("tags").and_then(Json::as_object) {
            for (k, v) in t {
                m.insert(format!("Tag: {k}"), v.clone());
            }
        }
    }
    for (i, s) in j.get("streams").and_then(Json::as_array).into_iter().flatten().enumerate() {
        if let Some(t) = s.get("tags").and_then(Json::as_object) {
            for (k, v) in t {
                if !matches!(k.as_str(), "handler_name" | "vendor_id" | "encoder") {
                    m.insert(format!("Stream {i}: {k}"), v.clone());
                }
            }
        }
    }
    Ok(())
}

/// Copy `f` to `out` with new metadata (`Some`) or none at all (`None`).
fn write(f: &FileInfo, out: &Path, fields: Option<&[(&'static str, String)]>, cx: &RunCx) -> Result<()> {
    match kind_of(f) {
        FileKind::Pdf => {
            let mut doc = engine_pdf::open_pdf(&f.path, "")?;
            match fields {
                Some(fs) => {
                    for (k, v) in fs {
                        let key = match *k {
                            "title" => "Title",
                            "author" => "Author",
                            "subject" | "comment" => "Subject",
                            "keywords" => "Keywords",
                            _ => continue,
                        };
                        engine_pdf::set_info(&mut doc, key, v)?;
                    }
                }
                None => {
                    for k in ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate", "Trapped"] {
                        engine_pdf::set_info(&mut doc, k, "")?;
                    }
                    let mut cat = doc.catalog()?;
                    cat.dict_delete("Metadata")?;
                    cat.dict_delete("PieceInfo")?;
                }
            }
            engine_pdf::save_pdf(&doc, out, &engine_pdf::Save::default())?;
        }
        FileKind::Audio => {
            use lofty::prelude::*;
            std::fs::copy(&f.path, out)?;
            let opts = lofty::config::WriteOptions::default();
            let mut tf = lofty::read_from_path(out).map_err(|e| anyhow!("{}: {e}", tr("files.metadata.err.read")))?;
            match fields {
                None => {
                    let types: Vec<_> = tf.tags().iter().map(|t| t.tag_type()).collect();
                    for t in types {
                        t.remove_from_path(out, opts).map_err(|e| anyhow!("{e}"))?;
                    }
                }
                Some(fs) => {
                    if tf.primary_tag().is_none() {
                        tf.insert_tag(lofty::tag::Tag::new(tf.primary_tag_type()));
                    }
                    let tag = tf.primary_tag_mut().ok_or_else(|| anyhow!("{}", tr("files.metadata.err.unsupported")))?;
                    for (k, v) in fs {
                        match *k {
                            "title" => tag.set_title(v.clone()),
                            "author" => tag.set_artist(v.clone()),
                            "subject" => tag.set_album(v.clone()),
                            "comment" => tag.set_comment(v.clone()),
                            "keywords" => tag.set_genre(v.clone()),
                            "year" => {
                                tag.insert_text(ItemKey::RecordingDate, v.clone());
                            }
                            "copyright" => {
                                tag.insert_text(ItemKey::CopyrightMessage, v.clone());
                            }
                            _ => {}
                        }
                    }
                    tf.save_to_path(out, opts).map_err(|e| anyhow!("{e}"))?;
                }
            }
        }
        FileKind::Video => {
            let mut ff = engine_media::Ffmpeg::new().input(&f.path).args(["-map", "0", "-c", "copy"]);
            match fields {
                None => ff = ff.args(["-map_metadata", "-1", "-map_chapters", "-1"]),
                Some(fs) => {
                    for (k, v) in fs {
                        let key = match *k {
                            "author" => "artist",
                            "subject" => "album",
                            "keywords" => "genre",
                            "year" => "date",
                            other => other,
                        };
                        ff = ff.arg("-metadata").arg(format!("{key}={v}"));
                    }
                }
            }
            let dur = engine_media::probe(&f.path).map(|p| p.duration).unwrap_or(0.0);
            ff.arg(out).duration(dur).run(cx)?;
        }
        _ => {
            if fields.is_some() {
                bail!("{}", tr("files.metadata.err.image_edit"));
            }
            let data = std::fs::read(&f.path)?;
            let clean = strip_image(&data)?;
            common_write(out, &clean)?;
        }
    }
    Ok(())
}

fn common_write(p: &Path, d: &[u8]) -> Result<()> {
    std::fs::write(p, d).map_err(|e| anyhow!("{}: {e}", p.display()))
}

/// Lossless metadata removal for JPEG, PNG and WebP (pixels untouched). JPEG keeps a minimal
/// EXIF with the orientation so photos stay upright, and the ICC profile so colours stay right.
pub fn strip_image(d: &[u8]) -> Result<Vec<u8>> {
    if d.starts_with(&[0xFF, 0xD8]) {
        return strip_jpeg(d);
    }
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        return strip_png(d);
    }
    if d.len() > 12 && &d[0..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        return strip_webp(d);
    }
    bail!("{}", tr("files.metadata.err.image_format"))
}

fn jpeg_orientation(d: &[u8]) -> u16 {
    exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(d))
        .ok()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .unwrap_or(1) as u16
}

fn strip_jpeg(d: &[u8]) -> Result<Vec<u8>> {
    let orient = jpeg_orientation(d);
    let mut out = vec![0xFF, 0xD8];
    if orient != 1 {
        // APP1 "Exif\0\0" + big-endian TIFF with one IFD entry: Orientation.
        let mut tiff = b"MM\0*\0\0\0\x08".to_vec();
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&0x0112u16.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        tiff.extend_from_slice(&orient.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes());
        let len = (2 + 6 + tiff.len()) as u16;
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(&tiff);
    }
    let mut i = 2;
    while i + 4 <= d.len() {
        anyhow::ensure!(d[i] == 0xFF, "{}", tr("files.metadata.err.corrupt"));
        let marker = d[i + 1];
        if marker == 0xD9 {
            out.extend_from_slice(&d[i..i + 2]);
            break;
        }
        if marker == 0xDA {
            // Start of scan: the rest is image data.
            out.extend_from_slice(&d[i..]);
            return Ok(out);
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&d[i..i + 2]);
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        anyhow::ensure!(i + 2 + len <= d.len(), "{}", tr("files.metadata.err.corrupt"));
        let seg = &d[i..i + 2 + len];
        let keep = match marker {
            0xE0 => true,                                       // JFIF
            0xE2 => seg.get(4..15) == Some(b"ICC_PROFILE"),     // colour profile
            0xEE => seg.get(4..9) == Some(b"Adobe"),            // colour transform flag
            0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,          // EXIF, XMP, IPTC, comments…
            _ => true,
        };
        if keep {
            out.extend_from_slice(seg);
        }
        i += 2 + len;
    }
    Ok(out)
}

fn strip_png(d: &[u8]) -> Result<Vec<u8>> {
    let mut out = d[..8].to_vec();
    let mut i = 8;
    while i + 12 <= d.len() {
        let len = u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as usize;
        let typ = &d[i + 4..i + 8];
        let end = i + 12 + len;
        anyhow::ensure!(end <= d.len(), "{}", tr("files.metadata.err.corrupt"));
        if !matches!(typ, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME") {
            out.extend_from_slice(&d[i..end]);
        }
        i = end;
    }
    Ok(out)
}

fn strip_webp(d: &[u8]) -> Result<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut i = 12;
    while i + 8 <= d.len() {
        let len = u32::from_le_bytes([d[i + 4], d[i + 5], d[i + 6], d[i + 7]]) as usize;
        let end = (i + 8 + len + (len & 1)).min(d.len());
        let typ = &d[i..i + 4];
        if typ != b"EXIF" && typ != b"XMP " {
            let mut c = d[i..end].to_vec();
            if typ == b"VP8X" && c.len() > 8 {
                c[8] &= !(0x08 | 0x04); // EXIF and XMP flags
            }
            chunks.extend_from_slice(&c);
        }
        i = end;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunks);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_strip_keeps_pixels() {
        let mut img = Vec::new();
        image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]))
            .write_to(&mut std::io::Cursor::new(&mut img), image::ImageFormat::Png)
            .unwrap();
        // Add a tEXt chunk before IEND.
        let iend = img.len() - 12;
        let mut chunk = Vec::new();
        let body = b"Author\0Someone";
        chunk.extend_from_slice(&(body.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(body);
        let mut crc = flate2::Crc::new();
        crc.update(b"tEXt");
        crc.update(body);
        chunk.extend_from_slice(&crc.sum().to_be_bytes());
        img.splice(iend..iend, chunk);
        let clean = strip_image(&img).unwrap();
        assert!(find(&clean, b"Someone").is_none());
        let a = image::load_from_memory(&img).unwrap().to_rgb8();
        let b = image::load_from_memory(&clean).unwrap().to_rgb8();
        assert_eq!(a, b);
    }
}
