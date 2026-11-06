//! Command palette (⌘K / Ctrl+K, and the title-bar search): tools, chains, pages, actions.

use castor_core::Family;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Nav, PageId, model};
use crate::theme::{fonts, tokens};
use crate::ui::{self, t};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    CheckUpdates,
    OpenOutputFolder,
    ToggleTheme,
    Quit,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::CheckUpdates, Action::OpenOutputFolder, Action::ToggleTheme, Action::Quit];
    fn key(self) -> &'static str {
        match self {
            Action::CheckUpdates => "check_updates",
            Action::OpenOutputFolder => "open_output",
            Action::ToggleTheme => "toggle_theme",
            Action::Quit => "quit",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Action::CheckUpdates => "refresh-cw",
            Action::OpenOutputFolder => "folder-open",
            Action::ToggleTheme => "sun-moon",
            Action::Quit => "power",
        }
    }
}

pub enum PaletteEvent {
    Dismiss,
    Go(Nav),
    Action(Action),
}

#[derive(Clone)]
pub enum Target {
    Nav(Nav),
    Action(Action),
}

#[derive(Clone)]
pub struct Item {
    pub group: &'static str,
    pub title: SharedString,
    pub detail: SharedString,
    pub family: Option<Family>,
    pub icon: &'static str,
    pub target: Target,
}

pub struct Palette {
    input: Entity<InputState>,
    items: Vec<Item>,
    sel: usize,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

/// All palette entries for a query, best first within groups (tools, chains, pages, actions).
pub fn entries(q: &str, cx: &App) -> Vec<Item> {
    let m = model(cx).read(cx);
    let reg = m.registry.clone();
    let recent = m.recent_tools(8);
    let favs = m.settings.favorites.clone();
    let mut out = Vec::new();
    let tools = crate::search::tools(&reg, q, &favs, &recent);
    let limit = if q.trim().is_empty() { 6 } else { 8 };
    for (tool, _) in tools.into_iter().take(limit) {
        let meta = tool.meta();
        out.push(Item {
            group: "app.palette.tools",
            title: meta.name().into(),
            detail: meta.desc().into(),
            family: Some(meta.family),
            icon: "",
            target: Target::Nav(Nav::Tool { id: meta.id.to_string(), files: vec![], values: None }),
        });
    }
    if !q.trim().is_empty() {
        let mut chains: Vec<(i32, String)> = castor_core::chain::templates()
            .into_iter()
            .map(|c| c.name)
            .chain(castor_core::chain::load_all().into_iter().map(|(_, c)| c.name))
            .map(|n| (crate::search::label_score(q, &n), n))
            .filter(|(s, _)| *s > 0)
            .collect();
        chains.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, name) in chains.into_iter().take(3) {
            out.push(Item {
                group: "app.palette.chains",
                title: name.clone().into(),
                detail: SharedString::default(),
                family: None,
                icon: "link",
                target: Target::Nav(Nav::Chain(name)),
            });
        }
    }
    for p in [PageId::Tools, PageId::Chains, PageId::Runs, PageId::Settings, PageId::Studios] {
        let title = p.title();
        if crate::search::label_score(q, &title) > 0 && (q.trim().len() > 0 || p != PageId::Tools) {
            out.push(Item {
                group: "app.palette.pages",
                title,
                detail: SharedString::default(),
                family: None,
                icon: match p {
                    PageId::Tools => "layout-grid",
                    PageId::Studios => "pen-tool",
                    PageId::Chains => "link",
                    PageId::Runs => "clock",
                    PageId::Settings => "settings",
                },
                target: Target::Nav(Nav::Page(p)),
            });
        }
    }
    // Studios, findable by name and by what they do ("sign", "redact", "subtitles"…).
    if !q.trim().is_empty() && cfg!(feature = "tools") {
        for kind in [crate::studio::StudioKind::Pdf, crate::studio::StudioKind::Media] {
            let title = kind.title();
            let words = castor_core::tr(&format!("app.studio.{}.keywords", kind.key()));
            let score = crate::search::label_score(q, &title).max(crate::search::label_score(q, &words));
            if score > 0 {
                out.push(Item {
                    group: "app.palette.pages",
                    title,
                    detail: crate::ui::t(&format!("app.studio.{}.desc", kind.key())),
                    family: None,
                    icon: if kind == crate::studio::StudioKind::Pdf { "file-pen-line" } else { "scissors" },
                    target: Target::Nav(Nav::Studio(kind, None)),
                });
            }
        }
    }
    for a in Action::ALL {
        let title = castor_core::tr(&format!("app.palette.action.{}", a.key()));
        if crate::search::label_score(q, &title) > 0 {
            out.push(Item {
                group: "app.palette.actions",
                title: title.into(),
                detail: SharedString::default(),
                family: None,
                icon: a.icon(),
                target: Target::Action(a),
            });
        }
    }
    out
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.search.placeholder")));
        input.update(cx, |i, cx| i.focus(window, cx));
        let mut subs = vec![cx.subscribe(&input, |this, st, ev: &InputEvent, cx| match ev {
            InputEvent::Change => {
                let q = st.read(cx).value().to_string();
                this.items = entries(&q, cx);
                this.sel = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.confirm(cx),
            _ => {}
        })];
        // Arrow keys and Escape go to the list even though the input has focus.
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        subs.push(cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let key = ev.keystroke.key.as_str();
            if !matches!(key, "up" | "down" | "escape") {
                return;
            }
            let _ = weak.update(cx, |p, cx| {
                match key {
                    "up" => p.sel = p.sel.saturating_sub(1),
                    "down" => p.sel = (p.sel + 1).min(p.items.len().saturating_sub(1)),
                    _ => cx.emit(PaletteEvent::Dismiss),
                }
                cx.notify();
            });
            cx.stop_propagation();
        }));
        Palette { input, items: entries("", cx), sel: 0, _subs: subs }
    }

    /// Type a query (screenshots, "Search" from elsewhere).
    pub fn set_query(&mut self, q: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_value(q.to_string(), window, cx));
        self.items = entries(q, cx);
        self.sel = 0;
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(it) = self.items.get(self.sel) {
            self.choose(it.target.clone(), cx);
        }
    }

    fn choose(&mut self, t: Target, cx: &mut Context<Self>) {
        match t {
            Target::Nav(n) => cx.emit(PaletteEvent::Go(n)),
            Target::Action(a) => cx.emit(PaletteEvent::Action(a)),
        }
    }
}

/// Run a palette action.
pub fn perform(a: Action, window: &mut Window, cx: &mut App) {
    match a {
        Action::CheckUpdates => {
            crate::updates::check_now(cx);
            crate::model::navigate(Nav::Page(PageId::Settings), cx);
        }
        Action::OpenOutputFolder => {
            let s = crate::model::settings(cx);
            let dir = s.output_folder.clone().unwrap_or_else(castor_core::paths::default_output_dir);
            let _ = std::fs::create_dir_all(&dir);
            let _ = castor_core::process::open(&dir);
        }
        Action::ToggleTheme => {
            use castor_core::settings::Appearance;
            let dark = gpui_kit::component::Theme::global(cx).is_dark();
            let next = if dark { Appearance::Light } else { Appearance::Dark };
            model(cx).update(cx, |m, cx| m.update_settings(cx, |s| s.appearance = next));
            crate::theme::apply(next, Some(window), cx);
        }
        Action::Quit => crate::app::quit(cx),
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last_group = "";
        for (i, it) in self.items.iter().enumerate() {
            if it.group != last_group {
                last_group = it.group;
                rows.push(
                    div()
                        .px(px(14.0))
                        .pt(px(10.0))
                        .pb(px(3.0))
                        .font_family(fonts().caps.clone())
                        .text_size(px(13.0))
                        .text_color(k.muted)
                        .child(ui::small_caps(&t(it.group)))
                        .into_any_element(),
                );
            }
            let sel = i == self.sel;
            let target = it.target.clone();
            rows.push(
                div()
                    .id(("palette-item", i))
                    .mx(px(6.0))
                    .px(px(8.0))
                    .h(px(36.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(3.0))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(k.surface).border_1().border_color(k.rule))
                    .hover(|s| s.bg(k.surface))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(target.clone(), cx)))
                    .child(match it.family {
                        Some(f) => ui::family_swatch(&k, f, 12.0).into_any_element(),
                        None => ui::icon(it.icon, k.muted).size(px(14.0)).into_any_element(),
                    })
                    .child(ui::sans().text_color(k.ink).font_weight(if sel { FontWeight::BOLD } else { FontWeight::NORMAL }).whitespace_nowrap().child(it.title.clone()))
                    .child(ui::roman().flex_1().min_w(px(0.0)).text_size(px(13.5)).text_color(k.muted).truncate().child(it.detail.clone()))
                    .when(sel, |d| d.child(ui::sans().text_size(px(12.5)).text_color(k.faint).child("↵")))
                    .into_any_element(),
            );
        }
        let empty = self.items.is_empty();
        div()
            .id("palette-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(if k.dark { hsla(0.0, 0.0, 0.0, 0.45) } else { hsla(0.0, 0.1, 0.1, 0.12) })
            .flex()
            .flex_col()
            .items_center()
            .pt(px(64.0))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(
                div()
                    .id("palette")
                    .w(px(620.0))
                    .max_h(px(480.0))
                    .flex()
                    .flex_col()
                    .bg(k.raised)
                    .border_1()
                    .border_color(k.rule)
                    .rounded(px(4.0))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().p(px(10.0)).child(Input::new(&self.input).cleanable(true)))
                    .child(ui::hrule(k.rule, 1.0))
                    .child(
                        div()
                            .id("palette-list")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .pb(px(8.0))
                            .children(rows)
                            .when(empty, |d| d.child(ui::roman().p(px(16.0)).italic().text_color(k.muted).child(t("app.palette.nothing")))),
                    )
                    .child(ui::hrule(k.rule, 1.0))
                    .child(
                        ui::sans()
                            .px(px(12.0))
                            .py(px(6.0))
                            .text_size(px(12.5))
                            .text_color(k.faint)
                            .child(t("app.palette.hint")),
                    ),
            )
    }
}
