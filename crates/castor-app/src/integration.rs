//! System integration: the Windows Explorer "Castor ›" menu (per-user registry keys, written by
//! the app and by the Velopack install/update hooks, removed on uninstall or from Settings) and
//! watch folders (new files in a folder run through a chain, results in a sub-folder).
//!
//! Both live in `Settings`: `explorer_menu` and `watch_folders` (0.9.0 kept them in
//! `tool_values["app.integration"]`; `Settings::load` moves them).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest};
use castor_core::settings::Settings;
use castor_core::FileInfo;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
pub use castor_core::settings::WatchFolder;

use crate::model::model;
use crate::theme::tokens;
use crate::ui::{self, t, ta};

/// Results of a watch folder go here, inside the folder (and are never picked up again).
pub const RESULTS_DIR: &str = "Castor";

/// Is the Explorer menu wanted? `None` = not chosen yet: on for installed builds, off for
/// development builds.
pub fn context_menu_enabled(s: &Settings) -> Option<bool> {
    s.explorer_menu
}

// ---- Windows Explorer menu ---------------------------------------------------------------

#[cfg(windows)]
pub mod explorer {
    //! `HKCU\Software\Classes\*\shell\Castor` (files) and `…\Directory\shell\Castor` (folders):
    //! a cascading "Castor" menu with "Open in Castor" and one entry per favourite tool, each
    //! running `castor.exe --tool <id> "%1"`. Several selected files start one process each; the
    //! single-instance channel gathers them into one queue.
    use std::path::Path;

    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const FILES: &str = r"Software\Classes\*\shell\Castor";
    const DIRS: &str = r"Software\Classes\Directory\shell\Castor";

    fn quote(p: &Path) -> String {
        format!("\"{}\"", p.display())
    }

    fn verb(parent: &RegKey, key: &str, label: &str, icon: &str, command: &str) -> std::io::Result<()> {
        let (k, _) = parent.create_subkey(format!(r"shell\{key}"))?;
        k.set_value("MUIVerb", &label)?;
        k.set_value("Icon", &icon)?;
        let (c, _) = k.create_subkey("command")?;
        c.set_value("", &command)?;
        Ok(())
    }

    /// Write the menu for `exe` with these favourites (tool id, localized name).
    pub fn register_with(exe: &Path, favorites: &[(String, String)], open_label: &str) -> std::io::Result<()> {
        unregister();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let icon = format!("{},0", quote(exe));
        for (root, with_tools) in [(FILES, true), (DIRS, false)] {
            let (k, _) = hkcu.create_subkey(root)?;
            k.set_value("MUIVerb", &"Castor")?;
            k.set_value("Icon", &icon)?;
            // An empty SubCommands value: the entries come from the "shell" sub-key.
            k.set_value("SubCommands", &"")?;
            verb(&k, "00open", open_label, &icon, &format!("{} {}", quote(exe), "\"%1\""))?;
            if with_tools {
                for (i, (id, name)) in favorites.iter().enumerate() {
                    verb(&k, &format!("{:02}{}", i + 1, id.replace('.', "-")), name, &icon, &format!("{} --tool {id} \"%1\"", quote(exe)))?;
                }
            }
        }
        Ok(())
    }

    /// Remove both menus (no error when they are absent).
    pub fn unregister() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(FILES);
        let _ = hkcu.delete_subkey_all(DIRS);
    }

    pub fn is_registered() -> bool {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(FILES).is_ok()
    }

    /// The exe the current menu points to.
    #[allow(dead_code)]
    pub fn registered_exe() -> Option<String> {
        let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{FILES}\shell\00open\command")).ok()?;
        let v: String = k.get_value("").ok()?;
        Some(v.split('"').nth(1)?.to_string())
    }
}

// ---- macOS Finder Quick Actions ------------------------------------------------------------

#[cfg(target_os = "macos")]
pub mod quick_actions {
    //! Finder Quick Actions (right-click → Quick Actions): one Automator `.workflow` bundle per
    //! entry in `~/Library/Services` ("Open in Castor" and one per favourite tool), built from the
    //! template shipped in `packaging/macos/quick-action` (embedded at build time). Each runs the
    //! app with `--tool <id>` on the selected files: forwarded to the running app through the
    //! single-instance channel, or `open -b` when Castor is not running.
    use std::path::{Path, PathBuf};

    const INFO: &str = include_str!("../../../packaging/macos/quick-action/Contents/Info.plist");
    const WFLOW: &str = include_str!("../../../packaging/macos/quick-action/Contents/document.wflow");
    /// Marks the bundles Castor wrote, so it only ever removes its own.
    const MARKER: &str = "castor-quick-action";
    const BUNDLE_ID: &str = "eu.basedpolymer.castor";

    fn services_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Services"))
    }

    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    fn sh_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    /// The shell script of one Quick Action.
    pub fn command(exe: &Path, tool: Option<&str>) -> String {
        let args = tool.map(|t| format!(" --tool {}", sh_quote(t))).unwrap_or_default();
        format!(
            "# Castor Quick Action: hand the files to the running app, or open it with them.\n\
             if [ \"$(osascript -e 'application id \"{bundle}\" is running' 2>/dev/null)\" = \"true\" ]; then\n\
             \x20 {exe}{args} \"$@\"\n\
             else\n\
             \x20 open -b {bundle} --args{args} \"$@\"\n\
             fi\n",
            exe = sh_quote(&exe.to_string_lossy()),
            bundle = BUNDLE_ID,
        )
    }

    fn write(dir: &Path, title: &str, script: &str) -> std::io::Result<()> {
        let c = dir.join("Contents");
        std::fs::create_dir_all(&c)?;
        std::fs::write(c.join("Info.plist"), INFO.replace("__TITLE__", &xml(title)))?;
        std::fs::write(c.join("document.wflow"), WFLOW.replace("__COMMAND__", &xml(script)))?;
        std::fs::write(c.join(MARKER), "")?;
        Ok(())
    }

    fn ours() -> Vec<PathBuf> {
        let Some(dir) = services_dir() else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        rd.flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "workflow") && p.join("Contents").join(MARKER).is_file())
            .collect()
    }

    /// Ask macOS to re-read the Services (otherwise new Quick Actions show after a log out).
    fn refresh() {
        let _ = castor_core::process::command("/System/Library/CoreServices/pbs").arg("-update").output();
    }

    /// Write "Open in Castor" and one Quick Action per favourite (tool id, localized name).
    pub fn install(exe: &Path, favorites: &[(String, String)], open_label: &str) -> std::io::Result<()> {
        remove_quietly();
        let Some(dir) = services_dir() else { return Ok(()) };
        std::fs::create_dir_all(&dir)?;
        let name = |t: &str| dir.join(format!("{}.workflow", castor_core::job::sanitize_file_name(t)));
        write(&name(open_label), open_label, &command(exe, None))?;
        for (id, tool) in favorites {
            let title = format!("Castor — {tool}");
            write(&name(&title), &title, &command(exe, Some(id)))?;
        }
        refresh();
        Ok(())
    }

    fn remove_quietly() {
        for p in ours() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    /// Remove every Quick Action Castor wrote.
    pub fn remove() {
        remove_quietly();
        refresh();
    }

    pub fn is_installed() -> bool {
        !ours().is_empty()
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn template_has_placeholders() {
            assert!(super::INFO.contains("__TITLE__"));
            assert!(super::WFLOW.contains("__COMMAND__"));
            let c = super::command(std::path::Path::new("/Applications/Castor.app/Contents/MacOS/castor"), Some("pdf.compress"));
            assert!(c.contains("--tool 'pdf.compress' \"$@\""), "{c}");
            assert!(c.contains("open -b eu.basedpolymer.castor"), "{c}");
        }
    }
}

/// Favourite tools as (id, localized name), for the Explorer menu and the Quick Actions.
fn favorite_names(s: &Settings) -> Vec<(String, String)> {
    s.favorites
        .iter()
        .map(|id| (id.clone(), castor_core::tr(&format!("{id}.name"))))
        .filter(|(id, n)| *n != format!("{id}.name"))
        .collect()
}

/// Write the Explorer menu (Windows) or the Finder Quick Actions (macOS) for this exe and the
/// favourites in `s`.
pub fn register_context_menu(s: &Settings) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        explorer::register_with(&exe, &favorite_names(s), &castor_core::tr("app.integration.open_in")).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        quick_actions::install(&exe, &favorite_names(s), &castor_core::tr("app.integration.open_in")).map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = favorite_names(s);
        Ok(())
    }
}

pub fn unregister_context_menu() {
    #[cfg(windows)]
    explorer::unregister();
    #[cfg(target_os = "macos")]
    quick_actions::remove();
}

/// Velopack install / update hook (Windows): write the menu without UI, fast.
pub fn hook_register() {
    let s = Settings::load();
    castor_core::i18n::set_language(&s.language);
    if context_menu_enabled(&s) != Some(false) {
        let _ = register_context_menu(&s);
    }
}

/// At launch: keep the menu in step with the favourites and the exe location. Installed Windows
/// builds get the Explorer menu by default; the Finder Quick Actions (macOS) and development
/// builds only when switched on in Settings.
pub fn sync_context_menu(cx: &mut App) {
    let s = crate::model::settings(cx).clone();
    let installed = !matches!(model(cx).read(cx).update, crate::model::UpdateState::Disabled);
    let want = context_menu_enabled(&s).unwrap_or(installed && cfg!(windows));
    cx.background_executor()
        .spawn(async move {
            if want {
                if let Err(e) = register_context_menu(&s) {
                    tracing::warn!("Explorer menu: {e}");
                }
            } else if context_menu_enabled(&s) == Some(false) {
                unregister_context_menu();
            }
        })
        .detach();
}

// ---- watch folders ------------------------------------------------------------------------

pub fn watch_folders(s: &Settings) -> Vec<WatchFolder> {
    s.watch_folders.clone()
}

pub fn store_watch_folders(s: &mut Settings, v: &[WatchFolder]) {
    s.watch_folders = v.to_vec();
}

/// Every chain by name: templates, then saved chains.
pub fn chains() -> Vec<castor_core::chain::Chain> {
    let mut v = castor_core::chain::templates();
    v.extend(castor_core::chain::load_all().into_iter().map(|(_, c)| c));
    v
}

/// Should a path seen in a watched folder be processed?
pub fn candidate(folder: &Path, p: &Path) -> bool {
    if !p.is_file() || p.parent() != Some(folder) {
        return false;
    }
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    !(name.starts_with('.') || name.starts_with('~') || lower.ends_with(".tmp") || lower.ends_with(".part") || lower.ends_with(".crdownload") || lower.ends_with(".castor-tmp") || lower == "desktop.ini" || lower == "thumbs.db")
}

#[derive(Default)]
struct Seen {
    size: u64,
    changed: Option<Instant>,
}

/// The watcher: notify events → candidates → stable size for 2 s → one chain run at a time.
pub struct Watcher {
    _inner: Option<notify::RecommendedWatcher>,
    rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    pending: HashMap<PathBuf, Seen>,
    queue: VecDeque<(PathBuf, usize)>,
    done: HashSet<(PathBuf, SystemTime)>,
    running: Option<(runner::RunHandle, PathBuf)>,
    folders: Vec<WatchFolder>,
    /// Shown in Settings and the tray tooltip.
    pub status: Option<String>,
    pub processed: usize,
}

impl Global for Watcher {}

impl Watcher {
    fn new() -> Watcher {
        Watcher { _inner: None, rx: None, pending: HashMap::new(), queue: VecDeque::new(), done: HashSet::new(), running: None, folders: Vec::new(), status: None, processed: 0 }
    }
}

/// (Re)start watching after a settings change.
pub fn restart_watching(cx: &mut App) {
    use notify::Watcher as _;
    let folders: Vec<WatchFolder> = watch_folders(crate::model::settings(cx)).into_iter().filter(|w| !w.paused && w.folder.is_dir()).collect();
    if cx.try_global::<Watcher>().is_none() {
        cx.set_global(Watcher::new());
        start_poll(cx);
    }
    let w = cx.global_mut::<Watcher>();
    w._inner = None;
    w.rx = None;
    w.folders = folders.clone();
    w.pending.retain(|p, _| folders.iter().any(|f| p.parent() == Some(f.folder.as_path())));
    if folders.is_empty() {
        w.status = None;
        crate::tray::set_activity(None, cx);
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    match notify::recommended_watcher(move |e| {
        let _ = tx.send(e);
    }) {
        Ok(mut watcher) => {
            for f in &folders {
                if let Err(e) = watcher.watch(&f.folder, notify::RecursiveMode::NonRecursive) {
                    tracing::warn!("watch {}: {e}", f.folder.display());
                }
            }
            let w = cx.global_mut::<Watcher>();
            w._inner = Some(watcher);
            w.rx = Some(rx);
            w.status = Some(ta("app.watch.watching", &[("count", &folders.len().to_string())]).to_string());
        }
        Err(e) => cx.global_mut::<Watcher>().status = Some(format!("{e}")),
    }
    let s = cx.global::<Watcher>().status.clone();
    crate::tray::set_activity(s, cx);
}

fn start_poll(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            cx.update(tick);
        }
    })
    .detach();
}

fn tick(cx: &mut App) {
    let reg = crate::model::registry(cx);
    let workspace = crate::model::active_workspace_name(cx);
    let mut finished = None;
    let mut status_changed = false;
    {
        let w = cx.global_mut::<Watcher>();
        // 1. File events → pending candidates.
        if let Some(rx) = &w.rx {
            while let Ok(Ok(ev)) = rx.try_recv() {
                if matches!(ev.kind, notify::EventKind::Create(_) | notify::EventKind::Modify(_)) {
                    for p in ev.paths {
                        if w.folders.iter().any(|f| candidate(&f.folder, &p)) {
                            w.pending.entry(p).or_default().changed = Some(Instant::now());
                        }
                    }
                }
            }
        }
        // 2. Stable for 2 s (same size, no event) → queue.
        let mut ready = Vec::new();
        for (p, seen) in w.pending.iter_mut() {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            if size != seen.size {
                seen.size = size;
                seen.changed = Some(Instant::now());
            } else if size > 0 && seen.changed.is_some_and(|c| c.elapsed() >= Duration::from_secs(2)) {
                ready.push(p.clone());
            }
        }
        for p in ready {
            w.pending.remove(&p);
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            if !w.done.insert((p.clone(), mtime)) {
                continue;
            }
            if let Some(i) = w.folders.iter().position(|f| p.parent() == Some(f.folder.as_path())) {
                w.queue.push_back((p, i));
            }
        }
        // 3. Progress of the current run.
        if let Some((h, _)) = &w.running {
            while let Ok(ev) = h.events.try_recv() {
                if let RunEvent::Finished(r) = ev {
                    finished = Some(*r);
                }
            }
        }
        if finished.is_some() {
            w.running = None;
            w.processed += 1;
            status_changed = true;
        }
        // 4. Next file.
        if w.running.is_none() {
            while let Some((p, i)) = w.queue.pop_front() {
                let Some(f) = w.folders.get(i).cloned() else { continue };
                let Some(chain) = chains().into_iter().find(|c| c.name == f.chain) else { continue };
                let Ok(steps) = chain.resolve(&reg) else { continue };
                let input = FileInfo::from_path(&p);
                if !steps.first().is_some_and(|s| s.tool.accepts(&input)) {
                    continue;
                }
                let req = RunRequest {
                    steps,
                    inputs: vec![input],
                    output: OutputPolicy::Folder(f.folder.join(RESULTS_DIR)),
                    chain_name: Some(chain.name.clone()),
                    max_parallel: 1,
                    record: true,
                };
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                w.status = Some(ta("app.watch.processing", &[("name", &name)]).to_string());
                w.running = Some((castor_core::runner::start_in(req, workspace.clone()), p));
                status_changed = true;
                break;
            }
            if w.running.is_none() && status_changed {
                w.status = Some(ta("app.watch.idle", &[("count", &w.folders.len().to_string()), ("done", &w.processed.to_string())]).to_string());
            }
        }
    }
    if let Some(r) = finished {
        model(cx).update(cx, |m, cx| m.push_run(r, cx));
    }
    if status_changed {
        let s = cx.global::<Watcher>().status.clone();
        crate::tray::set_activity(s, cx);
        cx.refresh_windows();
    }
}

// ---- Settings section ---------------------------------------------------------------------

pub struct IntegrationSection {
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl IntegrationSection {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        IntegrationSection { error: None, _subs: vec![cx.observe(&model(cx), |_, _, cx| cx.notify())] }
    }

    fn update_watch(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Vec<WatchFolder>)) {
        let mut list = watch_folders(crate::model::settings(cx));
        f(&mut list);
        model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| store_watch_folders(s, &list)));
        restart_watching(cx);
    }

    fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update(cx, |s, cx| {
                        let chain = chains().into_iter().next().map(|c| c.name).unwrap_or_default();
                        s.update_watch(cx, move |l| {
                            if !l.iter().any(|w| w.folder == p) {
                                l.push(WatchFolder { folder: p, chain, paused: false });
                            }
                        });
                    });
                }
            }
        })
        .detach();
    }
}

impl Render for IntegrationSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let s = crate::model::settings(cx).clone();
        let list = watch_folders(&s);
        let chain_names: Vec<String> = chains().into_iter().map(|c| c.name).collect();
        let status = cx.try_global::<Watcher>().and_then(|w| w.status.clone());
        #[cfg(windows)]
        let menu_on = explorer::is_registered();
        #[cfg(target_os = "macos")]
        let menu_on = quick_actions::is_installed();
        #[cfg(not(any(windows, target_os = "macos")))]
        let menu_on = false;
        let mac = cfg!(target_os = "macos");
        let mut col = div().flex().flex_col().gap(px(10.0));
        if cfg!(windows) || mac {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(24.0))
                    .py(px(6.0))
                    .child(
                        div()
                            .w(px(280.0))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(ui::sans().text_color(k.ink).child(if mac { t("app.integration.quick_actions") } else { t("app.integration.explorer") }))
                            .child(ui::roman().text_size(px(13.0)).line_height(px(16.5)).text_color(k.muted).child(if mac {
                                t("app.integration.quick_actions_help")
                            } else {
                                t("app.integration.explorer_help")
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(Switch::new("explorer-menu").checked(menu_on).on_change(cx.listener(|this, v: &bool, _, cx| {
                                let v = *v;
                                let s2 = crate::model::settings(cx).clone();
                                this.error = if v { register_context_menu(&s2).err() } else {
                                    unregister_context_menu();
                                    None
                                };
                                model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| s.explorer_menu = Some(v)));
                            })))
                            .when_some(self.error.clone(), |d, e| d.child(ui::sans().text_size(px(13.0)).text_color(k.bad).child(e))),
                    ),
            );
        }
        if !cfg!(windows) {
            col = col.child(ui::roman().text_size(px(14.0)).text_color(k.muted).max_w(px(700.0)).child(t("app.integration.finder_help")));
        }
        col.child(ui::hrule(k.rule, 1.0))
            .child(ui::sans().text_color(k.ink).child(t("app.watch.title")))
            .child(ui::roman().max_w(px(700.0)).text_size(px(13.5)).text_color(k.muted).child(ta("app.watch.help", &[("dir", RESULTS_DIR)])))
            .when(!list.is_empty(), |d| {
                d.child(
                    div().max_w(px(860.0)).flex().flex_col().child(ui::rule_strong(&k)).children(list.iter().enumerate().map(|(i, w)| {
                        let names = chain_names.clone();
                        let cur = names.iter().position(|n| *n == w.chain).unwrap_or(0);
                        let next = names.get((cur + 1) % names.len().max(1)).cloned().unwrap_or_default();
                        let missing = !names.contains(&w.chain);
                        div()
                            .id(("watch", i))
                            .min_h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .child(ui::icon(if w.paused { "pause" } else { "eye" }, if w.paused { k.faint } else { k.good }).size(px(14.0)))
                            .child(ui::mono().flex_1().min_w(px(0.0)).text_ellipsis_start().whitespace_nowrap().overflow_hidden().child(w.folder.display().to_string()))
                            .child(ui::sans().text_color(k.faint).child("→"))
                            .child(
                                Button::new(("watch-chain", i))
                                    .xsmall()
                                    .tooltip(t("app.watch.next_chain"))
                                    .label(if missing { t("app.watch.no_chain").to_string() } else { w.chain.clone() })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let next = next.clone();
                                        this.update_watch(cx, move |l| {
                                            if let Some(x) = l.get_mut(i) {
                                                x.chain = next;
                                            }
                                        });
                                    })),
                            )
                            .child(Button::new(("watch-pause", i)).xsmall().ghost().label(if w.paused { t("app.watch.resume") } else { t("app.watch.pause") }).on_click(cx.listener(move |this, _, _, cx| {
                                this.update_watch(cx, move |l| {
                                    if let Some(x) = l.get_mut(i) {
                                        x.paused = !x.paused;
                                    }
                                })
                            })))
                            .child(Button::new(("watch-rm", i)).xsmall().ghost().danger().label(t("app.common.delete")).on_click(cx.listener(move |this, _, _, cx| {
                                this.update_watch(cx, move |l| {
                                    if i < l.len() {
                                        l.remove(i);
                                    }
                                })
                            })))
                    })).child(ui::rule_strong(&k)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(Button::new("watch-add").small().icon(gpui_kit::component::IconName::Plus).label(t("app.watch.add")).on_click(cx.listener(|s, _, w, cx| s.add_folder(w, cx))))
                    .when_some(status, |d, st| d.child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(st))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{RESULTS_DIR, Settings, WatchFolder, candidate, context_menu_enabled, store_watch_folders, watch_folders};

    #[test]
    fn watch_settings_round_trip() {
        let mut s = Settings::default();
        let v = vec![WatchFolder { folder: "C:/in".into(), chain: "Web images".into(), paused: false }];
        store_watch_folders(&mut s, &v);
        assert_eq!(watch_folders(&s), v);
        assert_eq!(context_menu_enabled(&s), None);
    }

    #[test]
    fn candidates_skip_partial_and_results() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("photo.jpg");
        std::fs::write(&f, b"x").unwrap();
        let part = dir.path().join("big.mp4.part");
        std::fs::write(&part, b"x").unwrap();
        std::fs::create_dir_all(dir.path().join(RESULTS_DIR)).unwrap();
        let out = dir.path().join(RESULTS_DIR).join("photo.jpg");
        std::fs::write(&out, b"x").unwrap();
        assert!(candidate(dir.path(), &f));
        assert!(!candidate(dir.path(), &part));
        assert!(!candidate(dir.path(), &out), "results are never picked up again");
    }
}
