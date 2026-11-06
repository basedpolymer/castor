//! Human formatting of sizes and durations (locale-neutral, short).

/// 1536 → "1.5 KB", 19_000_000 → "19 MB". Decimal units (like Finder and Explorer's "MB" in practice).
/// Follows the UI language: French writes "1,5 ko", "19 Mo"; other languages use a decimal comma
/// where that is the norm (de, es, it, pt-BR) and keep KB/MB.
pub fn bytes(n: u64) -> String {
    let lang = crate::i18n::current_language();
    let fr = lang.starts_with("fr");
    let comma = fr || ["de", "es", "it", "pt"].iter().any(|l| lang.starts_with(l));
    let s = bytes_en(n, fr);
    if comma { s.replace('.', ",") } else { s }
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

/// 3.2 → "3.2 s", 75 → "1 min 15 s", 3700 → "1 h 1 min".
pub fn duration(secs: f64) -> String {
    if secs < 10.0 {
        format!("{secs:.1} s")
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

/// Percentage change "−82 %" / "+5 %" between two sizes.
pub fn change(before: u64, after: u64) -> String {
    if before == 0 {
        return String::new();
    }
    let p = (after as f64 - before as f64) / before as f64 * 100.0;
    if p < 0.0 { format!("−{:.0} %", -p) } else { format!("+{p:.0} %") }
}
