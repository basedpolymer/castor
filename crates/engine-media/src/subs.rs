//! Subtitle cues: SRT / VTT / ASS reading, SRT / VTT writing, and caption layout from a
//! transcript (line length, reading speed, word timings kept for karaoke styles).

use crate::whisper::{Transcript, Word};

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    /// Text, lines separated by '\n'.
    pub text: String,
    /// Word timings when known (from transcription), for karaoke styles.
    pub words: Vec<Word>,
}

impl Cue {
    pub fn new(start: f64, end: f64, text: impl Into<String>) -> Cue {
        Cue { start, end, text: text.into(), words: Vec::new() }
    }
}

fn ts(secs: f64, sep: char) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

/// SubRip text.
pub fn to_srt(cues: &[Cue]) -> String {
    let mut s = String::new();
    for (i, c) in cues.iter().enumerate() {
        s.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, ts(c.start, ','), ts(c.end, ','), c.text.trim()));
    }
    s
}

/// WebVTT text.
pub fn to_vtt(cues: &[Cue]) -> String {
    let mut s = String::from("WEBVTT\n\n");
    for c in cues {
        s.push_str(&format!("{} --> {}\n{}\n\n", ts(c.start, '.'), ts(c.end, '.'), c.text.trim()));
    }
    s
}

/// "00:01:02,500" / "00:01:02.500" / "01:02.500" / "0:01:02.50" (ASS) → seconds.
pub fn parse_ts(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Parse SRT or WebVTT (detected), or ASS/SSA `Dialogue:` lines. Tags are stripped.
pub fn parse(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    if text.contains("[Events]") || text.lines().any(|l| l.starts_with("Dialogue:")) {
        return parse_ass(&text);
    }
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let (a, b) = lines[ti].split_once("-->").unwrap();
        // VTT cue settings follow the end time ("00:00:02.000 align:start").
        let b = b.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_ts(a), parse_ts(b)) else { continue };
        let body = lines[ti + 1..].iter().map(|l| strip_tags(l)).collect::<Vec<_>>().join("\n");
        if !body.trim().is_empty() {
            cues.push(Cue::new(start, end, body.trim().to_string()));
        }
    }
    cues
}

fn parse_ass(text: &str) -> Vec<Cue> {
    let mut fields: Vec<String> = vec![];
    let mut cues = Vec::new();
    for line in text.lines() {
        if let Some(f) = line.strip_prefix("Format:") {
            fields = f.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
        } else if let Some(d) = line.strip_prefix("Dialogue:") {
            if fields.is_empty() {
                fields = ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
            }
            let parts: Vec<&str> = d.splitn(fields.len(), ',').collect();
            let get = |k: &str| fields.iter().position(|f| f == k).and_then(|i| parts.get(i)).map(|s| s.trim());
            let (Some(a), Some(b), Some(t)) = (get("start"), get("end"), get("text")) else { continue };
            let (Some(start), Some(end)) = (parse_ts(a), parse_ts(b)) else { continue };
            let mut body = String::new();
            let mut depth = 0;
            for c in t.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ if depth == 0 => body.push(c),
                    _ => {}
                }
            }
            let body = body.replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
            if !body.trim().is_empty() {
                cues.push(Cue::new(start, end, body.trim().to_string()));
            }
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

fn strip_tags(l: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in l.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ")
}

/// Layout rules for captions.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Max characters per line.
    pub max_chars: usize,
    /// Max lines per cue.
    pub max_lines: usize,
    /// Max cue duration, seconds.
    pub max_secs: f64,
    /// Pause that always starts a new cue, seconds.
    pub gap: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { max_chars: 42, max_lines: 2, max_secs: 6.0, gap: 0.7 }
    }
}

/// Cues from a transcript. With word timings, cues are built word by word (respecting line
/// length, pauses and sentence ends); without, one cue per segment, wrapped.
pub fn cues_from_transcript(t: &Transcript, layout: Layout) -> Vec<Cue> {
    let has_words = t.segments.iter().all(|s| !s.words.is_empty()) && !t.segments.is_empty();
    if !has_words {
        return t
            .segments
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .map(|s| Cue::new(s.start, s.end, wrap(s.text.trim(), layout.max_chars, usize::MAX)))
            .collect();
    }
    let budget = layout.max_chars * layout.max_lines;
    let mut cues = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let flush = |cur: &mut Vec<Word>, cues: &mut Vec<Cue>| {
        if cur.is_empty() {
            return;
        }
        let text = cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        cues.push(Cue {
            start: cur[0].start,
            end: cur.last().unwrap().end,
            text: wrap(&text, layout.max_chars, layout.max_lines),
            words: std::mem::take(cur),
        });
    };
    for w in t.words() {
        if let Some(last) = cur.last() {
            let len: usize = cur.iter().map(|w| w.text.chars().count() + 1).sum::<usize>() + w.text.chars().count();
            let sentence_end = last.text.ends_with(['.', '!', '?', '…', '。', '？', '！']);
            let long = w.end - cur[0].start > layout.max_secs;
            let pause = w.start - last.end > layout.gap;
            let comma_break = last.text.ends_with([',', ';', ':']) && len > budget * 2 / 3;
            if len > budget || long || pause || comma_break || (sentence_end && len > budget / 3) {
                flush(&mut cur, &mut cues);
            }
        }
        cur.push(w.clone());
    }
    flush(&mut cur, &mut cues);
    // Readability: keep each cue on screen at least ~0.8 s when the next one allows it.
    for i in 0..cues.len() {
        let next = cues.get(i + 1).map(|c| c.start).unwrap_or(f64::MAX);
        let c = &mut cues[i];
        if c.end - c.start < 0.8 {
            c.end = (c.start + 0.8).min(next);
        }
    }
    cues
}

/// Wrap `text` in lines of ≤ `max` characters, balanced, at most `max_lines` lines.
pub fn wrap(text: &str, max: usize, max_lines: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let total: usize = words.iter().map(|w| w.chars().count()).sum::<usize>() + words.len().saturating_sub(1);
    if total <= max {
        return words.join(" ");
    }
    // Balanced two-line split when it fits in two lines.
    if max_lines >= 2 && total <= max * 2 + 1 {
        let mut best = (usize::MAX, 1);
        let mut left = 0;
        for i in 1..words.len() {
            left += words[i - 1].chars().count() + usize::from(i > 1);
            let right = total - left - 1;
            if left <= max && right <= max {
                let d = left.abs_diff(right);
                if d < best.0 {
                    best = (d, i);
                }
            }
        }
        if best.0 != usize::MAX {
            return format!("{}\n{}", words[..best.1].join(" "), words[best.1..].join(" "));
        }
    }
    let mut lines: Vec<String> = vec![String::new()];
    for w in words {
        let l = lines.last_mut().unwrap();
        if !l.is_empty() && l.chars().count() + 1 + w.chars().count() > max {
            lines.push(w.to_string());
        } else {
            if !l.is_empty() {
                l.push(' ');
            }
            l.push_str(w);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whisper::Segment;

    #[test]
    fn srt_roundtrip() {
        let cues = vec![Cue::new(1.0, 2.5, "Hello"), Cue::new(3.0, 4.25, "two\nlines")];
        let s = to_srt(&cues);
        assert!(s.contains("00:00:01,000 --> 00:00:02,500"));
        assert_eq!(parse(&s), cues);
        let v = to_vtt(&cues);
        assert!(v.starts_with("WEBVTT"));
        assert_eq!(parse(&v), cues);
    }
    #[test]
    fn ass_parse() {
        let a = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\b1}Hi,\\Nthere\n";
        assert_eq!(parse(a), vec![Cue::new(1.5, 3.0, "Hi,\nthere")]);
    }
    #[test]
    fn wraps_balanced() {
        assert_eq!(wrap("one two three four five six seven eight nine ten", 30, 2), "one two three four five\nsix seven eight nine ten");
    }
    #[test]
    fn cues_split_on_pause() {
        let w = |s: f64, e: f64, t: &str| Word { start: s, end: e, text: t.into(), p: 1.0 };
        let t = Transcript {
            language: "en".into(),
            duration: 10.0,
            segments: vec![Segment {
                start: 0.0,
                end: 6.0,
                text: "Hello world. Again here".into(),
                words: vec![w(0.0, 0.5, "Hello"), w(0.5, 1.0, "world."), w(4.0, 4.5, "Again"), w(4.5, 5.0, "here")],
            }],
        };
        let c = cues_from_transcript(&t, Layout::default());
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "Hello world.");
        assert_eq!(c[1].words.len(), 2);
    }
}
