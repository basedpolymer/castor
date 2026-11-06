//! `castor_theme`: the TeXtured look (DESIGN §1–2) as a GPUI Kit theme plus the extra tokens
//! the components do not know about (paper, surface, raised, rules, family tints…).

use std::rc::Rc;
use std::sync::OnceLock;

use castor_core::Family;
use castor_core::settings::Appearance;
use gpui_kit::component::{Theme, ThemeMode, ThemeSet};
use gpui_kit::{App, Hsla, SharedString, Window, WindowAppearance, rgb};

/// Design tokens of one mode.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub paper: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub ink: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub rule: Hsla,
    pub rule_strong: Hsla,
    pub link: Hsla,
    pub good: Hsla,
    pub warn: Hsla,
    pub bad: Hsla,
    tints: [Hsla; 6],
}

const LIGHT: [u32; 12] = [
    0xfcfbf9, 0xf4f2ee, 0xffffff, 0x1b1616, 0x5c5a55, 0x8c8a84, 0xd9d6cf, 0x1b1616, 0x2f6db5, 0x2e7d4f, 0xa8791a,
    0xc0392b,
];
const DARK: [u32; 12] = [
    0x161514, 0x1e1d1b, 0x242321, 0xe8e6e1, 0xa6a39b, 0x75726b, 0x3a3833, 0xe8e6e1, 0x8db4ea, 0x6cc28f, 0xe0b652,
    0xf4897b,
];
const TINTS_LIGHT: [u32; 6] = [0xfbe2ff, 0xccfaff, 0xade1ff, 0xaafaba, 0xffeca4, 0xffdcc8];
const TINTS_DARK: [u32; 6] = [0x3b2840, 0x1d3a3d, 0x1c3046, 0x1d3b24, 0x3b3417, 0x3e2a1f];

fn hsla(c: u32) -> Hsla {
    rgb(c).into()
}

impl Tokens {
    pub fn new(dark: bool) -> Tokens {
        let c = if dark { DARK } else { LIGHT };
        let t = if dark { TINTS_DARK } else { TINTS_LIGHT };
        Tokens {
            dark,
            paper: hsla(c[0]),
            surface: hsla(c[1]),
            raised: hsla(c[2]),
            ink: hsla(c[3]),
            muted: hsla(c[4]),
            faint: hsla(c[5]),
            rule: hsla(c[6]),
            rule_strong: hsla(c[7]),
            link: hsla(c[8]),
            good: hsla(c[9]),
            warn: hsla(c[10]),
            bad: hsla(c[11]),
            tints: t.map(hsla),
        }
    }

    /// Pastel family tint: card swatch, hover wash, tag background.
    pub fn tint(&self, f: Family) -> Hsla {
        self.tints[Family::ALL.iter().position(|x| *x == f).unwrap_or(0)]
    }

    /// A stronger version of the family tint, readable as a diamond outline on paper.
    pub fn tint_ink(&self, f: Family) -> Hsla {
        let t = self.tint(f);
        if self.dark { Hsla { l: (t.l + 0.35).min(0.8), s: (t.s + 0.2).min(1.0), ..t } } else { Hsla { l: t.l * 0.62, s: (t.s * 0.9).min(1.0), ..t } }
    }

    /// Ink at a given opacity (ghost initials, washes).
    pub fn ink_a(&self, a: f32) -> Hsla {
        Hsla { a, ..self.ink }
    }
}

/// Current tokens.
pub fn tokens(cx: &App) -> Tokens {
    Tokens::new(Theme::global(cx).is_dark())
}

/// Font families, resolved after the fonts are registered (the family name the text system
/// reports differs between DirectWrite and CoreText: "LM Sans 10" vs "Latin Modern Sans").
#[derive(Clone, Debug)]
pub struct Fonts {
    pub sans: SharedString,
    pub roman: SharedString,
    /// Emphasis face (Latin Modern Roman Slanted when the platform lists it separately).
    #[allow(dead_code)]
    pub slanted: SharedString,
    pub caps: SharedString,
    pub mono: SharedString,
    pub chorus: SharedString,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();

pub fn fonts() -> &'static Fonts {
    FONTS.get_or_init(|| Fonts {
        sans: "LM Sans 10".into(),
        roman: "LM Roman 10".into(),
        slanted: "LM Roman Slanted 10".into(),
        caps: "LM Roman Caps 10".into(),
        mono: "LM Mono 10".into(),
        chorus: "TeX Gyre Chorus".into(),
    })
}

/// Register the bundled fonts and work out their family names.
pub fn register_fonts(cx: &mut App) {
    let ts = cx.text_system().clone();
    if let Err(e) = ts.add_fonts(crate::assets::FONTS.iter().map(|b| std::borrow::Cow::Borrowed(*b)).collect()) {
        tracing::warn!("could not register fonts: {e:#}");
    }
    let names = ts.all_font_names();
    let pick = |cands: &[&str]| -> SharedString {
        cands
            .iter()
            .find(|c| names.iter().any(|n| n.eq_ignore_ascii_case(c)))
            .map(|s| SharedString::from(s.to_string()))
            .unwrap_or_else(|| cands[0].to_string().into())
    };
    let f = Fonts {
        sans: pick(&["Latin Modern Sans 10", "LM Sans 10", "Latin Modern Sans"]),
        roman: pick(&["Latin Modern Roman 10", "LM Roman 10", "Latin Modern Roman"]),
        slanted: pick(&["Latin Modern Roman Slanted 10", "LM Roman Slanted 10", "Latin Modern Roman Slanted", "Latin Modern Roman 10"]),
        caps: pick(&["Latin Modern Roman Caps 10", "LM Roman Caps 10", "Latin Modern Roman Caps"]),
        mono: pick(&["Latin Modern Mono 10", "LM Mono 10", "Latin Modern Mono"]),
        chorus: pick(&["TeXGyreChorus", "TeX Gyre Chorus"]),
    };
    let ours: Vec<&String> = names.iter().filter(|n| n.contains("LM") || n.contains("Latin") || n.contains("Chorus")).collect();
    tracing::info!(?f, ?ours, "fonts");
    let _ = FONTS.set(f);
}

fn hex(c: Hsla) -> String {
    let r = gpui_kit::Rgba::from(c);
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        (r.r * 255.0).round() as u8,
        (r.g * 255.0).round() as u8,
        (r.b * 255.0).round() as u8,
        (r.a * 255.0).round() as u8
    )
}

fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (x, y) = (gpui_kit::Rgba::from(a), gpui_kit::Rgba::from(b));
    gpui_kit::Rgba { r: x.r + (y.r - x.r) * t, g: x.g + (y.g - x.g) * t, b: x.b + (y.b - x.b) * t, a: 1.0 }.into()
}

/// The GPUI Kit theme configuration (JSON, the same format as a theme file) of one mode.
pub fn theme_json(dark: bool) -> String {
    let k = Tokens::new(dark);
    let f = fonts();
    let alpha = |c: Hsla, a: f32| hex(Hsla { a, ..c });
    let primary_hover = mix(k.ink, k.paper, 0.18);
    let primary_active = mix(k.ink, k.paper, 0.3);
    let colors = serde_json::json!({
        "background": hex(k.paper),
        "foreground": hex(k.ink),
        "border": hex(k.rule),
        "input.border": hex(k.rule),
        "caret": hex(k.ink),
        "ring": hex(k.link),
        "link": hex(k.link),
        "link.hover": hex(k.link),
        "link.active": hex(k.link),
        "selection.background": alpha(k.link, 0.28),
        "muted.background": hex(k.surface),
        "muted.foreground": hex(k.muted),
        "accent.background": hex(k.surface),
        "accent.foreground": hex(k.ink),
        "popover.background": hex(k.raised),
        "popover.foreground": hex(k.ink),
        "primary.background": hex(k.ink),
        "primary.foreground": hex(k.paper),
        "primary.hover.background": hex(primary_hover),
        "primary.active.background": hex(primary_active),
        "secondary.background": hex(k.raised),
        "secondary.foreground": hex(k.ink),
        "secondary.hover.background": hex(k.surface),
        "secondary.active.background": hex(k.rule),
        "button.background": hex(k.raised),
        "button.foreground": hex(k.ink),
        "button.hover.background": hex(k.surface),
        "button.active.background": hex(k.rule),
        "button.primary.background": hex(k.ink),
        "button.primary.foreground": hex(k.paper),
        "button.primary.hover.background": hex(primary_hover),
        "button.primary.active.background": hex(primary_active),
        "button.secondary.background": hex(k.raised),
        "button.secondary.foreground": hex(k.ink),
        "button.secondary.hover.background": hex(k.surface),
        "button.secondary.active.background": hex(k.rule),
        "button.danger.background": hex(k.raised),
        "button.danger.foreground": hex(k.bad),
        "button.danger.hover.background": hex(k.surface),
        "button.danger.active.background": hex(k.rule),
        "danger.background": hex(k.bad),
        "danger.foreground": hex(k.paper),
        "danger.hover.background": hex(k.bad),
        "danger.active.background": hex(k.bad),
        "success.background": hex(k.good),
        "success.foreground": hex(k.paper),
        "warning.background": hex(k.warn),
        "warning.foreground": hex(k.paper),
        "info.background": hex(k.link),
        "info.foreground": hex(k.paper),
        "list.background": hex(k.raised),
        "list.hover.background": hex(k.surface),
        "list.active.background": alpha(k.link, 0.12),
        "list.active.border": hex(k.link),
        "list.even.background": hex(k.raised),
        "list.head.background": hex(k.raised),
        "table.background": hex(k.paper),
        "table.head.background": hex(k.paper),
        "table.head.foreground": hex(k.ink),
        "table.hover.background": hex(k.surface),
        "table.active.background": alpha(k.link, 0.1),
        "table.active.border": hex(k.link),
        "table.even.background": hex(k.paper),
        "table.row.border": hex(k.rule),
        "sidebar.background": hex(k.surface),
        "sidebar.foreground": hex(k.ink),
        "sidebar.border": hex(k.rule),
        "sidebar.accent.background": hex(k.raised),
        "sidebar.accent.foreground": hex(k.ink),
        "sidebar.primary.background": hex(k.ink),
        "sidebar.primary.foreground": hex(k.paper),
        "title_bar.background": hex(k.paper),
        "title_bar.border": hex(k.rule),
        "status_bar.background": hex(k.surface),
        "status_bar.border": hex(k.rule),
        "tab.background": hex(k.paper),
        "tab.foreground": hex(k.muted),
        "tab.active.background": hex(k.paper),
        "tab.active.foreground": hex(k.ink),
        "tab_bar.background": hex(k.paper),
        "tab_bar.segmented.background": hex(k.surface),
        "switch.background": hex(k.rule),
        "switch.thumb.background": hex(k.raised),
        "slider.background": hex(k.ink),
        "slider.thumb.background": hex(k.raised),
        "progress.bar.background": hex(k.ink),
        "skeleton.background": hex(k.surface),
        "scrollbar.background": alpha(k.paper, 0.0),
        "scrollbar.thumb.background": alpha(k.faint, 0.55),
        "scrollbar.thumb.hover.background": hex(k.faint),
        "drag.border": hex(k.link),
        "drop_target.background": alpha(k.link, 0.15),
        "group_box.background": hex(k.surface),
        "group_box.foreground": hex(k.ink),
        "description_list.label.background": hex(k.surface),
        "description_list.label.foreground": hex(k.muted),
        "accordion.background": hex(k.paper),
        "overlay": if dark { "#00000066".to_string() } else { "#1b161626".to_string() },
        "window.border": hex(k.rule),
    });
    let set = serde_json::json!({
        "name": "Castor",
        "author": "basedpolymer",
        "themes": [{
            "is_default": false,
            "name": if dark { "Castor Dark" } else { "Castor Light" },
            "mode": if dark { "dark" } else { "light" },
            "font.family": f.sans.to_string(),
            "font.size": 14.0,
            "mono_font.family": f.mono.to_string(),
            "mono_font.size": 13.0,
            "radius": 3,
            "radius.lg": 4,
            "shadow": true,
            "colors": colors,
        }],
    });
    set.to_string()
}

fn config(dark: bool) -> Rc<gpui_kit::component::ThemeConfig> {
    let set: ThemeSet = serde_json::from_str(&theme_json(dark)).expect("castor theme json");
    Rc::new(set.themes.into_iter().next().expect("one theme"))
}

/// Install both Castor themes and switch to the mode wanted by `appearance`.
pub fn apply(appearance: Appearance, window: Option<&Window>, cx: &mut App) {
    let dark = match appearance {
        Appearance::Light => false,
        Appearance::Dark => true,
        Appearance::System => {
            let a = window.map(|w| w.appearance()).unwrap_or_else(|| cx.window_appearance());
            matches!(a, WindowAppearance::Dark | WindowAppearance::VibrantDark)
        }
    };
    let (light, darkc) = (config(false), config(true));
    Theme::update(cx, |t| {
        t.light_theme = light;
        t.dark_theme = darkc;
    });
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    Theme::update(cx, |t| {
        t.font_family = fonts().sans.clone();
        t.mono_font_family = fonts().mono.clone();
        t.font_size = gpui_kit::px(14.0);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn theme_json_parses() {
        for dark in [false, true] {
            let set: gpui_kit::component::ThemeSet = serde_json::from_str(&super::theme_json(dark)).unwrap();
            assert_eq!(set.themes.len(), 1);
        }
    }
}
