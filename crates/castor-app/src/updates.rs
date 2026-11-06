//! In-app updates with Velopack: check (at launch, at most daily, if enabled), download with
//! progress, "Restart to update", or apply on quit when the user opted into auto-install.

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager};

use crate::model::{UpdateState, model};

const REPO: &str = "https://github.com/basedpolymer/castor";

#[derive(Default)]
struct Pending(Option<UpdateInfo>);
impl Global for Pending {}

/// The update feed: GitHub releases, or `CASTOR_UPDATE_FEED=<local folder or URL>` (dev/test
/// only: the packaging scripts test updates against a local feed).
pub fn feed() -> String {
    std::env::var("CASTOR_UPDATE_FEED").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| REPO.to_string())
}

fn manager() -> Result<UpdateManager, String> {
    // `AutoSource::new(REPO)` is `GithubSource::new(REPO, None, false)`; a non-URL string
    // becomes a `FileSource`.
    let source = velopack::sources::AutoSource::new(&feed());
    UpdateManager::new(source, None, None).map_err(|e| e.to_string())
}

fn set_state(s: UpdateState, cx: &mut App) {
    model(cx).update(cx, |m, cx| {
        m.update = s;
        cx.notify();
    });
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Is a launch-time check due? At most one a day (`Settings::last_update_check`).
pub fn check_due(last: Option<i64>, now: i64) -> bool {
    match last {
        Some(t) => now.saturating_sub(t) >= 24 * 3600 || t > now,
        None => true,
    }
}

/// At launch: decide whether updates are possible here, then check if it is time.
pub fn check_at_launch(cx: &mut App) {
    cx.set_global(Pending::default());
    if manager().is_err() {
        set_state(UpdateState::Disabled, cx);
        return;
    }
    if !crate::model::settings(cx).check_updates {
        return;
    }
    // Wave-1 builds kept the stamp in a file: take it over once.
    let legacy = castor_core::paths::data_dir().join("last-update-check");
    if let Some(t) = std::fs::read_to_string(&legacy).ok().and_then(|s| s.trim().parse::<i64>().ok()) {
        let _ = std::fs::remove_file(&legacy);
        model(cx).update(cx, |m, cx| m.update_settings(cx, |s| s.last_update_check = s.last_update_check.max(Some(t))));
    }
    if !check_due(crate::model::settings(cx).last_update_check, now_secs()) {
        return;
    }
    cx.spawn(async move |cx| {
        // Let the window settle first: the check never slows down startup.
        cx.background_executor().timer(Duration::from_secs(4)).await;
        let _ = cx.update(check_now);
    })
    .detach();
}

/// Check now (Settings button, palette action, tray menu).
pub fn check_now(cx: &mut App) {
    if matches!(model(cx).read(cx).update, UpdateState::Disabled) {
        return;
    }
    set_state(UpdateState::Checking, cx);
    let now = now_secs();
    model(cx).update(cx, |m, cx| m.update_settings(cx, |s| s.last_update_check = Some(now)));
    let task = cx.background_executor().spawn(async move {
        manager().and_then(|m| m.check_for_updates().map_err(|e| e.to_string()))
    });
    cx.spawn(async move |cx| {
        let res = task.await;
        let _ = cx.update(|cx| match res {
            Ok(UpdateCheck::UpdateAvailable(info)) => {
                let version = info.TargetFullRelease.Version.clone();
                let notes = info.TargetFullRelease.NotesMarkdown.clone();
                cx.global_mut::<Pending>().0 = Some(*info);
                set_state(UpdateState::Available { version, notes }, cx);
                if crate::model::settings(cx).auto_install_updates {
                    download(cx);
                }
            }
            Ok(_) => set_state(UpdateState::UpToDate, cx),
            Err(e) => {
                tracing::warn!("update check failed: {e}");
                set_state(UpdateState::Failed(e), cx);
            }
        });
    })
    .detach();
}

/// Download the pending update with progress.
pub fn download(cx: &mut App) {
    let Some(info) = cx.global::<Pending>().0.clone() else { return };
    let version = info.TargetFullRelease.Version.clone();
    set_state(UpdateState::Downloading { version: version.clone(), frac: 0.0 }, cx);
    let (tx, rx) = std::sync::mpsc::channel::<i16>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let _ = done_tx.send(manager().and_then(|m| m.download_updates(&info, Some(tx)).map_err(|e| e.to_string())));
    });
    let v2 = version.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let mut last = None;
            while let Ok(p) = rx.try_recv() {
                last = Some(p);
            }
            if let Some(p) = last {
                let v = v2.clone();
                let _ = cx.update(|cx| set_state(UpdateState::Downloading { version: v, frac: p as f32 / 100.0 }, cx));
            }
            if let Ok(res) = done_rx.try_recv() {
                let _ = cx.update(|cx| match res {
                    Ok(()) => {
                        set_state(UpdateState::Ready { version: v2.clone() }, cx);
                        notify_ready(&v2, cx);
                    }
                    Err(e) => set_state(UpdateState::Failed(e), cx),
                });
                break;
            }
        }
    })
    .detach();
}

fn notify_ready(version: &str, cx: &mut App) {
    let text = castor_core::tr_args("app.update.ready", &[("version", version)]);
    if let Some(h) = cx.global::<crate::app::MainWindow>().handle {
        let _ = h.update(cx, |_, window, cx| window.push_notification(text, cx));
    }
}

/// Apply now and restart.
pub fn restart(cx: &mut App) {
    let Some(info) = cx.global::<Pending>().0.clone() else { return };
    let s = model(cx).read(cx).settings.clone();
    let _ = s.save();
    if let Err(e) = manager().and_then(|m| m.apply_updates_and_restart(&info).map_err(|e| e.to_string())) {
        set_state(UpdateState::Failed(e), cx);
    }
}

/// On quit: apply a downloaded update silently if the user opted in.
pub fn apply_on_quit(cx: &mut App) {
    if !crate::model::settings(cx).auto_install_updates {
        return;
    }
    if !matches!(model(cx).read(cx).update, UpdateState::Ready { .. }) {
        return;
    }
    if let Some(info) = cx.try_global::<Pending>().and_then(|p| p.0.clone()) {
        if let Ok(m) = manager() {
            let _ = m.wait_exit_then_apply_updates(&info, true, false, Vec::<String>::new());
        }
    }
}
