//! Typst as a library: compile a source string (plus in-memory files) into a paged document and
//! export it as PDF, PNG or SVG. Used by the card, sheet, merge, formula and carousel tools.
//!
//! Fonts: Latin Modern (bundled in `assets/fonts`), TeX Gyre Chorus, New Computer Modern Math
//! (for formulas), plus any installed family a template asks for (looked up with fontdb and
//! loaded on demand). Typst never touches the network here: packages are not available.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use typst::diag::{FileError, FileResult, Severity, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
pub use typst_layout::PagedDocument;

macro_rules! font {
    ($f:literal) => {
        include_bytes!(concat!("../../../../assets/fonts/", $f)).as_slice()
    };
}

const BUNDLED: &[&[u8]] = &[
    font!("lmroman10-regular.otf"),
    font!("lmroman10-bold.otf"),
    font!("lmroman10-italic.otf"),
    font!("lmromanslant10-regular.otf"),
    font!("lmromancaps10-regular.otf"),
    font!("lmsans10-regular.otf"),
    font!("lmsans10-bold.otf"),
    font!("lmsans10-oblique.otf"),
    font!("lmmono10-regular.otf"),
    font!("texgyrechorus-mediumitalic.otf"),
    include_bytes!("../../assets/fonts/NewCMMath-Regular.otf").as_slice(),
];

/// Bundled font bytes, for the other renderers (resvg) too.
pub fn bundled_font_data() -> &'static [&'static [u8]] {
    BUNDLED
}

fn bundled_fonts() -> &'static Vec<Font> {
    static F: OnceLock<Vec<Font>> = OnceLock::new();
    F.get_or_init(|| BUNDLED.iter().flat_map(|d| Font::iter(Bytes::new(*d))).collect())
}

fn library() -> &'static LazyHash<Library> {
    static L: OnceLock<LazyHash<Library>> = OnceLock::new();
    L.get_or_init(|| LazyHash::new(Library::default()))
}

/// Installed fonts (scanned once, lazily).
pub fn system_db() -> &'static resvg::usvg::fontdb::Database {
    static DB: OnceLock<resvg::usvg::fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Load every face of an installed family (case-insensitive), as Typst fonts.
pub fn system_family(name: &str) -> Vec<Font> {
    static CACHE: OnceLock<parking_lot::Mutex<HashMap<String, Vec<Font>>>> = OnceLock::new();
    let key = name.trim().to_lowercase();
    if key.is_empty() {
        return Vec::new();
    }
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().get(&key) {
        return v.clone();
    }
    let db = system_db();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for face in db.faces() {
        if !face.families.iter().any(|(f, _)| f.to_lowercase() == key) {
            continue;
        }
        if let resvg::usvg::fontdb::Source::File(p) = &face.source {
            if !seen.insert((p.clone(), face.index)) {
                continue;
            }
            if let Ok(data) = std::fs::read(p) {
                if let Some(f) = Font::new(Bytes::new(data), face.index) {
                    out.push(f);
                }
            }
        }
    }
    cache.lock().insert(key, out.clone());
    out
}

/// A document to compile.
#[derive(Default, Clone)]
pub struct Doc {
    pub source: String,
    /// In-memory files, by virtual path ("/qr.svg", "/data.json").
    pub files: Vec<(String, Vec<u8>)>,
    /// Folder that relative paths in the source resolve to (user templates), if any.
    pub root: Option<PathBuf>,
    /// Installed font families to make available (besides the bundled ones).
    pub families: Vec<String>,
    /// `sys.inputs` for user templates (all values are strings).
    pub inputs: Vec<(String, String)>,
    /// Extra font files (bytes) to make available (font specimens).
    pub font_data: Vec<Vec<u8>>,
}

impl Doc {
    pub fn new(source: impl Into<String>) -> Doc {
        Doc { source: source.into(), ..Default::default() }
    }
    pub fn file(mut self, vpath: &str, data: impl Into<Vec<u8>>) -> Doc {
        self.files.push((vpath.to_string(), data.into()));
        self
    }
    pub fn json(self, vpath: &str, v: &serde_json::Value) -> Doc {
        let s = serde_json::to_vec(v).unwrap_or_default();
        self.file(vpath, s)
    }
    pub fn family(mut self, f: &str) -> Doc {
        if !f.trim().is_empty() {
            self.families.push(f.trim().to_string());
        }
        self
    }
    pub fn root(mut self, p: &Path) -> Doc {
        self.root = Some(p.to_path_buf());
        self
    }

    pub fn compile(&self) -> Result<PagedDocument> {
        compile(self)
    }
}

struct CastorWorld {
    main: FileId,
    source: Source,
    files: HashMap<FileId, Bytes>,
    root: Option<PathBuf>,
    fonts: Vec<Font>,
    book: LazyHash<FontBook>,
    library: Option<LazyHash<Library>>,
}

fn file_id(path: &str) -> Result<FileId> {
    let vp = VirtualPath::new(path).map_err(|e| anyhow!("bad virtual path {path}: {e:?}"))?;
    Ok(FileId::new(RootedPath::new(VirtualRoot::Project, vp)))
}

impl World for CastorWorld {
    fn library(&self) -> &LazyHash<Library> {
        self.library.as_ref().unwrap_or_else(|| library())
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            return Ok(self.source.clone());
        }
        let bytes = self.file(id)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Source::new(id, text.to_string()))
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            return Ok(Bytes::from_string(self.source.text().to_string()));
        }
        if let Some(b) = self.files.get(&id) {
            return Ok(b.clone());
        }
        let rel = id.vpath().get_without_slash().to_string();
        if let (Some(root), VirtualRoot::Project) = (&self.root, id.root()) {
            // Only inside the template's folder: VirtualPath is already normalised (no "..").
            let p = root.join(&rel);
            if p.starts_with(root) && p.is_file() {
                return std::fs::read(&p).map(Bytes::new).map_err(|e| FileError::from_io(e, &p));
            }
        }
        Err(FileError::NotFound(PathBuf::from(rel)))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        let (y, m, d) = today();
        Datetime::from_ymd(y, m, d)
    }
}

/// Local date (year, month, day).
pub fn today() -> (i32, u8, u8) {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    (now.year(), now.month() as u8, now.day())
}

/// Compile a document. Errors carry Typst's messages with line numbers.
pub fn compile(doc: &Doc) -> Result<PagedDocument> {
    let main = file_id("/main.typ")?;
    let mut files = HashMap::new();
    for (p, data) in &doc.files {
        files.insert(file_id(p)?, Bytes::new(data.clone()));
    }
    // Extra font files first: they win over a bundled face with the same family and variant.
    let mut fonts: Vec<Font> = doc.font_data.iter().flat_map(|d| Font::iter(Bytes::new(d.clone()))).collect();
    fonts.extend(bundled_fonts().iter().cloned());
    for fam in &doc.families {
        if !fonts.iter().any(|f| f.info().family.eq_ignore_ascii_case(fam)) {
            fonts.extend(system_family(fam));
        }
    }
    let book = LazyHash::new(FontBook::from_fonts(fonts.iter()));
    let library = if doc.inputs.is_empty() {
        None
    } else {
        let mut dict = typst::foundations::Dict::new();
        for (k, v) in &doc.inputs {
            dict.insert(k.as_str().into(), typst::foundations::Value::Str(v.as_str().into()));
        }
        Some(LazyHash::new(Library::builder().with_inputs(dict).build()))
    };
    let world = CastorWorld { main, source: Source::new(main, doc.source.clone()), files, root: doc.root.clone(), fonts, book, library };
    let warned = typst::compile::<PagedDocument>(&world);
    // Keep the memoisation cache bounded in long-running processes (the app, MCP).
    typst::comemo::evict(30);
    warned.output.map_err(|errs| anyhow!("{}", format_diagnostics(&world, &errs)))
}

fn format_diagnostics(world: &CastorWorld, diags: &[SourceDiagnostic]) -> String {
    let mut out = Vec::new();
    for d in diags.iter().filter(|d| d.severity == Severity::Error) {
        let mut line = String::new();
        if let Some(id) = d.span.id() {
            if let (Ok(src), Some(range)) = (world.source(id), typst::WorldExt::range(world, d.span)) {
                if let Some(l) = src.lines().byte_to_line(range.start) {
                    line = format!("line {}: ", l + 1);
                }
            }
        }
        let mut msg = format!("{line}{}", d.message);
        for h in &d.hints {
            msg.push_str(&format!(" (hint: {})", h.v));
        }
        out.push(msg);
    }
    if out.is_empty() { "the document could not be laid out".into() } else { out.join("\n") }
}

/// PDF bytes. Creator "Castor", no tagged-PDF overhead for tiny cards unless asked.
pub fn pdf(doc: &PagedDocument) -> Result<Vec<u8>> {
    let opts = typst_pdf::PdfOptions {
        creator: typst::foundations::Smart::Custom(Some("Castor".into())),
        ..Default::default()
    };
    typst_pdf::pdf(doc, &opts).map_err(|e| anyhow!("PDF export failed: {}", e.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))
}

/// Number of pages.
pub fn page_count(doc: &PagedDocument) -> usize {
    doc.pages().len()
}

/// Render page `i` at `ppt` pixels per point.
pub fn pixmap(doc: &PagedDocument, i: usize, ppt: f32) -> Result<resvg::tiny_skia::Pixmap> {
    let page = doc.pages().get(i).ok_or_else(|| anyhow!("page {} does not exist", i + 1))?;
    let opts = typst_render::RenderOptions { pixel_per_pt: (ppt as f64).into(), render_bleed: false };
    let pm = typst_render::render(page, &opts);
    // typst-render and resvg share tiny-skia 0.12: same type.
    Ok(pm)
}

/// Page size in points.
pub fn page_size(doc: &PagedDocument, i: usize) -> (f64, f64) {
    doc.pages().get(i).map(|p| (p.frame.width().to_pt(), p.frame.height().to_pt())).unwrap_or((0.0, 0.0))
}

/// Render page `i` so that it is exactly `width` pixels wide (height follows the aspect ratio).
pub fn png_width(doc: &PagedDocument, i: usize, width: u32) -> Result<Vec<u8>> {
    let (w, _) = page_size(doc, i);
    let ppt = width as f64 / w.max(1.0);
    let pm = pixmap(doc, i, ppt as f32)?;
    super::png::encode_pixmap(&pm)
}

/// Render page `i` with its longest side at most `max_side` pixels.
pub fn png_fit(doc: &PagedDocument, i: usize, max_side: u32) -> Result<Vec<u8>> {
    let (w, h) = page_size(doc, i);
    let ppt = max_side as f64 / w.max(h).max(1.0);
    let pm = pixmap(doc, i, ppt as f32)?;
    super::png::encode_pixmap(&pm)
}

/// SVG of page `i`.
pub fn svg(doc: &PagedDocument, i: usize) -> Result<String> {
    let page = doc.pages().get(i).ok_or_else(|| anyhow!("page {} does not exist", i + 1))?;
    Ok(typst_svg::svg(page, &typst_svg::SvgOptions { render_bleed: false, pretty: false }))
}

/// A Typst string literal (quotes included) for any text.
pub fn lit(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// "#rrggbb" → `rgb("#rrggbb")`, with alpha when not opaque.
pub fn color(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("rgb(\"#{:02x}{:02x}{:02x}\")", c[0], c[1], c[2])
    } else {
        format!("rgb(\"#{:02x}{:02x}{:02x}{:02x}\")", c[0], c[1], c[2], c[3])
    }
}

/// Families of every bundled font, for tests and the fonts tool.
pub fn bundled_families() -> Vec<String> {
    let mut v: Vec<String> = bundled_fonts().iter().map(|f| f.info().family.to_string()).collect();
    v.dedup();
    v
}

#[allow(dead_code)]
pub(crate) fn arc_doc(d: PagedDocument) -> Arc<PagedDocument> {
    Arc::new(d)
}
