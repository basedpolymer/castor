//! Bootstrap: GPUI Kit, fonts, theme, model, windows, tray, single-instance socket, updates.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use castor_core::settings::{Appearance, Settings};
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::cli::Args;
use crate::model::{Model, ModelHandle, Nav, PageId, model};
use crate::shell::Shell;

actions!(castor, [OpenPalette, Quit, CloseWindow, NewWindowNoop]);

/// The main window, if open.
#[derive(Default)]
pub struct MainWindow {
    pub handle: Option<AnyWindowHandle>,
    pub shell: Option<WeakEntity<Shell>>,
    /// Screenshot/launch options for the first window.
    pub size: Option<(u32, u32)>,
}
impl Global for MainWindow {}

/// Build the tool registry (plus the demo tools in development builds with `demo-tools`).
pub fn build_registry() -> castor_core::Registry {
    #[cfg(feature = "tools")]
    let reg = castor_tools::registry();
    #[cfg(not(feature = "tools"))]
    let reg = castor_core::Registry::new(Vec::new());
    #[cfg(feature = "demo-tools")]
    {
        let mut all = reg.all().to_vec();
        all.extend(crate::demo::tools());
        return castor_core::Registry::new(all);
    }
    #[allow(unreachable_code)]
    reg
}

pub fn run(args: Args) {
    let mut settings = Settings::load();
    if let Some(l) = &args.lang {
        settings.language = l.clone();
    }
    if let Some(t) = &args.theme {
        settings.appearance = match t.as_str() {
            "dark" => Appearance::Dark,
            "light" => Appearance::Light,
            _ => Appearance::System,
        };
    }
    set_language(&settings.language);

    let screenshot = args.page.is_some();
    let ipc_rx = if screenshot { None } else { crate::ipc::listen() };
    let (open_tx, open_rx) = crossbeam_channel::unbounded::<Vec<PathBuf>>();

    let application = gpui_kit::application().with_assets(crate::assets::Assets).with_quit_mode(QuitMode::Explicit);
    // macOS: files dropped on the Dock icon / "Open with".
    application.on_open_urls(move |urls| {
        let paths: Vec<PathBuf> = urls
            .iter()
            .filter_map(|u| u.strip_prefix("file://"))
            .map(|p| PathBuf::from(percent_decode(p)))
            .collect();
        if !paths.is_empty() {
            let _ = open_tx.send(paths);
        }
    });
    application.on_reopen(|cx| show_main_window(cx));

    application.run(move |cx| {
        gpui_kit::init(cx);
        crate::theme::register_fonts(cx);
        crate::theme::apply(settings.appearance, None, cx);
        bind_keys(cx);

        let registry = Arc::new(build_registry());
        let launch_hidden = args.background && settings.tray_icon;
        let m = cx.new(|cx| {
            let mut m = Model::new(registry, settings);
            m.load_history(cx);
            m
        });
        cx.set_global(ModelHandle(m.clone()));
        cx.set_global(MainWindow { size: args.size, ..Default::default() });
        cx.set_global(crate::home::HomeStart {
            family: args.family.as_deref().and_then(|f| castor_core::Family::ALL.iter().copied().find(|x| x.key() == f)),
            scroll: args.scroll,
        });
        cx.set_global(crate::thumbs::Thumbs::default());

        cx.on_action(|_: &Quit, cx| quit(cx));

        if args.page.as_deref() == Some("panel") {
            cx.set_global(crate::panel::KeepOpen);
            crate::panel::open_panel(None, cx);
        } else if !launch_hidden {
            show_main_window(cx);
            apply_args(&args, cx);
        }

        if let Some(out) = args.shot.clone() {
            schedule_shot(out, args.shot_delay.unwrap_or(2.5), args.page.as_deref() == Some("panel"), cx);
        }

        if !screenshot {
            crate::tray::init(cx);
            crate::updates::check_at_launch(cx);
        }

        // Poll the single-instance socket and macOS open-file events.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(80)).await;
                let mut forwarded = Vec::new();
                if let Some(rx) = &ipc_rx {
                    while let Ok(a) = rx.try_recv() {
                        forwarded.push(a);
                    }
                }
                while let Ok(paths) = open_rx.try_recv() {
                    forwarded.push(Args { files: paths, ..Default::default() });
                }
                if forwarded.is_empty() {
                    continue;
                }
                cx.update(|cx| {
                    for a in forwarded {
                        show_main_window(cx);
                        apply_args(&a, cx);
                    }
                });
            }
        })
        .detach();
    });
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Set the UI language for Castor and for GPUI Kit's own strings.
pub fn set_language(code: &str) {
    castor_core::i18n::set_language(code);
    gpui_kit::component::set_locale(&castor_core::i18n::current_language());
}

fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    let keys = ["cmd-k", "cmd-q", "cmd-w"];
    #[cfg(not(target_os = "macos"))]
    let keys = ["ctrl-k", "ctrl-q", "ctrl-w"];
    cx.bind_keys([
        KeyBinding::new(keys[0], OpenPalette, None),
        KeyBinding::new(keys[1], Quit, None),
        KeyBinding::new(keys[2], CloseWindow, None),
    ]);
}

/// Apply launch arguments (or arguments forwarded by a second launch).
pub fn apply_args(a: &Args, cx: &mut App) {
    let nav = if let Some(p) = &a.page {
        if let Some(id) = p.strip_prefix("tool:") {
            Some(Nav::Tool { id: id.to_string(), files: a.files.clone(), values: None })
        } else {
            match p.as_str() {
                "runs" => Some(Nav::Page(PageId::Runs)),
                "chains" => Some(Nav::Page(PageId::Chains)),
                "settings" => Some(Nav::Page(PageId::Settings)),
                "studios" => Some(Nav::Page(PageId::Studios)),
                "palette" => Some(Nav::Palette(a.query.clone())),
                p if p.starts_with("chain:") => Some(Nav::Chain(p[6..].to_string())),
                _ if !a.files.is_empty() => Some(Nav::Files(a.files.clone())),
                _ => Some(Nav::Page(PageId::Tools)),
            }
        }
    } else if let Some(id) = &a.tool {
        Some(Nav::Tool { id: id.clone(), files: a.files.clone(), values: None })
    } else if !a.files.is_empty() {
        Some(Nav::Files(a.files.clone()))
    } else {
        None
    };
    if let Some(n) = nav {
        crate::model::navigate(n, cx);
    }
    if a.run {
        let mw = cx.global::<MainWindow>();
        if let (Some(h), Some(shell)) = (mw.handle, mw.shell.as_ref().and_then(|s| s.upgrade())) {
            // Start the run on the next frame, once the tool screen exists.
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, cx| shell.update(cx, |s, cx| s.run_current_tool(window, cx)));
            });
        }
    }
}

/// Open the main window if needed and bring it to front.
pub fn show_main_window(cx: &mut App) {
    let mw = cx.global::<MainWindow>();
    if let (Some(h), Some(s)) = (mw.handle, mw.shell.as_ref().and_then(|w| w.upgrade())) {
        let _ = s;
        let _ = h.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let (w, h) = mw.size.unwrap_or((1180, 760));
    let bounds = Bounds::centered(None, size(px(w as f32), px(h as f32)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(880.0), px(560.0))),
        app_id: Some("eu.basedpolymer.castor".into()),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx))) {
        Ok((handle, shell)) => {
            let weak = shell.downgrade();
            let _ = handle.update(cx, |_, window, cx| {
                window.set_window_title("Castor");
                window.on_window_should_close(cx, |_, cx| {
                    let s = crate::model::settings(cx);
                    if !(s.tray_icon && s.keep_running_in_tray && crate::tray::is_active(cx)) {
                        cx.defer(|cx| quit(cx));
                    } else {
                        // Only the menu bar icon remains: hide the Dock icon.
                        #[cfg(target_os = "macos")]
                        cx.defer(|cx| cx.set_activation_policy(ActivationPolicy::Accessory));
                    }
                    true
                });
            });
            let mw = cx.global_mut::<MainWindow>();
            mw.handle = Some(handle);
            mw.shell = Some(weak);
            #[cfg(target_os = "macos")]
            {
                cx.set_activation_policy(ActivationPolicy::Regular);
                cx.activate(true);
            }
        }
        Err(e) => tracing::error!("could not open the main window: {e:#}"),
    }
}

/// Quit the app (after the in-flight settings save).
pub fn quit(cx: &mut App) {
    let s = model(cx).read(cx).settings.clone();
    let _ = s.save();
    crate::updates::apply_on_quit(cx);
    cx.quit();
}

/// `--shot`: render the main window (or the quick panel) offscreen to a PNG, then quit.
fn schedule_shot(out: PathBuf, delay: f64, panel: bool, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
        cx.update(|cx| {
            let handle = if panel { crate::panel::handle(cx) } else { cx.global::<MainWindow>().handle };
            let Some(handle) = handle else {
                tracing::error!("--shot: no window");
                cx.quit();
                return;
            };
            #[cfg(feature = "shots")]
            {
                let res = handle.update(cx, |_, window, _| window.render_to_image());
                match res {
                    Ok(Ok(img)) => {
                        if let Some(d) = out.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        match img.save(&out) {
                            Ok(()) => println!("{} ({} x {})", out.display(), img.width(), img.height()),
                            Err(e) => tracing::error!("--shot: {e}"),
                        }
                    }
                    Ok(Err(e)) => tracing::error!("--shot: {e:#}"),
                    Err(e) => tracing::error!("--shot: {e:#}"),
                }
            }
            #[cfg(not(feature = "shots"))]
            {
                let _ = (&out, handle);
                tracing::error!("--shot needs the `shots` cargo feature");
            }
            cx.quit();
        });
    })
    .detach();
}
