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
    /// Resolve against the registry. Unknown tool ids are an error; missing values get defaults.
    pub fn resolve(&self, reg: &Registry) -> crate::Result<Vec<Step>> {
        self.steps
            .iter()
            .map(|s| {
                let tool = reg.get(&s.tool).ok_or_else(|| anyhow::anyhow!("unknown tool '{}'", s.tool))?;
                let mut values = Values::defaults(&tool.options());
                for (k, v) in &s.values.0 {
                    values.set(k, v.clone());
                }
                Ok(Step { tool, values })
            })
            .collect()
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
        c("chain.template.scan_to_pdf", &[("image.images-to-pdf", &[]), ("pdf.ocr", &[]), ("pdf.compress", &[])]),
        c("chain.template.podcast", &[("audio.normalize", &[]), ("audio.compress", &[])]),
    ]
}
