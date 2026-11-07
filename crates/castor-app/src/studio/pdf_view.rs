//! PDF Studio: one PDF, page thumbnails on the left, the pages in the middle, the tools of the
//! current mode on the right, and a bottom bar with the primary action. Modes: Organize,
//! Annotate, Sign, Redact, Forms. Every change is a step of the edit script; Save runs it
//! through the runner (history, undo for "Overwrite").

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use castor_core::history::{RunRecord, RunStatus};
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::FileInfo;
use crossbeam_channel::Sender;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::pdf_engine::{
    self as eng, Field, FieldKind, Key, Mark, PageRef, PdfEdit, PlacedMark, R4, Renderer, Reply, Req, Rgb, Signature, TChar,
};
use crate::model::model;
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t, ta};

const RAIL_W: f32 = 148.0;
const PANEL_W: f32 = 268.0;
const GAP: f32 = 18.0;
const THUMB_PX: u32 = 220;
const MAX_IMAGES: usize = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Organize,
    Annotate,
    Sign,
    Redact,
    Forms,
}

impl Mode {
    const ALL: [Mode; 5] = [Mode::Organize, Mode::Annotate, Mode::Sign, Mode::Redact, Mode::Forms];
    fn key(self) -> &'static str {
        match self {
            Mode::Organize => "organize",
            Mode::Annotate => "annotate",
            Mode::Sign => "sign",
            Mode::Redact => "redact",
            Mode::Forms => "forms",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pen {
    Select,
    Highlight,
    Underline,
    Draw,
    Rect,
    Arrow,
    Text,
    Note,
}

impl Pen {
    const ALL: [Pen; 8] = [Pen::Select, Pen::Highlight, Pen::Underline, Pen::Draw, Pen::Rect, Pen::Arrow, Pen::Text, Pen::Note];
    fn key(self) -> &'static str {
        match self {
            Pen::Select => "select",
            Pen::Highlight => "highlight",
            Pen::Underline => "underline",
            Pen::Draw => "pen",
            Pen::Rect => "rect",
            Pen::Arrow => "arrow",
            Pen::Text => "text",
            Pen::Note => "note",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Pen::Select => "mouse-pointer",
            Pen::Highlight => "highlighter",
            Pen::Underline => "underline",
            Pen::Draw => "pen-line",
            Pen::Rect => "square",
            Pen::Arrow => "move-up-right",
            Pen::Text => "type",
            Pen::Note => "message-square",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fit {
    Width,
    Page,
    Custom,
}

/// What the mouse is doing on a page (source page coordinates).
#[derive(Clone, Debug)]
enum Drag {
    Box { page: usize, a: [f32; 2], b: [f32; 2] },
    Pen { page: usize, points: Vec<[f32; 2]> },
    Text { page: usize, from: usize, to: usize },
    Move { mark: usize, last: [f32; 2], moved: bool },
    Resize { mark: usize },
}

/// A page being dragged in the thumbnails rail.
#[derive(Clone)]
struct DraggedPage(usize, SharedString);

impl Render for DraggedPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        div().px(px(8.0)).py(px(4.0)).bg(k.raised).border_1().border_color(k.link).rounded(px(3.0)).child(ui::sans().text_size(px(13.0)).child(self.1.clone()))
    }
}

/// Rendered bitmaps and text, shared with the list closures (UI thread only).
struct Cache {
    tx: Option<Sender<Req>>,
    images: HashMap<Key, Arc<RenderImage>>,
    lru: VecDeque<Key>,
    pending: HashSet<Key>,
    evicted: Vec<Arc<RenderImage>>,
    texts: HashMap<(usize, usize), Rc<Vec<TChar>>>,
    text_pending: HashSet<(usize, usize)>,
    /// Page element bounds of the last frame, by page index (mouse → page coordinates).
    bounds: HashMap<usize, Bounds<Pixels>>,
}

impl Cache {
    fn image(&mut self, k: Key) -> Option<Arc<RenderImage>> {
        if let Some(i) = self.images.get(&k).cloned() {
            if let Some(pos) = self.lru.iter().position(|x| *x == k) {
                self.lru.remove(pos);
            }
            self.lru.push_back(k);
            return Some(i);
        }
        if !self.pending.contains(&k) {
            if let Some(tx) = &self.tx {
                let _ = tx.send(Req::Render(k));
                self.pending.insert(k);
            }
        }
        // While the right size renders, show any other size of the same page.
        self.images.iter().filter(|(x, _)| x.src == k.src && x.page == k.page && x.rotate == k.rotate).max_by_key(|(x, _)| x.px).map(|(_, i)| i.clone())
    }

    fn text(&mut self, src: usize, page: usize) -> Option<Rc<Vec<TChar>>> {
        if let Some(t) = self.texts.get(&(src, page)) {
            return Some(t.clone());
        }
        if self.text_pending.insert((src, page)) {
            if let Some(tx) = &self.tx {
                let _ = tx.send(Req::Text { src, page });
            }
        }
        None
    }

    fn insert(&mut self, k: Key, img: Arc<RenderImage>) {
        self.pending.remove(&k);
        if let Some(old) = self.images.insert(k, img) {
            self.evicted.push(old);
        }
        self.lru.push_back(k);
        while self.images.len() > MAX_IMAGES {
            let Some(old) = self.lru.pop_front() else { break };
            if let Some(i) = self.images.remove(&old) {
                self.evicted.push(i);
            }
        }
    }
}

/// A save in progress or done.
struct Saving {
    handle: Option<runner::RunHandle>,
    frac: f32,
    record: Option<RunRecord>,
    overwrite: bool,
}

pub struct PdfStudio {
    focus: FocusHandle,
    path: Option<PathBuf>,
    edit: PdfEdit,
    undo: Vec<PdfEdit>,
    redo: Vec<PdfEdit>,
    clean: PdfEdit,
    /// Page boxes of each source (points).
    sizes: Vec<Vec<R4>>,
    renderer: Option<Renderer>,
    cache: Rc<RefCell<Cache>>,
    error: Option<String>,
    loading: bool,
    mode: Mode,
    pen: Pen,
    color_ix: usize,
    zoom: f32,
    fit: Fit,
    current: usize,
    selected: BTreeSet<usize>,
    sel_mark: Option<usize>,
    drag: Option<Drag>,
    list: UniformListScrollHandle,
    rail: UniformListScrollHandle,
    sigs: Vec<(Signature, Option<(u32, u32)>)>,
    sig_ix: Option<usize>,
    sig_date: bool,
    fields: Option<Vec<Field>>,
    field_inputs: HashMap<String, Entity<InputState>>,
    search: Entity<InputState>,
    search_note: Option<String>,
    reviewer: Entity<InputState>,
    mark_text: Entity<InputState>,
    mark_text_for: Option<usize>,
    saving: Option<Saving>,
    viewport: (f32, f32),
    reopen_after_overwrite: bool,
    _poll: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

/// Highlight presets (family tints) and stroke presets (their deeper shade, plus ink).
fn presets(k: &Tokens, stroke: bool) -> Vec<Hsla> {
    let mut v: Vec<Hsla> = castor_core::Family::ALL.iter().map(|f| if stroke { k.tint_ink(*f) } else { k.tint(*f) }).collect();
    if stroke {
        v.insert(0, gpui_kit::rgb(0x1b1616).into());
        v.insert(1, gpui_kit::rgb(0xc0392b).into());
    } else {
        // Light tints read better as highlights in light mode even in the dark theme.
        v = castor_core::Family::ALL.iter().map(|f| crate::theme::Tokens::new(false).tint(*f)).collect();
        v.rotate_left(4); // Create (yellow) first
    }
    v
}

fn to_rgb(c: Hsla) -> Rgb {
    let r = gpui_kit::Rgba::from(c);
    [r.r, r.g, r.b]
}

fn from_rgb(c: Rgb, a: f32) -> Hsla {
    let mut h: Hsla = gpui_kit::Rgba { r: c[0], g: c[1], b: c[2], a: 1.0 }.into();
    h.a = a;
    h
}

impl PdfStudio {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.pdf.redact.search_ph")));
        let saved_reviewer = model(cx).read(cx).settings.pdf_reviewer.clone();
        let reviewer = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.pdf.reviewer_ph")).default_value(saved_reviewer.clone()));
        let mark_text = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.pdf.text_ph")));
        let subs = vec![
            cx.subscribe(&reviewer, |this, st, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = st.read(cx).value().to_string();
                    this.edit.reviewer = v.clone();
                    model(cx).update(cx, |m, cx| {
                        m.update_settings(cx, move |s| s.pdf_reviewer = v)
                    });
                }
            }),
            cx.subscribe(&mark_text, |this, st, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = st.read(cx).value().to_string();
                    if let Some(i) = this.mark_text_for {
                        if let Some(PlacedMark { mark: Mark::Note { text, .. } | Mark::Text { text, .. }, .. }) = this.edit.marks.get_mut(i) {
                            *text = v;
                            cx.notify();
                        }
                    }
                }
            }),
            cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.search_redact(window, cx);
                }
            }),
        ];
        let mut s = PdfStudio {
            focus: cx.focus_handle(),
            path: None,
            edit: PdfEdit { reviewer: saved_reviewer, ..Default::default() },
            undo: Vec::new(),
            redo: Vec::new(),
            clean: PdfEdit::default(),
            sizes: Vec::new(),
            renderer: None,
            cache: Rc::new(RefCell::new(Cache {
                tx: None,
                images: HashMap::new(),
                lru: VecDeque::new(),
                pending: HashSet::new(),
                evicted: Vec::new(),
                texts: HashMap::new(),
                text_pending: HashSet::new(),
                bounds: HashMap::new(),
            })),
            error: None,
            loading: false,
            mode: Mode::Organize,
            pen: Pen::Highlight,
            color_ix: 0,
            zoom: 1.0,
            fit: Fit::Width,
            current: 0,
            selected: BTreeSet::new(),
            sel_mark: None,
            drag: None,
            list: UniformListScrollHandle::new(),
            rail: UniformListScrollHandle::new(),
            sigs: Vec::new(),
            sig_ix: None,
            sig_date: false,
            fields: None,
            field_inputs: HashMap::new(),
            search,
            search_note: None,
            reviewer,
            mark_text,
            mark_text_for: None,
            saving: None,
            viewport: (800.0, 600.0),
            reopen_after_overwrite: false,
            _poll: None,
            _subs: subs,
        };
        s.load_signatures(cx);
        s
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn set_mode_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(m) = Mode::ALL.iter().find(|m| m.key() == key) {
            self.set_mode(*m, window, cx);
        }
    }

    // ---- opening ------------------------------------------------------------------------

    pub fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.as_ref().is_some_and(|s| s.handle.is_some()) {
            return;
        }
        // Drop the bitmaps of the previous document.
        {
            let mut c = self.cache.borrow_mut();
            let old: Vec<Arc<RenderImage>> = c.images.drain().map(|(_, i)| i).collect();
            c.evicted.extend(old);
            c.lru.clear();
            c.pending.clear();
            c.texts.clear();
            c.text_pending.clear();
            c.bounds.clear();
        }
        let r = Renderer::spawn();
        r.send(Req::Open(path.clone()));
        self.cache.borrow_mut().tx = Some(r.tx_clone());
        self.renderer = Some(r);
        self.path = Some(path);
        self.sizes.clear();
        self.edit = PdfEdit { reviewer: self.edit.reviewer.clone(), ..Default::default() };
        self.clean = self.edit.clone();
        self.undo.clear();
        self.redo.clear();
        self.error = None;
        self.loading = true;
        self.current = 0;
        self.selected.clear();
        self.sel_mark = None;
        self.fields = None;
        self.field_inputs.clear();
        self.saving = None;
        self.fit = Fit::Width;
        self.start_poll(window, cx);
        cx.notify();
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update_in(cx, |s, window, cx| s.open(p, window, cx));
                }
            }
        })
        .detach();
    }

    fn insert_from_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |s, cx| {
                    for p in paths {
                        if let Some(r) = &s.renderer {
                            r.send(Req::Open(p));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(30)).await;
                let alive = this.update_in(cx, |s, window, cx| s.poll(window, cx));
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        let replies: Vec<Reply> = self.renderer.as_ref().map(|r| r.rx.try_iter().take(32).collect()).unwrap_or_default();
        for rep in replies {
            changed = true;
            match rep {
                Reply::Opened { src, pages, path } => {
                    if src == 0 {
                        self.loading = false;
                        self.edit = PdfEdit::new(path, pages.len());
                        self.edit.reviewer = self.reviewer.read(cx).value().to_string();
                        self.clean = self.edit.clone();
                        self.sizes = vec![pages];
                        let setup = super::take_setup(cx);
                        if let Some(script) = setup.script.and_then(|p| std::fs::read_to_string(p).ok()) {
                            match serde_json::from_str::<PdfEdit>(&script) {
                                Ok(mut e) => {
                                    e.sources = self.edit.sources.clone();
                                    if e.pages.is_empty() {
                                        e.pages = self.edit.pages.clone();
                                    }
                                    self.edit = e;
                                }
                                Err(err) => tracing::warn!("--script: {err}"),
                            }
                        }
                        if let Some(m) = setup.mode {
                            self.set_mode_key(&m, window, cx);
                        }
                    } else {
                        // Pages inserted from another PDF, after the current page.
                        self.push_undo();
                        if self.sizes.len() <= src {
                            self.sizes.resize(src + 1, Vec::new());
                        }
                        let at = (self.current + 1).min(self.edit.pages.len());
                        let new: Vec<PageRef> = (0..pages.len()).map(|page| PageRef { src, page, rotate: 0 }).collect();
                        self.sizes[src] = pages;
                        while self.edit.sources.len() < src {
                            self.edit.sources.push(PathBuf::new());
                        }
                        self.edit.sources.push(path);
                        let n = new.len();
                        self.edit.pages.splice(at..at, new);
                        self.selected = (at..at + n).collect();
                        window.push_notification(ta("app.pdf.inserted", &[("count", &n.to_string())]).to_string(), cx);
                    }
                }
                Reply::Failed { path, error } => {
                    if self.sizes.is_empty() {
                        self.loading = false;
                        self.error = Some(error);
                    } else {
                        window.push_notification(format!("{}: {error}", path.display()), cx);
                    }
                }
                Reply::Image { key, width, height, bgra } => {
                    if let Some(buf) = image::RgbaImage::from_raw(width, height, bgra) {
                        let img = Arc::new(RenderImage::new(vec![image::Frame::new(buf)]));
                        self.cache.borrow_mut().insert(key, img);
                    }
                }
                Reply::Text { src, page, chars } => {
                    let mut c = self.cache.borrow_mut();
                    c.text_pending.remove(&(src, page));
                    c.texts.insert((src, page), Rc::new(chars));
                }
                Reply::Search { needle, hits } => {
                    if !hits.is_empty() {
                        self.push_undo();
                        for (src, page, r) in &hits {
                            let r = [r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0];
                            self.edit.marks.push(PlacedMark { src: *src, page: *page, mark: Mark::Redact { rect: r } });
                        }
                    }
                    self.search_note = Some(ta("app.pdf.redact.found", &[("count", &hits.len().to_string()), ("text", &needle)]).to_string());
                }
            }
        }
        let evicted: Vec<Arc<RenderImage>> = std::mem::take(&mut self.cache.borrow_mut().evicted);
        for img in evicted {
            cx.drop_image(img, Some(window));
        }
        if self.poll_save(cx) {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn load_signatures(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move {
            eng::saved_signatures()
                .into_iter()
                .map(|s| {
                    let dims = match &s {
                        Signature::Image { path } => image::image_dimensions(path).ok(),
                        _ => None,
                    };
                    (s, dims)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let sigs = task.await;
            let _ = this.update(cx, |s, cx| {
                s.sigs = sigs;
                if s.sig_ix.is_none() && !s.sigs.is_empty() {
                    s.sig_ix = Some(0);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- editing ------------------------------------------------------------------------

    fn push_undo(&mut self) {
        self.undo.push(self.edit.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.edit, prev));
            self.after_history(cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.edit, next));
            self.after_history(cx);
        }
    }

    fn after_history(&mut self, cx: &mut Context<Self>) {
        self.sel_mark = None;
        self.mark_text_for = None;
        self.selected.retain(|i| *i < self.edit.pages.len());
        self.current = self.current.min(self.edit.pages.len().saturating_sub(1));
        cx.notify();
    }

    fn changes(&self) -> usize {
        let mut n = self.edit.live_marks().count() + self.edit.fields.len();
        if self.edit.pages != self.clean.pages {
            n += 1;
        }
        n
    }

    fn dirty(&self) -> bool {
        self.edit != self.clean
    }

    /// Pages the organize actions apply to: the selection, or the current page.
    fn targets(&self) -> Vec<usize> {
        if self.selected.is_empty() { vec![self.current.min(self.edit.pages.len().saturating_sub(1))] } else { self.selected.iter().copied().collect() }
    }

    fn rotate(&mut self, by: i32, cx: &mut Context<Self>) {
        if self.edit.pages.is_empty() {
            return;
        }
        self.push_undo();
        for i in self.targets() {
            if let Some(p) = self.edit.pages.get_mut(i) {
                p.rotate = (p.rotate + by).rem_euclid(360);
            }
        }
        cx.notify();
    }

    fn delete_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let del = self.targets();
        if del.len() >= self.edit.pages.len() {
            window.push_notification(t("app.pdf.err.no_pages").to_string(), cx);
            return;
        }
        self.push_undo();
        for i in del.iter().rev() {
            self.edit.pages.remove(*i);
        }
        self.selected.clear();
        self.current = self.current.min(self.edit.pages.len() - 1);
        cx.notify();
    }

    /// Move the selected pages (or `from`) before index `to`.
    fn move_pages(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let moving: Vec<usize> = if self.selected.contains(&from) { self.selected.iter().copied().collect() } else { vec![from] };
        if moving.is_empty() || (moving.len() == 1 && (moving[0] == to || moving[0] + 1 == to)) {
            return;
        }
        self.push_undo();
        let pages: Vec<PageRef> = moving.iter().map(|i| self.edit.pages[*i]).collect();
        let before = moving.iter().filter(|i| **i < to).count();
        for i in moving.iter().rev() {
            self.edit.pages.remove(*i);
        }
        let at = (to - before).min(self.edit.pages.len());
        let n = pages.len();
        self.edit.pages.splice(at..at, pages);
        self.selected = (at..at + n).collect();
        self.current = at;
        cx.notify();
    }

    fn select_page(&mut self, i: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        if modifiers.shift {
            let (a, b) = (self.current.min(i), self.current.max(i));
            self.selected = (a..=b).collect();
        } else if modifiers.control || modifiers.platform {
            if !self.selected.insert(i) {
                self.selected.remove(&i);
            }
        } else {
            self.selected.clear();
            self.selected.insert(i);
        }
        self.current = i;
        self.list.scroll_to_item(i, ScrollStrategy::Top);
        cx.notify();
    }

    fn set_mode(&mut self, m: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = m;
        self.drag = None;
        self.sel_mark = None;
        self.mark_text_for = None;
        if m == Mode::Forms && self.fields.is_none() {
            self.load_fields(window, cx);
        }
        cx.notify();
    }

    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        self.fields = Some(Vec::new());
        let task = cx.background_executor().spawn(async move { eng::read_fields(&path) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| {
                let fields = res.unwrap_or_default();
                for f in &fields {
                    if matches!(f.kind, FieldKind::Text | FieldKind::Choice) && !s.field_inputs.contains_key(&f.name) {
                        let name = f.name.clone();
                        let ph: SharedString = if f.options.is_empty() { SharedString::default() } else { f.options.join(" · ").into() };
                        let st = cx.new(|cx| InputState::new(window, cx).default_value(f.value.clone()).placeholder(ph));
                        s._subs.push(cx.subscribe(&st, move |this, st, ev: &InputEvent, cx| {
                            if let InputEvent::Change = ev {
                                let v = st.read(cx).value().to_string();
                                this.edit.fields.insert(name.clone(), v);
                                cx.notify();
                            }
                        }));
                        s.field_inputs.insert(f.name.clone(), st);
                    }
                }
                s.fields = Some(fields);
                cx.notify();
            });
        })
        .detach();
    }

    fn search_redact(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let needle = self.search.read(cx).value().trim().to_string();
        if needle.is_empty() {
            return;
        }
        if let Some(r) = &self.renderer {
            r.send(Req::Search { needle, pages: eng::all_pages(&self.edit) });
        }
        cx.notify();
    }

    fn delete_mark(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.sel_mark.take() {
            if i < self.edit.marks.len() {
                self.push_undo();
                self.edit.marks.remove(i);
                self.mark_text_for = None;
                cx.notify();
            }
        }
    }

    fn clear_redactions(&mut self, cx: &mut Context<Self>) {
        if self.edit.redactions() > 0 {
            self.push_undo();
            self.edit.marks.retain(|m| !m.mark.is_redaction());
            self.sel_mark = None;
            cx.notify();
        }
    }

    // ---- geometry -----------------------------------------------------------------------

    /// Unrotated size of the page shown at index `i`.
    fn page_size(&self, r: PageRef) -> (f32, f32) {
        self.sizes.get(r.src).and_then(|v| v.get(r.page)).map(|b| (b[2] - b[0], b[3] - b[1])).unwrap_or((612.0, 792.0))
    }

    fn disp_size(&self, r: PageRef) -> (f32, f32) {
        let (w, h) = self.page_size(r);
        eng::rotated_size(w, h, r.rotate)
    }

    fn canvas_w(&self) -> f32 {
        (self.viewport.0 - crate::shell::SIDEBAR_W - RAIL_W - PANEL_W - 2.0).max(200.0)
    }

    fn update_fit(&mut self) {
        let max_w = self.edit.pages.iter().map(|p| self.disp_size(*p).0).fold(1.0f32, f32::max);
        let max_h = self.edit.pages.iter().map(|p| self.disp_size(*p).1).fold(1.0f32, f32::max);
        let cw = self.canvas_w() - 2.0 * GAP - 14.0;
        let ch = self.viewport.1 - 44.0 - 52.0 - 44.0 - 2.0 * GAP;
        match self.fit {
            Fit::Width => self.zoom = (cw / max_w).clamp(0.1, 6.0),
            Fit::Page => self.zoom = (cw / max_w).min(ch / max_h).clamp(0.1, 6.0),
            Fit::Custom => {}
        }
    }

    fn zoom_by(&mut self, f: f32, cx: &mut Context<Self>) {
        self.fit = Fit::Custom;
        let steps = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];
        let z = self.zoom;
        self.zoom = if f > 1.0 { steps.iter().copied().find(|s| *s > z + 0.01).unwrap_or(5.0) } else { steps.iter().rev().copied().find(|s| *s < z - 0.01).unwrap_or(0.25) };
        cx.notify();
    }

    /// Window position → (page index, source page point).
    fn hit(&self, pos: Point<Pixels>) -> Option<(usize, [f32; 2])> {
        let c = self.cache.borrow();
        let (i, b) = c.bounds.iter().find(|(_, b)| b.contains(&pos))?;
        let r = *self.edit.pages.get(*i)?;
        let (w, h) = self.page_size(r);
        let u = (pos.x - b.origin.x).as_f32() / self.zoom;
        let v = (pos.y - b.origin.y).as_f32() / self.zoom;
        Some((*i, eng::from_view([u, v], w, h, r.rotate)))
    }

    fn point_in(&self, i: usize, pos: Point<Pixels>) -> Option<[f32; 2]> {
        let c = self.cache.borrow();
        let b = c.bounds.get(&i)?;
        let r = *self.edit.pages.get(i)?;
        let (w, h) = self.page_size(r);
        let u = ((pos.x - b.origin.x).as_f32() / self.zoom).clamp(0.0, eng::rotated_size(w, h, r.rotate).0);
        let v = ((pos.y - b.origin.y).as_f32() / self.zoom).clamp(0.0, eng::rotated_size(w, h, r.rotate).1);
        Some(eng::from_view([u, v], w, h, r.rotate))
    }

    fn nearest_char(chars: &[TChar], p: [f32; 2]) -> Option<usize> {
        let mut best = None;
        let mut best_d = f32::MAX;
        for (i, c) in chars.iter().enumerate() {
            let dx = if p[0] < c.r[0] { c.r[0] - p[0] } else if p[0] > c.r[2] { p[0] - c.r[2] } else { 0.0 };
            let dy = if p[1] < c.r[1] { c.r[1] - p[1] } else if p[1] > c.r[3] { p[1] - c.r[3] } else { 0.0 };
            let d = dx + dy * 3.0;
            if d < best_d {
                best_d = d;
                best = Some(i);
            }
        }
        best.filter(|_| best_d < 40.0)
    }

    /// One rectangle per line for the characters `a..=b`.
    fn line_rects(chars: &[TChar], a: usize, b: usize) -> Vec<R4> {
        let (a, b) = (a.min(b), a.max(b).min(chars.len().saturating_sub(1)));
        let mut out: Vec<(u32, R4)> = Vec::new();
        for c in &chars[a..=b] {
            match out.last_mut() {
                Some((l, r)) if *l == c.line => *r = eng::union_all([*r, c.r].into_iter()),
                _ => out.push((c.line, c.r)),
            }
        }
        out.into_iter().map(|(_, r)| r).filter(|r| r[2] - r[0] > 0.5).collect()
    }

    // ---- mouse --------------------------------------------------------------------------

    fn on_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some((i, p)) = self.hit(ev.position) else { return };
        self.current = i;
        let r = self.edit.pages[i];
        match self.mode {
            Mode::Organize => {
                self.select_page(i, ev.modifiers, cx);
            }
            Mode::Annotate => match self.pen {
                Pen::Select => {
                    self.start_select(i, p, ev.click_count);
                }
                Pen::Highlight | Pen::Underline => {
                    let chars = self.cache.borrow_mut().text(r.src, r.page);
                    match chars.as_ref().and_then(|c| Self::nearest_char(c, p)) {
                        Some(ci) => self.drag = Some(Drag::Text { page: i, from: ci, to: ci }),
                        None => self.drag = Some(Drag::Box { page: i, a: p, b: p }),
                    }
                }
                Pen::Draw => self.drag = Some(Drag::Pen { page: i, points: vec![p] }),
                Pen::Rect | Pen::Arrow | Pen::Text => self.drag = Some(Drag::Box { page: i, a: p, b: p }),
                Pen::Note => {
                    self.push_undo();
                    let color = to_rgb(presets(&tokens(cx), false)[self.color_ix % 6]);
                    let at = [p[0] - eng::NOTE_SIDE / 2.0, p[1] - eng::NOTE_SIDE / 2.0];
                    self.edit.marks.push(PlacedMark { src: r.src, page: r.page, mark: Mark::Note { at, text: String::new(), color } });
                    self.select_mark(self.edit.marks.len() - 1, window, cx);
                }
            },
            Mode::Sign => {
                if self.start_select(i, p, 1) {
                    cx.notify();
                    return;
                }
                if let Some((sig, dims)) = self.sig_ix.and_then(|k| self.sigs.get(k)).cloned() {
                    self.push_undo();
                    let w = 150.0;
                    let h = match dims {
                        Some((iw, ih)) => w * ih as f32 / iw.max(1) as f32,
                        None => 46.0,
                    } + if self.sig_date { 14.0 } else { 0.0 };
                    let rect = [p[0] - w / 2.0, p[1] - h / 2.0, p[0] + w / 2.0, p[1] + h / 2.0];
                    let date = self.sig_date.then(eng::today);
                    self.edit.marks.push(PlacedMark { src: r.src, page: r.page, mark: Mark::Signature { rect, sig, date } });
                    self.sel_mark = Some(self.edit.marks.len() - 1);
                } else {
                    window.push_notification(t("app.pdf.sign.none").to_string(), cx);
                }
            }
            Mode::Redact => {
                if !self.start_select(i, p, 1) {
                    self.drag = Some(Drag::Box { page: i, a: p, b: p });
                }
            }
            Mode::Forms => {
                if let Some(f) = self.fields.as_ref().and_then(|fs| fs.iter().find(|f| r.src == 0 && f.page == r.page && p[0] >= f.rect[0] && p[0] <= f.rect[2] && p[1] >= f.rect[1] && p[1] <= f.rect[3])).cloned() {
                    match f.kind {
                        FieldKind::Check | FieldKind::Radio => {
                            let cur = self.field_value(&f);
                            let on = !(cur == f.on || (f.kind == FieldKind::Check && eng_truthy(&cur)));
                            let v = if on { f.on.clone() } else if f.kind == FieldKind::Check { "Off".into() } else { cur };
                            self.edit.fields.insert(f.name.clone(), v);
                        }
                        _ => {
                            if let Some(inp) = self.field_inputs.get(&f.name) {
                                inp.update(cx, |i, cx| i.focus(window, cx));
                            }
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    /// Select (and start moving) the topmost mark under `p` on page `i`. True when one was hit.
    fn start_select(&mut self, i: usize, p: [f32; 2], clicks: usize) -> bool {
        let r = self.edit.pages[i];
        let mode = self.mode;
        let hit = self
            .edit
            .marks_on(r.src, r.page)
            .filter(|(_, m)| match mode {
                Mode::Sign => matches!(m, Mark::Signature { .. }),
                Mode::Redact => m.is_redaction(),
                _ => !m.is_redaction() && !matches!(m, Mark::Signature { .. }),
            })
            .filter(|(_, m)| {
                let b = m.bounds();
                p[0] >= b[0] - 3.0 && p[0] <= b[2] + 3.0 && p[1] >= b[1] - 3.0 && p[1] <= b[3] + 3.0
            })
            .map(|(k, _)| k)
            .last();
        let Some(k) = hit else {
            self.sel_mark = None;
            self.mark_text_for = None;
            return false;
        };
        let b = self.edit.marks[k].mark.bounds();
        let on_handle = matches!(self.edit.marks[k].mark, Mark::Signature { .. } | Mark::Rect { .. } | Mark::Redact { .. } | Mark::Text { .. })
            && (p[0] - b[2]).abs() < 8.0 / self.zoom.max(0.2) + 4.0
            && (p[1] - b[3]).abs() < 8.0 / self.zoom.max(0.2) + 4.0;
        self.sel_mark = Some(k);
        self.mark_text_for = None;
        self.drag = Some(if on_handle { Drag::Resize { mark: k } } else { Drag::Move { mark: k, last: p, moved: false } });
        let _ = clicks;
        true
    }

    fn select_mark(&mut self, k: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.sel_mark = Some(k);
        let text = match self.edit.marks.get(k).map(|m| &m.mark) {
            Some(Mark::Note { text, .. } | Mark::Text { text, .. }) => Some(text.clone()),
            _ => None,
        };
        self.mark_text_for = text.as_ref().map(|_| k);
        if let Some(text) = text {
            self.mark_text.update(cx, |i, cx| {
                i.set_value(text, window, cx);
                i.focus(window, cx);
            });
        }
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(drag) = self.drag.clone() else { return };
        match drag {
            Drag::Box { page, a, .. } => {
                if let Some(p) = self.point_in(page, ev.position) {
                    self.drag = Some(Drag::Box { page, a, b: p });
                }
            }
            Drag::Pen { page, mut points } => {
                if let Some(p) = self.point_in(page, ev.position) {
                    if points.last().is_none_or(|l| (l[0] - p[0]).abs() + (l[1] - p[1]).abs() > 0.8) {
                        points.push(p);
                    }
                    self.drag = Some(Drag::Pen { page, points });
                }
            }
            Drag::Text { page, from, .. } => {
                let r = self.edit.pages[page];
                let chars = self.cache.borrow_mut().text(r.src, r.page);
                if let (Some(p), Some(chars)) = (self.point_in(page, ev.position), chars) {
                    if let Some(ci) = Self::nearest_char(&chars, p) {
                        self.drag = Some(Drag::Text { page, from, to: ci });
                    }
                }
            }
            Drag::Move { mark, last, moved } => {
                let Some(pm) = self.edit.marks.get(mark) else { return };
                let page = self.edit.pages.iter().position(|r| r.src == pm.src && r.page == pm.page);
                if let Some(p) = page.and_then(|pg| self.point_in(pg, ev.position)) {
                    if !moved {
                        self.push_undo();
                    }
                    self.edit.marks[mark].mark.translate(p[0] - last[0], p[1] - last[1]);
                    self.drag = Some(Drag::Move { mark, last: p, moved: true });
                }
            }
            Drag::Resize { mark } => {
                let Some(pm) = self.edit.marks.get(mark) else { return };
                let page = self.edit.pages.iter().position(|r| r.src == pm.src && r.page == pm.page);
                if let Some(p) = page.and_then(|pg| self.point_in(pg, ev.position)) {
                    if let Mark::Signature { rect, .. } | Mark::Rect { rect, .. } | Mark::Redact { rect } | Mark::Text { rect, .. } = &mut self.edit.marks[mark].mark {
                        rect[2] = p[0].max(rect[0] + 8.0);
                        rect[3] = p[1].max(rect[1] + 6.0);
                    }
                }
            }
        }
        cx.notify();
    }

    fn on_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        let k = tokens(cx);
        let tint = to_rgb(presets(&k, false)[self.color_ix % 6]);
        let stroke = to_rgb(presets(&k, true)[self.color_ix % 8]);
        match drag {
            Drag::Text { page, from, to } => {
                let r = self.edit.pages[page];
                let chars = self.cache.borrow_mut().text(r.src, r.page);
                if let Some(chars) = chars {
                    let rects = Self::line_rects(&chars, from, to);
                    if !rects.is_empty() {
                        self.push_undo();
                        let mark = if self.pen == Pen::Underline { Mark::Underline { rects, color: stroke } } else { Mark::Highlight { rects, color: tint } };
                        self.edit.marks.push(PlacedMark { src: r.src, page: r.page, mark });
                    }
                }
            }
            Drag::Box { page, a, b } => {
                let r = self.edit.pages[page];
                let rect = eng::norm([a[0], a[1], b[0], b[1]]);
                let big = rect[2] - rect[0] > 3.0 && rect[3] - rect[1] > 3.0;
                let mark = match (self.mode, self.pen) {
                    (Mode::Redact, _) if big => Some(Mark::Redact { rect }),
                    (Mode::Annotate, Pen::Highlight) if big => Some(Mark::Highlight { rects: vec![rect], color: tint }),
                    (Mode::Annotate, Pen::Underline) if big => Some(Mark::Underline { rects: vec![rect], color: stroke }),
                    (Mode::Annotate, Pen::Rect) if big => Some(Mark::Rect { rect, color: stroke, width: 1.5 }),
                    (Mode::Annotate, Pen::Arrow) if (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 6.0 => Some(Mark::Arrow { from: a, to: b, color: stroke, width: 1.5 }),
                    (Mode::Annotate, Pen::Text) => {
                        let rect = if big { rect } else { [a[0], a[1], a[0] + 160.0, a[1] + 28.0] };
                        Some(Mark::Text { rect, text: String::new(), color: stroke, size: 12.0 })
                    }
                    _ => None,
                };
                if let Some(mark) = mark {
                    self.push_undo();
                    let is_text = matches!(mark, Mark::Text { .. });
                    self.edit.marks.push(PlacedMark { src: r.src, page: r.page, mark });
                    if is_text {
                        self.select_mark(self.edit.marks.len() - 1, window, cx);
                    }
                }
            }
            Drag::Pen { page, points } => {
                if points.len() > 1 {
                    let r = self.edit.pages[page];
                    self.push_undo();
                    self.edit.marks.push(PlacedMark { src: r.src, page: r.page, mark: Mark::Pen { strokes: vec![points], color: stroke, width: 1.6 } });
                }
            }
            Drag::Move { mark, moved, .. } => {
                if !moved {
                    self.select_mark(mark, window, cx);
                }
            }
            Drag::Resize { .. } => {}
        }
        cx.notify();
    }

    fn field_value(&self, f: &Field) -> String {
        self.edit.fields.get(&f.name).cloned().unwrap_or_else(|| f.value.clone())
    }

    // ---- saving -------------------------------------------------------------------------

    fn ask_save(&mut self, overwrite: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_none() || self.saving.as_ref().is_some_and(|s| s.handle.is_some()) {
            return;
        }
        let redactions = self.edit.redactions();
        if !overwrite && redactions == 0 {
            self.save(overwrite, None, cx);
            return;
        }
        let this = cx.entity().downgrade();
        let k = tokens(cx);
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            let mut body = div().flex().flex_col().gap(px(10.0));
            if redactions > 0 {
                body = body.child(ui::roman().text_color(k.ink).child(ta("app.pdf.redact.warning", &[("count", &redactions.to_string())])));
            }
            if overwrite {
                body = body.child(ui::roman().text_color(k.ink).child(ta("app.pdf.overwrite.body", &[("name", &name)])));
            }
            let ok_label = match (overwrite, redactions > 0) {
                (true, _) => t("app.pdf.overwrite.ok"),
                (false, true) => t("app.pdf.redact.apply_save"),
                _ => t("app.pdf.save_copy"),
            };
            dialog
                .title(if redactions > 0 { t("app.pdf.redact.title") } else { t("app.pdf.overwrite.title") })
                .w(px(500.0))
                .child(body)
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().child(Button::new("save-cancel").label(t("common.cancel"))))
                        .child(DialogAction::new().child(Button::new("save-ok").danger().label(ok_label).on_click(move |_, _, cx| {
                            let _ = this.update(cx, |s, cx| s.save(overwrite, None, cx));
                        }))),
                )
        });
    }

    /// Save the whole edit (or only `pages`, for "Extract selection").
    fn save(&mut self, overwrite: bool, pages: Option<Vec<usize>>, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        let mut edit = self.edit.clone();
        edit.reviewer = self.reviewer.read(cx).value().trim().to_string();
        if let Some(sel) = &pages {
            edit.pages = sel.iter().filter_map(|i| self.edit.pages.get(*i).copied()).collect();
        }
        let Ok(script) = serde_json::to_string(&edit) else { return };
        let tool: Arc<dyn castor_core::Tool> =
            if pages.is_some() { Arc::new(super::tools::PdfStudioTool::with_suffix("pages")) } else { Arc::new(super::tools::PdfStudioTool::new()) };
        let req = RunRequest {
            steps: vec![Step { tool, values: super::tools::values(script, overwrite) }],
            inputs: vec![FileInfo::from_path(&path)],
            output: OutputPolicy::NextToInput { fallback: castor_core::paths::default_output_dir() },
            chain_name: None,
            max_parallel: 1,
            record: true,
        };
        self.saving = Some(Saving { handle: Some(crate::model::start_run(req, cx)), frac: 0.0, record: None, overwrite });
        if pages.is_none() {
            self.clean = self.edit.clone();
        }
        cx.notify();
    }

    /// Drain run events; true when something changed.
    fn poll_save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.saving.as_mut() else { return false };
        let Some(h) = s.handle.as_ref() else { return false };
        let mut changed = false;
        let mut finished = None;
        while let Ok(ev) = h.events.try_recv() {
            changed = true;
            match ev {
                RunEvent::JobProgress { frac, .. } => s.frac = frac,
                RunEvent::Finished(r) => finished = Some(*r),
                _ => {}
            }
        }
        if let Some(r) = finished {
            s.handle = None;
            let overwrite = s.overwrite;
            s.record = Some(r.clone());
            if r.status != RunStatus::Complete {
                // Not saved: the edits are still pending.
                self.clean = PdfEdit::default();
            }
            model(cx).update(cx, |m, cx| m.push_run(r.clone(), cx));
            if overwrite && r.status == RunStatus::Complete {
                // The marks are now part of the file: start again from it.
                self.reopen_after_overwrite = true;
            }
        }
        changed
    }

    // ---- rendering ----------------------------------------------------------------------

    fn render_empty(&mut self, k: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let tint = Hsla { a: 0.08, ..k.link };
        let link = k.link;
        div()
            .id("pdf-empty")
            .size_full()
            .px(px(crate::home::PAGE_PAD))
            .pb(px(32.0))
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(super::crumbs(k, t("app.studio.pdf.name"), None))
            .child(ui::page_header(cx, t("app.studio.pdf.name"), None, Some(t("app.studio.pdf.desc").into_any_element())))
            .child(
                div()
                    .id("pdf-drop")
                    .flex_1()
                    .min_h(px(220.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(14.0))
                    .border_1()
                    .border_dashed()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .bg(k.raised)
                    .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(link).bg(tint))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        if let Some(p) = paths.paths().iter().find(|p| is_pdf(p)) {
                            this.open(p.clone(), window, cx);
                        }
                    }))
                    .child(ui::icon("file-text", k.muted).size(px(28.0)))
                    .child(ui::roman().text_color(k.ink).child(t("app.studio.pdf.drop")))
                    .child(
                        Button::new("pdf-open")
                            .primary()
                            .label(format!("{} ({}O)", t("app.studio.open"), ui::cmd_key()))
                            .on_click(cx.listener(|this, _, window, cx| this.choose_file(window, cx))),
                    )
                    .when_some(self.error.clone(), |d, e| d.child(ui::sans().text_color(k.bad).max_w(px(520.0)).child(e))),
            )
            .into_any_element()
    }

    fn render_toolbar(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        let fit_key = match self.fit {
            Fit::Width => "width",
            Fit::Page => "page",
            Fit::Custom => "",
        };
        div()
            .h(px(44.0))
            .flex_none()
            .px(px(14.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .border_b_1()
            .border_color(k.rule)
            .child(div().flex_1().min_w(px(0.0)).child(super::crumbs(k, t("app.studio.pdf.name"), Some(name))))
            .child(ui::segmented(
                "pdf-mode",
                k,
                Mode::ALL.iter().map(|m| (m.key().to_string(), t(&format!("app.pdf.mode.{}", m.key())))).collect(),
                self.mode.key(),
                Rc::new(move |v, window, cx| {
                    let v = v.to_string();
                    let _ = this.update(cx, |s, cx| s.set_mode_key(&v, window, cx));
                }),
            ))
            .child(div().w(px(1.0)).h(px(20.0)).bg(k.rule))
            .child(Button::new("zoom-out").ghost().small().tooltip(t("app.pdf.zoom_out")).icon(gpui_kit::component::IconName::Minus).on_click(cx.listener(|s, _, _, cx| s.zoom_by(0.8, cx))))
            .child(ui::sans().w(px(48.0)).text_center().text_size(px(13.0)).text_color(k.muted).child(format!("{:.0} %", self.zoom * 100.0)))
            .child(Button::new("zoom-in").ghost().small().tooltip(t("app.pdf.zoom_in")).icon(gpui_kit::component::IconName::Plus).on_click(cx.listener(|s, _, _, cx| s.zoom_by(1.25, cx))))
            .child(ui::segmented(
                "pdf-fit",
                k,
                vec![("width".into(), t("app.pdf.fit_width")), ("page".into(), t("app.pdf.fit_page"))],
                fit_key,
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this2.update(cx, |s, cx| {
                        s.fit = if v == "page" { Fit::Page } else { Fit::Width };
                        cx.notify();
                    });
                }),
            ))
    }

    fn render_rail(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let k = *k;
        let pages = Rc::new(self.edit.pages.clone());
        let sizes = Rc::new(self.sizes.clone());
        let cache = self.cache.clone();
        let selected = self.selected.clone();
        let current = self.current;
        let this = cx.entity().downgrade();
        let organize = self.mode == Mode::Organize;
        let n = pages.len();
        let marks: HashSet<(usize, usize)> = self.edit.marks.iter().map(|m| (m.src, m.page)).collect();
        div()
            .w(px(RAIL_W))
            .flex_none()
            .h_full()
            .bg(k.surface)
            .border_r_1()
            .border_color(k.rule)
            .child(
                uniform_list("pdf-rail", n, move |range, _window, _cx| {
                    range
                        .map(|i| {
                            let r = pages[i];
                            let (w, h) = sizes.get(r.src).and_then(|v| v.get(r.page)).map(|b| (b[2] - b[0], b[3] - b[1])).unwrap_or((612.0, 792.0));
                            let (dw, dh) = eng::rotated_size(w, h, r.rotate);
                            let s = (104.0 / dw).min(132.0 / dh);
                            let thumb = cache.borrow_mut().image(Key { src: r.src, page: r.page, rotate: r.rotate, px: THUMB_PX });
                            let sel = selected.contains(&i);
                            let this_c = this.clone();
                            let this_d = this.clone();
                            let label: SharedString = format!("{}", i + 1).into();
                            let has_marks = marks.contains(&(r.src, r.page));
                            div()
                                .id(("rail-page", i))
                                .role(Role::Button)
                                .aria_label(ta("app.pdf.page_aria", &[("page", &(i + 1).to_string())]))
                                .h(px(166.0))
                                .w_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(5.0))
                                .cursor_pointer()
                                .on_click(move |ev, _, cx| {
                                    let mods = ev.modifiers();
                                    let _ = this_c.update(cx, |s, cx| s.select_page(i, mods, cx));
                                })
                                .when(organize, |d| {
                                    d.on_drag(DraggedPage(i, label.clone()), |v, _, _, cx| cx.new(|_| v.clone()))
                                        .drag_over::<DraggedPage>(move |s, _, _, _| s.border_t_2().border_color(k.link))
                                        .on_drop(move |dp: &DraggedPage, _, cx| {
                                            let from = dp.0;
                                            let _ = this_d.update(cx, |s, cx| s.move_pages(from, i, cx));
                                        })
                                })
                                .child(
                                    div()
                                        .w(px(dw * s))
                                        .h(px(dh * s))
                                        .bg(gpui_kit::white())
                                        .border_1()
                                        .border_color(if sel { k.link } else if i == current { k.muted } else { k.rule })
                                        .when(sel, |d| d.border_2())
                                        .overflow_hidden()
                                        .when_some(thumb, |d, im| d.child(img(im).size_full().object_fit(ObjectFit::Fill))),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(4.0))
                                        .child(ui::sans().text_size(px(12.5)).text_color(if sel { k.link } else { k.muted }).child(label))
                                        .when(has_marks, |d| d.child(ui::diamond_filled(6.0, k.muted)))
                                        .when(r.src > 0, |d| d.child(ui::icon("file-plus", k.faint).size(px(11.0)))),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                })
                .track_scroll(&self.rail)
                .size_full(),
            )
    }

    fn render_pages(&mut self, k: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = *k;
        let zoom = self.zoom;
        let scale = window.scale_factor();
        let max_h = self.edit.pages.iter().map(|p| self.disp_size(*p).1).fold(1.0f32, f32::max) * zoom;
        let max_w = self.edit.pages.iter().map(|p| self.disp_size(*p).0).fold(1.0f32, f32::max) * zoom;
        let item_h = max_h + GAP;
        let n = self.edit.pages.len();
        let this = cx.entity().downgrade();
        // Everything the rows draw, prepared once per frame.
        let views: Rc<Vec<PageView>> = Rc::new((0..n).map(|i| self.page_view(i, &k)).collect());
        let cache = self.cache.clone();
        let mode = self.mode;
        let pen = self.pen;
        let cursor = match (mode, pen) {
            (Mode::Organize, _) | (Mode::Annotate, Pen::Select) | (Mode::Forms, _) => CursorStyle::Arrow,
            (Mode::Annotate, Pen::Highlight | Pen::Underline | Pen::Text) => CursorStyle::IBeam,
            _ => CursorStyle::Crosshair,
        };
        let px_long = move |w: f32, h: f32| -> u32 {
            let long = w.max(h) * zoom * scale;
            // Steps of 128 device px: zooming does not re-render on every step.
            ((long / 128.0).ceil() * 128.0).clamp(128.0, 4096.0) as u32
        };
        let list = uniform_list("pdf-pages", n, move |range, _window, _cx| {
            range
                .map(|i| {
                    let v = &views[i];
                    let key = Key { src: v.r.src, page: v.r.page, rotate: v.r.rotate, px: px_long(v.dw, v.dh) };
                    let im = cache.borrow_mut().image(key);
                    let cache2 = cache.clone();
                    let (pw, ph) = (v.dw * zoom, v.dh * zoom);
                    div()
                        .id(("pdf-page", i))
                        .h(px(item_h))
                        .w(px(max_w.max(1.0) + 2.0 * GAP))
                        .min_w_full()
                        .flex()
                        .justify_center()
                        .pt(px(GAP))
                        .child(
                            div()
                                .relative()
                                .w(px(pw))
                                .h(px(ph))
                                .flex_none()
                                .bg(gpui_kit::white())
                                .border_1()
                                .border_color(k.rule)
                                .cursor(cursor)
                                .child(canvas(move |b, _, _| { cache2.borrow_mut().bounds.insert(i, b); }, |_, _, _, _| {}).absolute().size_full())
                                .when_some(im, |d, im| d.child(img(im).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Fill)))
                                .child(v.overlay(&k, zoom, i, this.clone()))
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.list)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .size_full();
        div()
            .id("pdf-canvas")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .bg(k.surface)
            .on_mouse_down(MouseButton::Left, cx.listener(|s, ev: &MouseDownEvent, w, cx| s.on_down(ev, w, cx)))
            .on_mouse_move(cx.listener(|s, ev: &MouseMoveEvent, w, cx| s.on_move(ev, w, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|s, ev: &MouseUpEvent, w, cx| s.on_up(ev, w, cx)))
            .on_scroll(cx.listener(|s, ev: &ScrollEvent, _, cx| {
                if ev.modifiers.control || ev.modifiers.platform {
                    let dy = match ev.delta {
                        ScrollDelta::Pixels(p) => p.y.as_f32(),
                        ScrollDelta::Lines(l) => l.y,
                    };
                    s.zoom_by(if dy > 0.0 { 1.25 } else { 0.8 }, cx);
                    cx.stop_propagation();
                }
            }))
            .child(list)
    }

    fn page_view(&self, i: usize, k: &Tokens) -> PageView {
        let r = self.edit.pages[i];
        let (w, h) = self.page_size(r);
        let (dw, dh) = eng::rotated_size(w, h, r.rotate);
        let mut marks: Vec<(usize, Mark)> = self.edit.marks_on(r.src, r.page).map(|(k, m)| (k, m.clone())).collect();
        // Live preview of what the mouse is drawing.
        let tint = to_rgb(presets(k, false)[self.color_ix % 6]);
        let stroke = to_rgb(presets(k, true)[self.color_ix % 8]);
        match &self.drag {
            Some(Drag::Box { page, a, b }) if *page == i => {
                let rect = eng::norm([a[0], a[1], b[0], b[1]]);
                let m = match (self.mode, self.pen) {
                    (Mode::Redact, _) => Mark::Redact { rect },
                    (_, Pen::Arrow) => Mark::Arrow { from: *a, to: *b, color: stroke, width: 1.5 },
                    (_, Pen::Highlight) => Mark::Highlight { rects: vec![rect], color: tint },
                    _ => Mark::Rect { rect, color: stroke, width: 1.0 },
                };
                marks.push((usize::MAX, m));
            }
            Some(Drag::Pen { page, points }) if *page == i => marks.push((usize::MAX, Mark::Pen { strokes: vec![points.clone()], color: stroke, width: 1.6 })),
            Some(Drag::Text { page, from, to }) if *page == i => {
                if let Some(chars) = self.cache.borrow().texts.get(&(r.src, r.page)) {
                    marks.push((usize::MAX, Mark::Highlight { rects: Self::line_rects(chars, *from, *to), color: tint }));
                }
            }
            _ => {}
        }
        let fields: Vec<(Field, String)> = if self.mode == Mode::Forms && r.src == 0 {
            self.fields.as_ref().map(|fs| fs.iter().filter(|f| f.page == r.page).map(|f| (f.clone(), self.field_value(f))).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };
        PageView { r, w, h, dw, dh, marks, sel: self.sel_mark, fields }
    }

    fn render_panel(&mut self, k: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match self.mode {
            Mode::Organize => self.panel_organize(k, cx).into_any_element(),
            Mode::Annotate => self.panel_annotate(k, cx).into_any_element(),
            Mode::Sign => self.panel_sign(k, window, cx).into_any_element(),
            Mode::Redact => self.panel_redact(k, cx).into_any_element(),
            Mode::Forms => self.panel_forms(k, cx).into_any_element(),
        };
        div()
            .id("pdf-panel")
            .w(px(PANEL_W))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(k.rule)
            .bg(k.paper)
            .child(div().p(px(16.0)).flex().flex_col().gap(px(14.0)).child(body))
    }

    fn panel_organize(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.targets().len();
        let total = self.edit.pages.len();
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.pdf.mode.organize")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.pdf.organize.help")))
            .child(ui::sans().text_color(k.ink).child(if self.selected.is_empty() {
                ta("app.pdf.organize.current", &[("page", &(self.current + 1).to_string()), ("total", &total.to_string())])
            } else {
                crate::ui::tn("app.pdf.organize.selected", n)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("rot-left").small().icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("rotate-ccw"))).label(t("app.pdf.rotate_left")).on_click(cx.listener(|s, _, _, cx| s.rotate(270, cx))))
                    .child(Button::new("rot-right").small().icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("rotate-cw"))).label(t("app.pdf.rotate_right")).on_click(cx.listener(|s, _, _, cx| s.rotate(90, cx)))),
            )
            .child(Button::new("del-pages").small().danger().icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("trash"))).label(ta("app.pdf.delete_pages", &[("count", &n.to_string())])).on_click(cx.listener(|s, _, w, cx| s.delete_pages(w, cx))))
            .child(ui::hrule(k.rule, 1.0))
            .child(Button::new("insert-pdf").small().icon(gpui_kit::component::IconName::Plus).label(t("app.pdf.insert")).on_click(cx.listener(|s, _, w, cx| s.insert_from_file(w, cx))))
            .child(
                Button::new("extract")
                    .small()
                    .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("file-output")))
                    .label(crate::ui::tn("app.pdf.extract", n))
                    .on_click(cx.listener(|s, _, _, cx| {
                        let pages = s.targets();
                        s.save(false, Some(pages), cx);
                    })),
            )
            .child(
                Button::new("select-all")
                    .small()
                    .ghost()
                    .label(t("app.pdf.select_all"))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.selected = (0..s.edit.pages.len()).collect();
                        cx.notify();
                    })),
            )
    }

    fn color_row(&self, k: &Tokens, stroke: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let cols = presets(k, stroke);
        let sel = self.color_ix % cols.len();
        div().flex().flex_row().gap(px(6.0)).children(cols.into_iter().enumerate().map(|(i, c)| {
            div()
                .id(("color", i))
                .role(Role::RadioButton)
                .aria_label(ta("app.pdf.colour_aria", &[("n", &(i + 1).to_string())]))
                .size(px(22.0))
                .rounded(px(3.0))
                .bg(c)
                .border_1()
                .border_color(if i == sel { k.ink } else { k.rule })
                .when(i == sel, |d| d.border_2())
                .cursor_pointer()
                .on_click(cx.listener(move |s, _, _, cx| {
                    s.color_ix = i;
                    cx.notify();
                }))
        }))
    }

    fn panel_annotate(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let pen = self.pen;
        let stroke = !matches!(pen, Pen::Highlight | Pen::Note);
        let editing = self.mark_text_for.is_some();
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.pdf.annotate.tools")))
            .child(div().flex().flex_row().flex_wrap().gap(px(4.0)).children(Pen::ALL.iter().map(|p| {
                let p = *p;
                Button::new(SharedString::from(format!("pen-{}", p.key())))
                    .small()
                    .when(p == pen, |b| b.primary())
                    .tooltip(t(&format!("app.pdf.pen.{}", p.key())))
                    .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide(p.icon())))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.pen = p;
                        s.sel_mark = None;
                        cx.notify();
                    }))
            })))
            .child(ui::sans().text_size(px(13.5)).text_color(k.ink).child(t(&format!("app.pdf.pen.{}", pen.key()))))
            .child(ui::roman().text_size(px(13.5)).text_color(k.muted).child(t(&format!("app.pdf.pen.{}.help", pen.key()))))
            .when(pen != Pen::Select, |d| d.child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.pdf.colour")))).child(self.color_row(k, stroke, cx)))
            .when(editing, |d| d.child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.pdf.text")))).child(Input::new(&self.mark_text)))
            .when(self.sel_mark.is_some(), |d| {
                d.child(Button::new("del-mark").small().danger().label(t("app.pdf.delete_mark")).on_click(cx.listener(|s, _, _, cx| s.delete_mark(cx))))
            })
            .child(ui::hrule(k.rule, 1.0))
            .child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.pdf.reviewer"))))
            .child(Input::new(&self.reviewer))
            .child(ui::roman().text_size(px(13.0)).text_color(k.faint).child(t("app.pdf.reviewer.help")))
    }

    fn panel_sign(&mut self, k: &Tokens, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sel = self.sig_ix;
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.pdf.sign.title")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.pdf.sign.help")))
            .children(self.sigs.iter().enumerate().map(|(i, (s, _))| {
                let body: AnyElement = match s {
                    Signature::Typed { name, color } => div()
                        .font_family(crate::theme::fonts().chorus.clone())
                        .text_size(px(24.0))
                        .text_color(from_rgb(*color, 1.0))
                        .truncate()
                        .child(name.clone())
                        .into_any_element(),
                    Signature::Image { path } => img(path.clone()).h(px(40.0)).max_w_full().object_fit(ObjectFit::Contain).into_any_element(),
                };
                div()
                    .id(("sig", i))
                    .role(Role::RadioButton)
                    .aria_label(ta("app.pdf.sig_aria", &[("n", &(i + 1).to_string())]))
                    .h(px(58.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(if sel == Some(i) { k.ink } else { k.rule })
                    .when(sel == Some(i), |d| d.border_2())
                    .bg(gpui_kit::white())
                    .cursor_pointer()
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.sig_ix = Some(i);
                        cx.notify();
                    }))
                    .child(body)
            }))
            .when(self.sigs.is_empty(), |d| d.child(ui::roman().italic().text_color(k.faint).child(t("app.pdf.sign.empty"))))
            .child(Button::new("new-sig").small().icon(gpui_kit::component::IconName::Plus).label(t("app.pdf.sign.new")).on_click(cx.listener(|s, _, w, cx| s.new_signature(w, cx))))
            .child(Checkbox::new("sig-date").label(t("app.pdf.sign.date")).checked(self.sig_date).on_change(cx.listener(|s, v: &bool, _, cx| {
                s.sig_date = *v;
                cx.notify();
            })))
            .when(self.sel_mark.is_some(), |d| {
                d.child(ui::roman().text_size(px(13.5)).text_color(k.muted).child(t("app.pdf.sign.resize")))
                    .child(Button::new("del-sig").small().danger().label(t("app.pdf.delete_mark")).on_click(cx.listener(|s, _, _, cx| s.delete_mark(cx))))
            })
    }

    fn panel_redact(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.edit.redactions();
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.pdf.mode.redact")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.pdf.redact.help")))
            .child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.pdf.redact.search"))))
            .child(div().flex().flex_row().gap(px(6.0)).child(div().flex_1().child(Input::new(&self.search))).child(Button::new("redact-find").small().label(t("app.pdf.redact.mark")).on_click(cx.listener(|s, _, w, cx| s.search_redact(w, cx)))))
            .when_some(self.search_note.clone(), |d, n| d.child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(n)))
            .child(ui::hrule(k.rule, 1.0))
            .child(ui::sans().text_color(k.ink).child(ta("app.pdf.redact.count", &[("count", &n.to_string())])))
            .when(self.sel_mark.is_some(), |d| d.child(Button::new("del-box").small().label(t("app.pdf.delete_mark")).on_click(cx.listener(|s, _, _, cx| s.delete_mark(cx)))))
            .when(n > 0, |d| {
                d.child(Button::new("redact-clear").small().ghost().label(t("app.pdf.redact.clear")).on_click(cx.listener(|s, _, _, cx| s.clear_redactions(cx))))
                    .child(Button::new("redact-apply").danger().w_full().label(t("app.pdf.redact.apply")).on_click(cx.listener(|s, _, w, cx| s.ask_save(false, w, cx))))
            })
            .child(ui::roman().text_size(px(13.0)).italic().text_color(k.muted).child(t("app.pdf.redact.note")))
    }

    fn panel_forms(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let fields = self.fields.clone().unwrap_or_default();
        let mut seen_radio = HashSet::new();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (ix, f) in fields.iter().enumerate() {
            let label = ui::sans().text_size(px(13.0)).text_color(k.muted).truncate().child(format!("{} · {}", f.label, ta("app.pdf.forms.page", &[("page", &(f.page + 1).to_string())])));
            match f.kind {
                FieldKind::Text | FieldKind::Choice => {
                    if let Some(inp) = self.field_inputs.get(&f.name) {
                        rows.push(div().flex().flex_col().gap(px(4.0)).child(label).child(Input::new(inp)).into_any_element());
                    }
                }
                FieldKind::Check => {
                    let checked = { let v = self.field_value(f); v == f.on || eng_truthy(&v) };
                    let (name, on) = (f.name.clone(), f.on.clone());
                    rows.push(
                        Checkbox::new(("field-cb", ix))
                            .label(f.label.clone())
                            .checked(checked)
                            .on_change(cx.listener(move |s, v: &bool, _, cx| {
                                s.edit.fields.insert(name.clone(), if *v { on.clone() } else { "Off".into() });
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
                FieldKind::Radio => {
                    if !seen_radio.insert(f.name.clone()) {
                        continue;
                    }
                    let opts: Vec<(String, SharedString)> = fields.iter().filter(|g| g.name == f.name && !g.on.is_empty()).map(|g| (g.on.clone(), g.on.clone().into())).collect();
                    let cur = self.field_value(f);
                    let name = f.name.clone();
                    let this = cx.entity().downgrade();
                    rows.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(label)
                            .child(ui::segmented(
                                SharedString::from(format!("radio-{ix}")),
                                k,
                                opts,
                                &cur,
                                Rc::new(move |v, _, cx| {
                                    let v = v.to_string();
                                    let name = name.clone();
                                    let _ = this.update(cx, |s, cx| {
                                        s.edit.fields.insert(name, v);
                                        cx.notify();
                                    });
                                }),
                            ))
                            .into_any_element(),
                    );
                }
            }
        }
        let loading = self.fields.as_ref().is_some_and(|f| f.is_empty()) && self.field_inputs.is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.pdf.mode.forms")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(if fields.is_empty() {
                if loading && self.fields.is_none() { t("app.pdf.forms.loading") } else { t("app.pdf.forms.none") }
            } else {
                ta("app.pdf.forms.count", &[("count", &fields.len().to_string())])
            }))
            .children(rows)
    }

    fn new_signature(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let pad = cx.new(|cx| super::sign_pad::SignPad::new(window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            let pad = pad.clone();
            let pad2 = pad.clone();
            dialog
                .title(t("app.pdf.sign.new"))
                .w(px(560.0))
                .child(pad.clone())
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().child(Button::new("sig-cancel").label(t("common.cancel"))))
                        .child(DialogAction::new().child(Button::new("sig-ok").primary().label(t("app.pdf.sign.keep")).on_click(move |_, window, cx| {
                            let res = pad2.update(cx, |p, cx| p.save(window, cx));
                            match res {
                                Ok(()) => {
                                    let _ = this.update(cx, |s, cx| {
                                        s.sig_ix = Some(0);
                                        s.load_signatures(cx);
                                    });
                                }
                                Err(e) => window.push_notification(e, cx),
                            }
                        }))),
                )
        });
    }

    fn render_bottom(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.edit.pages.len();
        let top = self.list.0.borrow().base_handle.logical_scroll_top().0.min(total.saturating_sub(1));
        let changes = self.changes();
        let saving = self.saving.as_ref().and_then(|s| s.handle.as_ref().map(|_| s.frac));
        let done = self.saving.as_ref().and_then(|s| s.record.clone());
        let redactions = self.edit.redactions();
        let dirty = self.dirty();
        let status = if total > 0 { ta("app.pdf.status", &[("page", &(top + 1).to_string()), ("total", &total.to_string())]) } else { SharedString::default() };
        div()
            .h(px(52.0))
            .flex_none()
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .border_t_1()
            .border_color(k.rule)
            .bg(k.paper)
            .child(ui::sans().text_color(k.ink).child(status))
            .when(changes > 0, |d| d.child(ui::sans().text_color(k.muted).child(crate::ui::tn("app.pdf.changes", changes))))
            .child(div().flex_1().min_w(px(0.0)).flex().flex_row().items_center().gap(px(10.0)).map(|d| {
                if let Some(f) = saving {
                    d.child(div().w(px(160.0)).child(Progress::new("save-progress").value(f * 100.0))).child(ui::sans().text_color(k.muted).child(t("app.pdf.saving")))
                } else if let Some(r) = done {
                    super::run_summary(k, &r, d)
                } else {
                    d
                }
            }))
            .child(
                Button::new("undo-edit")
                    .ghost()
                    .small()
                    .tooltip(format!("{} ({}Z)", t("app.pdf.undo"), ui::cmd_key()))
                    .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("undo-2")))
                    .disabled(self.undo.is_empty())
                    .on_click(cx.listener(|s, _, _, cx| s.undo(cx))),
            )
            .child(Button::new("overwrite").label(t("app.pdf.overwrite")).disabled(!dirty || saving.is_some()).on_click(cx.listener(|s, _, w, cx| s.ask_save(true, w, cx))))
            .child(
                Button::new("save-copy")
                    .primary()
                    .label(if redactions > 0 { t("app.pdf.redact.apply_save") } else { t("app.pdf.save_copy") })
                    .disabled(!dirty || saving.is_some())
                    .on_click(cx.listener(|s, _, w, cx| s.ask_save(false, w, cx))),
            )
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let cmd = ks.modifiers.control || ks.modifiers.platform;
        match ks.key.as_str() {
            "o" if cmd => self.choose_file(window, cx),
            "s" if cmd => self.ask_save(false, window, cx),
            "z" if cmd && ks.modifiers.shift => self.redo(cx),
            "z" if cmd => self.undo(cx),
            "y" if cmd => self.redo(cx),
            "=" | "+" if cmd => self.zoom_by(1.25, cx),
            "-" if cmd => self.zoom_by(0.8, cx),
            "0" if cmd => {
                self.fit = Fit::Width;
                cx.notify();
            }
            _ if !self.focus.is_focused(window) => return,
            "delete" | "backspace" => {
                if self.sel_mark.is_some() {
                    self.delete_mark(cx);
                } else if self.mode == Mode::Organize && !self.selected.is_empty() {
                    self.delete_pages(window, cx);
                }
            }
            "pagedown" | "down" if self.mode == Mode::Organize || ks.key == "pagedown" => {
                let i = (self.current + 1).min(self.edit.pages.len().saturating_sub(1));
                self.select_page(i, Modifiers::default(), cx);
            }
            "pageup" | "up" if self.mode == Mode::Organize || ks.key == "pageup" => {
                let i = self.current.saturating_sub(1);
                self.select_page(i, Modifiers::default(), cx);
            }
            "escape" => {
                self.sel_mark = None;
                self.drag = None;
                self.selected.clear();
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

fn eng_truthy(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on" | "x")
}

pub fn is_pdf(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// What one page row draws.
struct PageView {
    r: PageRef,
    w: f32,
    h: f32,
    dw: f32,
    dh: f32,
    marks: Vec<(usize, Mark)>,
    sel: Option<usize>,
    fields: Vec<(Field, String)>,
}

impl PageView {
    fn vr(&self, r: R4, zoom: f32) -> (f32, f32, f32, f32) {
        let v = eng::rect_to_view(r, self.w, self.h, self.r.rotate);
        (v[0] * zoom, v[1] * zoom, (v[2] - v[0]) * zoom, (v[3] - v[1]) * zoom)
    }

    fn vp(&self, p: [f32; 2], zoom: f32) -> Point<Pixels> {
        let v = eng::to_view(p, self.w, self.h, self.r.rotate);
        point(px(v[0] * zoom), px(v[1] * zoom))
    }

    fn overlay(&self, k: &Tokens, zoom: f32, _page: usize, _this: WeakEntity<PdfStudio>) -> impl IntoElement {
        let mut root = div().absolute().top_0().left_0().size_full();
        let mut paths: Vec<(Vec<Vec<Point<Pixels>>>, Hsla, f32, bool)> = Vec::new();
        let boxed = |x: f32, y: f32, w: f32, h: f32| div().absolute().left(px(x)).top(px(y)).w(px(w.max(1.0))).h(px(h.max(1.0)));
        for (i, m) in &self.marks {
            let selected = self.sel == Some(*i);
            match m {
                Mark::Highlight { rects, color } => {
                    for r in rects {
                        let (x, y, w, h) = self.vr(*r, zoom);
                        root = root.child(boxed(x, y, w, h).bg(from_rgb(*color, 0.45)));
                    }
                }
                Mark::Underline { rects, color } => {
                    for r in rects {
                        let (x, y, w, h) = self.vr(*r, zoom);
                        root = root.child(boxed(x, y + h - 1.5, w, 1.6).bg(from_rgb(*color, 1.0)));
                    }
                }
                Mark::Pen { strokes, color, width } => {
                    paths.push((strokes.iter().map(|s| s.iter().map(|p| self.vp(*p, zoom)).collect()).collect(), from_rgb(*color, 1.0), width * zoom, false));
                }
                Mark::Arrow { from, to, color, width } => {
                    paths.push((vec![vec![self.vp(*from, zoom), self.vp(*to, zoom)]], from_rgb(*color, 1.0), width * zoom, true));
                }
                Mark::Rect { rect, color, width } => {
                    let (x, y, w, h) = self.vr(*rect, zoom);
                    root = root.child(boxed(x, y, w, h).border(px((width * zoom).max(1.0))).border_color(from_rgb(*color, 1.0)));
                }
                Mark::Note { at, text, color } => {
                    let (x, y, w, h) = self.vr([at[0], at[1], at[0] + eng::NOTE_SIDE, at[1] + eng::NOTE_SIDE], zoom);
                    root = root.child(
                        boxed(x, y, w, h)
                            .rounded(px(2.0))
                            .bg(from_rgb(*color, 1.0))
                            .border_1()
                            .border_color(gpui_kit::rgb(0x1b1616))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg().path(crate::assets::lucide("message-square")).size(px((w * 0.65).max(8.0))).text_color(gpui_kit::rgb(0x1b1616))),
                    );
                    if !text.is_empty() {
                        root = root.child(
                            div()
                                .absolute()
                                .left(px(if x + w + 186.0 > self.dw * zoom { (x - 184.0).max(0.0) } else { x + w + 4.0 }))
                                .top(px(y))
                                .w(px(180.0))
                                .px(px(6.0))
                                .py(px(3.0))
                                .rounded(px(2.0))
                                .bg(gpui_kit::rgb(0xffffff))
                                .border_1()
                                .border_color(gpui_kit::rgb(0xd9d6cf))
                                .font_family(crate::theme::fonts().sans.clone())
                                .text_size(px(12.5))
                                .text_color(gpui_kit::rgb(0x1b1616))
                                .child(text.clone()),
                        );
                    }
                }
                Mark::Text { rect, text, color, size } => {
                    let (x, y, w, h) = self.vr(*rect, zoom);
                    root = root.child(
                        boxed(x, y, w, h)
                            .border_1()
                            .border_dashed()
                            .border_color(Hsla { a: 0.35, ..from_rgb(*color, 1.0) })
                            .px(px(2.0))
                            .font_family(crate::theme::fonts().sans.clone())
                            .text_size(px(size * zoom))
                            .line_height(px(size * zoom * 1.2))
                            .text_color(from_rgb(*color, 1.0))
                            .overflow_hidden()
                            .child(text.clone()),
                    );
                }
                Mark::Signature { rect, sig, date } => {
                    let (x, y, w, h) = self.vr(*rect, zoom);
                    let date_h = if date.is_some() { (w / 14.0).clamp(6.0, 11.0) * zoom * 1.6 } else { 0.0 };
                    let body: AnyElement = match sig {
                        Signature::Typed { name, color } => div()
                            .w_full()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_family(crate::theme::fonts().chorus.clone())
                            .text_size(px(((h - date_h) * 0.62).max(6.0)))
                            .text_color(from_rgb(*color, 1.0))
                            .whitespace_nowrap()
                            .child(name.clone())
                            .into_any_element(),
                        Signature::Image { path } => img(path.clone()).w_full().flex_1().object_fit(ObjectFit::Contain).into_any_element(),
                    };
                    root = root.child(
                        boxed(x, y, w, h)
                            .flex()
                            .flex_col()
                            .child(body)
                            .when_some(date.clone(), |d, dt| {
                                d.child(div().font_family(crate::theme::fonts().sans.clone()).text_size(px((date_h / 1.6).max(6.0))).text_color(gpui_kit::rgb(0x404040)).child(dt))
                            }),
                    );
                }
                Mark::Redact { rect } => {
                    let (x, y, w, h) = self.vr(*rect, zoom);
                    root = root.child(
                        boxed(x, y, w, h)
                            .bg(Hsla { a: 0.55, ..gpui_kit::black() })
                            .border_1()
                            .border_color(k.bad)
                            .when(*i == usize::MAX, |d| d.border_dashed()),
                    );
                }
            }
            if selected {
                let (x, y, w, h) = self.vr(m.bounds(), zoom);
                root = root.child(boxed(x - 3.0, y - 3.0, w + 6.0, h + 6.0).border_1().border_dashed().border_color(k.link));
                if matches!(m, Mark::Signature { .. } | Mark::Rect { .. } | Mark::Redact { .. } | Mark::Text { .. }) {
                    let (bx, by) = self.vr([m.bounds()[2], m.bounds()[3], m.bounds()[2], m.bounds()[3]], zoom).into_xy();
                    root = root.child(boxed(bx - 4.0, by - 4.0, 8.0, 8.0).bg(k.link).rounded(px(1.0)));
                }
            }
        }
        for (f, value) in &self.fields {
            let (x, y, w, h) = self.vr(f.rect, zoom);
            let filled = !value.is_empty() && value != "Off";
            root = root.child(
                boxed(x, y, w, h)
                    .bg(Hsla { a: if filled { 0.10 } else { 0.18 }, ..k.link })
                    .border_1()
                    .border_color(Hsla { a: 0.6, ..k.link })
                    .overflow_hidden()
                    .px(px(2.0))
                    .font_family(crate::theme::fonts().sans.clone())
                    .text_size(px((h * 0.62).clamp(7.0, 14.0)))
                    .text_color(gpui_kit::rgb(0x1b1616))
                    .child(match f.kind {
                        FieldKind::Check | FieldKind::Radio => if filled && (*value == f.on || eng_truthy(value)) { "✓".to_string() } else { String::new() },
                        _ => value.clone(),
                    }),
            );
        }
        if !paths.is_empty() {
            root = root.child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        for (strokes, color, width, arrow) in &paths {
                            for s in strokes {
                                if s.len() < 2 {
                                    continue;
                                }
                                let mut pb = PathBuilder::stroke(px(width.max(1.0)));
                                pb.move_to(b.origin + s[0]);
                                for p in &s[1..] {
                                    pb.line_to(b.origin + *p);
                                }
                                if *arrow {
                                    let (a, e) = (s[0], s[s.len() - 1]);
                                    let (dx, dy) = ((e.x - a.x).as_f32(), (e.y - a.y).as_f32());
                                    let len = (dx * dx + dy * dy).sqrt().max(0.01);
                                    let (ux, uy) = (dx / len, dy / len);
                                    let head = (width * 5.0).max(8.0);
                                    for side in [-1.0f32, 1.0] {
                                        let hx = e.x.as_f32() - head * (ux * 0.87 - side * uy * 0.5);
                                        let hy = e.y.as_f32() - head * (uy * 0.87 + side * ux * 0.5);
                                        pb.move_to(b.origin + e);
                                        pb.line_to(b.origin + point(px(hx), px(hy)));
                                    }
                                }
                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, *color);
                                }
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        root
    }
}

trait IntoXy {
    fn into_xy(self) -> (f32, f32);
}
impl IntoXy for (f32, f32, f32, f32) {
    fn into_xy(self) -> (f32, f32) {
        (self.0, self.1)
    }
}

impl Focusable for PdfStudio {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PdfStudio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let vs = window.viewport_size();
        self.viewport = (vs.width.as_f32(), vs.height.as_f32());
        if std::mem::take(&mut self.reopen_after_overwrite) {
            if let Some(p) = self.path.clone() {
                let rec = self.saving.take();
                self.open(p, window, cx);
                self.saving = rec;
            }
        }
        let body: AnyElement = if self.path.is_none() || self.error.is_some() && self.sizes.is_empty() {
            self.render_empty(&k, cx)
        } else if self.loading {
            div().size_full().flex().items_center().justify_center().child(ui::roman().italic().text_color(k.faint).child(t("app.studio.loading"))).into_any_element()
        } else {
            self.update_fit();
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_toolbar(&k, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex()
                        .flex_row()
                        .child(self.render_rail(&k, cx))
                        .child(self.render_pages(&k, window, cx))
                        .child(self.render_panel(&k, window, cx)),
                )
                .child(self.render_bottom(&k, cx))
                .into_any_element()
        };
        div()
            .id("pdf-studio")
            .key_context("PdfStudio")
            .track_focus(&self.focus)
            .size_full()
            .on_key_down(cx.listener(|s, ev: &KeyDownEvent, w, cx| s.on_key(ev, w, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(p) = paths.paths().iter().find(|p| is_pdf(p)) {
                    if this.path.is_none() || !this.dirty() {
                        this.open(p.clone(), window, cx);
                    } else if let Some(r) = &this.renderer {
                        r.send(Req::Open(p.clone()));
                    }
                }
            }))
            .child(body)
    }
}
