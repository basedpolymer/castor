//! Every one-time download Castor knows (models, OCR languages, Whisper models, helper programs),
//! with a readable name and the tools that use it. Settings → Models & helpers lists them all.
//!
//! The list is built from the tools themselves: each tool's `downloads()` is asked with its
//! default values (the "offline pack"), then with every value of each choice and language
//! option, so a new model or language shows up here without being declared twice. The engine
//! catalogs (every OCR language, every Whisper size and precision) complete it.

use std::collections::{BTreeMap, HashMap, HashSet};

use castor_core::download::Asset;
use castor_core::{OptionKind, Registry, Value, Values};

/// One downloadable file.
#[derive(Clone, Debug)]
pub struct Download {
    pub asset: Asset,
    /// What it is: "BiRefNet lite", "Français", "Small · 8-bit". Proper nouns and endonyms,
    /// or the tool's own (translated) choice label.
    pub detail: String,
    /// Ids of the tools that can use it.
    pub used_by: Vec<&'static str>,
    /// Needed by at least one tool with its default options: part of the offline pack.
    pub default: bool,
}

impl Download {
    /// "Remove background — BiRefNet lite" (first tool's name, in the UI language).
    pub fn title(&self) -> String {
        match self.used_by.first() {
            Some(id) => format!("{} — {}", castor_core::tr(&format!("{id}.name")), self.detail),
            None => self.detail.clone(),
        }
    }

    /// Names of every tool that uses it, in the UI language.
    pub fn tool_names(&self) -> Vec<String> {
        self.used_by.iter().map(|id| castor_core::tr(&format!("{id}.name"))).collect()
    }

    /// First folder of the id ("models", "tessdata", "whisper"…): the family of the file.
    pub fn group(&self) -> &'static str {
        self.asset.id.split('/').next().unwrap_or("")
    }
}

#[derive(Default)]
struct Entry {
    asset: Option<Asset>,
    detail: Option<String>,
    used_by: Vec<&'static str>,
    default: bool,
}

fn note(map: &mut BTreeMap<&'static str, Entry>, a: Asset, tool: Option<&'static str>, detail: Option<String>, default: bool) {
    let e = map.entry(a.id).or_default();
    if let Some(t) = tool {
        if !e.used_by.contains(&t) {
            e.used_by.push(t);
        }
    }
    if e.detail.is_none() {
        e.detail = detail.filter(|d| !d.trim().is_empty());
    }
    e.default |= default;
    if e.asset.is_none() {
        e.asset = Some(a);
    }
}

/// "small.en-q5" → "Small (en) · 5-bit"; "turbo-q8" → "Large v3 turbo · 8-bit".
fn whisper_detail(key: &str) -> String {
    let (base, variant) = match key.split_once('-') {
        Some((b, v)) => (b, Some(v)),
        None => (key, None),
    };
    let (size, en) = match base.strip_suffix(".en") {
        Some(s) => (s, true),
        None => (base, false),
    };
    let mut s = match size {
        "turbo" => "Large v3 turbo".to_string(),
        other => capitalize(other),
    };
    if en {
        s.push_str(" (en)");
    }
    s.push_str(match variant {
        Some("q8") => " · 8-bit",
        Some("q5") => " · 5-bit",
        _ => "",
    });
    s
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Readable name from the file name when nothing better is known: "models/yunet-2023mar.onnx"
/// → "yunet 2023mar".
fn humanize(id: &str) -> String {
    let file = id.rsplit('/').next().unwrap_or(id);
    let stem = file.split_once('.').map(|(s, _)| s).unwrap_or(file);
    let stem = stem.strip_prefix("ggml-").unwrap_or(stem);
    if stem.starts_with("silero") {
        return "Silero VAD".into();
    }
    stem.replace(['-', '_'], " ")
}

/// Every download, sorted by family then title.
pub fn catalog(reg: &Registry) -> Vec<Download> {
    let mut map: BTreeMap<&'static str, Entry> = BTreeMap::new();
    let mut lang_names: HashMap<&'static str, &'static str> = HashMap::new();

    for tool in reg.all() {
        let id = tool.meta().id;
        let specs = tool.options();
        let defaults = Values::defaults(&specs);
        for a in tool.downloads(&defaults) {
            note(&mut map, a, Some(id), None, true);
        }
        for spec in &specs {
            // (stored value, label) for every value of the option.
            let variants: Vec<(Value, String)> = match &spec.kind {
                OptionKind::Choice(cs) => cs.iter().map(|c| (Value::Text(c.value.into()), spec.choice_label(id, c))).collect(),
                OptionKind::Language { list, .. } => {
                    for (c, n) in list.iter() {
                        lang_names.entry(*c).or_insert(*n);
                    }
                    list.iter().map(|(c, n)| (Value::Text((*c).into()), (*n).to_string())).collect()
                }
                OptionKind::Toggle => vec![(Value::Bool(true), String::new()), (Value::Bool(false), String::new())],
                _ => continue,
            };
            let mut seen: Vec<(String, HashSet<&'static str>)> = Vec::new();
            for (value, label) in variants {
                let mut v = defaults.clone();
                v.set(spec.key, value);
                let assets = tool.downloads(&v);
                seen.push((label, assets.iter().map(|a| a.id).collect()));
                for a in assets {
                    note(&mut map, a, Some(id), None, false);
                }
            }
            // An asset that some values of this option need and others do not is "selected" by
            // it: name it after the first value that brings it.
            let all: HashSet<&'static str> = seen.iter().flat_map(|(_, s)| s.iter().copied()).collect();
            for a in all {
                if seen.iter().all(|(_, s)| s.contains(a)) {
                    continue;
                }
                if let Some((label, _)) = seen.iter().find(|(l, s)| s.contains(a) && !l.is_empty()) {
                    if let Some(e) = map.get_mut(a) {
                        if e.detail.is_none() {
                            e.detail = Some(label.clone());
                        }
                    }
                }
            }
        }
    }

    // The engine catalogs: every OCR language and every Whisper model, not only the ones the
    // tools' options can reach one at a time.
    for a in engine_pdf::ocr::CATALOG.iter() {
        note(&mut map, a.clone(), None, None, false);
    }
    for (key, a) in engine_media::whisper::MODEL_ASSETS.iter() {
        if let Some(e) = map.get_mut(a.id) {
            // Size and precision say more than the "model" choice label alone.
            e.detail = Some(whisper_detail(key));
        } else {
            note(&mut map, a.clone(), None, Some(whisper_detail(key)), false);
        }
    }

    // Files no option reaches are used by the tools that use their siblings (same folder),
    // except generic ML models, which are not interchangeable.
    let mut by_group: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    for (id, e) in &map {
        let id: &'static str = *id;
        let g: &'static str = id.split('/').next().unwrap_or("");
        let tools = by_group.entry(g).or_default();
        for t in &e.used_by {
            if !tools.contains(t) {
                tools.push(t);
            }
        }
    }

    let mut out: Vec<Download> = map
        .into_iter()
        .filter_map(|(id, e)| {
            let asset = e.asset?;
            let group = id.split('/').next().unwrap_or("");
            let used_by = if e.used_by.is_empty() && group != "models" { by_group.get(group).cloned().unwrap_or_default() } else { e.used_by };
            let detail = e.detail.unwrap_or_else(|| {
                let code = id.strip_prefix("tessdata/").and_then(|f| f.strip_suffix(".traineddata"));
                match code.and_then(|c| lang_names.get(c)) {
                    Some(n) => n.to_string(),
                    None => humanize(id),
                }
            });
            Some(Download { asset, detail, used_by, default: e.default })
        })
        .collect();
    out.sort_by(|a, b| a.group().cmp(b.group()).then_with(|| castor_core::files::natural_cmp(&a.title(), &b.title())));
    out
}

/// Download one file (resumable, checked), the way the tool that uses it would: yt-dlp through
/// its installer (release checksum, executable bit), everything else with `download::ensure`.
/// `progress(done, total)` is called as bytes arrive.
pub fn install(
    asset: &Asset,
    cancel: &castor_core::CancelToken,
    progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> castor_core::Result<std::path::PathBuf> {
    if asset.id == engine_media::ytdlp::asset().id {
        let tmp = castor_core::paths::temp_dir().join("downloads");
        std::fs::create_dir_all(&tmp)?;
        let cx = castor_core::RunCx::new(tmp.clone(), tmp, "", cancel.clone())
            .on_progress(move |f| progress((f.clamp(0.0, 1.0) * 1000.0) as u64, 1000));
        return engine_media::ytdlp::ensure(&cx);
    }
    castor_core::download::ensure(asset, cancel, &progress)
}

/// What every tool needs with its default options: the offline pack.
pub fn offline_pack(reg: &Registry) -> Vec<Asset> {
    catalog(reg).into_iter().filter(|d| d.default).map(|d| d.asset).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_names() {
        assert_eq!(whisper_detail("small"), "Small");
        assert_eq!(whisper_detail("small.en-q5"), "Small (en) · 5-bit");
        assert_eq!(whisper_detail("turbo-q8"), "Large v3 turbo · 8-bit");
    }

    #[test]
    fn humanized_names() {
        assert_eq!(humanize("models/yunet-2023mar.onnx"), "yunet 2023mar");
        assert_eq!(humanize("whisper/ggml-silero-v6.2.0.bin"), "Silero VAD");
    }

    #[test]
    fn the_catalog_is_complete_and_named() {
        castor_core::i18n::set_language("en");
        let c = catalog(&crate::registry());
        let ids: HashSet<&str> = c.iter().map(|d| d.asset.id).collect();
        assert_eq!(ids.len(), c.len(), "one row per file");
        for a in engine_pdf::ocr::CATALOG.iter() {
            assert!(ids.contains(a.id), "{} listed", a.id);
        }
        for (_, a) in engine_media::whisper::MODEL_ASSETS.iter() {
            assert!(ids.contains(a.id), "{} listed", a.id);
        }
        assert!(c.iter().any(|d| d.default), "some tools need downloads with their defaults");
        let fra = c.iter().find(|d| d.asset.id == "tessdata/fra.traineddata").unwrap();
        assert!(!fra.used_by.is_empty(), "OCR languages are used by the OCR tools");
        assert!(!fra.detail.contains("traineddata"), "readable name: {}", fra.detail);
    }
}
