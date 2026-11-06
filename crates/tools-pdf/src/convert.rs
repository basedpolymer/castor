//! Conversions: PDF → Markdown, PDF → EPUB, e-book → PDF, Office → PDF (LibreOffice).

use std::io::Write;
use std::path::PathBuf;

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileKind, InputSpec, Job, Output, Requirement, Result, RunCx, Tool, ToolMeta, ToolStatus};

use crate::common::*;
use crate::structure::{self, Options};

pub struct ToMarkdown {
    meta: ToolMeta,
}

impl ToMarkdown {
    pub fn new() -> Self {
        ToMarkdown { meta: meta("pdf.to-markdown", Family::Pdf, PDF_IN, "MD", &["markdown", "text", "convert", "notes", "llm", "obsidian"], "") }
    }
}

impl Tool for ToMarkdown {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::toggle("images", true), OptionSpec::toggle("page_breaks", false), password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let doc = engine_pdf::open_document(&input.path, password(v))?;
        let elems = structure::analyse(&doc, &Options { images: v.bool("images"), page_breaks: v.bool("page_breaks"), min_image: 24 }, cx, 0.0, 0.9)?;
        let out = out_for(cx, input, "md");
        let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let img_dir_name = format!("{stem}-images");
        let img_dir = cx.out_dir().join(&img_dir_name);
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        let md = structure::to_markdown(&elems, |i, png| {
            std::fs::create_dir_all(&img_dir)?;
            let p = img_dir.join(format!("img-{i}.png"));
            std::fs::write(&p, png)?;
            cleanup.add(&p);
            outs.push(Output::new(&p));
            Ok(format!("{img_dir_name}/img-{i}.png"))
        })?;
        cleanup.add(&out);
        std::fs::write(&out, md)?;
        let words_n = elems.len();
        let mut all = vec![Output::new(&out).with_note(tr_args("pdf.to-markdown.note", &[("blocks", &words_n.to_string()), ("images", &outs.len().to_string())]))];
        all.extend(outs);
        cleanup.keep();
        Ok(all)
    }
}

// ------------------------------------------------------------------------------------------

pub struct ToEpub {
    meta: ToolMeta,
}

impl ToEpub {
    pub fn new() -> Self {
        let mut m = meta("pdf.to-epub", Family::Pdf, PDF_IN, "EPUB", &["ebook", "kindle", "kobo", "reflow", "e-reader"], "");
        m.status = ToolStatus::Beta;
        ToEpub { meta: m }
    }
}

fn epub(title: &str, chapters: &[(String, String)], images: &[(usize, Vec<u8>)], lang: &str) -> Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("mimetype", stored)?;
        z.write_all(b"application/epub+zip")?;
        z.start_file("META-INF/container.xml", deflated)?;
        z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>
"#)?;
        z.start_file("OEBPS/style.css", deflated)?;
        z.write_all(b"body{font-family:serif;line-height:1.45;margin:0 4%}h1,h2,h3,h4{font-family:sans-serif;line-height:1.2}p{text-indent:0;margin:0 0 .6em}p.img{text-align:center}img{max-width:100%}pre{white-space:pre-wrap;font-size:.85em}table{border-collapse:collapse}td,th{border:1px solid #999;padding:.2em .4em}\n")?;
        let t = structure::xml_escape(title);
        let mut manifest = String::new();
        let mut spine = String::new();
        let mut nav = String::new();
        for (i, (ct, body)) in chapters.iter().enumerate() {
            let name = format!("ch{:03}.xhtml", i + 1);
            let ctitle = if ct.trim().is_empty() { format!("{title} ({})", i + 1) } else { ct.clone() };
            z.start_file(format!("OEBPS/{name}"), deflated)?;
            write!(
                z,
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\"><head><meta charset=\"utf-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head><body>\n{body}</body></html>\n",
                structure::xml_escape(&ctitle)
            )?;
            manifest.push_str(&format!("<item id=\"c{}\" href=\"{name}\" media-type=\"application/xhtml+xml\"/>\n", i + 1));
            spine.push_str(&format!("<itemref idref=\"c{}\"/>\n", i + 1));
            nav.push_str(&format!("<li><a href=\"{name}\">{}</a></li>\n", structure::xml_escape(&ctitle)));
        }
        for (i, png) in images {
            z.start_file(format!("OEBPS/images/img-{i}.png"), stored)?;
            z.write_all(png)?;
            manifest.push_str(&format!("<item id=\"i{i}\" href=\"images/img-{i}.png\" media-type=\"image/png\"/>\n"));
        }
        z.start_file("OEBPS/nav.xhtml", deflated)?;
        write!(
            z,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><meta charset=\"utf-8\"/><title>{t}</title></head><body><nav epub:type=\"toc\" id=\"toc\"><h1>{t}</h1><ol>\n{nav}</ol></nav></body></html>\n"
        )?;
        let id = format!("urn:castor:{:x}", {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            title.hash(&mut h);
            chapters.len().hash(&mut h);
            h.finish()
        });
        let modified = {
            let (y, mo, d, hh, mi, ss) = engine_pdf::civil(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
            );
            format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mi:02}:{ss:02}Z")
        };
        z.start_file("OEBPS/content.opf", deflated)?;
        write!(
            z,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"uid\">{id}</dc:identifier><dc:title>{t}</dc:title><dc:language>{lang}</dc:language><meta property=\"dcterms:modified\">{modified}</meta></metadata><manifest><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/><item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n{manifest}</manifest><spine>\n{spine}</spine></package>\n"
        )?;
        z.finish()?;
    }
    Ok(buf.into_inner())
}

impl Tool for ToEpub {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::toggle("images", true), OptionSpec::text("language", "en").advanced(), password_opt()]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let doc = engine_pdf::open_document(&input.path, password(v))?;
        let elems = structure::analyse(&doc, &Options { images: v.bool("images"), page_breaks: false, min_image: 24 }, cx, 0.0, 0.85)?;
        let mut images = Vec::new();
        let chapters = structure::to_xhtml_chapters(&elems, |i, png| images.push((i, png.to_vec())));
        anyhow::ensure!(!chapters.is_empty(), "{}", tr("pdf.to-epub.err.no_text"));
        let title = {
            let t = doc.metadata(mupdf::MetadataName::Title).unwrap_or_default();
            if t.trim().is_empty() { input.stem() } else { t }
        };
        let lang = v.str("language").trim();
        let bytes = epub(&title, &chapters, &images, if lang.is_empty() { "en" } else { lang })?;
        let out = out_for(cx, input, "epub");
        std::fs::write(&out, bytes)?;
        Ok(vec![Output::new(out).with_note(tr_args("pdf.to-epub.note", &[("chapters", &chapters.len().to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct FromEbook {
    meta: ToolMeta,
}

impl FromEbook {
    pub fn new() -> Self {
        FromEbook {
            meta: meta(
                "pdf.from-ebook",
                Family::Pdf,
                InputSpec::kinds(&[FileKind::Ebook, FileKind::Html, FileKind::Text]).with_extensions(&["cbz", "fb2", "xhtml", "txt"]),
                "PDF",
                &["epub", "mobi", "fb2", "cbz", "comic", "print ebook", "convert"],
                "",
            ),
        }
    }
}

impl Tool for FromEbook {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "page_size",
                vec![Choice::lit("a5", "A5"), Choice::lit("a4", "A4"), Choice::lit("letter", "Letter"), Choice::key("pocket"), Choice::key("ereader")],
                "a5",
            ),
            OptionSpec::float("font_size", 6.0, 24.0, 11.0).unit("pt"),
            OptionSpec::float("margin", 0.0, 40.0, 15.0).unit("mm"),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let (w, h) = match v.str("page_size") {
            "a4" => (595.28, 841.89),
            "letter" => (612.0, 792.0),
            "pocket" => (311.8, 498.9),  // 110 × 176 mm
            "ereader" => (255.1, 340.2), // 90 × 120 mm, 6" readers
            _ => (419.53, 595.28),
        };
        let m = mm(v.float("margin"));
        let mut ctx = mupdf::Context::get();
        ctx.set_user_css(&format!("@page {{ margin: {m:.1}pt }}"))?;
        let mut doc = engine_pdf::open_document(&input.path, "")?;
        cx.message(tr("pdf.from-ebook.msg.layout"));
        if doc.is_reflowable()? {
            doc.layout(w, h, v.float("font_size").clamp(4.0, 48.0) as f32)?;
        }
        cx.check_cancel()?;
        cx.progress(0.3);
        let outline = doc.outlines().unwrap_or_default();
        let mut pdf = doc.convert_to_pdf(0, -1, 0)?;
        let _ = ctx.set_user_css("");
        cx.progress(0.85);
        if !outline.is_empty() {
            let _ = pdf.set_outlines(&outline);
        }
        let title = doc.metadata(mupdf::MetadataName::Title).unwrap_or_default();
        if !title.trim().is_empty() {
            engine_pdf::set_info(&mut pdf, "Title", &title)?;
        }
        let author = doc.metadata(mupdf::MetadataName::Author).unwrap_or_default();
        if !author.trim().is_empty() {
            engine_pdf::set_info(&mut pdf, "Author", &author)?;
        }
        engine_pdf::stamp_producer(&mut pdf)?;
        let out = cx.output_named(&format!("{}.pdf", input.stem()));
        engine_pdf::save_pdf(&pdf, &out, &engine_pdf::Save::default())?;
        let n = pdf.page_count()?.to_string();
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &n)]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct FromOffice {
    meta: ToolMeta,
}

impl FromOffice {
    pub fn new() -> Self {
        let mut m = meta(
            "pdf.from-office",
            Family::Pdf,
            InputSpec::kinds(&[FileKind::Office]).with_extensions(&["odg", "csv", "txt", "html", "htm"]),
            "PDF",
            &["word", "docx", "excel", "xlsx", "powerpoint", "pptx", "libreoffice", "odt", "convert"],
            "",
        );
        m.requires = &[Requirement::LibreOffice];
        m.serial = true;
        FromOffice { meta: m }
    }
}

/// The LibreOffice binary, if installed.
pub fn soffice() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    for base in [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)"), std::env::var_os("LOCALAPPDATA")].into_iter().flatten() {
        let b = PathBuf::from(base);
        cands.push(b.join("LibreOffice").join("program").join("soffice.exe"));
        cands.push(b.join("Programs").join("LibreOffice").join("program").join("soffice.exe"));
    }
    #[cfg(target_os = "macos")]
    {
        cands.push(PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"));
        if let Some(h) = std::env::var_os("HOME") {
            cands.push(PathBuf::from(h).join("Applications/LibreOffice.app/Contents/MacOS/soffice"));
        }
    }
    if let Some(p) = cands.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    castor_core::paths::find_program("soffice").or_else(|| castor_core::paths::find_program("libreoffice"))
}

fn file_url(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') { s } else { format!("/{s}") };
    let mut o = String::from("file://");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

impl Tool for FromOffice {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn unavailable(&self) -> Option<String> {
        soffice().is_none().then(|| tr("pdf.from-office.unavailable"))
    }
    fn run(&self, job: &Job, _v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let exe = soffice().ok_or_else(|| anyhow::anyhow!("{}", tr("pdf.from-office.unavailable")))?;
        let work = cx.tmp_dir().join("lo");
        let profile = cx.tmp_dir().join("lo-profile");
        std::fs::create_dir_all(&work)?;
        cx.message(tr("pdf.from-office.msg"));
        let mut cmd = castor_core::process::command(&exe);
        cmd.arg(format!("-env:UserInstallation={}", file_url(&profile)))
            .args(["--headless", "--norestore", "--nolockcheck", "--convert-to", "pdf", "--outdir"])
            .arg(&work)
            .arg(&input.path);
        castor_core::process::run(cmd, &cx.cancel_token(), |_| {}, |_| {})?;
        let produced = std::fs::read_dir(&work)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
            .ok_or_else(|| anyhow::anyhow!("{}", tr("pdf.from-office.err.failed")))?;
        let out = cx.output_named(&format!("{}.pdf", input.stem()));
        std::fs::copy(&produced, &out)?;
        Ok(vec![Output::new(out)])
    }
}
