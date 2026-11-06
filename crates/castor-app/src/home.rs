//! Tools home (DESIGN §4): drop zone, family tabs, numbered family sections of cards, and after
//! a drop the "Suggested" row plus highlighted/faded cards.

use std::path::PathBuf;
use std::sync::Arc;

use castor_core::{Family, FileInfo, FileKind, Tool, ToolStatus};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Nav, PageId, model, navigate};
use crate::shell::SIDEBAR_W;
use crate::theme::{Tokens, fonts, tokens};
use crate::ui::{self, t, ta};

pub const PAGE_PAD: f32 = 44.0;

/// Launch options for the home (screenshots): family filter and scroll offset, used once.
#[derive(Default)]
pub struct HomeStart {
    pub family: Option<Family>,
    pub scroll: Option<f32>,
}
impl Global for HomeStart {}

pub struct Home {
    focus: FocusHandle,
    files: Vec<FileInfo>,
    family: Option<Family>,
    /// Keyboard selection: index into `order`.
    sel: Option<usize>,
    /// Card order as rendered (for arrow keys).
    order: Vec<String>,
    cols: usize,
    scroll: ScrollHandle,
    loading: bool,
}

impl Home {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let start = if cx.has_global::<HomeStart>() { std::mem::take(cx.global_mut::<HomeStart>()) } else { HomeStart::default() };
        let scroll = ScrollHandle::new();
        if let Some(y) = start.scroll {
            scroll.set_offset(point(px(0.0), px(-y)));
        }
        Home {
            focus: cx.focus_handle(),
            files: Vec::new(),
            family: start.family,
            sel: None,
            order: Vec::new(),
            cols: 4,
            scroll,
            loading: false,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// Show these files: folders are expanded off the UI thread.
    pub fn set_files(&mut self, paths: Vec<PathBuf>, _window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.sel = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { castor_core::files::expand(&paths, true) }).await;
            let _ = this.update(cx, |h, cx| {
                h.files = files;
                h.loading = false;
                h.family = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_files(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.sel = None;
        cx.notify();
    }

    fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t("app.home.choose")),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update_in(cx, |h, window, cx| h.set_files(paths, window, cx));
            }
        })
        .detach();
    }

    fn open_tool(&self, id: &str, cx: &mut App) {
        let reg = crate::model::registry(cx);
        let Some(tool) = reg.get(id) else { return };
        let files: Vec<PathBuf> = self.files.iter().filter(|f| tool.accepts(f)).map(|f| f.path.clone()).collect();
        navigate(Nav::Tool { id: id.to_string(), files, values: None }, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let n = self.order.len();
        if n == 0 {
            return;
        }
        let cols = self.cols.max(1);
        let cur = self.sel;
        let next = match ev.keystroke.key.as_str() {
            "/" => {
                navigate(Nav::Palette(None), cx);
                return;
            }
            "right" => Some(cur.map(|i| (i + 1).min(n - 1)).unwrap_or(0)),
            "left" => Some(cur.map(|i| i.saturating_sub(1)).unwrap_or(0)),
            "down" => Some(cur.map(|i| (i + cols).min(n - 1)).unwrap_or(0)),
            "up" => Some(cur.map(|i| i.saturating_sub(cols)).unwrap_or(0)),
            "home" => Some(0),
            "end" => Some(n - 1),
            "enter" => {
                if let Some(id) = cur.and_then(|i| self.order.get(i)).cloned() {
                    self.open_tool(&id, cx);
                }
                return;
            }
            "escape" => {
                if !self.files.is_empty() {
                    self.clear_files(cx);
                }
                self.sel = None;
                cx.notify();
                return;
            }
            _ => return,
        };
        self.sel = next;
        cx.stop_propagation();
        cx.notify();
    }

    /// Tools that best fit the dropped files, best first.
    pub fn suggested(files: &[FileInfo], tools: &[Arc<dyn Tool>], favorites: &[String], recent: &[String], n: usize) -> Vec<String> {
        if files.is_empty() {
            return Vec::new();
        }
        let main_kind = dominant_kind(files);
        let mut scored: Vec<(i32, String)> = tools
            .iter()
            .filter(|t| t.meta().status != ToolStatus::Soon && t.unavailable().is_none())
            .filter_map(|t| {
                let m = t.meta();
                let accepted = files.iter().filter(|f| t.accepts(f)).count();
                if accepted == 0 {
                    return None;
                }
                let mut s = (accepted * 10 / files.len()) as i32 * 10;
                if m.inputs.kinds.contains(&main_kind) {
                    s += 30;
                }
                if m.inputs.kinds.contains(&FileKind::Any) {
                    s -= 25;
                }
                if family_for_kind(main_kind) == Some(m.family) {
                    s += 20;
                }
                if favorites.iter().any(|f| f == m.id) {
                    s += 15;
                }
                if let Some(p) = recent.iter().position(|r| r == m.id) {
                    s += 12 - p.min(10) as i32;
                }
                if m.status == ToolStatus::Beta {
                    s -= 3;
                }
                Some((s, m.id.to_string()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(n).map(|(_, id)| id).collect()
    }
}

fn dominant_kind(files: &[FileInfo]) -> FileKind {
    let mut counts: Vec<(FileKind, usize)> = Vec::new();
    for f in files {
        match counts.iter_mut().find(|(k, _)| *k == f.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((f.kind, 1)),
        }
    }
    counts.into_iter().max_by_key(|c| c.1).map(|c| c.0).unwrap_or(FileKind::Other)
}

fn family_for_kind(k: FileKind) -> Option<Family> {
    Some(match k {
        FileKind::Pdf | FileKind::Office | FileKind::Ebook => Family::Pdf,
        FileKind::Image | FileKind::Raw | FileKind::Svg => Family::Image,
        FileKind::Video => Family::Video,
        FileKind::Audio => Family::Audio,
        FileKind::Archive | FileKind::Other | FileKind::Text | FileKind::Table => Family::Files,
        _ => return None,
    })
}

/// Short input label for cards: "PDF", "Image · RAW", "—" for no input.
pub fn input_label(tool: &dyn Tool) -> String {
    let i = &tool.meta().inputs;
    if i.min == 0 && i.kinds.is_empty() && i.extensions.is_empty() {
        return "—".into();
    }
    let mut parts: Vec<String> = i.kinds.iter().take(2).map(|k| kind_label(*k)).collect();
    if parts.is_empty() {
        parts = i.extensions.iter().take(2).map(|e| e.to_uppercase()).collect();
    }
    if i.folders && !i.kinds.contains(&FileKind::Any) {
        parts.push(castor_core::tr("app.kind.folder"));
    }
    parts.join(" · ")
}

pub fn kind_label(k: FileKind) -> String {
    let key = match k {
        FileKind::Pdf => "pdf",
        FileKind::Image => "image",
        FileKind::Raw => "raw",
        FileKind::Svg => "svg",
        FileKind::Video => "video",
        FileKind::Audio => "audio",
        FileKind::Office => "office",
        FileKind::Ebook => "ebook",
        FileKind::Html => "html",
        FileKind::Markdown => "markdown",
        FileKind::Text => "text",
        FileKind::Table => "table",
        FileKind::Subtitles => "subtitles",
        FileKind::Archive => "archive",
        FileKind::Font => "font",
        FileKind::Other => "other",
        FileKind::Any => "any",
    };
    castor_core::tr(&format!("app.kind.{key}"))
}

#[derive(Clone, Copy, Default)]
pub struct CardState {
    /// Number of dropped files this tool takes (Some(0) = not for these files).
    pub takes: Option<usize>,
    pub selected: bool,
    pub favorite: bool,
}

/// A tool card (DESIGN §4).
pub fn tool_card(k: &Tokens, tool: &Arc<dyn Tool>, st: CardState, id_prefix: &str) -> Stateful<Div> {
    let m = tool.meta();
    let id = m.id.to_string();
    let group: SharedString = format!("{id_prefix}-{id}").into();
    let soon = m.status == ToolStatus::Soon;
    let unavailable = tool.unavailable();
    let faded = st.takes == Some(0) || soon;
    let highlight = matches!(st.takes, Some(n) if n > 0);
    let fav = st.favorite;
    let id_fav = id.clone();
    let tint = k.tint(m.family);
    div()
        .id(group.clone())
        .role(Role::Button)
        .aria_label(format!("{} — {}", m.name(), m.desc()))
        .tooltip({
            let text: SharedString = m.desc().into();
            move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
        })
        .group(group.clone())
        .relative()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(12.0))
        .min_h(px(128.0))
        .overflow_hidden()
        .bg(k.raised)
        .border_1()
        .rounded(px(3.0))
        .border_color(if st.selected { k.link } else if highlight { k.ink } else { k.rule })
        .when(st.selected, |d| d.border_2().p(px(11.0)))
        .when(faded, |d| d.opacity(0.4))
        .cursor_pointer()
        .hover(move |s| s.bg(tint))
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .child(div().pt(px(3.0)).child(ui::family_swatch(k, m.family, 12.0)))
                .child(
                    ui::sans()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(ui::size::CARD_NAME))
                        .line_height(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(k.ink)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(m.name()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{group}-star")))
                        .flex_none()
                        .when(!fav, |d| d.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let id = id_fav.clone();
                            model(cx).update(cx, |m, cx| m.toggle_favorite(&id, cx));
                        })
                        .child(ui::icon("star", if fav { k.warn } else { k.faint }).size(px(14.0))),
                ),
        )
        .child(
            ui::roman()
                .text_size(px(ui::size::SMALL))
                .line_height(px(17.0))
                .text_color(k.muted)
                .line_clamp(3)
                .text_ellipsis()
                .min_h(px(51.0))
                .child(m.desc()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .mt_auto()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .font_family(fonts().caps.clone())
                        .text_size(px(13.0))
                        .text_color(k.muted)
                        .truncate()
                        .child(format!("{} → {}", input_label(tool.as_ref()), m.output)),
                )
                .when(soon, |d| d.child(ui::tag(k, t("app.status.soon"), k.surface)))
                .when(m.status == ToolStatus::Beta, |d| d.child(ui::tag(k, t("app.status.beta"), k.surface)))
                .when(unavailable.is_some() && !soon, |d| d.child(ui::icon("triangle-alert", k.warn).size(px(13.0))))
                .when_some(st.takes.filter(|n| *n > 0), |d, n| {
                    d.child(
                        ui::sans()
                            .px(px(6.0))
                            .rounded(px(2.0))
                            .bg(k.ink)
                            .text_color(k.paper)
                            .text_size(px(12.5))
                            .child(ta("common.n_files", &[("count", &n.to_string())])),
                    )
                })
                .when(st.takes == Some(0), |d| {
                    d.child(ui::roman().text_size(px(12.5)).italic().text_color(k.muted).child(t("app.home.not_for_files")))
                }),
        )
}

/// Number of grid columns for a content width.
pub fn columns(content_w: f32) -> usize {
    (((content_w + 12.0) / (208.0 + 12.0)).floor() as usize).clamp(2, 5)
}

impl Focusable for Home {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let m = model(cx).read(cx);
        let reg = m.registry.clone();
        let favorites = m.settings.favorites.clone();
        let recent = m.recent_tools(10);
        let all = reg.all().to_vec();
        let content_w = (window.viewport_size().width.as_f32() - SIDEBAR_W - 2.0 * PAGE_PAD).max(400.0);
        self.cols = columns(content_w);
        let cols = self.cols;
        let has_files = !self.files.is_empty();
        let suggested = Home::suggested(&self.files, &all, &favorites, &recent, cols.min(5));

        // Families present, in catalogue order.
        let families: Vec<(Family, usize)> =
            Family::ALL.iter().map(|f| (*f, reg.family(*f).count())).filter(|(_, n)| *n > 0).collect();

        let mut order: Vec<String> = Vec::new();
        let sel_id = self.sel.and_then(|i| self.order.get(i)).cloned();
        let card_state = |t: &Arc<dyn Tool>| CardState {
            takes: if has_files { Some(self.files.iter().filter(|f| t.accepts(f)).count()) } else { None },
            selected: sel_id.as_deref() == Some(t.meta().id),
            favorite: favorites.iter().any(|f| f == t.meta().id),
        };

        let mut body = div().flex().flex_col().gap(px(26.0));

        // Suggested row.
        if has_files && !suggested.is_empty() {
            let cards: Vec<AnyElement> = suggested
                .iter()
                .filter_map(|id| reg.get(id))
                .map(|tool| {
                    order.push(tool.meta().id.to_string());
                    let id = tool.meta().id.to_string();
                    tool_card(&k, &tool, card_state(&tool), "sug")
                        .on_click(cx.listener(move |this, _, _, cx| this.open_tool(&id, cx)))
                        .into_any_element()
                })
                .collect();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        ui::sans()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .text_size(px(ui::size::SECTION))
                            .font_weight(FontWeight::BOLD)
                            .child(ui::diamond(12.0, k.ink))
                            .child(t("app.home.suggested")),
                    )
                    .child(div().grid().grid_cols(cols as u16).gap(px(12.0)).children(cards)),
            );
        }

        let mut n = 0;
        for (fam, _) in &families {
            if self.family.is_some_and(|f| f != *fam) {
                continue;
            }
            let mut tools: Vec<Arc<dyn Tool>> = reg.family(*fam).cloned().collect();
            if has_files {
                // Tools that take the files first, the rest after (faded).
                tools.sort_by_key(|t| if self.files.iter().any(|f| t.accepts(f)) { 0 } else { 1 });
            }
            n += 1;
            let cards: Vec<AnyElement> = tools
                .iter()
                .map(|tool| {
                    order.push(tool.meta().id.to_string());
                    let id = tool.meta().id.to_string();
                    let soon = tool.meta().status == ToolStatus::Soon;
                    tool_card(&k, tool, card_state(tool), "card")
                        .when(!soon, |d| d.on_click(cx.listener(move |this, _, _, cx| this.open_tool(&id, cx))))
                        .into_any_element()
                })
                .collect();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(ui::section_title(&k, n, fam.name()))
                    .child(div().grid().grid_cols(cols as u16).gap(px(12.0)).children(cards)),
            );
        }
        self.order = order;

        let total = reg.all().len();
        let subtitle = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.0))
            .child(ta("app.home.count", &[("count", &total.to_string())]))
            .child("·")
            .child(t("app.home.offline"))
            .child("·")
            .child(t("app.home.free"))
            .into_any_element();

        div()
            .id("home")
            .key_context("Home")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .px(px(PAGE_PAD))
                    .pb(px(48.0))
                    .flex()
                    .flex_col()
                    .gap(px(22.0))
                    .child(ui::page_header(cx, t("app.nav.tools"), Some(total.to_string()), Some(subtitle)))
                    .child(self.render_drop_zone(&k, cx))
                    .child(self.render_tabs(&k, &families, cx))
                    .child(body),
            )
    }
}

impl Home {
    fn render_drop_zone(&self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let link = k.link;
        let tint = Hsla { a: 0.08, ..k.link };
        let content: AnyElement = if self.files.is_empty() {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .child(ui::icon("file-down", k.muted).size(px(20.0)))
                .child(
                    ui::roman()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_color(k.ink)
                        .child(if self.loading { t("app.home.reading") } else { t("app.home.drop") }),
                )
                .child(
                    gpui_kit::component::button::Button::new("choose-files")
                        .label(t("app.home.choose"))
                        .on_click(cx.listener(|this, _, window, cx| this.choose_files(window, cx))),
                )
                .into_any_element()
        } else {
            let n = self.files.len();
            let size: u64 = self.files.iter().map(|f| f.size).sum();
            let names: Vec<String> = self.files.iter().take(3).map(|f| f.name()).collect();
            let more = if n > 3 { format!(" …") } else { String::new() };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(14.0))
                .child(ui::icon("files", k.ink).size(px(20.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .child(
                            ui::sans()
                                .font_weight(FontWeight::BOLD)
                                .child(format!("{} · {}", ta("common.n_files", &[("count", &n.to_string())]), castor_core::units::bytes(size))),
                        )
                        .child(ui::roman().text_size(px(13.5)).text_color(k.muted).truncate().child(format!("{}{more}", names.join(", ")))),
                )
                .child(
                    ui::roman().text_size(px(13.5)).text_color(k.muted).child(t("app.home.pick_tool")),
                )
                .child(
                    gpui_kit::component::button::Button::new("clear-files")
                        .label(t("app.common.clear"))
                        .on_click(cx.listener(|this, _, _, cx| this.clear_files(cx))),
                )
                .into_any_element()
        };
        div()
            .id("drop-zone")
            .px(px(18.0))
            .py(px(16.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(if self.files.is_empty() { k.rule } else { k.ink })
            .bg(k.raised)
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(link).bg(tint))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                this.set_files(paths.paths().to_vec(), window, cx);
            }))
            .child(content)
    }

    fn render_tabs(&self, k: &Tokens, families: &[(Family, usize)], cx: &mut Context<Self>) -> impl IntoElement {
        let mut items: Vec<(Option<Family>, String, Option<usize>)> = vec![(None, castor_core::tr("app.home.all"), None)];
        for (f, n) in families {
            items.push((Some(*f), f.name(), Some(*n)));
        }
        let current = self.family;
        div().flex().flex_row().flex_wrap().items_center().gap(px(4.0)).children(items.into_iter().enumerate().map(|(i, (f, name, count))| {
            let sel = current == f;
            div()
                .id(("fam-tab", i))
                .tab_index(0)
                .px(px(8.0))
                .py(px(3.0))
                .flex()
                .flex_row()
                .items_baseline()
                .gap(px(5.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .when(sel, |d| d.bg(k.ink).text_color(k.paper))
                .when(!sel, |d| d.text_color(k.ink).hover(|s| s.bg(k.surface)))
                .focus_visible(|s| s.border_1().border_color(k.link))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.family = f;
                    this.sel = None;
                    cx.notify();
                }))
                .child(ui::sans().child(name))
                .when_some(count, |d, n| {
                    d.child(ui::sans().text_size(px(12.5)).text_color(if sel { k.paper } else { k.muted }).child(n.to_string()))
                })
        }))
    }
}

/// "Studios" — visual editors arrive in a later version.
pub fn render_studios(_window: &mut Window, cx: &mut App) -> impl IntoElement {
    let k = tokens(cx);
    div().id("studios").size_full().overflow_y_scroll().child(
        div()
            .px(px(PAGE_PAD))
            .pb(px(48.0))
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(ui::page_header(cx, PageId::Studios.title(), None, Some(t("app.studios.subtitle").into_any_element())))
            .child(ui::section_title(&k, 1, t("app.studios.soon_title")))
            .child(ui::roman().max_w(px(640.0)).text_color(k.ink).child(t("app.studios.body")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .children(["pdf", "trim", "image"].iter().map(|s| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .child(ui::diamond(10.0, k.muted))
                            .child(ui::roman().text_color(k.ink).child(t(&format!("app.studios.{s}"))))
                    })),
            ),
    )
}
