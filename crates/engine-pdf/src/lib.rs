//! PDF engine: MuPDF (rendering, text, editing, redaction, encryption, layout of e-books)
//! plus Tesseract OCR (built into MuPDF), behind a small safe API.
//!
//! Owner: agent A (docs/specs/A-pdf.md). Public, stable API used by other crates:
//! [`ocr`] (`languages`, `assets`, `recognize`), [`thumbnail`], [`page_count`].
//!
//! The `mupdf` crate is re-exported for the PDF tools. What it lacks (image recompression,
//! font subsetting, recolouring, OCR) goes through `csrc/castor_mupdf.c`, see [`optimize`].

use std::path::Path;

use anyhow::{Context, Result};
pub use mupdf;
use mupdf::pdf::{PdfDocument, PdfWriteOptions};
use mupdf::{Colorspace, Document, Matrix, Pixmap};

mod ffi;
pub mod draw;
pub mod fonts;
pub mod ocr;
pub mod optimize;
pub mod pages;
pub mod scan;
pub mod text;
pub mod textlayer;

/// Producer string written into PDFs Castor creates.
pub const PRODUCER: &str = "Castor";

fn tr(key: &str) -> String {
    castor_core::tr(key)
}

/// A path as the UTF-8 string MuPDF expects (MuPDF converts it to UTF-16 on Windows).
pub fn path_str(p: &Path) -> Result<&str> {
    p.to_str().with_context(|| format!("{}: {}", tr("pdf.err.path"), p.display()))
}

fn check_readable(path: &Path) -> Result<()> {
    let md = std::fs::metadata(path).with_context(|| format!("{}: {}", tr("pdf.err.missing"), path.display()))?;
    anyhow::ensure!(md.len() > 0, "{}", tr("pdf.err.empty"));
    Ok(())
}

/// Open a PDF for editing. `password` may be empty. Errors are user-readable.
pub fn open_pdf(path: &Path, password: &str) -> Result<PdfDocument> {
    check_readable(path)?;
    let mut doc = PdfDocument::open(path_str(path)?)
        .map_err(|e| anyhow::anyhow!("{} ({e})", tr("pdf.err.corrupt")))?;
    if doc.needs_password()? {
        anyhow::ensure!(!password.is_empty(), "{}", tr("pdf.err.password_needed"));
        anyhow::ensure!(doc.authenticate(password)?, "{}", tr("pdf.err.password_wrong"));
    }
    Ok(doc)
}

/// Open any document MuPDF understands (PDF, XPS, EPUB, MOBI, FB2, CBZ, images, HTML, TXT…).
pub fn open_document(path: &Path, password: &str) -> Result<Document> {
    check_readable(path)?;
    let mut doc =
        Document::open(path_str(path)?).map_err(|e| anyhow::anyhow!("{} ({e})", tr("pdf.err.corrupt")))?;
    if doc.needs_password()? {
        anyhow::ensure!(!password.is_empty(), "{}", tr("pdf.err.password_needed"));
        anyhow::ensure!(doc.authenticate(password)?, "{}", tr("pdf.err.password_wrong"));
    }
    Ok(doc)
}

/// Number of pages (fast: no page is loaded).
pub fn page_count(path: &Path) -> Result<usize> {
    let doc = open_document(path, "")?;
    Ok(doc.page_count()?.max(0) as usize)
}

/// PNG thumbnail of page `page` (0-based), longest side ≤ `max_side`.
pub fn thumbnail(path: &Path, page: usize, max_side: u32) -> Result<Vec<u8>> {
    let doc = open_document(path, "")?;
    let n = doc.page_count()?.max(0) as usize;
    anyhow::ensure!(page < n, "page {} does not exist ({} pages)", page + 1, n);
    let page = doc.load_page(page as i32)?;
    let b = page.bounds()?;
    let longest = b.width().max(b.height()).max(1.0);
    let scale = max_side.max(1) as f32 / longest;
    let pix = page.to_pixmap(&Matrix::new_scale(scale, scale), &Colorspace::device_rgb(), false, true)?;
    let mut out = Vec::new();
    pix.write_to(&mut out, mupdf::ImageFormat::PNG)?;
    Ok(out)
}

/// Render a page at `dpi`. `gray` = 1 channel, otherwise RGB. Annotations and widgets included.
pub fn render(page: &mupdf::Page, dpi: f32, gray: bool) -> Result<Pixmap> {
    let s = dpi / 72.0;
    let cs = if gray { Colorspace::device_gray() } else { Colorspace::device_rgb() };
    Ok(page.to_pixmap(&Matrix::new_scale(s, s), &cs, false, true)?)
}

/// Copy a pixmap (no alpha) into an `image` buffer.
pub fn pixmap_to_rgb(pix: &Pixmap) -> image::RgbImage {
    let (w, h, n, stride) = (pix.width(), pix.height(), pix.n() as usize, pix.stride() as usize);
    let s = pix.samples();
    let mut out = image::RgbImage::new(w, h);
    for y in 0..h as usize {
        let row = &s[y * stride..];
        for x in 0..w as usize {
            let p = &row[x * n..];
            let px = if n >= 3 { [p[0], p[1], p[2]] } else { [p[0], p[0], p[0]] };
            out.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    out
}

/// Copy a pixmap into a gray `image` buffer (luma of RGB).
pub fn pixmap_to_gray(pix: &Pixmap) -> image::GrayImage {
    let (w, h, n, stride) = (pix.width(), pix.height(), pix.n() as usize, pix.stride() as usize);
    let s = pix.samples();
    let mut out = image::GrayImage::new(w, h);
    for y in 0..h as usize {
        let row = &s[y * stride..];
        for x in 0..w as usize {
            let p = &row[x * n..];
            let v = if n >= 3 { ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8 } else { p[0] };
            out.put_pixel(x as u32, y as u32, image::Luma([v]));
        }
    }
    out
}

/// How to save a document edited through the safe API.
#[derive(Clone, Debug)]
pub struct Save {
    /// 0 none … 4 maximum (3 = remove unused + de-duplicate, a good default).
    pub garbage: i32,
    /// Clean (re-write) content streams.
    pub clean: bool,
    /// Rebuild appearance streams of annotations and widgets.
    pub appearance: bool,
    pub encrypt: Option<Encrypt>,
    /// Keep the original encryption of the input (only meaningful for authenticated docs).
    pub keep_encryption: bool,
}

impl Default for Save {
    fn default() -> Self {
        Save { garbage: 3, clean: false, appearance: false, encrypt: None, keep_encryption: false }
    }
}

/// AES-256 encryption settings.
#[derive(Clone, Debug)]
pub struct Encrypt {
    pub user_password: String,
    pub owner_password: String,
    /// `mupdf::pdf::Permission` bits.
    pub permissions: u32,
}

/// Save a document to `out`, atomically (temporary file in the same folder, then rename).
/// Sets the Producer to "Castor" unless `keep_producer`.
pub fn save_pdf(doc: &PdfDocument, out: &Path, opts: &Save) -> Result<()> {
    let mut w = PdfWriteOptions::default();
    w.set_garbage_level(opts.garbage).set_compress(true).set_compress_images(true).set_compress_fonts(true);
    w.set_clean(opts.clean).set_appearance(opts.appearance);
    if let Some(e) = &opts.encrypt {
        anyhow::ensure!(e.user_password.len() < 127 && e.owner_password.len() < 127, "{}", tr("pdf.err.password_long"));
        w.set_encryption(mupdf::pdf::Encryption::Aes256)
            .set_user_password(&e.user_password)
            .set_owner_password(&e.owner_password)
            .set_permissions(mupdf::pdf::Permission::from_bits_truncate(e.permissions));
    } else if opts.keep_encryption {
        w.set_encryption(mupdf::pdf::Encryption::Keep);
    } else {
        w.set_encryption(mupdf::pdf::Encryption::None);
    }
    let tmp = tmp_sibling(out);
    let res = doc.save_with_options(path_str(&tmp)?, w);
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::anyhow!("{}: {e}", tr("pdf.err.save")));
    }
    std::fs::rename(&tmp, out).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// `dir/.name.castor-tmp` next to `out`.
pub fn tmp_sibling(out: &Path) -> std::path::PathBuf {
    let name = out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    out.with_file_name(format!(".{name}.castor-tmp"))
}

/// A PDF text string object: plain for ASCII, UTF-16BE with a byte-order mark otherwise.
pub fn text_string(doc: &PdfDocument, value: &str) -> Result<mupdf::pdf::PdfObject> {
    if value.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Ok(doc.new_string(value)?);
    }
    let mut hex = String::from("<FEFF");
    for u in value.encode_utf16() {
        hex.push_str(&format!("{u:04X}"));
    }
    hex.push('>');
    Ok(doc.new_object_from_str(&hex)?)
}

/// Set the Info dictionary entry `key` (e.g. "Producer") to `value` ("" removes it).
pub fn set_info(doc: &mut PdfDocument, key: &str, value: &str) -> Result<()> {
    let mut trailer = doc.trailer()?;
    let mut info = match trailer.get_dict("Info")? {
        Some(i) if i.is_dict()? => i,
        _ => {
            if value.is_empty() {
                return Ok(());
            }
            let d = doc.new_dict()?;
            let d = doc.add_object(&d)?;
            trailer.dict_put("Info", d.clone())?;
            d
        }
    };
    if value.is_empty() {
        info.dict_delete(key)?;
    } else {
        let v = text_string(doc, value)?;
        info.dict_put(key, v)?;
    }
    Ok(())
}

/// Read an Info entry as UTF-8 ("" if missing).
pub fn get_info(doc: &PdfDocument, key: &str) -> String {
    use mupdf::MetadataName as M;
    let name = match key {
        "Title" => M::Title,
        "Author" => M::Author,
        "Subject" => M::Subject,
        "Keywords" => M::Keywords,
        "Creator" => M::Creator,
        "Producer" => M::Producer,
        "CreationDate" => M::CreationDate,
        "ModDate" => M::ModDate,
        _ => return String::new(),
    };
    doc.metadata(name).unwrap_or_default()
}

/// Mark a document as produced by Castor and refresh its modification date.
pub fn stamp_producer(doc: &mut PdfDocument) -> Result<()> {
    set_info(doc, "Producer", PRODUCER)?;
    set_info(doc, "ModDate", &pdf_date_now())?;
    Ok(())
}

/// Current time as a PDF date string "D:YYYYMMDDHHmmSSZ" (UTC).
pub fn pdf_date_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(secs as i64);
    format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z")
}

/// Unix seconds → (year, month, day, hour, minute, second), UTC.
pub fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }
}
