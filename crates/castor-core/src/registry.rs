//! The set of all tools, and search over them.

use std::sync::Arc;

use crate::files::FileInfo;
use crate::tool::{Family, Tool, ToolStatus};

#[derive(Clone, Default)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Registry {
        let mut seen = std::collections::HashSet::new();
        for t in &tools {
            assert!(seen.insert(t.meta().id), "duplicate tool id {}", t.meta().id);
        }
        Registry { tools }
    }
    pub fn all(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
    pub fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.meta().id == id).cloned()
    }
    pub fn family(&self, f: Family) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.iter().filter(move |t| t.meta().family == f)
    }
    /// Tools that can take every one of these files.
    pub fn for_files<'a>(&'a self, files: &'a [FileInfo]) -> impl Iterator<Item = &'a Arc<dyn Tool>> + 'a {
        self.tools.iter().filter(move |t| {
            t.meta().status != ToolStatus::Soon && !files.is_empty() && files.iter().all(|f| t.accepts(f))
        })
    }

    /// Simple ranked search over localised name, description, id and keywords.
    /// The app may layer smarter search on top; this is the always-available baseline.
    pub fn search(&self, query: &str) -> Vec<(Arc<dyn Tool>, i32)> {
        let q = fold(query);
        let words: Vec<&str> = q.split_whitespace().collect();
        if words.is_empty() {
            return self.tools.iter().map(|t| (t.clone(), 0)).collect();
        }
        let mut out: Vec<(Arc<dyn Tool>, i32)> = self
            .tools
            .iter()
            .filter_map(|t| {
                let m = t.meta();
                let name = fold(&m.name());
                let desc = fold(&m.desc());
                let kw_local = fold(&crate::tr(&format!("{}.keywords", m.id)));
                let kw = format!("{} {} {}", m.id.replace(['.', '-'], " "), m.keywords.join(" "), kw_local);
                let mut score = 0;
                for w in &words {
                    let s = if name.starts_with(w) {
                        40
                    } else if name.split_whitespace().any(|x| x.starts_with(w)) {
                        30
                    } else if kw.split_whitespace().any(|x| x.starts_with(w)) {
                        20
                    } else if desc.contains(w) {
                        10
                    } else if name.contains(w) || kw.contains(w) {
                        8
                    } else {
                        0
                    };
                    if s == 0 && w.len() > 2 {
                        return None; // every word of 3+ letters must match somewhere
                    }
                    score += s;
                }
                if m.status == ToolStatus::Soon {
                    score -= 15;
                }
                (score > 0).then(|| (t.clone(), score))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }
}

/// Lowercase and strip accents (é → e) for matching.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ß' => 's',
            _ => c,
        })
        .collect()
}
