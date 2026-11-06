//! User settings, TOML in `{config_dir}/settings.toml`. Unknown keys are kept as defaults.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::options::Values;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputLocation {
    /// Next to each original (default).
    #[default]
    NextToOriginal,
    /// In `output_folder`, in a sub-folder per tool and day.
    Folder,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "" = follow the system language.
    pub language: String,
    pub appearance: Appearance,
    pub output_location: OutputLocation,
    pub output_folder: Option<PathBuf>,
    /// Open the output folder when a run completes.
    pub reveal_when_done: bool,
    /// Menu bar icon (macOS) / system tray icon (Windows).
    pub tray_icon: bool,
    /// Keep running in the menu bar / tray when the window is closed.
    pub keep_running_in_tray: bool,
    pub launch_at_login: bool,
    /// Global shortcut that opens the quick panel, e.g. "ctrl-alt-space" / "cmd-alt-space". "" = off.
    pub global_hotkey: String,
    pub check_updates: bool,
    /// Install updates without asking (downloaded in the background, applied on restart).
    pub auto_install_updates: bool,
    /// Tool ids, in the user's order.
    pub favorites: Vec<String>,
    /// Last values per tool id (secrets never stored).
    pub tool_values: BTreeMap<String, Values>,
    /// Optional API key for "Write with AI" features (stored here only if the user opts in).
    pub ai_api_key: Option<String>,
    /// Max jobs in parallel (0 = automatic).
    pub max_parallel: usize,
    /// Unix seconds of the last update check (checks run at most daily).
    pub last_update_check: Option<i64>,
    /// Main window position and size in logical pixels (x, y, width, height), restored at launch.
    pub window_bounds: Option<[f32; 4]>,
    /// Workspaces (brand, output folder…), managed by the app. Empty = the default workspace.
    pub workspaces: Vec<toml::Table>,
    /// Name of the active workspace ("" = default).
    pub active_workspace: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::new(),
            appearance: Appearance::System,
            output_location: OutputLocation::NextToOriginal,
            output_folder: None,
            reveal_when_done: false,
            tray_icon: true,
            keep_running_in_tray: cfg!(target_os = "macos"),
            launch_at_login: false,
            global_hotkey: if cfg!(target_os = "macos") { "cmd-alt-space".into() } else { "ctrl-alt-space".into() },
            check_updates: true,
            auto_install_updates: false,
            favorites: vec!["pdf.compress".into(), "image.convert".into(), "image.remove-background".into(), "video.compress".into()],
            tool_values: BTreeMap::new(),
            ai_api_key: None,
            max_parallel: 0,
            last_update_check: None,
            window_bounds: None,
            workspaces: Vec::new(),
            active_workspace: String::new(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config_dir().join("settings.toml")
    }
    pub fn load() -> Settings {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) -> crate::Result<()> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        // Write then rename: never leave a half-written settings file.
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }

    pub fn output_policy(&self, tool_id: &str) -> crate::runner::OutputPolicy {
        use crate::runner::OutputPolicy;
        let base = self.output_folder.clone().unwrap_or_else(crate::paths::default_output_dir);
        match self.output_location {
            OutputLocation::NextToOriginal => OutputPolicy::NextToInput { fallback: base },
            OutputLocation::Folder => {
                let day = time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
                    .format(time::macros::format_description!("[year]-[month]-[day]"))
                    .unwrap_or_default();
                let name = crate::tr(&format!("{tool_id}.name"));
                OutputPolicy::Folder(base.join(crate::job::sanitize_file_name(&format!("{name} {day}"))))
            }
        }
    }
}
