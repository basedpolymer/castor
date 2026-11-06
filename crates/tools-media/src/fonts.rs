//! Fonts for drawtext / libass: Latin Modern is bundled (GUST licence); other families are looked
//! up among the system fonts. The chosen file is copied into the job's temp folder so filter
//! graphs refer to it by a plain relative name (no path escaping).

use std::path::Path;
use std::sync::OnceLock;

const LM_SANS: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-regular.otf");
const LM_SANS_BOLD: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-bold.otf");
const LM_ROMAN: &[u8] = include_bytes!("../../../assets/fonts/lmroman10-regular.otf");
const LM_ROMAN_BOLD: &[u8] = include_bytes!("../../../assets/fonts/lmroman10-bold.otf");

/// Default caption / watermark family.
pub const DEFAULT_FAMILY: &str = "Latin Modern Sans";

/// A font ready for ffmpeg: file name inside the temp dir and its family name (for ASS).
pub struct FontPick {
    pub file: String,
    pub family: String,
}

fn system_db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Family name (name ID 1, what libass matches on) of a font file.
fn family_of(data: &[u8]) -> Option<String> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let names = face.names();
    (0..names.len())
        .filter_map(|i| names.get(i))
        .filter(|n| n.name_id == ttf_parser::name_id::FAMILY)
        .find_map(|n| n.to_string())
}

/// Put a font for `family` (bold or not) into `dir`.
pub fn prepare(family: &str, bold: bool, dir: &Path) -> anyhow::Result<FontPick> {
    let f = family.trim();
    let lower = f.to_ascii_lowercase();
    let bundled = if f.is_empty() || lower.contains("latin modern sans") || lower.starts_with("lm sans") {
        Some(if bold { LM_SANS_BOLD } else { LM_SANS })
    } else if lower.contains("latin modern") || lower.starts_with("lm roman") {
        Some(if bold { LM_ROMAN_BOLD } else { LM_ROMAN })
    } else {
        None
    };
    if let Some(data) = bundled {
        return write(data, "otf", dir);
    }
    let db = system_db();
    let q = fontdb::Query {
        families: &[fontdb::Family::Name(f)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        ..Default::default()
    };
    if let Some(id) = db.query(&q) {
        if let Some(face) = db.face(id) {
            let fam = face.families.first().map(|(n, _)| n.clone()).unwrap_or_else(|| f.to_string());
            if let Some(data) = db.with_face_data(id, |d, _| d.to_vec()) {
                let ext = match &face.source {
                    fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => {
                        p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "ttf".into())
                    }
                    _ => "ttf".into(),
                };
                let name = format!("font.{ext}");
                let family = family_of(&data).unwrap_or(fam);
                std::fs::write(dir.join(&name), data)?;
                return Ok(FontPick { file: name, family });
            }
        }
    }
    // Unknown family: fall back to the bundled sans.
    write(if bold { LM_SANS_BOLD } else { LM_SANS }, "otf", dir)
}

fn write(data: &[u8], ext: &str, dir: &Path) -> anyhow::Result<FontPick> {
    let name = format!("font.{ext}");
    std::fs::write(dir.join(&name), data)?;
    Ok(FontPick { file: name, family: family_of(data).unwrap_or_else(|| DEFAULT_FAMILY.to_string()) })
}

/// Bundled Latin Modern faces for the PDF transcript: (regular, bold, italic-less).
pub fn lm_roman() -> (&'static [u8], &'static [u8]) {
    (LM_ROMAN, LM_ROMAN_BOLD)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_family_names() {
        let d = tempfile::tempdir().unwrap();
        let f = super::prepare("", true, d.path()).unwrap();
        assert_eq!(f.family, "LM Sans 10");
        let f = super::prepare("Latin Modern Roman", false, d.path()).unwrap();
        assert_eq!(f.family, "LM Roman 10");
    }
}
