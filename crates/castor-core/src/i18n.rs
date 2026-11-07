//! Translations. Strings live in `/locales/*.yml` (rust-i18n "version 2" files: one key, all
//! languages). English is the reference and the fallback.
//!
//! Files are split by owner so that parallel work never edits the same file:
//! `app.yml` (shell, settings, common), `pdf.yml`, `image.yml`, `media.yml` (video + audio),
//! `create.yml`, `files.yml`.
//!
//! Placeholders use `%{name}`: `tr_args("pdf.compress.done", &[("saved", "18 MB")])`.

/// Languages shipped. Codes follow BCP 47.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt-BR", "Português (Brasil)"),
    ("ja", "日本語"),
    ("zh-CN", "简体中文"),
    ("ko", "한국어"),
];

/// Translate a key in the current language (falls back to English, then to the key itself).
pub fn tr(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

/// Translate with `%{name}` placeholders.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = tr(key);
    for (k, v) in args {
        s = s.replace(&format!("%{{{k}}}"), v);
    }
    s
}

/// "1 file" / "3 files" in the UI language (never "1 files").
pub fn n_files(n: usize) -> String {
    if n == 1 { tr("app.tool.one_file") } else { tr_args("common.n_files", &[("count", &n.to_string())]) }
}

/// True if the key exists in the current language or in English.
pub fn has(key: &str) -> bool {
    tr(key) != key
}

/// Tests that switch the (global) UI language hold this lock, so they never run interleaved.
#[cfg(test)]
pub(crate) static TEST_LANGUAGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Set the UI language. "" = system language (if shipped), else English.
pub fn set_language(code: &str) {
    let code = if code.is_empty() { system_language() } else { code.to_string() };
    rust_i18n::set_locale(&code);
}

pub fn current_language() -> String {
    rust_i18n::locale().to_string()
}

/// Best shipped match for the OS language: "fr-FR" → "fr", "pt-BR" → "pt-BR", "zh-Hans-CN" → "zh-CN".
pub fn system_language() -> String {
    let sys = sys_locale::get_locale().unwrap_or_else(|| "en".into());
    let lower = sys.to_ascii_lowercase().replace('_', "-");
    if lower.starts_with("zh") {
        return "zh-CN".into();
    }
    if lower.starts_with("pt") {
        return "pt-BR".into();
    }
    let primary = lower.split('-').next().unwrap_or("en");
    LANGUAGES.iter().find(|(c, _)| c.eq_ignore_ascii_case(primary)).map(|(c, _)| c.to_string()).unwrap_or_else(|| "en".into())
}
