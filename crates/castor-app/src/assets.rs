//! Embedded assets: the Latin Modern / TeX Gyre fonts, Castor's own SVG marks, and the full
//! Lucide catalogue from GPUI Kit.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// Fonts registered with the text system at startup (GUST Font License, see assets/fonts).
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/lmsans10-regular.otf"),
    include_bytes!("../../../assets/fonts/lmsans10-bold.otf"),
    include_bytes!("../../../assets/fonts/lmsans10-oblique.otf"),
    include_bytes!("../../../assets/fonts/lmroman10-regular.otf"),
    include_bytes!("../../../assets/fonts/lmroman10-bold.otf"),
    include_bytes!("../../../assets/fonts/lmroman10-italic.otf"),
    include_bytes!("../../../assets/fonts/lmromanslant10-regular.otf"),
    include_bytes!("../../../assets/fonts/lmromancaps10-regular.otf"),
    include_bytes!("../../../assets/fonts/lmmono10-regular.otf"),
    include_bytes!("../../../assets/fonts/texgyrechorus-mediumitalic.otf"),
];

/// Diamond outline (1.5 px stroke at 16 px): the brand mark, bullets, the end of the page rule.
pub const DIAMOND: &str = "castor/diamond.svg";
/// Filled diamond: family swatches, the selected anchor, the current step.
pub const DIAMOND_FILLED: &str = "castor/diamond-filled.svg";

const DIAMOND_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M8 1.6 14.4 8 8 14.4 1.6 8Z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="miter"/></svg>"#;
const DIAMOND_FILLED_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path d="M8 1.6 14.4 8 8 14.4 1.6 8Z" fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linejoin="miter"/></svg>"#;

/// The Castor mark (bitten diamond + Chorus C), from assets/brand (coordinator-owned).
const MARK_SVG: &str = include_str!("../../../assets/brand/castor-mark.svg");
pub const MARK_LIGHT: &str = "castor/mark-light.svg";
pub const MARK_DARK: &str = "castor/mark-dark.svg";

/// Tray icons (36 px PNG; the OS scales them down for 100 % displays).
pub const TRAY_BLACK_36: &[u8] = include_bytes!("../../../assets/brand/png/castor-tray-36.png");
pub const TRAY_WHITE_36: &[u8] = include_bytes!("../../../assets/brand/png/castor-tray-white-36.png");

fn mark(dark: bool) -> &'static [u8] {
    static L: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    static D: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    if dark {
        // Swap ink and paper so the mark stays visible on dark paper.
        D.get_or_init(|| MARK_SVG.replace("#1b1616", "#__INK__").replace("#fbfaf7", "#161514").replace("#__INK__", "#e8e6e1").into_bytes())
    } else {
        L.get_or_init(|| MARK_SVG.as_bytes().to_vec())
    }
}

/// Decode a PNG into RGBA pixels (for the tray icon).
pub fn png_rgba(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

/// Asset source: our own marks first, then every Lucide icon bundled with GPUI Kit.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            DIAMOND => Ok(Some(Cow::Borrowed(DIAMOND_SVG))),
            DIAMOND_FILLED => Ok(Some(Cow::Borrowed(DIAMOND_FILLED_SVG))),
            MARK_LIGHT => Ok(Some(Cow::Borrowed(mark(false)))),
            MARK_DARK => Ok(Some(Cow::Borrowed(mark(true)))),
            _ => gpui_kit::assets::AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::AllAssets.list(path)
    }
}

/// Path of a Lucide icon by its kebab-case name ("folder-open").
pub fn lucide(name: &str) -> SharedString {
    format!("icons/{name}.svg").into()
}
