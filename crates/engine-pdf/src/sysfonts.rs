//! System fonts for MuPDF (fonts a document names but does not embed, CJK and fallback glyphs).
//!
//! Replaces the `mupdf` crate's `system-fonts` loader (font-kit), which enumerated every
//! installed font on *each* lookup and never remembered a miss: a single page of a paper with a
//! few symbols cost seconds, and an EPUB minutes. Here the installed fonts are indexed once
//! (names read with `ttf-parser`, index cached on disk), every answer — found or not — is
//! memoised, and font files are read once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use mupdf::font_loader::{FontHints, FontLoader};
use mupdf::{CjkFontOrdering, Font};
use parking_lot::Mutex;

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    index: u32,
    ps: String,
    families: Vec<String>,
    bold: bool,
    italic: bool,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').flat_map(|c| c.to_lowercase()).collect()
}

fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        if let Some(w) = std::env::var_os("WINDIR") {
            v.push(PathBuf::from(w).join("Fonts"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        v.push(PathBuf::from("/System/Library/Fonts"));
        v.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
        v.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join("Library/Fonts"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        v.push(PathBuf::from("/usr/share/fonts"));
        v.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".local/share/fonts"));
        }
    }
    v
}

fn font_files(dir: &std::path::Path, out: &mut Vec<PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 3 {
                font_files(&p, out, depth + 1);
            }
            continue;
        }
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            out.push(p);
        }
    }
}

fn read_at(f: &mut std::fs::File, off: u64, len: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut b = vec![0u8; len];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

fn be16(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}
fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Read only the table directory, `name` and `head` of each face (a few KB per file instead of
/// the whole font: indexing ~1 000 Windows fonts takes a fraction of a second).
fn parse_file(path: &PathBuf, out: &mut Vec<Entry>) {
    let Ok(mut f) = std::fs::File::open(path) else { return };
    let Some(hdr) = read_at(&mut f, 0, 12) else { return };
    let offsets: Vec<u32> = if &hdr[0..4] == b"ttcf" {
        let n = be32(&hdr, 8).min(64) as usize;
        match read_at(&mut f, 12, 4 * n) {
            Some(b) => (0..n).map(|i| be32(&b, 4 * i)).collect(),
            None => return,
        }
    } else {
        vec![0]
    };
    for (index, off) in offsets.into_iter().enumerate() {
        let Some(dir) = read_at(&mut f, off as u64, 12) else { continue };
        let n = be16(&dir, 4).min(256) as usize;
        let Some(recs) = read_at(&mut f, off as u64 + 12, 16 * n) else { continue };
        let (mut name, mut head) = (None, None);
        for r in 0..n {
            let rec = &recs[16 * r..16 * r + 16];
            let loc = (be32(rec, 8) as u64, be32(rec, 12) as usize);
            match &rec[0..4] {
                b"name" => name = Some(loc),
                b"head" => head = Some(loc),
                _ => {}
            }
        }
        let Some((no, nl)) = name else { continue };
        let Some(ndata) = read_at(&mut f, no, nl.min(1 << 20)) else { continue };
        let Some(table) = ttf_parser::name::Table::parse(&ndata) else { continue };
        let mut ps = String::new();
        let mut families = Vec::new();
        for nm in table.names {
            let Some(s) = nm.to_string() else { continue };
            match nm.name_id {
                ttf_parser::name_id::POST_SCRIPT_NAME if ps.is_empty() => ps = norm(&s),
                ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY | ttf_parser::name_id::FULL_NAME => {
                    let fam = norm(&s);
                    if !families.contains(&fam) {
                        families.push(fam);
                    }
                }
                _ => {}
            }
        }
        let style = head.and_then(|(ho, _)| read_at(&mut f, ho, 54)).map(|h| be16(&h, 44)).unwrap_or(0);
        out.push(Entry { path: path.clone(), index: index as u32, ps, families, bold: style & 1 != 0, italic: style & 2 != 0 });
    }
}

/// Index of installed fonts; cached in `cache_dir()/fonts-index-2.tsv`, rebuilt when the set of
/// font files (names, sizes) changes.
fn index() -> &'static Vec<Entry> {
    static IDX: OnceLock<Vec<Entry>> = OnceLock::new();
    IDX.get_or_init(|| {
        let mut files = Vec::new();
        for d in font_dirs() {
            font_files(&d, &mut files, 0);
        }
        files.sort();
        let stamp = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for f in &files {
                f.hash(&mut h);
                std::fs::metadata(f).map(|m| m.len()).unwrap_or(0).hash(&mut h);
            }
            format!("{:016x}", h.finish())
        };
        let cache = castor_core::paths::cache_dir().join("fonts-index-2.tsv");
        if let Ok(s) = std::fs::read_to_string(&cache) {
            let mut lines = s.lines();
            if lines.next() == Some(stamp.as_str()) {
                let v: Vec<Entry> = lines
                    .filter_map(|l| {
                        let p: Vec<&str> = l.split('\t').collect();
                        (p.len() == 6).then(|| Entry {
                            path: PathBuf::from(p[0]),
                            index: p[1].parse().unwrap_or(0),
                            ps: p[2].to_string(),
                            families: p[3].split('|').filter(|x| !x.is_empty()).map(String::from).collect(),
                            bold: p[4] == "1",
                            italic: p[5] == "1",
                        })
                    })
                    .collect();
                if !v.is_empty() {
                    return v;
                }
            }
        }
        let mut v = Vec::new();
        for f in &files {
            parse_file(f, &mut v);
        }
        let mut s = format!("{stamp}\n");
        for e in &v {
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                e.path.display(),
                e.index,
                e.ps,
                e.families.join("|"),
                e.bold as u8,
                e.italic as u8
            ));
        }
        if let Some(d) = cache.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&cache, s);
        v
    })
}

type Key = String;

struct Cache {
    answers: HashMap<Key, Option<usize>>,
    data: HashMap<PathBuf, Arc<Vec<u8>>>,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache { answers: HashMap::new(), data: HashMap::new() }))
}

fn load_entry(i: usize) -> Option<Font> {
    let e = index().get(i)?;
    let data = {
        let mut c = cache().lock();
        if let Some(d) = c.data.get(&e.path) {
            d.clone()
        } else {
            let d = Arc::new(std::fs::read(&e.path).ok()?);
            c.data.insert(e.path.clone(), d.clone());
            d
        }
    };
    let name = e.families.first().cloned().unwrap_or_else(|| e.ps.clone());
    Font::from_bytes_with_index(&name, e.index as i32, &data).ok()
}

/// Memoised lookup: `key` → the index of the best entry found by `find`.
fn memo(key: Key, find: impl FnOnce() -> Option<usize>) -> Option<Font> {
    if let Some(ans) = cache().lock().answers.get(&key).copied() {
        return ans.and_then(load_entry);
    }
    let ans = find();
    cache().lock().answers.insert(key, ans);
    ans.and_then(load_entry)
}

fn best_in_family(family: &str, bold: bool, italic: bool) -> Option<usize> {
    let f = norm(family);
    let idx = index();
    let mut best: Option<(i32, usize)> = None;
    for (i, e) in idx.iter().enumerate() {
        if !e.families.iter().any(|x| *x == f) {
            continue;
        }
        let score = (e.bold == bold) as i32 * 2 + (e.italic == italic) as i32 * 2 + (e.index == 0) as i32;
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

fn first_family(families: &[&str], bold: bool, italic: bool) -> Option<usize> {
    families.iter().find_map(|f| best_in_family(f, bold, italic))
}

fn by_name(name: &str, hints: FontHints) -> Option<usize> {
    // "ABCDEF+Name,Bold" → "Name", bold.
    let name = name.split_once('+').filter(|(p, _)| p.len() == 6).map(|(_, n)| n).unwrap_or(name);
    let n = norm(name);
    let idx = index();
    if let Some(i) = idx.iter().position(|e| e.ps == n) {
        return Some(i);
    }
    let mut base = name.split([',', '-']).next().unwrap_or(name).to_string();
    for suffix in ["MT", "PS", "IdentityH"] {
        if let Some(b) = base.strip_suffix(suffix) {
            base = b.to_string();
        }
    }
    let lower = name.to_lowercase();
    let bold = hints.bold || lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
    let italic = hints.italic || lower.contains("italic") || lower.contains("oblique");
    best_in_family(&base, bold, italic)
}

#[cfg(windows)]
mod families {
    pub const SANS: &[&str] = &["Segoe UI", "Arial", "Tahoma"];
    pub const SYMBOLS: &[&str] = &["Segoe UI Symbol", "Cambria Math", "Segoe UI Emoji", "Arial Unicode MS"];
    pub const GB: &[&str] = &["Microsoft YaHei", "SimSun", "DengXian"];
    pub const CNS: &[&str] = &["Microsoft JhengHei", "MingLiU", "PMingLiU"];
    pub const JAPAN: &[&str] = &["Yu Gothic", "Meiryo", "MS Gothic", "MS Mincho"];
    pub const KOREA: &[&str] = &["Malgun Gothic", "Batang", "Gulim"];
    pub const INDIC: &[&str] = &["Nirmala UI", "Mangal"];
    pub const THAI: &[&str] = &["Leelawadee UI", "Tahoma"];
    pub const ETHIOPIC: &[&str] = &["Ebrima", "Nyala"];
}

#[cfg(not(windows))]
mod families {
    pub const SANS: &[&str] = &["Helvetica Neue", "Helvetica", "Arial", "DejaVu Sans", "Noto Sans"];
    pub const SYMBOLS: &[&str] = &["Apple Symbols", "STIX Two Math", "Menlo", "DejaVu Sans", "Noto Sans Symbols"];
    pub const GB: &[&str] = &["PingFang SC", "Songti SC", "Heiti SC", "Noto Sans CJK SC"];
    pub const CNS: &[&str] = &["PingFang TC", "Songti TC", "Heiti TC", "Noto Sans CJK TC"];
    pub const JAPAN: &[&str] = &["Hiragino Sans", "Hiragino Mincho ProN", "Noto Sans CJK JP"];
    pub const KOREA: &[&str] = &["Apple SD Gothic Neo", "AppleGothic", "Noto Sans CJK KR"];
    pub const INDIC: &[&str] = &["Kohinoor Devanagari", "Devanagari Sangam MN", "Noto Sans Devanagari"];
    pub const THAI: &[&str] = &["Thonburi", "Ayuthaya", "Noto Sans Thai"];
    pub const ETHIOPIC: &[&str] = &["Kefa", "Noto Sans Ethiopic"];
}

// UCDN script codes and fz_text_language values (stable in MuPDF's public headers).
const SCRIPT_HEBREW: u32 = 5;
const SCRIPT_ARABIC: u32 = 6;
const SCRIPT_DEVANAGARI: u32 = 9;
const SCRIPT_TAMIL: u32 = 14;
const SCRIPT_THAI: u32 = 19;
const SCRIPT_HANGUL: u32 = 24;
const SCRIPT_ETHIOPIC: u32 = 25;
const SCRIPT_HIRAGANA: u32 = 32;
const SCRIPT_KATAKANA: u32 = 33;
const SCRIPT_HAN: u32 = 35;
const LANG_JA: u32 = 37;
const LANG_KO: u32 = 416;
const LANG_ZH_HANT: u32 = 14822;

/// Loader registered with MuPDF by [`install`].
pub struct CastorFonts;

impl FontLoader for CastorFonts {
    fn load_font(&self, name: &str, hints: FontHints) -> Option<Font> {
        // Generic CSS families of the HTML/EPUB engine. MuPDF would otherwise use its built-in
        // URW fonts, whose text cannot be extracted again from the converted PDF (no Unicode
        // map); Latin Modern has one, and matches the house style.
        use crate::fonts as f;
        let generic: Option<&'static [u8]> = match (name, hints.bold, hints.italic) {
            ("serif", true, _) => Some(f::LM_ROMAN_BOLD),
            ("serif", false, true) => Some(f::LM_ROMAN_ITALIC),
            ("serif", false, false) => Some(f::LM_ROMAN),
            ("sans-serif", true, _) => Some(f::LM_SANS_BOLD),
            ("sans-serif", false, true) => Some(f::LM_SANS_OBLIQUE),
            ("sans-serif", false, false) => Some(f::LM_SANS),
            ("monospace", ..) => Some(f::LM_MONO),
            _ => None,
        };
        if let Some(bytes) = generic {
            return Font::from_bytes(name, bytes).ok();
        }
        let key = format!("n:{name}:{}{}{}", hints.bold as u8, hints.italic as u8, hints.needs_exact_metrics as u8);
        let font = memo(key, || by_name(name, hints))?;
        if hints.needs_exact_metrics && ((hints.bold && !font.is_bold()) || (hints.italic && !font.is_italic())) {
            return None;
        }
        Some(font)
    }

    fn load_cjk_font(&self, _name: &str, ordering: CjkFontOrdering, _serif: bool) -> Option<Font> {
        let fams = match ordering {
            CjkFontOrdering::AdobeCns => families::CNS,
            CjkFontOrdering::AdobeGb => families::GB,
            CjkFontOrdering::AdobeJapan => families::JAPAN,
            CjkFontOrdering::AdobeKorea => families::KOREA,
        };
        memo(format!("cjk:{fams:?}"), || first_family(fams, false, false))
    }

    fn load_fallback_font(&self, script: u32, language: u32, hints: FontHints) -> Option<Font> {
        let fams: &[&str] = match script {
            SCRIPT_HAN => match language {
                LANG_JA => families::JAPAN,
                LANG_KO => families::KOREA,
                LANG_ZH_HANT => families::CNS,
                _ => families::GB,
            },
            SCRIPT_HIRAGANA | SCRIPT_KATAKANA => families::JAPAN,
            SCRIPT_HANGUL => families::KOREA,
            SCRIPT_DEVANAGARI..=SCRIPT_TAMIL => families::INDIC,
            SCRIPT_THAI => families::THAI,
            SCRIPT_ETHIOPIC => families::ETHIOPIC,
            SCRIPT_HEBREW | SCRIPT_ARABIC | 1..=4 => families::SANS,
            _ => families::SYMBOLS,
        };
        memo(format!("fb:{script}:{language}:{}{}", hints.bold as u8, hints.italic as u8), || {
            first_family(fams, hints.bold, hints.italic).or_else(|| first_family(fams, false, false))
        })
    }
}

/// Register the loader (once, before documents are opened).
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| mupdf::set_font_loader(CastorFonts));
}
