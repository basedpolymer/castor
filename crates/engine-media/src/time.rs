//! Timecodes typed by people: "90", "1:30", "01:02:03.5", "1m30s", "2h", "1:30,25".

/// Parse a time typed by a person into seconds. Empty → None. Errors are user-facing.
pub fn parse_time(s: &str) -> anyhow::Result<Option<f64>> {
    let t = s.trim().to_ascii_lowercase().replace(',', ".");
    if t.is_empty() {
        return Ok(None);
    }
    let bad = || anyhow::anyhow!(castor_core::tr_args("media.err.bad_time", &[("value", s.trim())]));
    let v = if t.contains(':') {
        let parts: Vec<&str> = t.split(':').collect();
        if parts.len() > 3 {
            return Err(bad());
        }
        let mut secs = 0.0;
        for (i, p) in parts.iter().enumerate() {
            let x: f64 = p.trim().parse().map_err(|_| bad())?;
            if x < 0.0 || (i > 0 && x >= 60.0) {
                return Err(bad());
            }
            secs = secs * 60.0 + x;
        }
        secs
    } else if t.chars().any(|c| matches!(c, 'h' | 'm' | 's')) {
        // 1h2m3.5s / 90s / 2m / 1h
        let mut secs = 0.0;
        let mut num = String::new();
        let mut any = false;
        let mut chars = t.chars().peekable();
        while let Some(c) = chars.next() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
            } else if c.is_whitespace() {
                continue;
            } else {
                let mul = match c {
                    'h' => 3600.0,
                    'm' if chars.peek() == Some(&'s') => {
                        chars.next();
                        0.001
                    }
                    'm' => 60.0,
                    's' => 1.0,
                    _ => return Err(bad()),
                };
                let x: f64 = num.parse().map_err(|_| bad())?;
                secs += x * mul;
                num.clear();
                any = true;
            }
        }
        if !num.is_empty() {
            secs += num.parse::<f64>().map_err(|_| bad())?;
            any = true;
        }
        if !any {
            return Err(bad());
        }
        secs
    } else {
        t.parse::<f64>().map_err(|_| bad())?
    };
    if !v.is_finite() || v < 0.0 {
        return Err(bad());
    }
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::parse_time;
    #[test]
    fn times() {
        let p = |s| parse_time(s).unwrap();
        assert_eq!(p(""), None);
        assert_eq!(p("90"), Some(90.0));
        assert_eq!(p("1:30"), Some(90.0));
        assert_eq!(p("01:02:03.5"), Some(3723.5));
        assert_eq!(p("1:02:03,5"), Some(3723.5));
        assert_eq!(p("1m30s"), Some(90.0));
        assert_eq!(p("2h"), Some(7200.0));
        assert_eq!(p("500ms"), Some(0.5));
        assert!(parse_time("1:75").is_err());
        assert!(parse_time("abc").is_err());
        assert!(parse_time("-3").is_err());
    }
}
