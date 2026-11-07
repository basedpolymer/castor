//! App-wide state: the tool registry, settings (saved in the background), the runs history and
//! the update status. One `Entity<Model>` lives in a global; views observe it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use castor_core::Registry;
use castor_core::history::RunRecord;
use castor_core::options::Values;
use castor_core::settings::Settings;
use gpui_kit::*;

/// Where to go. Sent by the palette, the tray, the quick panel, the single-instance socket…
#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    Page(PageId),
    /// Open a tool, optionally with files queued and values to start from.
    Tool { id: String, files: Vec<PathBuf>, values: Option<Values> },
    /// Show the tools home with these files dropped.
    Files(Vec<PathBuf>),
    /// Open a chain in the editor (index into the chains page list, by name).
    Chain(String),
    /// Open the command palette, optionally with a query typed in.
    Palette(Option<String>),
    /// Open a studio, optionally on a file.
    Studio(crate::studio::StudioKind, Option<PathBuf>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    Tools,
    Studios,
    Chains,
    Runs,
    Settings,
}

impl PageId {
    pub fn key(self) -> &'static str {
        match self {
            PageId::Tools => "tools",
            PageId::Studios => "studios",
            PageId::Chains => "chains",
            PageId::Runs => "runs",
            PageId::Settings => "settings",
        }
    }
    pub fn title(self) -> SharedString {
        crate::ui::t(&format!("app.nav.{}", self.key()))
    }
}

/// State of the in-app updater (see `updates.rs`).
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// Not installed through Velopack (development build).
    Disabled,
    Idle,
    Checking,
    UpToDate,
    Available { version: String, notes: String },
    Downloading { version: String, frac: f32 },
    /// Downloaded, applies on restart.
    Ready { version: String },
    Failed(String),
}

pub enum ModelEvent {
    Navigate(Nav),
}

pub struct Model {
    pub registry: Arc<Registry>,
    pub settings: Settings,
    pub history: Vec<RunRecord>,
    pub history_loaded: bool,
    pub update: UpdateState,
    /// Current app version.
    pub version: &'static str,
    save_task: Option<Task<()>>,
}

impl EventEmitter<ModelEvent> for Model {}

pub struct ModelHandle(pub Entity<Model>);
impl Global for ModelHandle {}

/// The model entity.
pub fn model(cx: &App) -> Entity<Model> {
    cx.global::<ModelHandle>().0.clone()
}

/// Read-only access to settings.
pub fn settings(cx: &App) -> &Settings {
    &cx.global::<ModelHandle>().0.read(cx).settings
}

/// Name of the active workspace, if one is active.
pub fn active_workspace_name(cx: &App) -> Option<String> {
    Some(settings(cx).active_workspace.clone()).filter(|w| !w.is_empty())
}

/// Start a run from the app: like `runner::start`, recorded with the active workspace
/// (`RunRecord::workspace`, shown in the Runs page).
pub fn start_run(req: castor_core::runner::RunRequest, cx: &App) -> castor_core::runner::RunHandle {
    castor_core::runner::start_in(req, active_workspace_name(cx))
}

pub fn registry(cx: &App) -> Arc<Registry> {
    cx.global::<ModelHandle>().0.read(cx).registry.clone()
}

/// Ask the main window to navigate (opens it if needed).
pub fn navigate(nav: Nav, cx: &mut App) {
    crate::app::show_main_window(cx);
    let m = model(cx);
    m.update(cx, |_, cx| cx.emit(ModelEvent::Navigate(nav)));
}

impl Model {
    pub fn new(registry: Arc<Registry>, settings: Settings) -> Model {
        Model {
            registry,
            settings,
            history: Vec::new(),
            history_loaded: false,
            update: UpdateState::Idle,
            version: env!("CARGO_PKG_VERSION"),
            save_task: None,
        }
    }

    /// Change settings and save them a moment later on a background thread.
    pub fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        cx.notify();
        let snapshot = self.settings.clone();
        self.save_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = snapshot.save() {
                        tracing::warn!("could not save settings: {e:#}");
                    }
                })
                .await;
        }));
    }

    /// Like `update_settings` without re-rendering the views (window bounds while dragging…).
    pub fn update_settings_quiet(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        let snapshot = self.settings.clone();
        self.save_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = snapshot.save() {
                        tracing::warn!("could not save settings: {e:#}");
                    }
                })
                .await;
        }));
    }

    /// Load the history off the UI thread.
    pub fn load_history(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let runs = cx.background_executor().spawn(async move { castor_core::history::load(10_000) }).await;
            let _ = this.update(cx, |m, cx| {
                m.history = runs;
                m.history_loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    /// A run just finished: put it on top of the history.
    pub fn push_run(&mut self, r: RunRecord, cx: &mut Context<Self>) {
        if !self.settings.active_workspace.is_empty() {
            let id = r.id.clone();
            self.update_settings_quiet(cx, move |s| crate::workspaces::record_run(s, &id));
        }
        self.history.retain(|x| x.id != r.id);
        self.history.insert(0, r);
        cx.notify();
    }

    /// Tool ids used recently, most recent first, without duplicates.
    pub fn recent_tools(&self, n: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.history {
            for id in &r.steps {
                if !out.contains(id) && self.registry.get(id).is_some() {
                    out.push(id.clone());
                }
            }
            if out.len() >= n {
                break;
            }
        }
        out.truncate(n);
        out
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.settings.favorites.iter().any(|f| f == id)
    }

    pub fn toggle_favorite(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.update_settings(cx, move |s| {
            if let Some(i) = s.favorites.iter().position(|f| *f == id) {
                s.favorites.remove(i);
            } else {
                s.favorites.push(id);
            }
        });
        // The Explorer menu lists the favourites.
        cx.defer(crate::integration::sync_context_menu);
    }

    /// Move favourite `from` to the place of `to` (drag and drop in the sidebar).
    pub fn move_favorite(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        if from == to {
            return;
        }
        let (from, to) = (from.to_string(), to.to_string());
        self.update_settings(cx, move |s| {
            let Some(i) = s.favorites.iter().position(|f| *f == from) else { return };
            let item = s.favorites.remove(i);
            let j = s.favorites.iter().position(|f| *f == to).map(|j| if j >= i { j + 1 } else { j }).unwrap_or(s.favorites.len());
            s.favorites.insert(j.min(s.favorites.len()), item);
        });
        cx.defer(crate::integration::sync_context_menu);
    }

    /// Last values used for a tool, merged over its defaults (unknown keys dropped).
    pub fn tool_values(&self, id: &str) -> Values {
        let Some(tool) = self.registry.get(id) else { return Values::default() };
        let specs = tool.options();
        let mut v = Values::defaults(&specs);
        if let Some(saved) = self.settings.tool_values.get(id) {
            for (k, val) in &saved.0 {
                if v.0.contains_key(k) {
                    v.set(k, val.clone());
                }
            }
        }
        // The active workspace's brand (logo, colours, signature) by option key name.
        if let Some(ws) = crate::workspaces::active(&self.settings) {
            crate::workspaces::prefill(&mut v, &specs, &ws);
        }
        v
    }

    /// Output policy for a tool: the active workspace's folder, else the Settings one.
    pub fn output_policy(&self, tool_id: &str) -> castor_core::runner::OutputPolicy {
        crate::workspaces::output_policy(&self.settings, tool_id)
    }
}
