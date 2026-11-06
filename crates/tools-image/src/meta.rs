//! Metadata without recompression: embed / strip EXIF, XMP, IPTC and ICC in JPEG, PNG and WebP
//! containers (img-parts), build XMP packets and IPTC records.

use anyhow::{Result, bail};
use img_parts::jpeg::{Jpeg, JpegSegment, markers};
use img_parts::png::{Png, PngChunk};
use img_parts::riff::{RiffChunk, RiffContent};
use img_parts::webp::WebP;
use img_parts::{Bytes, ImageEXIF, ImageICC};

use crate::codec::Fmt;

const XMP_NS: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXT_NS: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

fn write_jpeg(j: Jpeg) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    j.encoder().write_to(&mut out)?;
    Ok(out)
}
fn write_png(p: Png) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    p.encoder().write_to(&mut out)?;
    Ok(out)
}
fn write_webp(w: WebP) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    w.encoder().write_to(&mut out)?;
    Ok(out)
}

/// Position just before the first SOF/DHT/DQT/SOS segment (after APPn segments).
fn jpeg_insert_pos(j: &Jpeg) -> usize {
    j.segments().iter().position(|s| !(0xE0..=0xEF).contains(&s.marker()) && s.marker() != markers::COM).unwrap_or(0)
}

/// Embed ICC / EXIF / XMP into an encoded JPEG, PNG or WebP (replacing existing ones).
pub fn embed(bytes: Vec<u8>, fmt: Fmt, icc: Option<&[u8]>, exif: Option<&[u8]>, xmp: Option<&[u8]>) -> Result<Vec<u8>> {
    let b = Bytes::from(bytes);
    match fmt {
        Fmt::Jpeg => {
            let mut j = Jpeg::from_bytes(b)?;
            if let Some(icc) = icc {
                j.set_icc_profile(Some(Bytes::copy_from_slice(icc)));
            }
            if let Some(exif) = exif {
                j.set_exif(Some(Bytes::copy_from_slice(exif)));
            }
            if let Some(xmp) = xmp {
                set_jpeg_xmp(&mut j, Some(xmp));
            }
            write_jpeg(j)
        }
        Fmt::Png => {
            let mut p = Png::from_bytes(b)?;
            if let Some(icc) = icc {
                p.set_icc_profile(Some(Bytes::copy_from_slice(icc)));
            }
            if let Some(exif) = exif {
                p.set_exif(Some(Bytes::copy_from_slice(exif)));
            }
            if let Some(xmp) = xmp {
                set_png_xmp(&mut p, Some(xmp));
            }
            write_png(p)
        }
        Fmt::Webp => {
            let mut w = WebP::from_bytes(b)?;
            if let Some(icc) = icc {
                w.set_icc_profile(Some(Bytes::copy_from_slice(icc)));
            }
            if let Some(xmp) = xmp {
                w.remove_chunks_by_id(*b"XMP ");
                w.chunks_mut().push(RiffChunk::new(*b"XMP ", RiffContent::Data(Bytes::copy_from_slice(xmp))));
            }
            // set_exif also recomputes the VP8X flags (ICC/EXIF/XMP/alpha).
            let e = exif.map(Bytes::copy_from_slice).or_else(|| w.exif());
            w.set_exif(e);
            write_webp(w)
        }
        _ => bail!("cannot embed metadata in {}", fmt.ext()),
    }
}

fn set_jpeg_xmp(j: &mut Jpeg, xmp: Option<&[u8]>) {
    j.segments_mut().retain(|s| {
        !(s.marker() == markers::APP1 && (s.contents().starts_with(XMP_NS) || s.contents().starts_with(XMP_EXT_NS)))
    });
    if let Some(xmp) = xmp {
        let mut c = XMP_NS.to_vec();
        c.extend_from_slice(xmp);
        if c.len() <= 65_533 {
            // After the EXIF segment if there is one.
            let pos = j
                .segments()
                .iter()
                .position(|s| s.marker() == markers::APP1 && s.contents().starts_with(b"Exif\0\0"))
                .map(|p| p + 1)
                .unwrap_or_else(|| jpeg_insert_pos(j).min(1));
            j.segments_mut().insert(pos, JpegSegment::new_with_contents(markers::APP1, Bytes::from(c)));
        }
    }
}

fn set_png_xmp(p: &mut Png, xmp: Option<&[u8]>) {
    p.chunks_mut().retain(|c| !(c.kind() == *b"iTXt" && c.contents().starts_with(b"XML:com.adobe.xmp\0")));
    if let Some(xmp) = xmp {
        // keyword \0 compression-flag compression-method language \0 translated \0 text
        let mut c = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        c.extend_from_slice(xmp);
        let pos = p.chunks().len().min(1);
        p.chunks_mut().insert(pos, PngChunk::new(*b"iTXt", Bytes::from(c)));
    }
}

/// The XMP packet of a JPEG, PNG or WebP file, if any.
pub fn read_xmp(data: &[u8], fmt: Fmt) -> Option<Vec<u8>> {
    let b = Bytes::copy_from_slice(data);
    match fmt {
        Fmt::Jpeg => Jpeg::from_bytes(b).ok()?.segments().iter().find_map(|s| {
            (s.marker() == markers::APP1 && s.contents().starts_with(XMP_NS)).then(|| s.contents()[XMP_NS.len()..].to_vec())
        }),
        Fmt::Png => Png::from_bytes(b).ok()?.chunks().iter().find_map(|c| {
            let k = b"XML:com.adobe.xmp\0";
            if c.kind() != *b"iTXt" || !c.contents().starts_with(k) {
                return None;
            }
            // keyword\0 flag method lang\0 translated\0 text (uncompressed only)
            let rest = &c.contents()[k.len()..];
            if rest.first() != Some(&0) {
                return None;
            }
            let rest = &rest[2..];
            let lang_end = rest.iter().position(|b| *b == 0)?;
            let rest = &rest[lang_end + 1..];
            let tr_end = rest.iter().position(|b| *b == 0)?;
            Some(rest[tr_end + 1..].to_vec())
        }),
        Fmt::Webp => WebP::from_bytes(b).ok()?.chunk_by_id(*b"XMP ").and_then(|c| c.content().data()).map(|d| d.to_vec()),
        _ => None,
    }
}

/// Insert an IPTC-IIM record block (APP13 "Photoshop 3.0") into a JPEG.
pub fn set_jpeg_iptc(bytes: Vec<u8>, iim: &[u8]) -> Result<Vec<u8>> {
    let mut j = Jpeg::from_bytes(Bytes::from(bytes))?;
    j.remove_segments_by_marker(markers::APP13);
    let mut c = b"Photoshop 3.0\0".to_vec();
    c.extend_from_slice(b"8BIM");
    c.extend_from_slice(&0x0404u16.to_be_bytes());
    c.extend_from_slice(&[0, 0]); // empty Pascal name, padded to even
    c.extend_from_slice(&(iim.len() as u32).to_be_bytes());
    c.extend_from_slice(iim);
    if iim.len() % 2 == 1 {
        c.push(0);
    }
    let pos = jpeg_insert_pos(&j);
    j.segments_mut().insert(pos, JpegSegment::new_with_contents(markers::APP13, Bytes::from(c)));
    write_jpeg(j)
}

/// IPTC-IIM records (record 2). `fields`: (dataset number, value); UTF-8 declared.
pub fn iptc_records(fields: &[(u8, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    // 1:90 coded character set = UTF-8 (ESC % G)
    out.extend_from_slice(&[0x1C, 1, 90, 0, 3, 0x1B, 0x25, 0x47]);
    for (ds, v) in fields {
        let b = v.as_bytes();
        let b = &b[..b.len().min(2000)];
        out.extend_from_slice(&[0x1C, 2, *ds]);
        out.extend_from_slice(&(b.len() as u16).to_be_bytes());
        out.extend_from_slice(b);
    }
    out
}

/// What to keep when stripping.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keep {
    /// Colour profile (changes how the image looks if removed: keep by default).
    pub icc: bool,
    /// Orientation (rewritten as a minimal EXIF holding only this tag).
    pub orientation: bool,
    /// Copyright / author (XMP dc:rights / dc:creator, EXIF Copyright/Artist are rewritten).
    pub copyright: bool,
}

/// Summary of what was removed, for the result note.
#[derive(Debug, Default, Clone)]
pub struct Stripped {
    pub exif: bool,
    pub gps: bool,
    pub xmp: bool,
    pub iptc: bool,
    pub comments: bool,
    pub icc: bool,
    pub other: usize,
}

impl Stripped {
    pub fn any(&self) -> bool {
        self.exif || self.xmp || self.iptc || self.comments || self.icc || self.other > 0
    }
}

/// Remove metadata from an encoded JPEG / PNG / WebP without touching the pixels.
pub fn strip(bytes: Vec<u8>, fmt: Fmt, keep: Keep) -> Result<(Vec<u8>, Stripped)> {
    let mut s = Stripped::default();
    let b = Bytes::from(bytes);
    match fmt {
        Fmt::Jpeg => {
            let mut j = Jpeg::from_bytes(b)?;
            let exif = j.exif();
            let orientation = exif.as_deref().and_then(crate::codec::exif_orientation);
            let copyright = if keep.copyright { exif.as_deref().map(exif_copyright).unwrap_or_default() } else { (None, None) };
            s.gps = exif.as_deref().map(has_gps).unwrap_or(false);
            let mut kept_xmp = None;
            if keep.copyright {
                kept_xmp = jpeg_xmp(&j).and_then(|x| rights_only_xmp(&x));
            }
            let before = j.segments().len();
            j.segments_mut().retain(|seg| {
                let m = seg.marker();
                let c = seg.contents();
                match m {
                    markers::APP1 if c.starts_with(b"Exif\0\0") => {
                        s.exif = true;
                        false
                    }
                    markers::APP1 if c.starts_with(XMP_NS) || c.starts_with(XMP_EXT_NS) => {
                        s.xmp = true;
                        false
                    }
                    markers::APP2 if c.starts_with(b"ICC_PROFILE\0") => {
                        if keep.icc {
                            true
                        } else {
                            s.icc = true;
                            false
                        }
                    }
                    markers::APP13 => {
                        s.iptc = true;
                        false
                    }
                    markers::COM => {
                        s.comments = true;
                        false
                    }
                    // JFIF (APP0) and Adobe (APP14, colour transform) are needed for decoding.
                    markers::APP0 | markers::APP14 => true,
                    // MPF (APP2 "MPF\0") holds pointers to embedded previews/depth maps.
                    0xE1..=0xEF => {
                        s.other += 1;
                        false
                    }
                    _ => true,
                }
            });
            let _ = before;
            let minimal = minimal_exif(if keep.orientation { orientation } else { None }, copyright.0.as_deref(), copyright.1.as_deref());
            if let Some(e) = minimal {
                j.set_exif(Some(Bytes::from(e)));
            }
            if let Some(x) = kept_xmp {
                set_jpeg_xmp(&mut j, Some(x.as_bytes()));
            }
            Ok((write_jpeg(j)?, s))
        }
        Fmt::Png => {
            let mut p = Png::from_bytes(b)?;
            let exif = p.exif();
            let orientation = exif.as_deref().and_then(crate::codec::exif_orientation);
            s.gps = exif.as_deref().map(has_gps).unwrap_or(false);
            p.chunks_mut().retain(|c| match &c.kind() {
                b"eXIf" => {
                    s.exif = true;
                    false
                }
                b"iTXt" | b"tEXt" | b"zTXt" => {
                    if c.contents().starts_with(b"XML:com.adobe.xmp") {
                        s.xmp = true;
                    } else if c.contents().starts_with(b"Raw profile type") {
                        s.exif = true;
                    } else {
                        s.comments = true;
                    }
                    false
                }
                b"iCCP" => {
                    if keep.icc {
                        true
                    } else {
                        s.icc = true;
                        false
                    }
                }
                b"tIME" | b"caBX" => {
                    s.other += 1;
                    false
                }
                _ => true,
            });
            if let Some(e) = minimal_exif(if keep.orientation { orientation } else { None }, None, None) {
                p.set_exif(Some(Bytes::from(e)));
            }
            Ok((write_png(p)?, s))
        }
        Fmt::Webp => {
            let mut w = WebP::from_bytes(b)?;
            let exif = w.exif();
            let orientation = exif.as_deref().and_then(crate::codec::exif_orientation);
            s.gps = exif.as_deref().map(has_gps).unwrap_or(false);
            s.exif = exif.is_some();
            s.xmp = w.has_chunk(*b"XMP ");
            w.remove_chunks_by_id(*b"XMP ");
            if !keep.icc && w.has_chunk(*b"ICCP") {
                s.icc = true;
                w.set_icc_profile(None);
            }
            w.set_exif(minimal_exif(if keep.orientation { orientation } else { None }, None, None).map(Bytes::from));
            Ok((write_webp(w)?, s))
        }
        _ => bail!("cannot strip metadata of {} without re-encoding", fmt.ext()),
    }
}

fn jpeg_xmp(j: &Jpeg) -> Option<String> {
    j.segments()
        .iter()
        .find(|s| s.marker() == markers::APP1 && s.contents().starts_with(XMP_NS))
        .map(|s| String::from_utf8_lossy(&s.contents()[XMP_NS.len()..]).into_owned())
}

/// Keep only dc:rights and dc:creator of an XMP packet (rebuilt).
fn rights_only_xmp(x: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(x).ok()?;
    let dc = "http://purl.org/dc/elements/1.1/";
    let text_of = |name: &str| -> Option<String> {
        let n = doc.descendants().find(|n| n.tag_name().namespace() == Some(dc) && n.tag_name().name() == name)?;
        let t: Vec<String> = n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        if t.is_empty() { None } else { Some(t.join("; ")) }
    };
    let rights = text_of("rights");
    let creator = text_of("creator");
    if rights.is_none() && creator.is_none() {
        return None;
    }
    Some(String::from_utf8_lossy(&build_xmp(&XmpFields { rights, creator, ..Default::default() })).into_owned())
}

fn has_gps(exif: &[u8]) -> bool {
    exif::Reader::new()
        .read_raw(exif.to_vec())
        .map(|r| r.fields().any(|f| f.ifd_num == exif::In::PRIMARY && f.tag.context() == exif::Context::Gps))
        .unwrap_or(false)
}

fn exif_copyright(exif: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(r) = exif::Reader::new().read_raw(exif.to_vec()) else { return (None, None) };
    let get = |t: exif::Tag| {
        r.get_field(t, exif::In::PRIMARY).and_then(|f| match &f.value {
            exif::Value::Ascii(v) => v.first().map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string()),
            _ => None,
        })
    };
    (get(exif::Tag::Artist).filter(|s| !s.is_empty()), get(exif::Tag::Copyright).filter(|s| !s.is_empty()))
}

/// A minimal little-endian EXIF block (TIFF structure) with Orientation / Artist / Copyright.
pub fn minimal_exif(orientation: Option<u8>, artist: Option<&str>, copyright: Option<&str>) -> Option<Vec<u8>> {
    let orientation = orientation.filter(|o| *o > 1);
    let mut entries: Vec<(u16, u16, Vec<u8>)> = Vec::new(); // tag, type, value bytes
    if let Some(o) = orientation {
        entries.push((0x0112, 3, (o as u16).to_le_bytes().to_vec()));
    }
    if let Some(a) = artist {
        let mut v = a.as_bytes().to_vec();
        v.push(0);
        entries.push((0x013B, 2, v));
    }
    if let Some(c) = copyright {
        let mut v = c.as_bytes().to_vec();
        v.push(0);
        entries.push((0x8298, 2, v));
    }
    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|e| e.0);
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    let n = entries.len();
    out.extend_from_slice(&(n as u16).to_le_bytes());
    let mut data_off = 8 + 2 + n * 12 + 4;
    let mut data = Vec::new();
    for (tag, ty, v) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        let count = if *ty == 3 { 1u32 } else { v.len() as u32 };
        out.extend_from_slice(&count.to_le_bytes());
        if v.len() <= 4 {
            let mut f = v.clone();
            f.resize(4, 0);
            out.extend_from_slice(&f);
        } else {
            out.extend_from_slice(&(data_off as u32).to_le_bytes());
            data.extend_from_slice(v);
            if v.len() % 2 == 1 {
                data.push(0);
            }
            data_off = 8 + 2 + n * 12 + 4 + data.len();
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
    Some(out)
}

/// Fields written by `image.seo`.
#[derive(Clone, Debug, Default)]
pub struct XmpFields {
    pub title: Option<String>,
    pub description: Option<String>,
    pub alt: Option<String>,
    pub keywords: Vec<String>,
    pub creator: Option<String>,
    pub rights: Option<String>,
    /// Decimal degrees.
    pub gps: Option<(f64, f64)>,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn gps_xmp(v: f64, pos: char, neg: char) -> String {
    let a = v.abs();
    let deg = a.trunc();
    let min = (a - deg) * 60.0;
    format!("{},{:.6}{}", deg as i64, min, if v >= 0.0 { pos } else { neg })
}

/// An XMP packet (dc, Iptc4xmpCore alt text, exif GPS, xmp CreatorTool = Castor).
pub fn build_xmp(f: &XmpFields) -> Vec<u8> {
    let mut d = String::new();
    let alt = |name: &str, v: &str| {
        format!("   <{name}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{name}>\n", esc(v))
    };
    if let Some(t) = &f.title {
        d += &alt("dc:title", t);
    }
    if let Some(t) = &f.description {
        d += &alt("dc:description", t);
    }
    if let Some(t) = &f.rights {
        d += &alt("dc:rights", t);
    }
    if let Some(c) = &f.creator {
        d += &format!("   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n", esc(c));
    }
    if !f.keywords.is_empty() {
        d += "   <dc:subject><rdf:Bag>";
        for k in &f.keywords {
            d += &format!("<rdf:li>{}</rdf:li>", esc(k));
        }
        d += "</rdf:Bag></dc:subject>\n";
    }
    if let Some(a) = &f.alt {
        d += &alt("Iptc4xmpCore:AltTextAccessibility", a);
    }
    if let Some((lat, lon)) = f.gps {
        d += &format!(
            "   <exif:GPSLatitude>{}</exif:GPSLatitude>\n   <exif:GPSLongitude>{}</exif:GPSLongitude>\n   <exif:GPSVersionID>2.2.0.0</exif:GPSVersionID>\n",
            gps_xmp(lat, 'N', 'S'),
            gps_xmp(lon, 'E', 'W')
        );
    }
    let s = format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Castor\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:Iptc4xmpCore=\"http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/\" xmlns:exif=\"http://ns.adobe.com/exif/1.0/\">\n   <xmp:CreatorTool>Castor</xmp:CreatorTool>\n{d}  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    );
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimal_exif_roundtrip() {
        let e = minimal_exif(Some(6), Some("basedpolymer"), Some("(c) 2026")).unwrap();
        assert_eq!(crate::codec::exif_orientation(&e), Some(6));
        assert_eq!(exif_copyright(&e), (Some("basedpolymer".into()), Some("(c) 2026".into())));
    }
    #[test]
    fn xmp_parses() {
        let x = build_xmp(&XmpFields { title: Some("A & B".into()), keywords: vec!["x".into()], gps: Some((48.85, -2.35)), ..Default::default() });
        let s = String::from_utf8(x).unwrap();
        assert!(roxmltree::Document::parse(&s).is_ok());
        assert!(s.contains("2,21.000000W"));
    }
}

/// Copy of an EXIF block with the Orientation tag set to 1 (pixels were made upright).
/// Returns the input unchanged if it has no orientation tag or cannot be parsed.
pub fn reset_orientation(exif: &[u8]) -> Vec<u8> {
    let mut v = exif.to_vec();
    let off = if v.starts_with(b"Exif\0\0") { 6 } else { 0 };
    let t = &v[off..];
    if t.len() < 8 {
        return v;
    }
    let le = match &t[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return v,
    };
    let rd16 = |b: &[u8], i: usize| -> Option<u16> {
        let s = b.get(i..i + 2)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let rd32 = |b: &[u8], i: usize| -> Option<u32> {
        let s = b.get(i..i + 4)?;
        Some(if le { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) })
    };
    let Some(ifd) = rd32(t, 4).map(|x| x as usize) else { return v };
    let Some(n) = rd16(t, ifd) else { return v };
    for i in 0..n as usize {
        let e = ifd + 2 + i * 12;
        if rd16(t, e) == Some(0x0112) {
            let pos = off + e + 8;
            if pos + 2 <= v.len() {
                let one = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
                v[pos..pos + 2].copy_from_slice(&one);
            }
            break;
        }
    }
    v
}

#[cfg(test)]
mod orient_tests {
    #[test]
    fn reset() {
        let e = super::minimal_exif(Some(6), None, None).unwrap();
        let r = super::reset_orientation(&e);
        assert_eq!(crate::codec::exif_orientation(&r), Some(1));
    }
}
