//! Studios: full-window editors for jobs a form cannot do (DESIGN: one main canvas, one side
//! panel, a bottom bar with the primary action). They use the engines directly for display and
//! save through the runner (history, undo).
//!
//! - PDF Studio (`pdf_view`, `pdf_engine`): organize, annotate, sign, redact, fill forms.
//! - Trim & Caption Studio (`media_view`, `media_engine`): cut, remove silences, edit by
//!   transcript, captions.

use castor_core::history::{RunRecord, RunStatus};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::home::PAGE_PAD;
use crate::model::{Nav, PageId, navigate};
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t};

#[cfg(feature = "tools")]
pub mod media_engine;
#[cfg(feature = "tools")]
pub mod media_view;
#[cfg(feature = "tools")]
pub mod pdf_engine;
#[cfg(feature = "tools")]
pub mod pdf_view;
#[cfg(feature = "tools")]
pub mod sign_pad;
#[cfg(feature = "tools")]
pub mod tools;
#[cfg(all(test, feature = "tools"))]
mod e2e_tests;

/// `--mode` / `--script` of the launch (deterministic screenshots, demos): taken once by the
/// studio that opens first.
#[derive(Default)]
pub struct Setup {
    pub mode: Option<String>,
    pub script: Option<std::path::PathBuf>,
}
impl Global for Setup {}

pub fn take_setup(cx: &mut App) -> Setup {
    cx.try_global::<Setup>().is_some().then(|| std::mem::take(cx.global_mut::<Setup>())).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StudioKind {
    Pdf,
    Media,
}

impl StudioKind {
    pub fn key(self) -> &'static str {
        match self {
            StudioKind::Pdf => "pdf",
            StudioKind::Media => "media",
        }
    }
    pub fn title(self) -> SharedString {
        t(&format!("app.studio.{}.name", self.key()))
    }
    /// The studio that edits this file, if any.
    pub fn for_path(p: &std::path::Path) -> Option<StudioKind> {
        match castor_core::FileInfo::from_path(p).kind {
            castor_core::FileKind::Pdf => Some(StudioKind::Pdf),
            castor_core::FileKind::Video | castor_core::FileKind::Audio => Some(StudioKind::Media),
            _ => None,
        }
    }
}

/// "Studios / PDF Studio / file.pdf".
pub fn crumbs(k: &Tokens, studio: SharedString, file: Option<String>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .text_size(px(13.5))
        .min_w(px(0.0))
        .child(ui::link("crumb-studios", k, t("app.nav.studios"), |_, _, cx| navigate(Nav::Page(PageId::Studios), cx)))
        .child(ui::sans().text_color(k.faint).child("/"))
        .child(ui::sans().text_color(if file.is_some() { k.muted } else { k.ink }).whitespace_nowrap().child(studio))
        .when_some(file, |d, f| d.child(ui::sans().text_color(k.faint).child("/")).child(ui::sans().text_color(k.ink).truncate().child(f)))
}

/// Status, summary line and "Show in folder" of a finished save / export.
pub fn run_summary(k: &Tokens, r: &RunRecord, d: Div) -> Div {
    let (color, status) = match r.status {
        RunStatus::Complete => (k.good, t("app.studio.saved")),
        RunStatus::Cancelled => (k.muted, t("app.status.cancelled")),
        _ => (k.bad, t("app.status.failed")),
    };
    let first = r.outputs.first().cloned();
    let name = first.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let err = r.errors.first().cloned();
    d.child(ui::sans().text_color(color).font_weight(FontWeight::BOLD).whitespace_nowrap().child(status))
        .when(r.status == RunStatus::Complete, |d| d.child(ui::sans().text_color(k.ink).truncate().min_w(px(0.0)).child(name)))
        .when_some(err, |d, e| d.child(ui::sans().text_color(k.bad).truncate().min_w(px(0.0)).child(e)))
        .when_some(first.filter(|_| r.status == RunStatus::Complete), |d, p| {
            let p2 = p.clone();
            d.child(Button::new("studio-open-result").small().ghost().tooltip(t("common.open")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("external-link"))).on_click(move |_, _, _| {
                let _ = castor_core::process::open(&p2);
            }))
            .child(Button::new("studio-reveal").small().ghost().tooltip(t("common.reveal")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("folder-open"))).on_click(move |_, _, _| {
                let _ = castor_core::process::reveal(&p);
            }))
        })
}

/// A tool for a history record: the registry's, or a studio's own (`studio.pdf`, …).
pub fn find_tool(id: &str, cx: &App) -> Option<std::sync::Arc<dyn castor_core::Tool>> {
    let reg = crate::model::registry(cx);
    if let Some(t) = reg.get(id) {
        return Some(t);
    }
    #[cfg(feature = "tools")]
    {
        tools::by_id(id, reg)
    }
    #[cfg(not(feature = "tools"))]
    {
        None
    }
}

/// The Studios page: what each studio does, and a way in.
pub fn render_page(_window: &mut Window, cx: &mut App) -> impl IntoElement {
    let k = tokens(cx);
    let available = cfg!(feature = "tools");
    let card = |n: usize, kind: StudioKind, icon: &'static str, k: &Tokens| {
        let key = kind.key();
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(ui::section_title(k, n, kind.title()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(18.0))
                    .items_start()
                    .child(
                        div()
                            .size(px(64.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(k.rule)
                            .bg(k.tint(if kind == StudioKind::Pdf { castor_core::Family::Pdf } else { castor_core::Family::Video }))
                            .child(ui::icon(icon, k.ink).size(px(26.0))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .max_w(px(620.0))
                            .child(ui::roman().text_color(k.ink).child(t(&format!("app.studio.{key}.desc"))))
                            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t(&format!("app.studio.{key}.features"))))
                            .child(
                                div().flex().flex_row().gap(px(8.0)).pt(px(4.0)).child(
                                    Button::new(SharedString::from(format!("open-studio-{key}")))
                                        .primary()
                                        .small()
                                        .label(t(&format!("app.studio.{key}.open")))
                                        .disabled(!available)
                                        .on_click(move |_, _, cx| navigate(Nav::Studio(kind, None), cx)),
                                ),
                            ),
                    ),
            )
    };
    div().id("studios").size_full().overflow_y_scroll().child(
        div()
            .px(px(PAGE_PAD))
            .pb(px(48.0))
            .flex()
            .flex_col()
            .gap(px(28.0))
            .child(ui::page_header(cx, PageId::Studios.title(), Some("2".into()), Some(t("app.studios.subtitle").into_any_element())))
            .when(!available, |d| d.child(ui::roman().italic().text_color(k.warn).child(t("app.studio.unavailable"))))
            .child(card(1, StudioKind::Pdf, "file-pen-line", &k))
            .child(card(2, StudioKind::Media, "scissors", &k))
            .child(ui::roman().text_size(px(14.0)).italic().text_color(k.muted).child(t("app.studio.footer"))),
    )
}
