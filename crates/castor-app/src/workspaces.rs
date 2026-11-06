//! Workspaces: a name, a logo, two brand colours, a signature, a default output folder and the
//! runs made while it was active. Stored in `Settings::workspaces` (one TOML table each) and
//! `Settings::active_workspace` (name; "" = the default, unbranded workspace).
//!
//! Brand pre-fill convention (no tool code change needed): when a workspace is active, tool
//! options are pre-filled by key name —
//! - `logo` (a file option) ← the workspace logo,
//! - `color` (a colour option) ← the first brand colour, `accent` (colour) ← the second,
//! - `colors` (text) ← both colours, one per line ("Brand #rrggbb"),
//! - `signature` (a file option) ← the workspace signature image.
//!
//! Tool authors who want the brand in their tool just use these keys.

use std::path::PathBuf;

use castor_core::options::{OptionKind, OptionSpec, Value, Values};
use castor_core::runner::OutputPolicy;
use castor_core::settings::{OutputLocation, Settings};
use serde::{Deserialize, Serialize};

/// Run ids remembered per workspace (history filter).
const MAX_RUNS: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub name: String,
    pub logo: Option<PathBuf>,
    /// First brand colour, "#rrggbb" ("" = none).
    pub color: String,
    /// Second brand colour.
    pub accent: String,
    pub signature: Option<PathBuf>,
    pub output_folder: Option<PathBuf>,
    /// Runs made in this workspace, oldest first.
    pub runs: Vec<String>,
}

impl Workspace {
    pub fn from_table(t: &toml::Table) -> Option<Workspace> {
        toml::Value::Table(t.clone()).try_into().ok()
    }

    pub fn to_table(&self) -> toml::Table {
        toml::Value::try_from(self).ok().and_then(|v| v.as_table().cloned()).unwrap_or_default()
    }

    /// The brand colour as GPUI colour (sidebar swatch).
    pub fn swatch(&self) -> Option<gpui_kit::Hsla> {
        parse_hex(&self.color).map(|[r, g, b]| gpui_kit::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }.into())
    }
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 || !h.is_char_boundary(6) {
        return None;
    }
    let v = u32::from_str_radix(&h[..6], 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn all(s: &Settings) -> Vec<Workspace> {
    s.workspaces.iter().filter_map(Workspace::from_table).filter(|w| !w.name.trim().is_empty()).collect()
}

pub fn active(s: &Settings) -> Option<Workspace> {
    if s.active_workspace.is_empty() {
        return None;
    }
    all(s).into_iter().find(|w| w.name == s.active_workspace)
}

pub fn store(s: &mut Settings, list: &[Workspace]) {
    s.workspaces = list.iter().map(Workspace::to_table).collect();
    if !s.active_workspace.is_empty() && !list.iter().any(|w| w.name == s.active_workspace) {
        s.active_workspace.clear();
    }
}

/// A name not used yet ("Workspace 2"…).
pub fn new_name(list: &[Workspace], base: &str) -> String {
    (1..).map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") }).find(|n| !list.iter().any(|w| &w.name == n)).unwrap_or_default()
}

/// Pre-fill brand values (see the module docs). Only keys the workspace defines are touched.
pub fn prefill(values: &mut Values, specs: &[OptionSpec], ws: &Workspace) {
    let color = |c: &str| parse_hex(c).map(|[r, g, b]| Value::Text(format!("#{r:02x}{g:02x}{b:02x}ff")));
    for spec in specs {
        let v = match (spec.key, &spec.kind) {
            ("logo", OptionKind::Path { folder: false, .. }) => ws.logo.clone().map(|p| Value::Path(Some(p))),
            ("signature", OptionKind::Path { folder: false, .. }) => ws.signature.clone().map(|p| Value::Path(Some(p))),
            ("color", OptionKind::Color) => color(&ws.color),
            ("accent", OptionKind::Color) => color(&ws.accent),
            ("colors", OptionKind::Text { .. }) => {
                let lines: Vec<String> = [&ws.color, &ws.accent].iter().filter(|c| parse_hex(c).is_some()).map(|c| format!("{} {}", ws.name, c.trim())).collect();
                (!lines.is_empty()).then(|| Value::Text(lines.join("\n")))
            }
            _ => None,
        };
        if let Some(v) = v {
            values.set(spec.key, v);
        }
    }
}

/// Output policy for a tool: the active workspace's folder when it has one, else Settings.
pub fn output_policy(s: &Settings, tool_id: &str) -> OutputPolicy {
    match active(s).and_then(|w| w.output_folder) {
        Some(folder) => {
            let mut s2 = s.clone();
            s2.output_location = OutputLocation::Folder;
            s2.output_folder = Some(folder);
            s2.output_policy(tool_id)
        }
        None => s.output_policy(tool_id),
    }
}

/// Remember that run `id` was made in the active workspace.
pub fn record_run(s: &mut Settings, id: &str) {
    let Some(mut ws) = active(s) else { return };
    ws.runs.push(id.to_string());
    if ws.runs.len() > MAX_RUNS {
        let drop = ws.runs.len() - MAX_RUNS;
        ws.runs.drain(..drop);
    }
    let mut list = all(s);
    if let Some(w) = list.iter_mut().find(|w| w.name == ws.name) {
        *w = ws;
    }
    store(s, &list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_prefill() {
        let ws = Workspace { name: "Acme".into(), color: "#c0392b".into(), accent: "#ffeca4".into(), logo: Some("logo.png".into()), ..Default::default() };
        let mut s = Settings::default();
        store(&mut s, std::slice::from_ref(&ws));
        s.active_workspace = "Acme".into();
        assert_eq!(active(&s), Some(ws.clone()));
        let specs = vec![
            OptionSpec::color("color", "#000000"),
            OptionSpec::color("accent", "#000000"),
            OptionSpec::file("logo", &["png"]),
            OptionSpec::choice("other", vec![castor_core::Choice::key("a")], "a"),
        ];
        let mut v = Values::defaults(&specs);
        prefill(&mut v, &specs, &ws);
        assert_eq!(v.str("color"), "#c0392bff");
        assert_eq!(v.str("accent"), "#ffeca4ff");
        assert_eq!(v.path("logo"), Some(std::path::Path::new("logo.png")));
        record_run(&mut s, "r1");
        assert_eq!(active(&s).unwrap().runs, vec!["r1".to_string()]);
        assert!(matches!(output_policy(&s, "x"), OutputPolicy::NextToInput { .. }));
    }
}
