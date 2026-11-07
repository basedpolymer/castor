//! Human formatting of sizes and durations (locale-neutral, short).

/// 1536 → "1.5 KB", 19_000_000 → "19 MB". Decimal units (like Finder and Explorer's "MB" in practice).
/// Follows the UI language: French writes "1,5 ko", "19 Mo"; other languages use a decimal comma
/// where that is the norm (de, es, it, pt-BR) and keep KB/MB.
pub fn bytes(n: u64) -> String {
    let fr = crate::i18n::current_language().starts_with("fr");
    local_decimal(bytes_en(n, fr))
}

/// Languages that write a decimal comma: French, German, Spanish, Italian, Portuguese.
fn decimal_comma() -> bool {
    let lang = crate::i18n::current_language();
    ["fr", "de", "es", "it", "pt"].iter().any(|l| lang.starts_with(l))
}

/// "2.1 s" → "2,1 s" in the languages that write a decimal comma.
fn local_decimal(s: String) -> String {
    if decimal_comma() { s.replace('.', ",") } else { s }
}

fn bytes_en(n: u64, fr: bool) -> String {
    const EN: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    const FR: [&str; 5] = ["o", "ko", "Mo", "Go", "To"];
    let units = if fr { FR } else { EN };
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", units[0])
    } else if v < 10.0 {
        format!("{v:.1} {}", units[i])
    } else {
        format!("{v:.0} {}", units[i])
    }
}

/// 3.2 → "3.2 s" ("3,2 s" in French), 75 → "1 min 15 s", 3700 → "1 h 1 min".
pub fn duration(secs: f64) -> String {
    if secs < 10.0 {
        local_decimal(format!("{secs:.1} s"))
    } else if secs < 60.0 {
        format!("{secs:.0} s")
    } else if secs < 3600.0 {
        format!("{} min {} s", (secs / 60.0) as u64, (secs % 60.0) as u64)
    } else {
        format!("{} h {} min", (secs / 3600.0) as u64, ((secs % 3600.0) / 60.0) as u64)
    }
}

/// Timestamp "00:01:02.500" from seconds (for subtitles, trims).
pub fn timecode(secs: f64) -> String {
    let ms = (secs * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

/// Percentage change "−82 %" / "+5 %" between two sizes. A file that is not empty never shows
/// "−100 %": 813 KB → 105 B is "−99,9 %" (decimal comma where the language uses one).
pub fn change(before: u64, after: u64) -> String {
    if before == 0 {
        return String::new();
    }
    let p = (after as f64 - before as f64) / before as f64 * 100.0;
    if p < 0.0 {
        let down = -p;
        if after > 0 && down.round() >= 100.0 {
            // Truncate, never round up to a misleading 100 %.
            return local_decimal(format!("−{:.1} %", (down * 10.0).floor().min(999.0) / 10.0));
        }
        format!("−{down:.0} %")
    } else {
        format!("+{p:.0} %")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_language() {
        let _lock = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("en");
        assert_eq!(change(1000, 180), "−82 %");
        assert_eq!(change(813_000, 105), "−99.9 %", "a file that is not empty never shows −100 %");
        assert_eq!(change(1000, 0), "−100 %");
        assert_eq!(change(100, 105), "+5 %");
        assert_eq!(duration(2.14), "2.1 s");
        assert_eq!(bytes(1500), "1.5 KB");
        crate::i18n::set_language("fr");
        assert_eq!(duration(2.14), "2,1 s");
        assert_eq!(bytes(1500), "1,5 ko");
        assert_eq!(change(813_000, 105), "−99,9 %");
        crate::i18n::set_language("en");
    }
}
