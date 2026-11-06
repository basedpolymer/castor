//! Smarter tool search on top of `Registry::search`: plain sentences ("make this pdf smaller"),
//! intent synonyms (smaller → compress), recency and favourites boosts, fuzzy-ish matching.

use std::sync::Arc;

use castor_core::registry::fold;
use castor_core::{Registry, Tool};

/// Words that carry no meaning in a request ("make this pdf smaller").
const STOP: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "my", "your", "our", "to", "into", "in", "on", "of", "for",
    "from", "with", "and", "or", "it", "its", "i", "me", "we", "want", "wanna", "need", "please", "make", "can", "you",
    "how", "do", "turn", "get", "some", "all", "file", "files", "one", "is", "be", "as", "so", "more", "less",
    // French
    "le", "la", "les", "un", "une", "des", "du", "de", "d", "l", "ce", "cet", "cette", "ces", "mon", "ma", "mes", "ton",
    "ta", "tes", "en", "et", "ou", "a", "au", "aux", "je", "veux", "voudrais", "faire", "rendre", "fichier", "fichiers",
    "pour", "avec", "sur", "dans", "plus", "moins", "s", "il", "te", "plait", "stp", "svp",
];

/// Intent words → words that appear in tool ids / keywords.
const SYNONYMS: &[(&str, &str)] = &[
    ("smaller", "compress"),
    ("shrink", "compress"),
    ("reduce", "compress"),
    ("lighter", "compress"),
    ("lighten", "compress"),
    ("squeeze", "compress"),
    ("petit", "compress"),
    ("leger", "compress"),
    ("alleger", "compress"),
    ("reduire", "compress"),
    ("compresser", "compress"),
    ("bigger", "upscale"),
    ("enlarge", "upscale"),
    ("agrandir", "upscale"),
    ("join", "merge"),
    ("combine", "merge"),
    ("fusionner", "merge"),
    ("assembler", "merge"),
    ("cut", "split"),
    ("separate", "split"),
    ("decouper", "split"),
    ("diviser", "split"),
    ("searchable", "ocr"),
    ("scan", "ocr"),
    ("texte", "text"),
    ("lock", "encrypt"),
    ("password", "encrypt"),
    ("proteger", "encrypt"),
    ("chiffrer", "encrypt"),
    ("unlock", "unlock"),
    ("deverrouiller", "unlock"),
    ("background", "background"),
    ("fond", "background"),
    ("detourer", "background"),
    ("transparent", "background"),
    ("convertir", "convert"),
    ("change", "convert"),
    ("transform", "convert"),
    ("jpeg", "jpg"),
    ("photo", "image"),
    ("photos", "image"),
    ("picture", "image"),
    ("pictures", "image"),
    ("images", "image"),
    ("pdfs", "pdf"),
    ("videos", "video"),
    ("film", "video"),
    ("movie", "video"),
    ("son", "audio"),
    ("sound", "audio"),
    ("music", "audio"),
    ("musique", "audio"),
    ("rotate", "rotate"),
    ("tourner", "rotate"),
    ("pivoter", "rotate"),
    ("redimensionner", "resize"),
    ("scale", "resize"),
    ("sign", "sign"),
    ("signer", "sign"),
    ("signature", "sign"),
    ("watermark", "watermark"),
    ("filigrane", "watermark"),
    ("rogner", "crop"),
    ("recadrer", "crop"),
    ("trim", "trim"),
    ("raccourcir", "trim"),
    ("gif", "gif"),
    ("subtitle", "subtitles"),
    ("transcribe", "transcribe"),
    ("transcrire", "transcribe"),
];

/// Normalised meaningful words of a query, with synonyms added.
pub fn query_words(q: &str) -> Vec<String> {
    let folded = fold(q);
    let mut out: Vec<String> = Vec::new();
    for w in folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if STOP.contains(&w) {
            continue;
        }
        let w = w.to_string();
        if let Some((_, syn)) = SYNONYMS.iter().find(|(a, _)| *a == w) {
            if !out.iter().any(|x| x == syn) {
                out.push(syn.to_string());
            }
        }
        // Simple plural: "pdfs" → "pdf".
        let w = if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_string() } else { w };
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// Is `needle` a subsequence of `hay` (fuzzy match for short typos like "cmprs")?
pub fn subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Ranked tools for a query. `favorites` and `recent` (most recent first) boost results.
pub fn tools(reg: &Registry, q: &str, favorites: &[String], recent: &[String]) -> Vec<(Arc<dyn Tool>, i32)> {
    let words = query_words(q);
    let boost = |t: &Arc<dyn Tool>| -> i32 {
        let id = t.meta().id;
        let mut b = 0;
        if favorites.iter().any(|f| f == id) {
            b += 6;
        }
        if let Some(p) = recent.iter().position(|r| r == id) {
            b += 8 - p.min(7) as i32;
        }
        b
    };
    if words.is_empty() {
        let mut all: Vec<(Arc<dyn Tool>, i32)> = reg.all().iter().map(|t| (t.clone(), boost(t))).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1));
        return all;
    }
    // 1. The baseline search on the whole cleaned query.
    let mut scored: Vec<(Arc<dyn Tool>, i32)> = reg.search(&words.join(" "));
    // 2. Word by word: a tool matching more words ranks higher.
    if scored.len() < 3 && words.len() > 1 {
        let mut acc: Vec<(Arc<dyn Tool>, i32, usize)> = Vec::new();
        for w in &words {
            for (t, s) in reg.search(w) {
                match acc.iter_mut().find(|(x, _, _)| x.meta().id == t.meta().id) {
                    Some(e) => {
                        e.1 += s;
                        e.2 += 1;
                    }
                    None => acc.push((t, s, 1)),
                }
            }
        }
        for (t, s, n) in acc {
            let s = s + (n as i32 - 1) * 25;
            if !scored.iter().any(|(x, _)| x.meta().id == t.meta().id) {
                scored.push((t, s - 5));
            }
        }
    }
    // 3. Fuzzy fallback on names and ids.
    if scored.is_empty() {
        let q = words.join("");
        for t in reg.all() {
            let name = fold(&t.meta().name()).replace(' ', "");
            let id = t.meta().id.replace(['.', '-'], "");
            if q.len() >= 3 && (subsequence(&q, &name) || subsequence(&q, &id)) {
                scored.push((t.clone(), 5));
            }
        }
    }
    for (t, s) in &mut scored {
        *s += boost(t);
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.meta().id.cmp(b.0.meta().id)));
    scored
}

/// Generic fuzzy-ish score of a label for pages, actions and chains (0 = no match).
pub fn label_score(q: &str, label: &str) -> i32 {
    let q = fold(q.trim());
    if q.is_empty() {
        return 1;
    }
    let l = fold(label);
    if l.starts_with(&q) {
        40
    } else if l.split_whitespace().any(|w| w.starts_with(&q)) {
        30
    } else if l.contains(&q) {
        20
    } else if q.len() >= 2 && subsequence(&q.replace(' ', ""), &l.replace(' ', "")) {
        8
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sentence_words() {
        assert_eq!(query_words("make this pdf smaller"), vec!["pdf", "compress", "smaller"]);
        assert_eq!(query_words("Rendre ce PDF plus léger"), vec!["pdf", "compress", "leger"]);
    }
    #[test]
    fn fuzzy() {
        assert!(subsequence("cmprs", "compress"));
        assert!(!subsequence("xyz", "compress"));
        assert!(label_score("set", "Settings") > 0);
        assert_eq!(label_score("zzz", "Settings"), 0);
    }
}
