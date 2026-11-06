//! Menu bar (macOS) / system tray (Windows) icon, its menu, and the global shortcut that
//! toggles the quick panel.

use std::str::FromStr;
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton as TrayButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::model::{Nav, model, navigate};

#[derive(Default)]
struct Tray {
    icon: Option<TrayIcon>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    open: Option<MenuId>,
    check: Option<MenuId>,
    quit: Option<MenuId>,
    recent: Vec<(MenuId, String)>,
    polling: bool,
}
impl Global for Tray {}

/// True when the tray/menu bar icon exists.
pub fn is_active(cx: &App) -> bool {
    cx.try_global::<Tray>().is_some_and(|t| t.icon.is_some())
}

/// Is the Windows taskbar dark? (`SystemUsesLightTheme` = 0.)
#[cfg(windows)]
fn taskbar_is_dark() -> bool {
    let out = castor_core::process::command("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "SystemUsesLightTheme"])
        .output();
    match out {
        Ok(o) => !String::from_utf8_lossy(&o.stdout).contains("0x1"),
        Err(_) => true,
    }
}

fn icon() -> Option<tray_icon::Icon> {
    #[cfg(target_os = "macos")]
    let png = crate::assets::TRAY_BLACK_36;
    #[cfg(windows)]
    let png = if taskbar_is_dark() { crate::assets::TRAY_WHITE_36 } else { crate::assets::TRAY_BLACK_36 };
    #[cfg(not(any(windows, target_os = "macos")))]
    let png = crate::assets::TRAY_BLACK_36;
    let (rgba, w, h) = crate::assets::png_rgba(png)?;
    tray_icon::Icon::from_rgba(rgba, w, h).ok()
}

fn build_menu(cx: &mut App) -> (Menu, Option<MenuId>, Option<MenuId>, Option<MenuId>, Vec<(MenuId, String)>) {
    let menu = Menu::new();
    let open = MenuItem::new(castor_core::tr("app.tray.open"), true, None);
    let _ = menu.append(&open);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let recent_ids = model(cx).read(cx).recent_tools(5);
    let mut recent = Vec::new();
    for id in recent_ids {
        let item = MenuItem::new(castor_core::tr(&format!("{id}.name")), true, None);
        let _ = menu.append(&item);
        recent.push((item.id().clone(), id));
    }
    if !recent.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let check = MenuItem::new(castor_core::tr("app.palette.action.check_updates"), true, None);
    let quit = MenuItem::new(castor_core::tr("app.tray.quit"), true, None);
    let _ = menu.append(&check);
    let _ = menu.append(&quit);
    (menu, Some(open.id().clone()), Some(check.id().clone()), Some(quit.id().clone()), recent)
}

/// Create the icon (if enabled), register the shortcut, start polling events.
pub fn init(cx: &mut App) {
    cx.set_global(Tray::default());
    sync(cx);
    register_hotkey(cx);
    start_polling(cx);
}

/// Show or hide the icon to match the settings.
pub fn sync(cx: &mut App) {
    let want = crate::model::settings(cx).tray_icon;
    let has = cx.global::<Tray>().icon.is_some();
    if want && !has {
        let (menu, open, check, quit, recent) = build_menu(cx);
        let Some(ic) = icon() else { return };
        let built = TrayIconBuilder::new()
            .with_tooltip("Castor")
            .with_icon(ic)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build();
        match built {
            Ok(t) => {
                let tr = cx.global_mut::<Tray>();
                tr.icon = Some(t);
                tr.open = open;
                tr.check = check;
                tr.quit = quit;
                tr.recent = recent;
            }
            Err(e) => tracing::warn!("tray icon unavailable: {e}"),
        }
    } else if !want && has {
        cx.global_mut::<Tray>().icon = None;
    }
}

/// Rebuild the menu (language change, new recent tools).
pub fn rebuild_menu(cx: &mut App) {
    if cx.try_global::<Tray>().is_none_or(|t| t.icon.is_none()) {
        return;
    }
    let (menu, open, check, quit, recent) = build_menu(cx);
    let tr = cx.global_mut::<Tray>();
    if let Some(i) = &tr.icon {
        i.set_menu(Some(Box::new(menu)));
    }
    tr.open = open;
    tr.check = check;
    tr.quit = quit;
    tr.recent = recent;
}

/// "ctrl-alt-space" (GPUI style) → global-hotkey's "ctrl+alt+Space".
pub fn parse_hotkey(s: &str) -> Option<HotKey> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<String> = s
        .split('-')
        .map(|p| match p {
            "cmd" | "platform" => "super".to_string(),
            "ctrl" => "control".to_string(),
            "space" => "Space".to_string(),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => format!("Key{}", k.to_ascii_uppercase()),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
            k => {
                let mut c = k.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect();
    HotKey::from_str(&parts.join("+")).ok()
}

/// (Re)register the global shortcut from the settings.
pub fn register_hotkey(cx: &mut App) {
    let wanted = parse_hotkey(&crate::model::settings(cx).global_hotkey);
    if !cx.has_global::<Tray>() {
        return;
    }
    let tr = cx.global_mut::<Tray>();
    if tr.hotkeys.is_none() {
        match GlobalHotKeyManager::new() {
            Ok(m) => tr.hotkeys = Some(m),
            Err(e) => {
                tracing::warn!("global shortcuts unavailable: {e}");
                return;
            }
        }
    }
    let mgr = tr.hotkeys.as_ref().unwrap();
    if let Some(old) = tr.hotkey.take() {
        let _ = mgr.unregister(old);
    }
    if let Some(h) = wanted {
        match mgr.register(h) {
            Ok(()) => tr.hotkey = Some(h),
            Err(e) => tracing::warn!("could not register the shortcut: {e}"),
        }
    }
}

fn start_polling(cx: &mut App) {
    if cx.global::<Tray>().polling {
        return;
    }
    cx.global_mut::<Tray>().polling = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(50)).await;
            let mut tray_events = Vec::new();
            while let Ok(e) = TrayIconEvent::receiver().try_recv() {
                tray_events.push(e);
            }
            let mut menu_events = Vec::new();
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                menu_events.push(e);
            }
            let mut hotkey = false;
            while let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
                if e.state() == HotKeyState::Pressed {
                    hotkey = true;
                }
            }
            if tray_events.is_empty() && menu_events.is_empty() && !hotkey {
                continue;
            }
            cx.update(|cx| {
                for e in tray_events {
                    if let TrayIconEvent::Click { button: TrayButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
                        let anchor = Some((rect.position.x, rect.position.y, rect.size.width as f64, rect.size.height as f64));
                        crate::panel::toggle(anchor, cx);
                    }
                }
                for e in menu_events {
                    on_menu(e.id(), cx);
                }
                if hotkey {
                    crate::panel::toggle(None, cx);
                }
            });
        }
    })
    .detach();
}

fn on_menu(id: &MenuId, cx: &mut App) {
    let tr = cx.global::<Tray>();
    if Some(id) == tr.open.as_ref() {
        crate::app::show_main_window(cx);
    } else if Some(id) == tr.check.as_ref() {
        crate::updates::check_now(cx);
        navigate(Nav::Page(crate::model::PageId::Settings), cx);
    } else if Some(id) == tr.quit.as_ref() {
        crate::app::quit(cx);
    } else if let Some((_, tool)) = tr.recent.iter().find(|(m, _)| m == id).cloned() {
        navigate(Nav::Tool { id: tool, files: vec![], values: None }, cx);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hotkeys_parse() {
        assert!(super::parse_hotkey("ctrl-alt-space").is_some());
        assert!(super::parse_hotkey("cmd-alt-space").is_some());
        assert!(super::parse_hotkey("ctrl-shift-k").is_some());
        assert!(super::parse_hotkey("").is_none());
    }
}
