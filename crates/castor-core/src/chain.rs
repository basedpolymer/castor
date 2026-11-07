//! Chains: saved sequences of tool steps, stored as TOML in `{config_dir}/chains/*.toml`.
//!
//! ```toml
//! name = "Web-ready photos"
//! [[steps]]
//! tool = "image.resize"
//! values = { mode = "fit", size = [2048, 2048] }
//! [[steps]]
//! tool = "image.compress"
//! values = { quality = 82 }
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::options::Values;
use crate::registry::Registry;
use crate::runner::Step;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChainStep {
    pub tool: String,
    #[serde(default)]
    pub values: Values,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Chain {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<ChainStep>,
    /// Built-in template (not stored on disk, cannot be deleted).
    #[serde(skip)]
    pub builtin: bool,
}

impl Chain {
    /// Resolve against the registry, leniently (saved chains in the app). Unknown tool ids are
    /// an error; missing values get defaults. Every stored value goes through
    /// [`OptionSpec::check`](crate::options::OptionSpec::check), so "1200x630" becomes a real
    /// size; a key the tool no longer has, or a value it no longer accepts, keeps the default
    /// and is logged. Use [`Chain::resolve_strict`] where a typo must stop the run.
    pub fn resolve(&self, reg: &Registry) -> crate::Result<Vec<Step>> {
        self.steps
            .iter()
            .map(|s| {
                let tool = reg.get(&s.tool).ok_or_else(|| anyhow::anyhow!("unknown tool '{}'", s.tool))?;
                let specs = tool.options();
                let mut values = Values::defaults(&specs);
                for (k, v) in &s.values.0 {
                    match specs.iter().find(|o| o.key == k).map(|o| o.check(v)) {
                        Some(Ok(v)) => values.set(k, v),
                        Some(Err(e)) => tracing::warn!(tool = %s.tool, option = %k, "chain value ignored: {e:#}"),
                        None => tracing::warn!(tool = %s.tool, option = %k, "chain option ignored: the tool has no such option"),
                    }
                }
                Ok(Step { tool, values })
            })
            .collect()
    }

    /// Resolve strictly (command line, MCP server): an empty chain, an unknown tool, a tool that
    /// is not available, an unknown option key or a value of the wrong type or out of range are
    /// errors, all listed in one message, and nothing runs.
    pub fn resolve_strict(&self, reg: &Registry) -> crate::Result<Vec<Step>> {
        anyhow::ensure!(!self.steps.is_empty(), "the chain has no steps");
        let mut problems = Vec::new();
        let mut steps = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            let at = format!("step {} ({})", i + 1, s.tool);
            let Some(tool) = reg.get(&s.tool) else {
                problems.push(format!("{at}: unknown tool"));
                continue;
            };
            if tool.meta().status == crate::tool::ToolStatus::Soon {
                problems.push(format!("{at}: not available yet"));
            } else if let Some(why) = tool.unavailable() {
                problems.push(format!("{at}: {why}"));
            }
            let specs = tool.options();
            let mut values = Values::defaults(&specs);
            for (k, v) in &s.values.0 {
                match specs.iter().find(|o| o.key == k) {
                    None => problems.push(format!(
                        "{at}: no option '{k}' (options: {})",
                        specs.iter().map(|o| o.key).collect::<Vec<_>>().join(", ")
                    )),
                    Some(spec) => match spec.check(v) {
                        Ok(v) => values.set(k, v),
                        Err(e) => problems.push(format!("{at}: option '{k}': {e:#}")),
                    },
                }
            }
            steps.push(Step { tool, values });
        }
        anyhow::ensure!(problems.is_empty(), "{}", problems.join("\n"));
        Ok(steps)
    }

    pub fn to_toml(&self) -> crate::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
    pub fn from_toml(s: &str) -> crate::Result<Chain> {
        Ok(toml::from_str(s)?)
    }
}

pub fn dir() -> PathBuf {
    crate::paths::config_dir().join("chains")
}

/// User chains, sorted by name.
pub fn load_all() -> Vec<(PathBuf, Chain)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") {
                if let Ok(c) = std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|s| Chain::from_toml(&s)) {
                    out.push((p, c));
                }
            }
        }
    }
    out.sort_by(|a, b| crate::files::natural_cmp(&a.1.name, &b.1.name));
    out
}

pub fn save(chain: &Chain) -> crate::Result<PathBuf> {
    std::fs::create_dir_all(dir())?;
    let p = dir().join(format!("{}.toml", crate::job::sanitize_file_name(&chain.name)));
    std::fs::write(&p, chain.to_toml()?)?;
    Ok(p)
}

/// Tesseract code of the UI language: the OCR step of "Scans to searchable PDF" reads the
/// language the user works in (English when there is no match).
pub fn ocr_language() -> &'static str {
    let l = crate::i18n::current_language();
    const MAP: &[(&str, &str)] =
        &[("fr", "fra"), ("de", "deu"), ("es", "spa"), ("it", "ita"), ("pt", "por"), ("ja", "jpn"), ("zh", "chi_sim"), ("ko", "kor")];
    MAP.iter().find(|(p, _)| l.starts_with(p)).map(|(_, c)| *c).unwrap_or("eng")
}

/// Built-in templates. Tool ids that do not exist (yet) are skipped by the UI.
pub fn templates() -> Vec<Chain> {
    fn c(name_key: &str, steps: &[(&str, &[(&str, crate::options::Value)])]) -> Chain {
        Chain {
            name: crate::tr(name_key),
            steps: steps
                .iter()
                .map(|(t, vals)| ChainStep {
                    tool: t.to_string(),
                    values: Values(vals.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()),
                })
                .collect(),
            builtin: true,
        }
    }
    use crate::options::Value as V;
    vec![
        c(
            "chain.template.web_photos",
            &[("image.resize", &[("mode", V::Text("fit".into())), ("size", V::Size(2048, 2048))]), ("image.compress", &[])],
        ),
        c("chain.template.social", &[("image.resize", &[("preset", V::Text("instagram-portrait".into()))]), ("image.convert", &[("format", V::Text("jpg".into()))])]),
        c("chain.template.straighten", &[("image.rotate", &[]), ("image.compress", &[])]),
        c(
            "chain.template.scan_to_pdf",
            &[("image.images-to-pdf", &[]), ("pdf.ocr", &[("languages", V::Text(ocr_language().into()))]), ("pdf.compress", &[])],
        ),
        c("chain.template.podcast", &[("audio.normalize", &[]), ("audio.compress", &[])]),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn scan_template_reads_the_ui_language() {
        let _lock = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(super::ocr_language(), "fra");
        let t = super::templates().into_iter().find(|c| c.steps.iter().any(|s| s.tool == "pdf.ocr")).unwrap();
        let ocr = t.steps.iter().find(|s| s.tool == "pdf.ocr").unwrap();
        assert_eq!(ocr.values.get("languages").and_then(|v| v.as_str()), Some("fra"));
        crate::i18n::set_language("en");
        assert_eq!(super::ocr_language(), "eng");
    }
}
