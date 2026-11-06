//! House-style building blocks: page header with ghost initial and page rule, numbered section
//! titles, captions, booktabs rules, diamonds, segmented controls, tags.

use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets;
use crate::theme::{Tokens, fonts, tokens};

/// Translate an app string.
pub fn t(key: &str) -> SharedString {
    castor_core::tr(key).into()
}

/// Translate with `%{name}` placeholders.
pub fn ta(key: &str, args: &[(&str, &str)]) -> SharedString {
    castor_core::tr_args(key, args).into()
}

/// Sizes (DESIGN §2).
pub mod size {
    pub const UI: f32 = 14.0;
    pub const SECTION: f32 = 20.0;
    pub const PAGE_TITLE: f32 = 30.0;
    pub const GHOST: f32 = 110.0;
    pub const PROSE: f32 = 15.0;
    pub const CAPTION: f32 = 13.5;
    pub const SMALL: f32 = 13.5;
    pub const MONO: f32 = 13.0;
    pub const CARD_NAME: f32 = 14.5;
}

pub fn sans() -> Div {
    div().font_family(fonts().sans.clone()).text_size(px(size::UI))
}
pub fn roman() -> Div {
    div().font_family(fonts().roman.clone()).text_size(px(size::PROSE))
}
pub fn mono() -> Div {
    div().font_family(fonts().mono.clone()).text_size(px(size::MONO))
}
pub fn caps() -> Div {
    div().font_family(fonts().caps.clone()).text_size(px(13.0))
}

/// The diamond motif.
pub fn diamond(side: f32, color: Hsla) -> Svg {
    svg().path(assets::DIAMOND).size(px(side)).flex_none().text_color(color)
}
pub fn diamond_filled(side: f32, color: Hsla) -> Svg {
    svg().path(assets::DIAMOND_FILLED).size(px(side)).flex_none().text_color(color)
}

/// A Lucide icon as an svg element (16 px by default).
pub fn icon(name: &str, color: Hsla) -> Svg {
    svg().path(assets::lucide(name)).size(px(16.0)).flex_none().text_color(color)
}

/// Family swatch: a diamond filled with the family tint, outlined in its deeper shade.
pub fn family_swatch(k: &Tokens, f: castor_core::Family, side: f32) -> Div {
    div()
        .relative()
        .size(px(side))
        .flex_none()
        .child(diamond_filled(side, k.tint(f)).absolute().top_0().left_0())
        .child(diamond(side, k.tint_ink(f)).absolute().top_0().left_0())
}

/// Page header (DESIGN §2–3): ghost initial in Chorus behind the title, the page rule with a
/// number top-right, and an optional subtitle in roman.
pub fn page_header(cx: &App, title: SharedString, number: Option<String>, subtitle: Option<AnyElement>) -> Div {
    let k = tokens(cx);
    let initial: String = title.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div()
        .relative()
        .w_full()
        .pt(px(34.0))
        .child(page_rule(&k, number))
        .child(
            div()
                .relative()
                .child(
                    div()
                        .absolute()
                        .left(px(-22.0))
                        .top(px(-58.0))
                        .font_family(fonts().chorus.clone())
                        .italic()
                        .text_size(px(size::GHOST))
                        .line_height(px(size::GHOST * 1.1))
                        .text_color(k.ink_a(0.10))
                        .child(initial),
                )
                .child(
                    sans()
                        .relative()
                        .text_size(px(size::PAGE_TITLE))
                        .line_height(px(38.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(k.ink)
                        .child(title),
                ),
        )
        .when_some(subtitle, |d, s| d.child(roman().mt(px(6.0)).text_color(k.muted).child(s)))
}

/// Thin rule ending in a diamond, number above it (like the page number of /nutrition).
pub fn page_rule(k: &Tokens, number: Option<String>) -> Div {
    div()
        .absolute()
        .top(px(0.0))
        .right(px(0.0))
        .w(px(260.0))
        .flex()
        .flex_col()
        .items_end()
        .child(
            sans()
                .h(px(18.0))
                .pr(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(k.ink)
                .child(number.unwrap_or_default()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .child(div().flex_1().h(px(1.0)).bg(k.rule))
                .child(diamond(9.0, k.muted)),
        )
}

/// "1.  PDF" — numbered section title.
pub fn section_title(k: &Tokens, n: usize, title: impl Into<SharedString>) -> Div {
    sans()
        .flex()
        .flex_row()
        .gap(px(12.0))
        .text_size(px(size::SECTION))
        .font_weight(FontWeight::BOLD)
        .text_color(k.ink)
        .child(format!("{n}."))
        .child(title.into())
}

/// A smaller numbered heading for panels ("1. Options").
pub fn sub_title(k: &Tokens, n: usize, title: impl Into<SharedString>) -> Div {
    sans()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .text_size(px(16.5))
        .font_weight(FontWeight::BOLD)
        .text_color(k.ink)
        .child(format!("{n}."))
        .child(title.into())
}

/// "Figure 1 / caption" or "Table 1 — caption" (label bold).
pub fn caption(k: &Tokens, label: impl Into<SharedString>, sep: &str, text: impl Into<SharedString>) -> Div {
    roman()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.0))
        .text_size(px(size::CAPTION))
        .text_color(k.muted)
        .child(div().font_weight(FontWeight::BOLD).text_color(k.ink).child(label.into()))
        .child(sep.to_string())
        .child(text.into())
}

/// Horizontal rule.
pub fn hrule(color: Hsla, thick: f32) -> Div {
    div().w_full().h(px(thick)).flex_none().bg(color)
}

/// Booktabs strong rule (1.2 px).
pub fn rule_strong(k: &Tokens) -> Div {
    hrule(k.rule_strong, 1.2)
}

/// Small tag ("PDF → PDF", "Soon", "3 files").
pub fn tag(k: &Tokens, text: impl Into<SharedString>, bg: Hsla) -> Div {
    caps()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(2.0))
        .bg(bg)
        .text_color(k.muted)
        .whitespace_nowrap()
        .child(text.into())
}

pub type OnSelect = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Segmented control: ink fill on the selected segment (DESIGN §3).
pub fn segmented(
    id: impl Into<SharedString>,
    k: &Tokens,
    items: Vec<(String, SharedString)>,
    selected: &str,
    on_select: OnSelect,
) -> Div {
    let id: SharedString = id.into();
    let n = items.len();
    // The outer row keeps the control as wide as its segments, not as its parent.
    div().flex().flex_row().child(div()
        .flex()
        .flex_row()
        .flex_wrap()
        .border_1()
        .border_color(k.rule)
        .rounded(px(3.0))
        .bg(k.raised)
        .overflow_hidden()
        .children(items.into_iter().enumerate().map(|(i, (value, label))| {
            let sel = value == selected;
            let cb = on_select.clone();
            let v2 = value.clone();
            let cb2 = on_select.clone();
            let v3 = value.clone();
            sans()
                .id(SharedString::from(format!("{id}-{value}")))
                .role(Role::RadioButton)
                .aria_label(label.clone())
                .tab_index(0)
                .h(px(28.0))
                .px(px(11.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.5))
                .cursor_pointer()
                .when(i + 1 < n, |d| d.border_r_1().border_color(k.rule))
                .when(sel, |d| d.bg(k.ink).text_color(k.paper))
                .when(!sel, |d| d.text_color(k.ink).hover(|s| s.bg(k.surface)))
                .focus_visible(|s| s.border_2().border_color(k.link))
                .on_click(move |_, w, cx| cb(&v2, w, cx))
                .on_key_down(move |ev, w, cx| {
                    if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                        cb2(&v3, w, cx);
                    }
                })
                .child(label)
                .test_support()
        })))
}

/// A link-coloured clickable text.
pub fn link(id: impl Into<ElementId>, k: &Tokens, label: impl Into<SharedString>, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    sans()
        .id(id)
        .text_color(k.link)
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(label.into())
        .on_click(on_click)
}

/// Primary command key name for display.
pub fn cmd_key() -> &'static str {
    if cfg!(target_os = "macos") { "⌘" } else { "Ctrl " }
}



/// Small-caps label ("FAVORITES"): letter-spaced capitals (GPUI has no letter-spacing, so hair
/// spaces between the letters do it).
pub fn small_caps(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && c != ' ' {
            out.push('\u{200A}');
        }
        out.extend(c.to_uppercase());
    }
    out
}
