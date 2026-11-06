//! The main window: title bar with search, sidebar, the current page, the command palette.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::{ActiveTheme as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::{CloseWindow, OpenPalette};
use crate::chains::ChainsPage;
use crate::home::Home;
use crate::model::{ModelEvent, Nav, PageId, UpdateState, model};
use crate::palette::{Palette, PaletteEvent};
use crate::runs::RunsPage;
use crate::settings_page::SettingsPage;
use crate::theme::{fonts, tokens};
use crate::tool_screen::ToolScreen;
use crate::ui::{self, t};

pub const SIDEBAR_W: f32 = 216.0;

#[derive(Clone, PartialEq)]
enum Current {
    Page(PageId),
    Tool(String),
}

pub struct Shell {
    focus: FocusHandle,
    current: Current,
    home: Entity<Home>,
    tools: HashMap<String, Entity<ToolScreen>>,
    runs: Option<Entity<RunsPage>>,
    chains: Option<Entity<ChainsPage>>,
    settings: Option<Entity<SettingsPage>>,
    palette: Option<Entity<Palette>>,
    _subs: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = model(cx);
        let mut subs = vec![
            cx.subscribe_in(&m, window, |this, _, ev: &ModelEvent, window, cx| match ev {
                ModelEvent::Navigate(nav) => this.navigate(nav.clone(), window, cx),
            }),
            cx.observe(&m, |_, _, cx| cx.notify()),
        ];
        // Follow the OS appearance live when Appearance = System.
        subs.push(cx.observe_window_appearance(window, |_, window, cx| {
            let a = crate::model::settings(cx).appearance;
            if a == castor_core::settings::Appearance::System {
                crate::theme::apply(a, Some(window), cx);
            }
        }));
        let home = cx.new(|cx| Home::new(window, cx));
        let focus = cx.focus_handle();
        Shell {
            focus,
            current: Current::Page(PageId::Tools),
            home,
            tools: HashMap::new(),
            runs: None,
            chains: None,
            settings: None,
            palette: None,
            _subs: subs,
        }
    }

    pub fn navigate(&mut self, nav: Nav, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        match nav {
            Nav::Page(p) => self.show_page(p, window, cx),
            Nav::Files(paths) => {
                self.current = Current::Page(PageId::Tools);
                self.home.update(cx, |h, cx| h.set_files(paths, window, cx));
            }
            Nav::Tool { id, files, values } => {
                if crate::model::registry(cx).get(&id).is_none() {
                    tracing::warn!("unknown tool {id}");
                    return;
                }
                let screen = self.tool_screen(&id, window, cx);
                screen.update(cx, |s, cx| {
                    if let Some(v) = values {
                        s.set_values(v, window, cx);
                    }
                    if !files.is_empty() {
                        s.set_files(files, window, cx);
                    }
                    s.focus(window, cx);
                });
                self.current = Current::Tool(id);
            }
            Nav::Chain(name) => {
                self.show_page(PageId::Chains, window, cx);
                if let Some(c) = &self.chains {
                    c.update(cx, |c, cx| c.open_by_name(&name, window, cx));
                }
            }
            Nav::Palette(q) => self.open_palette_with(q, window, cx),
        }
        cx.notify();
    }

    fn tool_screen(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<ToolScreen> {
        if let Some(s) = self.tools.get(id) {
            return s.clone();
        }
        let id2 = id.to_string();
        let s = cx.new(|cx| ToolScreen::new(&id2, window, cx));
        self.tools.insert(id.to_string(), s.clone());
        s
    }

    fn show_page(&mut self, p: PageId, window: &mut Window, cx: &mut Context<Self>) {
        match p {
            PageId::Runs if self.runs.is_none() => self.runs = Some(cx.new(|cx| RunsPage::new(window, cx))),
            PageId::Chains if self.chains.is_none() => self.chains = Some(cx.new(|cx| ChainsPage::new(window, cx))),
            PageId::Settings if self.settings.is_none() => {
                self.settings = Some(cx.new(|cx| SettingsPage::new(window, cx)))
            }
            _ => {}
        }
        if p == PageId::Tools {
            self.home.update(cx, |h, cx| h.focus(window, cx));
        }
        self.current = Current::Page(p);
        cx.notify();
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_with(None, window, cx);
    }

    pub fn open_palette_with(&mut self, query: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let p = cx.new(|cx| Palette::new(window, cx));
        if let Some(q) = query {
            p.update(cx, |p, cx| p.set_query(&q, window, cx));
        }
        let sub = cx.subscribe_in(&p, window, |this, _, ev: &PaletteEvent, window, cx| {
            this.palette = None;
            match ev {
                PaletteEvent::Dismiss => {}
                PaletteEvent::Go(nav) => this.navigate(nav.clone(), window, cx),
                PaletteEvent::Action(a) => crate::palette::perform(*a, window, cx),
            }
            cx.notify();
        });
        self._subs.push(sub);
        self.palette = Some(p);
        cx.notify();
    }

    /// For `--run`: start the current tool screen's run.
    pub fn run_current_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Current::Tool(id) = &self.current {
            if let Some(s) = self.tools.get(id).cloned() {
                s.update(cx, |s, cx| s.start(window, cx));
            }
        }
    }

    fn on_drop_files(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = paths.paths().to_vec();
        match &self.current {
            Current::Tool(id) => {
                if let Some(s) = self.tools.get(id).cloned() {
                    s.update(cx, |s, cx| s.add_files(paths, window, cx));
                }
            }
            _ => self.navigate(Nav::Files(paths), window, cx),
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let dark = cx.theme().is_dark();
        TitleBar::new()
            .bg(k.paper)
            .border_color(k.rule)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .h_full()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .w(px(SIDEBAR_W - 24.0))
                            .flex_none()
                            .child(img(if dark { crate::assets::MARK_DARK } else { crate::assets::MARK_LIGHT }).size(px(18.0)))
                            .child(
                                ui::sans()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(k.ink)
                                    .child(t("app.name")),
                            ),
                    )
                    .child(div().flex_1().flex().justify_center().child(self.render_search(cx)))
                    .child(div().w(px(SIDEBAR_W - 24.0)).flex_none()),
            )
    }

    fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        div()
            .id("title-search")
            .w(px(460.0))
            .max_w_full()
            .h(px(24.0))
            .px(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(k.rule)
            .bg(k.raised)
            .cursor_pointer()
            .hover(|s| s.border_color(k.muted))
            // The title bar starts a window move on mouse down: keep clicks here.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)))
            .child(ui::icon("search", k.faint).size(px(14.0)))
            .child(
                ui::sans()
                    .flex_1()
                    .text_size(px(13.5))
                    .text_color(k.faint)
                    .truncate()
                    .child(t("app.search.placeholder")),
            )
            .child(
                ui::sans()
                    .text_size(px(12.5))
                    .text_color(k.muted)
                    .px(px(5.0))
                    .rounded(px(2.0))
                    .border_1()
                    .border_color(k.rule)
                    .child(format!("{}K", ui::cmd_key())),
            )
    }

    fn nav_item(&self, id: &'static str, label: SharedString, icon: &'static str, selected: bool, nav: Nav, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        div()
            .id(id)
            .role(Role::Link)
            .aria_label(label.clone())
            .tab_index(0)
            .h(px(30.0))
            .px(px(10.0))
            .mx(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(9.0))
            .rounded(px(3.0))
            .cursor_pointer()
            .when(selected, |d| d.bg(k.raised).border_1().border_color(k.rule))
            .when(!selected, |d| d.hover(|s| s.bg(k.ink_a(0.04))))
            .focus_visible(|s| s.border_1().border_color(k.link))
            .on_click(cx.listener(move |this, _, window, cx| this.navigate(nav.clone(), window, cx)))
            .child(ui::icon(icon, if selected { k.ink } else { k.muted }).size(px(15.0)))
            .child(
                ui::sans()
                    .text_color(k.ink)
                    .when(selected, |d| d.font_weight(FontWeight::BOLD))
                    .child(label),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let m = model(cx).read(cx);
        let reg = m.registry.clone();
        let favorites: Vec<String> = m.settings.favorites.iter().filter(|id| reg.get(id).is_some()).cloned().collect();
        let update = m.update.clone();
        let version = m.version;
        let cur = self.current.clone();
        let is = |p: PageId| cur == Current::Page(p);
        let fav_items: Vec<AnyElement> = favorites
            .iter()
            .map(|id| {
                let tool = reg.get(id).unwrap();
                let meta = tool.meta();
                let selected = cur == Current::Tool(id.clone());
                let nav = Nav::Tool { id: id.clone(), files: vec![], values: None };
                div()
                    .id(SharedString::from(format!("fav-{id}")))
                    .h(px(28.0))
                    .px(px(10.0))
                    .mx(px(8.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(9.0))
                    .rounded(px(3.0))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(k.raised).border_1().border_color(k.rule))
                    .when(!selected, |d| d.hover(|s| s.bg(k.tint(meta.family))))
                    .on_click(cx.listener(move |this, _, window, cx| this.navigate(nav.clone(), window, cx)))
                    .child(ui::family_swatch(&k, meta.family, 10.0))
                    .child(ui::sans().text_size(px(13.5)).text_color(k.ink).truncate().child(meta.name()))
                    .into_any_element()
            })
            .collect();
        let dot = matches!(update, UpdateState::Ready { .. } | UpdateState::Available { .. });

        div()
            .w(px(SIDEBAR_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(k.surface)
            .border_r_1()
            .border_color(k.rule)
            .pt(px(12.0))
            .child(self.nav_item("nav-tools", PageId::Tools.title(), "layout-grid", is(PageId::Tools), Nav::Page(PageId::Tools), cx))
            .child(self.nav_item("nav-studios", PageId::Studios.title(), "pen-tool", is(PageId::Studios), Nav::Page(PageId::Studios), cx))
            .child(self.nav_item("nav-chains", PageId::Chains.title(), "link", is(PageId::Chains), Nav::Page(PageId::Chains), cx))
            .child(self.nav_item("nav-runs", PageId::Runs.title(), "clock", is(PageId::Runs), Nav::Page(PageId::Runs), cx))
            .child(
                div()
                    .id("favorites")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .pt(px(20.0))
                    .child(
                        div()
                            .px(px(18.0))
                            .pb(px(6.0))
                            .font_family(fonts().caps.clone())
                            .text_size(px(13.0))
                            .text_color(k.muted)
                            .child(ui::small_caps(&t("app.nav.favorites"))),
                    )
                    .children(fav_items)
                    .when(favorites.is_empty(), |d| {
                        d.child(
                            ui::roman()
                                .px(px(18.0))
                                .text_size(px(13.5))
                                .text_color(k.faint)
                                .child(t("app.nav.favorites_empty")),
                        )
                    }),
            )
            .child(ui::hrule(k.rule, 1.0))
            .child(div().pt(px(8.0)).child(self.nav_item(
                "nav-settings",
                PageId::Settings.title(),
                "settings",
                is(PageId::Settings),
                Nav::Page(PageId::Settings),
                cx,
            )))
            .child(
                div()
                    .id("version")
                    .px(px(18.0))
                    .py(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Nav::Page(PageId::Settings), window, cx)))
                    .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(format!("v{version}")))
                    .when(dot, |d| {
                        d.child(div().size(px(7.0)).rounded_full().bg(k.link))
                            .child(ui::sans().text_size(px(13.0)).text_color(k.link).child(t("app.update.ready_short")))
                    }),
            )
    }

    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.current.clone() {
            Current::Page(PageId::Tools) => self.home.clone().into_any_element(),
            Current::Page(PageId::Studios) => crate::home::render_studios(window, cx).into_any_element(),
            Current::Page(PageId::Runs) => self.runs.clone().map(|e| e.into_any_element()).unwrap_or_else(|| div().into_any_element()),
            Current::Page(PageId::Chains) => self.chains.clone().map(|e| e.into_any_element()).unwrap_or_else(|| div().into_any_element()),
            Current::Page(PageId::Settings) => {
                self.settings.clone().map(|e| e.into_any_element()).unwrap_or_else(|| div().into_any_element())
            }
            Current::Tool(id) => self.tools.get(&id).cloned().map(|e| e.into_any_element()).unwrap_or_else(|| div().into_any_element()),
        }
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let page = self.render_page(window, cx);
        div()
            .id("shell")
            .key_context("Shell")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(k.paper)
            .text_color(k.ink)
            .font_family(fonts().sans.clone())
            .text_size(px(14.0))
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.on_drop_files(paths, window, cx)))
            .child(self.render_title_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.render_sidebar(cx))
                    .child(div().flex_1().min_w(px(0.0)).h_full().child(page)),
            )
            .when_some(self.palette.clone(), |d, p| d.child(p))
    }
}
