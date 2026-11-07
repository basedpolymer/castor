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

/// A folder whose new files run through a chain (results in a `Castor` sub-folder).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WatchFolder {
    pub folder: PathBuf,
    /// Chain name (a template or a saved chain).
    pub chain: String,
    #[serde(default)]
    pub paused: bool,
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
    /// Windows Explorer "Castor ›" menu, once chosen in Settings. `None` = the default (on for
    /// installed builds, off for development builds).
    pub explorer_menu: Option<bool>,
    /// Watch folders: new files run through a chain.
    pub watch_folders: Vec<WatchFolder>,
    /// Author name written on PDF Studio annotations.
    pub pdf_reviewer: String,
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
            explorer_menu: None,
            watch_folders: Vec::new(),
            pdf_reviewer: String::new(),
        }
    }
}

/// Where 0.9.0 kept what now has its own fields.
const OLD_INTEGRATION: &str = "app.integration";
const OLD_PDF_STUDIO: &str = "studio.pdf";

impl Settings {
    pub fn path() -> PathBuf {
        crate::paths::config_dir().join("settings.toml")
    }
    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
        s.migrate();
        s
    }

    /// Move 0.9.0 settings kept in `tool_values` to their own fields (saved on the next save).
    pub fn migrate(&mut self) {
        use crate::options::Value;
        if let Some(old) = self.tool_values.remove(OLD_INTEGRATION) {
            if let (None, Some(Value::Bool(b))) = (self.explorer_menu, old.get("context_menu")) {
                self.explorer_menu = Some(*b);
            }
            if let (true, Some(Value::Text(j))) = (self.watch_folders.is_empty(), old.get("watch")) {
                self.watch_folders = serde_json::from_str(j).unwrap_or_default();
            }
        }
        if let Some(v) = self.tool_values.get_mut(OLD_PDF_STUDIO) {
            if let Some(Value::Text(name)) = v.0.remove("reviewer") {
                if self.pdf_reviewer.is_empty() {
                    self.pdf_reviewer = name;
                }
            }
            if v.0.is_empty() {
                self.tool_values.remove(OLD_PDF_STUDIO);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{Value, Values};

    #[test]
    fn old_settings_move_to_their_fields() {
        let mut s = Settings::default();
        let mut integ = Values::default();
        integ.set("context_menu", Value::Bool(false));
        integ.set("watch", Value::Text(r#"[{"folder":"C:/in","chain":"Web images","paused":false}]"#.into()));
        s.tool_values.insert(OLD_INTEGRATION.into(), integ);
        let mut pdf = Values::default();
        pdf.set("reviewer", Value::Text("Mathis".into()));
        s.tool_values.insert(OLD_PDF_STUDIO.into(), pdf);
        s.migrate();
        assert_eq!(s.explorer_menu, Some(false));
        assert_eq!(s.watch_folders, vec![WatchFolder { folder: "C:/in".into(), chain: "Web images".into(), paused: false }]);
        assert_eq!(s.pdf_reviewer, "Mathis");
        assert!(s.tool_values.is_empty());
        // Round trip through TOML.
        let back: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back.watch_folders, s.watch_folders);
    }
}
