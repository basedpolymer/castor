//! PDF Studio, engine side: the edit script (what the user did, serialisable so a save goes
//! through the runner and lands in the history), a renderer thread that owns the open
//! documents, and [`apply`], which writes the edited PDF with MuPDF through engine-pdf.
//!
//! Coordinates of marks are in the *source page's* visible space (MuPDF "fitz" space: points,
//! origin top-left, the page's own /Rotate already applied). The studio's extra rotation of a
//! page is applied on top when it is displayed and when the file is saved, so marks survive
//! reordering and rotating.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use castor_core::RunCx;
use crossbeam_channel::{Receiver, Sender};
use engine_pdf::mupdf::pdf::{
    AnnotationArea, LineEndingStyle, PdfDocument, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod, PdfRedactOptions,
    PdfRedactTextMethod,
};
use engine_pdf::mupdf::color::AnnotationColor;
use engine_pdf::mupdf::{self, Colorspace, Matrix, Point, Quad, Rect};
use serde::{Deserialize, Serialize};

pub type Rgb = [f32; 3];
/// x0, y0, x1, y1 in points.
pub type R4 = [f32; 4];

/// One page of the result: which source page, with how much extra clockwise rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRef {
    pub src: usize,
    pub page: usize,
    #[serde(default)]
    pub rotate: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Signature {
    /// A typed name, drawn in TeX Gyre Chorus.
    Typed { name: String, color: Rgb },
    /// A PNG (transparent background) or JPEG.
    Image { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mark {
    Highlight { rects: Vec<R4>, color: Rgb },
    Underline { rects: Vec<R4>, color: Rgb },
    Pen { strokes: Vec<Vec<[f32; 2]>>, color: Rgb, width: f32 },
    Rect { rect: R4, color: Rgb, width: f32 },
    Arrow { from: [f32; 2], to: [f32; 2], color: Rgb, width: f32 },
    /// A comment (sticky note) with the reviewer's name.
    Note { at: [f32; 2], text: String, color: Rgb },
    /// Text written on the page.
    Text { rect: R4, text: String, color: Rgb, size: f32 },
    Signature { rect: R4, sig: Signature, date: Option<String> },
    Redact { rect: R4 },
}

impl Mark {
    /// Bounding box in page space (hit testing, selection outline).
    pub fn bounds(&self) -> R4 {
        match self {
            Mark::Highlight { rects, .. } | Mark::Underline { rects, .. } => union_all(rects.iter().copied()),
            Mark::Pen { strokes, width, .. } => {
                let b = union_all(strokes.iter().flatten().map(|p| [p[0], p[1], p[0], p[1]]));
                grow(b, width / 2.0 + 1.0)
            }
            Mark::Rect { rect, .. } | Mark::Text { rect, .. } | Mark::Signature { rect, .. } | Mark::Redact { rect } => norm(*rect),
            Mark::Arrow { from, to, width, .. } => grow(norm([from[0], from[1], to[0], to[1]]), width + 4.0),
            Mark::Note { at, .. } => [at[0], at[1], at[0] + NOTE_SIDE, at[1] + NOTE_SIDE],
        }
    }
    pub fn is_redaction(&self) -> bool {
        matches!(self, Mark::Redact { .. })
    }
    /// Move by (dx, dy) points.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let mv = |r: &mut R4| {
            r[0] += dx;
            r[2] += dx;
            r[1] += dy;
            r[3] += dy;
        };
        match self {
            Mark::Highlight { rects, .. } | Mark::Underline { rects, .. } => rects.iter_mut().for_each(mv),
            Mark::Pen { strokes, .. } => strokes.iter_mut().flatten().for_each(|p| {
                p[0] += dx;
                p[1] += dy
            }),
            Mark::Rect { rect, .. } | Mark::Text { rect, .. } | Mark::Signature { rect, .. } | Mark::Redact { rect } => mv(rect),
            Mark::Arrow { from, to, .. } => {
                for p in [from, to] {
                    p[0] += dx;
                    p[1] += dy;
                }
            }
            Mark::Note { at, .. } => {
                at[0] += dx;
                at[1] += dy;
            }
        }
    }
}

/// Side of a comment icon, in points.
pub const NOTE_SIDE: f32 = 18.0;

/// A mark on a source page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacedMark {
    pub src: usize,
    pub page: usize,
    pub mark: Mark,
}

/// Everything the user did in the studio.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfEdit {
    /// Source documents; `sources[0]` is the PDF that was opened.
    pub sources: Vec<PathBuf>,
    /// The pages of the result, in order.
    pub pages: Vec<PageRef>,
    pub marks: Vec<PlacedMark>,
    /// Form field values (fields of `sources[0]`), by field name.
    pub fields: BTreeMap<String, String>,
    /// Author of annotations and comments.
    pub reviewer: String,
}

impl PdfEdit {
    pub fn new(path: PathBuf, pages: usize) -> PdfEdit {
        PdfEdit { sources: vec![path], pages: (0..pages).map(|page| PageRef { src: 0, page, rotate: 0 }).collect(), ..Default::default() }
    }

    /// The result is the first source, all pages in their order (rotations aside).
    pub fn same_page_order(&self, src0_pages: usize) -> bool {
        self.pages.len() == src0_pages && self.pages.iter().enumerate().all(|(i, p)| p.src == 0 && p.page == i)
    }

    /// Marks on a source page.
    pub fn marks_on(&self, src: usize, page: usize) -> impl Iterator<Item = (usize, &Mark)> {
        self.marks.iter().enumerate().filter(move |(_, m)| m.src == src && m.page == page).map(|(i, m)| (i, &m.mark))
    }

    pub fn redactions(&self) -> usize {
        self.marks.iter().filter(|m| m.mark.is_redaction()).count()
    }

    /// Marks on pages that are still part of the result.
    pub fn live_marks(&self) -> impl Iterator<Item = &PlacedMark> {
        self.marks.iter().filter(|m| self.pages.iter().any(|p| p.src == m.src && p.page == m.page))
    }
}

pub fn norm(r: R4) -> R4 {
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

fn grow(r: R4, d: f32) -> R4 {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}

pub fn union_all(it: impl Iterator<Item = R4>) -> R4 {
    let mut out: Option<R4> = None;
    for r in it {
        let r = norm(r);
        out = Some(match out {
            None => r,
            Some(o) => [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])],
        });
    }
    out.unwrap_or([0.0; 4])
}

fn rect(r: R4) -> Rect {
    let r = norm(r);
    Rect { x0: r[0], y0: r[1], x1: r[2], y1: r[3] }
}

fn color(c: Rgb) -> AnnotationColor {
    AnnotationColor::Rgb { red: c[0], green: c[1], blue: c[2] }
}

// ---- rotation helpers -------------------------------------------------------------------

/// Size of a `w × h` page once rotated by `rot` degrees clockwise.
pub fn rotated_size(w: f32, h: f32, rot: i32) -> (f32, f32) {
    if rot.rem_euclid(180) == 90 { (h, w) } else { (w, h) }
}

/// Source page point → displayed point (page `w × h`, extra rotation `rot` clockwise).
pub fn to_view(p: [f32; 2], w: f32, h: f32, rot: i32) -> [f32; 2] {
    let [x, y] = p;
    match rot.rem_euclid(360) {
        90 => [h - y, x],
        180 => [w - x, h - y],
        270 => [y, w - x],
        _ => [x, y],
    }
}

/// Displayed point → source page point.
pub fn from_view(p: [f32; 2], w: f32, h: f32, rot: i32) -> [f32; 2] {
    let [u, v] = p;
    match rot.rem_euclid(360) {
        90 => [v, h - u],
        180 => [w - u, h - v],
        270 => [w - v, u],
        _ => [u, v],
    }
}

pub fn rect_to_view(r: R4, w: f32, h: f32, rot: i32) -> R4 {
    let a = to_view([r[0], r[1]], w, h, rot);
    let b = to_view([r[2], r[3]], w, h, rot);
    norm([a[0], a[1], b[0], b[1]])
}

// ---- renderer thread --------------------------------------------------------------------

/// A rendered page bitmap: longest side `px` device pixels, `rotate` applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub src: usize,
    pub page: usize,
    pub rotate: i32,
    pub px: u32,
}

/// A character with its box (text selection for highlight / underline).
#[derive(Clone, Copy, Debug)]
pub struct TChar {
    pub c: char,
    pub r: R4,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Check,
    Radio,
    Choice,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    /// What people read: the field's tooltip (/TU), else the last part of its name.
    pub label: String,
    pub kind: FieldKind,
    pub page: usize,
    pub rect: R4,
    pub value: String,
    /// "On" state of a checkbox / radio button.
    pub on: String,
    pub options: Vec<String>,
    #[allow(dead_code)]
    pub multiline: bool,
}

pub enum Req {
    Open(PathBuf),
    Render(Key),
    Text { src: usize, page: usize },
    Search { needle: String, pages: Vec<(usize, usize)> },
}

pub enum Reply {
    Opened { src: usize, pages: Vec<R4>, path: PathBuf },
    Failed { path: PathBuf, error: String },
    Image { key: Key, width: u32, height: u32, bgra: Vec<u8> },
    Text { src: usize, page: usize, chars: Vec<TChar> },
    Search { needle: String, hits: Vec<(usize, usize, R4)> },
}

/// Handle on the renderer thread (stops when dropped).
pub struct Renderer {
    tx: Sender<Req>,
    pub rx: Receiver<Reply>,
}

impl Renderer {
    pub fn spawn() -> Renderer {
        let (tx, req_rx) = crossbeam_channel::unbounded::<Req>();
        let (rep_tx, rx) = crossbeam_channel::unbounded::<Reply>();
        std::thread::Builder::new()
            .name("pdf-studio-render".into())
            .spawn(move || worker(req_rx, rep_tx))
            .expect("renderer thread");
        Renderer { tx, rx }
    }
    pub fn tx_clone(&self) -> Sender<Req> {
        self.tx.clone()
    }
    pub fn send(&self, r: Req) {
        let _ = self.tx.send(r);
    }
}

fn worker(rx: Receiver<Req>, tx: Sender<Reply>) {
    let mut docs: Vec<mupdf::Document> = Vec::new();
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(r) = rx.try_recv() {
            batch.push(r);
        }
        // Everything but renders first (in order), then renders newest first, one size per page.
        let mut renders: Vec<Key> = Vec::new();
        for r in batch {
            match r {
                Req::Render(k) => {
                    renders.retain(|x| !(x.src == k.src && x.page == k.page && x.rotate == k.rotate && x.px != k.px && x.px > 400 && k.px > 400));
                    if !renders.contains(&k) {
                        renders.push(k);
                    }
                }
                Req::Open(path) => {
                    let res = engine_pdf::open_document(&path, "").and_then(|d| {
                        let n = d.page_count()?.max(0) as usize;
                        let mut pages = Vec::with_capacity(n);
                        for i in 0..n {
                            let b = d.load_page(i as i32)?.bounds()?;
                            pages.push([b.x0, b.y0, b.x1, b.y1]);
                        }
                        Ok((d, pages))
                    });
                    match res {
                        Ok((d, pages)) => {
                            docs.push(d);
                            let _ = tx.send(Reply::Opened { src: docs.len() - 1, pages, path });
                        }
                        Err(e) => {
                            let _ = tx.send(Reply::Failed { path, error: format!("{e:#}") });
                        }
                    }
                }
                Req::Text { src, page } => {
                    let chars = docs.get(src).map(|d| page_chars(d, page).unwrap_or_default()).unwrap_or_default();
                    let _ = tx.send(Reply::Text { src, page, chars });
                }
                Req::Search { needle, pages } => {
                    let mut hits = Vec::new();
                    for (src, page) in pages {
                        let Some(d) = docs.get(src) else { continue };
                        let Ok(p) = d.load_page(page as i32) else { continue };
                        if let Ok(qs) = p.search(&needle, 500) {
                            for q in qs.iter() {
                                hits.push((src, page, quad_rect(q)));
                            }
                        }
                    }
                    let _ = tx.send(Reply::Search { needle, hits });
                }
            }
        }
        for k in renders.into_iter().rev() {
            if let Some(d) = docs.get(k.src) {
                if let Ok((width, height, bgra)) = render_page(d, k) {
                    if tx.send(Reply::Image { key: k, width, height, bgra }).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn quad_rect(q: &Quad) -> R4 {
    [q.ul.x.min(q.ll.x), q.ul.y.min(q.ur.y), q.ur.x.max(q.lr.x), q.ll.y.max(q.lr.y)]
}

fn render_page(d: &mupdf::Document, k: Key) -> Result<(u32, u32, Vec<u8>)> {
    let page = d.load_page(k.page as i32)?;
    let b = page.bounds()?;
    let longest = b.width().max(b.height()).max(1.0);
    let s = k.px.max(16) as f32 / longest;
    let pix = page.to_pixmap(&Matrix::new_scale(s, s), &Colorspace::device_rgb(), false, true)?;
    let rgb = engine_pdf::pixmap_to_rgb(&pix);
    let rgb = match k.rotate.rem_euclid(360) {
        90 => image::imageops::rotate90(&rgb),
        180 => image::imageops::rotate180(&rgb),
        270 => image::imageops::rotate270(&rgb),
        _ => rgb,
    };
    let (w, h) = rgb.dimensions();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in rgb.pixels() {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    Ok((w, h, out))
}

fn page_chars(d: &mupdf::Document, page: usize) -> Result<Vec<TChar>> {
    let p = d.load_page(page as i32)?;
    let tp = p.to_text_page(mupdf::TextPageFlags::empty())?;
    let mut out = Vec::new();
    let mut line = 0u32;
    for b in tp.blocks() {
        for l in b.lines() {
            for ch in l.chars() {
                let Some(c) = ch.char() else { continue };
                out.push(TChar { c, r: quad_rect(&ch.quad()), line });
            }
            line += 1;
        }
    }
    Ok(out)
}

/// Form fields of a PDF (text, checkboxes, radio buttons, choices), in page order.
pub fn read_fields(path: &Path) -> Result<Vec<Field>> {
    use mupdf::pdf::WidgetType;
    let doc = engine_pdf::open_pdf(path, "")?;
    let mut out = Vec::new();
    for i in 0..doc.page_count()?.max(0) {
        let page = doc.load_pdf_page(i)?;
        let mut lines: Option<Vec<(String, R4)>> = None;
        for w in page.widgets() {
            let Some(name) = w.name().ok().flatten() else { continue };
            let kind = match w.r#type() {
                Ok(WidgetType::Text) => FieldKind::Text,
                Ok(WidgetType::Checkbox) => FieldKind::Check,
                Ok(WidgetType::RadioButton) => FieldKind::Radio,
                Ok(WidgetType::Combobox | WidgetType::Listbox) => FieldKind::Choice,
                _ => continue,
            };
            if w.is_readonly().unwrap_or(false) {
                continue;
            }
            let Ok(r) = w.annotation().rect() else { continue };
            let value = w.value().ok().flatten().unwrap_or_default();
            let on = on_states(&w).into_iter().next().unwrap_or_default();
            let multiline = w.field_flags().map(|f| f.contains(mupdf::pdf::FieldFlags::MULTILINE)).unwrap_or(false);
            let options = choice_options(&w);
            let mut label = w.label().ok().flatten().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).map(|l| short_name(&l)).unwrap_or_else(|| short_name(&name));
            if !label.contains(' ') {
                // No tooltip, only an internal name: use what is printed next to the field.
                let lines = lines.get_or_insert_with(|| text_lines(&doc, i as usize));
                if let Some(t) = printed_label(lines, [r.x0, r.y0, r.x1, r.y1], kind == FieldKind::Check || kind == FieldKind::Radio) {
                    label = t;
                }
            }
            out.push(Field { label, name, kind, page: i as usize, rect: [r.x0, r.y0, r.x1, r.y1], value, on, options, multiline });
        }
    }
    Ok(out)
}

/// Text lines of a page with their boxes.
fn text_lines(doc: &mupdf::Document, page: usize) -> Vec<(String, R4)> {
    let mut out: Vec<(u32, String, R4)> = Vec::new();
    for c in page_chars(doc, page).unwrap_or_default() {
        match out.last_mut() {
            Some((l, t, r)) if *l == c.line => {
                t.push(c.c);
                *r = union_all([*r, c.r].into_iter());
            }
            _ => out.push((c.line, c.c.to_string(), c.r)),
        }
    }
    out.into_iter().map(|(_, t, r)| (t.split_whitespace().collect::<Vec<_>>().join(" "), r)).filter(|(t, _)| t.chars().any(|c| c.is_alphabetic())).collect()
}

/// The words printed next to a field: right of a checkbox, else above or left of a box.
pub fn printed_label(lines: &[(String, R4)], f: R4, check: bool) -> Option<String> {
    let (fy, fh) = ((f[1] + f[3]) / 2.0, (f[3] - f[1]).max(6.0));
    let same_row = |r: &R4| ((r[1] + r[3]) / 2.0 - fy).abs() < fh.max(8.0) * 0.8;
    let overlaps_x = |r: &R4| r[0] < f[2] && r[2] > f[0];
    let right = lines.iter().filter(|(_, r)| same_row(r) && r[0] >= f[2] - 1.0 && r[0] - f[2] < 40.0).min_by(|a, b| a.1[0].total_cmp(&b.1[0]));
    let above = lines.iter().filter(|(_, r)| overlaps_x(r) && r[3] <= f[1] + 3.0 && f[1] - r[3] < 16.0).max_by(|a, b| a.1[3].total_cmp(&b.1[3]));
    let left = lines.iter().filter(|(_, r)| same_row(r) && r[2] <= f[0] + 1.0 && f[0] - r[2] < 60.0).max_by(|a, b| a.1[2].total_cmp(&b.1[2]));
    let pick = if check { right.or(left).or(above) } else { above.or(left).or(right) }?;
    // A label over several lines: start from its first line.
    let mut block = vec![pick];
    while let Some(up) = lines.iter().filter(|(_, r)| {
        let top = block.last().unwrap().1;
        (r[0] - top[0]).abs() < 15.0 && r[3] <= top[1] + 1.0 && top[1] - r[3] < 3.5
    }).max_by(|a, b| a.1[3].total_cmp(&b.1[3])) {
        if block.len() >= 4 || block.iter().any(|b| std::ptr::eq(*b, up)) {
            break;
        }
        block.push(up);
    }
    let text: String = block.iter().rev().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" ");
    let text = text.trim_end_matches([':', ' ']);
    let mut t: String = text.chars().take(70).collect();
    if text.chars().count() > 70 {
        t = format!("{}…", t.trim_end());
    }
    (t.chars().count() >= 2).then_some(t)
}

/// "topmostSubform[0].Page1[0].f1_01[0]" → "f1_01" (labels that are sentences stay whole).
fn short_name(n: &str) -> String {
    if n.contains(' ') {
        return n.to_string();
    }
    let last = n.rsplit('.').next().unwrap_or(n);
    last.split('[').next().unwrap_or(last).to_string()
}

fn on_states(w: &mupdf::pdf::PdfWidget) -> Vec<String> {
    let mut out = Vec::new();
    let obj = w.annotation().object();
    if let Ok(Some(ap)) = obj.get_dict("AP") {
        if let Ok(Some(n)) = ap.get_dict("N") {
            for i in 0..n.dict_len().unwrap_or(0) {
                if let Ok(Some(k)) = n.get_dict_key(i as i32) {
                    if let Ok(name) = k.as_name() {
                        let s = String::from_utf8_lossy(&name).into_owned();
                        if s != "Off" {
                            out.push(s);
                        }
                    }
                }
            }
        }
    }
    out
}

fn choice_options(w: &mupdf::pdf::PdfWidget) -> Vec<String> {
    let mut out = Vec::new();
    let obj = w.annotation().object();
    if let Ok(Some(opt)) = obj.get_dict_inheritable("Opt") {
        for i in 0..opt.len().unwrap_or(0) {
            if let Ok(Some(item)) = opt.get_array(i as i32) {
                // [export, display] pairs or plain strings.
                let s = if item.is_array().unwrap_or(false) {
                    item.get_array(1).ok().flatten().and_then(|d| d.as_string().ok()).unwrap_or_default()
                } else {
                    item.as_string().unwrap_or_default()
                };
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
    }
    out
}

// ---- saving -----------------------------------------------------------------------------

/// What a save did, for the run note.
#[derive(Debug, Default)]
pub struct Applied {
    pub pages: usize,
    pub annotations: usize,
    pub redactions: usize,
    pub signatures: usize,
    pub fields: usize,
}

fn redact_options() -> PdfRedactOptions {
    PdfRedactOptions {
        black_boxes: false,
        image_method: PdfRedactImageMethod::Pixels,
        line_art: PdfRedactLineArtMethod::RemoveIfCovered,
        text: PdfRedactTextMethod::Remove,
    }
}

/// Write the edited PDF to `out`.
pub fn apply(edit: &PdfEdit, out: &Path, cx: &RunCx) -> Result<Applied> {
    anyhow::ensure!(!edit.sources.is_empty(), "nothing to save");
    anyhow::ensure!(!edit.pages.is_empty(), "{}", castor_core::tr("app.pdf.err.no_pages"));
    let mut docs: Vec<PdfDocument> = Vec::new();
    for p in &edit.sources {
        docs.push(engine_pdf::open_pdf(p, "").with_context(|| p.display().to_string())?);
    }
    let mut done = Applied { pages: edit.pages.len(), ..Default::default() };

    // 1. Marks, grouped by source page (only pages that are kept).
    let mut by_page: BTreeMap<(usize, usize), Vec<&Mark>> = BTreeMap::new();
    for m in edit.live_marks() {
        by_page.entry((m.src, m.page)).or_default().push(&m.mark);
    }
    let n = by_page.len().max(1);
    for (k, ((src, page), marks)) in by_page.into_iter().enumerate() {
        cx.check_cancel()?;
        let doc = docs.get_mut(src).context("source")?;
        apply_marks(doc, page, &marks, &edit.reviewer, &mut done)?;
        cx.progress(0.6 * (k + 1) as f32 / n as f32);
    }

    // 2. Form fields of the first source.
    if !edit.fields.is_empty() {
        done.fields = fill_fields(&mut docs[0], &edit.fields)?;
    }

    // 3. Page order and rotation.
    let src0_pages = docs[0].page_count()?.max(0) as usize;
    let mut dst = if edit.same_page_order(src0_pages) {
        docs.swap_remove(0)
    } else {
        let mut dst = PdfDocument::new();
        let mut run: Vec<usize> = Vec::new();
        let mut run_src = edit.pages[0].src;
        let flush = |dst: &mut PdfDocument, docs: &mut Vec<PdfDocument>, src: usize, run: &mut Vec<usize>| -> Result<()> {
            if !run.is_empty() {
                engine_pdf::pages::copy_pages(dst, &mut docs[src], run, None)?;
                run.clear();
            }
            Ok(())
        };
        for p in &edit.pages {
            if p.src != run_src {
                flush(&mut dst, &mut docs, run_src, &mut run)?;
                run_src = p.src;
            }
            run.push(p.page);
        }
        flush(&mut dst, &mut docs, run_src, &mut run)?;
        // Keep the metadata of the opened document.
        for k in ["Title", "Author", "Subject", "Keywords", "Creator", "CreationDate"] {
            let v = engine_pdf::get_info(&docs[0], k);
            if !v.is_empty() {
                engine_pdf::set_info(&mut dst, k, &v)?;
            }
        }
        dst
    };
    for (i, p) in edit.pages.iter().enumerate() {
        if p.rotate.rem_euclid(360) != 0 {
            let mut page = dst.load_pdf_page(i as i32)?;
            let r = page.rotation()?;
            page.set_rotation((r + p.rotate).rem_euclid(360))?;
        }
    }
    cx.progress(0.8);
    engine_pdf::stamp_producer(&mut dst)?;
    // garbage 4 after redactions: removed text and images must really be gone from the file.
    let garbage = if done.redactions > 0 { 4 } else { 3 };
    engine_pdf::save_pdf(&dst, out, &engine_pdf::Save { garbage, ..Default::default() })?;
    cx.progress(1.0);
    Ok(done)
}

fn pt(p: [f32; 2]) -> Point {
    Point { x: p[0], y: p[1] }
}

fn apply_marks(doc: &mut PdfDocument, page_no: usize, marks: &[&Mark], reviewer: &str, done: &mut Applied) -> Result<()> {
    let mut page = doc.load_pdf_page(page_no as i32)?;
    // Redactions first: they remove content; what is added afterwards stays.
    let boxes: Vec<R4> = marks.iter().filter_map(|m| if let Mark::Redact { rect } = m { Some(norm(*rect)) } else { None }).collect();
    if !boxes.is_empty() {
        for r in &boxes {
            page.add_redact_annotation(AnnotationArea::Rect(rect(*r)))?;
        }
        page.apply_redactions_with_options(redact_options())?;
        let mut canvas = engine_pdf::draw::Canvas::new(&page)?;
        for r in &boxes {
            canvas.rect(doc, &mut page, rect(*r), Some([0.0, 0.0, 0.0]), None, 1.0)?;
        }
        canvas.commit(doc, &mut page, true)?;
        // Verify: no character may remain under a box.
        let left = page_chars(doc, page_no)?;
        for c in left {
            let (cx_, cy_) = ((c.r[0] + c.r[2]) / 2.0, (c.r[1] + c.r[3]) / 2.0);
            if !c.c.is_whitespace() && boxes.iter().any(|b| cx_ > b[0] && cx_ < b[2] && cy_ > b[1] && cy_ < b[3]) {
                anyhow::bail!("{}", castor_core::tr_args("app.pdf.err.redact_verify", &[("page", &(page_no + 1).to_string())]));
            }
        }
        done.redactions += boxes.len();
    }
    let mut canvas = engine_pdf::draw::Canvas::new(&page)?;
    for m in marks {
        let annot = match m {
            Mark::Redact { .. } => None,
            Mark::Highlight { rects, color: c } | Mark::Underline { rects, color: c } => {
                let quads: Vec<Quad> = rects.iter().map(|r| Quad::from(rect(*r))).collect();
                if quads.is_empty() {
                    continue;
                }
                let mut a = if matches!(m, Mark::Highlight { .. }) { page.add_highlight_annotation(quads)? } else { page.add_underline_annotation(quads)? };
                a.set_color(color(*c))?;
                Some(a)
            }
            Mark::Pen { strokes, color: c, width } => {
                let strokes: Vec<Vec<Point>> = strokes.iter().filter(|s| !s.is_empty()).map(|s| s.iter().map(|p| pt(*p)).collect()).collect();
                if strokes.is_empty() {
                    continue;
                }
                let mut a = page.add_ink_annotation(strokes)?;
                a.set_color(color(*c))?;
                a.set_border_width(*width)?;
                Some(a)
            }
            Mark::Rect { rect: r, color: c, width } => {
                let mut a = page.add_rect_annotation(rect(*r))?;
                a.set_color(color(*c))?;
                a.set_border_width(*width)?;
                Some(a)
            }
            Mark::Arrow { from, to, color: c, width } => {
                let mut a = page.add_line_annotation(pt(*from), pt(*to))?;
                a.set_line_ending_styles(LineEndingStyle::None, LineEndingStyle::OpenArrow)?;
                a.set_color(color(*c))?;
                a.set_border_width(*width)?;
                Some(a)
            }
            Mark::Note { at, text, color: c } => {
                let r = Rect { x0: at[0], y0: at[1], x1: at[0] + NOTE_SIDE, y1: at[1] + NOTE_SIDE };
                let mut a = page.add_text_annotation(r, text)?;
                a.set_color(color(*c))?;
                Some(a)
            }
            Mark::Text { rect: r, text, color: c, size } => {
                let mut a = page.add_free_text_annotation(rect(*r), text)?;
                a.set_default_appearance("Helv", *size, Some(color(*c)))?;
                Some(a)
            }
            Mark::Signature { rect: r, sig, date } => {
                draw_signature(doc, &mut page, &mut canvas, norm(*r), sig, date.as_deref())?;
                done.signatures += 1;
                None
            }
        };
        if let Some(mut a) = annot {
            if !reviewer.trim().is_empty() {
                a.set_author(reviewer.trim())?;
            }
            let _ = a.update();
            done.annotations += 1;
        }
    }
    canvas.commit(doc, &mut page, true)?;
    Ok(())
}

fn draw_signature(doc: &mut PdfDocument, page: &mut PdfPage, canvas: &mut engine_pdf::draw::Canvas, r: R4, sig: &Signature, date: Option<&str>) -> Result<()> {
    use engine_pdf::fonts::Face;
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let date_size = (w / 14.0).clamp(6.0, 11.0);
    let date_h = if date.is_some() { date_size * 1.6 } else { 0.0 };
    let sig_h = (h - date_h).max(4.0);
    match sig {
        Signature::Image { path } => {
            let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
            let img = mupdf::Image::from_bytes(&bytes)?;
            // Fit the image in the box, keeping its proportions.
            let (iw, ih) = (img.width().max(1) as f32, img.height().max(1) as f32);
            let s = (w / iw).min(sig_h / ih);
            let (dw, dh) = (iw * s, ih * s);
            let obj = doc.add_image(&img)?;
            let x = r[0] + (w - dw) / 2.0;
            canvas.image_obj(doc, page, &obj, Rect { x0: x, y0: r[1], x1: x + dw, y1: r[1] + dh }, 0.0, 1.0)?;
        }
        Signature::Typed { name, color } => {
            let w1 = canvas.measure(doc, page, Face::Chorus, 1.0, name)?.max(0.01);
            let (asc, desc) = canvas.metrics(doc, page, Face::Chorus)?;
            let size = (w / w1).min(sig_h / (asc - desc).max(0.1));
            let x = r[0] + (w - w1 * size) / 2.0;
            canvas.text(doc, page, Face::Chorus, size, name, x, r[1] + asc * size, 0.0, *color, 1.0)?;
        }
    }
    if let Some(d) = date {
        canvas.text(doc, page, Face::Helvetica, date_size, d, r[0], r[3] - date_size * 0.4, 0.0, [0.25, 0.25, 0.25], 1.0)?;
    }
    Ok(())
}

fn truthy(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on" | "x" | "oui" | "checked")
}

fn fill_fields(doc: &mut PdfDocument, values: &BTreeMap<String, String>) -> Result<usize> {
    use mupdf::pdf::WidgetType;
    let mut set = 0;
    for i in 0..doc.page_count()?.max(0) {
        let page = doc.load_pdf_page(i)?;
        let widgets: Vec<mupdf::pdf::PdfWidget> = page.widgets().collect();
        for mut w in widgets {
            let Some(name) = w.name().ok().flatten() else { continue };
            let Some(val) = values.get(&name) else { continue };
            let value = match w.r#type().unwrap_or(WidgetType::Unknown) {
                WidgetType::Checkbox => {
                    let states = on_states(&w);
                    if truthy(val) || states.iter().any(|s| s == val) { states.first().cloned().unwrap_or_else(|| "Yes".into()) } else { "Off".into() }
                }
                WidgetType::RadioButton => {
                    let states = on_states(&w);
                    if states.iter().any(|s| s == val) { val.clone() } else { continue }
                }
                WidgetType::Signature | WidgetType::Button | WidgetType::Unknown => continue,
                _ => val.clone(),
            };
            if w.set_value(doc, &value, true).unwrap_or(false) {
                let _ = w.update();
                set += 1;
            }
        }
    }
    Ok(set)
}

/// Saved signatures (`{data}/signatures`, shared with the Sign PDF tool): typed names (`.txt`)
/// and images, newest first.
pub fn saved_signatures() -> Vec<Signature> {
    let dir = signatures_dir();
    let mut items: Vec<(std::time::SystemTime, Signature)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
                Some("txt") => {
                    if let Ok(name) = std::fs::read_to_string(&p) {
                        let name = name.trim().to_string();
                        if !name.is_empty() {
                            items.push((t, Signature::Typed { name, color: [0.106, 0.165, 0.42] }));
                        }
                    }
                }
                Some("png" | "jpg" | "jpeg") => items.push((t, Signature::Image { path: p })),
                _ => {}
            }
        }
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.into_iter().map(|x| x.1).collect()
}

pub fn signatures_dir() -> PathBuf {
    castor_core::paths::data_dir().join("signatures")
}

/// Save a drawn signature (strokes in a `w × h` box) as a transparent PNG, 4× for crisp print.
pub fn save_drawn_signature(strokes: &[Vec<[f32; 2]>], w: f32, h: f32, ink: Rgb) -> Result<PathBuf> {
    let scale = 4.0;
    let pts: Vec<[f32; 2]> = strokes.iter().flatten().copied().collect();
    anyhow::ensure!(!pts.is_empty(), "empty signature");
    // Crop to the ink with a small margin.
    let b = union_all(pts.iter().map(|p| [p[0], p[1], p[0], p[1]]));
    let pad = 6.0;
    let (x0, y0) = ((b[0] - pad).max(0.0), (b[1] - pad).max(0.0));
    let (x1, y1) = ((b[2] + pad).min(w), (b[3] + pad).min(h));
    let (iw, ih) = (((x1 - x0) * scale).ceil().max(8.0) as u32, ((y1 - y0) * scale).ceil().max(8.0) as u32);
    let mut img = image::RgbaImage::new(iw, ih);
    let ink_px = [(ink[0] * 255.0) as u8, (ink[1] * 255.0) as u8, (ink[2] * 255.0) as u8];
    let radius = 1.4 * scale;
    let mut dot = |cx: f32, cy: f32| {
        let (xa, xb) = ((cx - radius - 1.0).max(0.0) as u32, ((cx + radius + 1.0) as u32).min(iw - 1));
        let (ya, yb) = ((cy - radius - 1.0).max(0.0) as u32, ((cy + radius + 1.0) as u32).min(ih - 1));
        for y in ya..=yb {
            for x in xa..=xb {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = ((radius + 0.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
                let p = img.get_pixel_mut(x, y);
                if a > p[3] {
                    *p = image::Rgba([ink_px[0], ink_px[1], ink_px[2], a]);
                }
            }
        }
    };
    for s in strokes {
        for seg in s.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt() * scale;
            let steps = (len / 1.5).ceil().max(1.0) as usize;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                dot((a[0] + (b[0] - a[0]) * t - x0) * scale, (a[1] + (b[1] - a[1]) * t - y0) * scale);
            }
        }
        if s.len() == 1 {
            dot((s[0][0] - x0) * scale, (s[0][1] - y0) * scale);
        }
    }
    let dir = signatures_dir();
    std::fs::create_dir_all(&dir)?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = castor_core::job::unique_path(&dir.join(format!("drawn-{secs}.png")));
    img.save(&path)?;
    Ok(path)
}

/// A typed signature kept for next time (same format as the Sign PDF tool).
pub fn save_typed_signature(name: &str) -> Result<PathBuf> {
    let dir = signatures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = castor_core::job::unique_path(&dir.join(format!("{}.txt", castor_core::job::sanitize_file_name(name))));
    std::fs::write(&path, name)?;
    Ok(path)
}

/// Copy an imported signature image into the signatures folder.
pub fn import_signature(src: &Path) -> Result<PathBuf> {
    let dir = signatures_dir();
    std::fs::create_dir_all(&dir)?;
    // Knock the white paper out so the ink sits on the page.
    let img = image::open(src).with_context(|| src.display().to_string())?;
    let mut rgba = img.to_rgba8();
    for p in rgba.pixels_mut() {
        let l = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
        let a = if l >= 235 { 0 } else if l <= 170 { 255 } else { (235 - l) * 255 / 65 };
        p[3] = (p[3] as u32).min(a) as u8;
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "signature".into());
    let path = castor_core::job::unique_path(&dir.join(format!("{}.png", castor_core::job::sanitize_file_name(&stem))));
    rgba.save(&path)?;
    Ok(path)
}

/// Today's date for the date line under a signature (localised format).
pub fn today() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    castor_core::tr_args(
        "pdf.sign.date_fmt",
        &[("y", &format!("{:04}", now.year())), ("m", &format!("{:02}", now.month() as u8)), ("d", &format!("{:02}", now.day()))],
    )
}

/// Pages of every source, for searches.
pub fn all_pages(edit: &PdfEdit) -> Vec<(usize, usize)> {
    let mut seen = HashMap::new();
    edit.pages.iter().filter(|p| seen.insert((p.src, p.page), ()).is_none()).map(|p| (p.src, p.page)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_round_trip() {
        for rot in [0, 90, 180, 270] {
            let p = [12.0, 30.0];
            let v = to_view(p, 200.0, 100.0, rot);
            let back = from_view(v, 200.0, 100.0, rot);
            assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4, "{rot}");
        }
        // Clockwise: the source's top-left corner ends top-right.
        assert_eq!(to_view([0.0, 0.0], 200.0, 100.0, 90), [100.0, 0.0]);
        assert_eq!(rotated_size(200.0, 100.0, 270), (100.0, 200.0));
    }

    #[test]
    fn edit_script_round_trips_as_json() {
        let mut e = PdfEdit::new("a.pdf".into(), 3);
        e.pages.swap(0, 2);
        e.marks.push(PlacedMark { src: 0, page: 1, mark: Mark::Redact { rect: [10.0, 10.0, 50.0, 20.0] } });
        e.marks.push(PlacedMark { src: 0, page: 2, mark: Mark::Signature { rect: [0.0; 4], sig: Signature::Typed { name: "B".into(), color: [0.0; 3] }, date: None } });
        let s = serde_json::to_string(&e).unwrap();
        let back: PdfEdit = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
        assert!(!e.same_page_order(3));
        assert_eq!(e.redactions(), 1);
    }
}
