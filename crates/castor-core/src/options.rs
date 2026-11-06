//! Declarative option schema.
//!
//! Labels come from the locales, by key:
//! - label:        `{tool_id}.opt.{key}`
//! - help (opt.):  `{tool_id}.opt.{key}.help`
//! - choice label: `{tool_id}.opt.{key}.{choice_value}` (choice values are stable ids)
//! Shared option labels (quality, format, …) may use `common.opt.{key}` instead:
//! set [`OptionSpec::common`] and the UI looks there.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A choice in a [`OptionKind::Choice`] list.
#[derive(Clone, Debug)]
pub struct Choice {
    /// Stable value stored in [`Values`] and chains, e.g. "max".
    pub value: &'static str,
    /// Optional untranslated label (formats like "WebP", "1080p", "A4"). If None, the label is
    /// looked up at `{tool}.opt.{key}.{value}`.
    pub literal: Option<&'static str>,
}

impl Choice {
    pub const fn key(value: &'static str) -> Self {
        Choice { value, literal: None }
    }
    pub const fn lit(value: &'static str, label: &'static str) -> Self {
        Choice { value, literal: Some(label) }
    }
}

/// 9-point anchor for positions (watermarks, page numbers, stamps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];
    /// (x, y) in 0..=2 grid coordinates.
    pub fn grid(self) -> (u8, u8) {
        let i = Anchor::ALL.iter().position(|a| *a == self).unwrap_or(4) as u8;
        (i % 3, i / 3)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top-left",
            Anchor::Top => "top",
            Anchor::TopRight => "top-right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom-left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom-right",
        }
    }
    pub fn parse(s: &str) -> Option<Anchor> {
        Anchor::ALL.iter().copied().find(|a| a.as_str() == s)
    }
}

/// What the user edits.
#[derive(Clone, Debug)]
pub enum OptionKind {
    /// Segmented control if ≤ 4 choices, dropdown otherwise.
    Choice(Vec<Choice>),
    Toggle,
    /// Integer with bounds. `slider: true` renders a slider + number.
    Int { min: i64, max: i64, step: i64, unit: Option<&'static str>, slider: bool },
    Float { min: f64, max: f64, step: f64, unit: Option<&'static str>, slider: bool },
    Text { multiline: bool, placeholder_key: Option<&'static str> },
    /// Masked text, never stored in chains or history.
    Password,
    /// RGBA colour, stored as "#rrggbbaa".
    Color,
    /// A file or folder picker. `filters` = extensions without dot.
    Path { folder: bool, filters: &'static [&'static str] },
    /// Page ranges like "1-3, 5, 8-" (1-based). Empty = all pages.
    PageRange,
    /// 9-point position picker.
    Anchor,
    /// Width × height in pixels.
    Size,
    /// A font family name from the system/library.
    Font,
    /// Language code(s) for OCR / transcription, from the tool's own list of
    /// (code, endonym) pairs, e.g. ("fra", "Français"). `multi` allows several, stored "fra+eng".
    Language { multi: bool, list: &'static [(&'static str, &'static str)] },
}

/// Show an option only when another option has a given value.
#[derive(Clone, Debug)]
pub enum Cond {
    Eq(&'static str, &'static str),
    Ne(&'static str, &'static str),
    True(&'static str),
    False(&'static str),
}

impl Cond {
    pub fn eval(&self, v: &Values) -> bool {
        match self {
            Cond::Eq(k, s) => v.get(k).map(|x| x.as_str() == Some(*s)).unwrap_or(false),
            Cond::Ne(k, s) => v.get(k).map(|x| x.as_str() != Some(*s)).unwrap_or(true),
            Cond::True(k) => v.bool(k),
            Cond::False(k) => !v.bool(k),
        }
    }
}

/// One option of a tool.
#[derive(Clone, Debug)]
pub struct OptionSpec {
    pub key: &'static str,
    pub kind: OptionKind,
    pub default: Value,
    pub visible_if: Option<Cond>,
    /// Hidden under "More options".
    pub advanced: bool,
    /// Label lives under `common.opt.{key}` (shared wording).
    pub common: bool,
}

impl OptionSpec {
    fn new(key: &'static str, kind: OptionKind, default: Value) -> Self {
        OptionSpec { key, kind, default, visible_if: None, advanced: false, common: false }
    }
    pub fn choice(key: &'static str, choices: Vec<Choice>, default: &'static str) -> Self {
        Self::new(key, OptionKind::Choice(choices), Value::Text(default.into()))
    }
    pub fn toggle(key: &'static str, default: bool) -> Self {
        Self::new(key, OptionKind::Toggle, Value::Bool(default))
    }
    pub fn int(key: &'static str, min: i64, max: i64, default: i64) -> Self {
        Self::new(key, OptionKind::Int { min, max, step: 1, unit: None, slider: false }, Value::Int(default))
    }
    pub fn float(key: &'static str, min: f64, max: f64, default: f64) -> Self {
        Self::new(key, OptionKind::Float { min, max, step: 0.1, unit: None, slider: false }, Value::Float(default))
    }
    pub fn text(key: &'static str, default: &str) -> Self {
        Self::new(key, OptionKind::Text { multiline: false, placeholder_key: None }, Value::Text(default.into()))
    }
    pub fn multiline(key: &'static str, default: &str) -> Self {
        Self::new(key, OptionKind::Text { multiline: true, placeholder_key: None }, Value::Text(default.into()))
    }
    pub fn password(key: &'static str) -> Self {
        Self::new(key, OptionKind::Password, Value::Text(String::new()))
    }
    pub fn color(key: &'static str, default: &str) -> Self {
        Self::new(key, OptionKind::Color, Value::Text(default.into()))
    }
    pub fn file(key: &'static str, filters: &'static [&'static str]) -> Self {
        Self::new(key, OptionKind::Path { folder: false, filters }, Value::Path(None))
    }
    pub fn folder(key: &'static str) -> Self {
        Self::new(key, OptionKind::Path { folder: true, filters: &[] }, Value::Path(None))
    }
    pub fn pages(key: &'static str) -> Self {
        Self::new(key, OptionKind::PageRange, Value::Text(String::new()))
    }
    pub fn anchor(key: &'static str, default: Anchor) -> Self {
        Self::new(key, OptionKind::Anchor, Value::Text(default.as_str().into()))
    }
    pub fn size(key: &'static str, w: u32, h: u32) -> Self {
        Self::new(key, OptionKind::Size, Value::Size(w, h))
    }
    pub fn font(key: &'static str, default: &str) -> Self {
        Self::new(key, OptionKind::Font, Value::Text(default.into()))
    }
    pub fn language(key: &'static str, list: &'static [(&'static str, &'static str)], multi: bool, default: &str) -> Self {
        Self::new(key, OptionKind::Language { multi, list }, Value::Text(default.into()))
    }

    // Builder tweaks.
    pub fn slider(mut self) -> Self {
        match &mut self.kind {
            OptionKind::Int { slider, .. } | OptionKind::Float { slider, .. } => *slider = true,
            _ => {}
        }
        self
    }
    pub fn step(mut self, s: f64) -> Self {
        match &mut self.kind {
            OptionKind::Int { step, .. } => *step = s.max(1.0) as i64,
            OptionKind::Float { step, .. } => *step = s,
            _ => {}
        }
        self
    }
    pub fn unit(mut self, u: &'static str) -> Self {
        match &mut self.kind {
            OptionKind::Int { unit, .. } | OptionKind::Float { unit, .. } => *unit = Some(u),
            _ => {}
        }
        self
    }
    pub fn placeholder(mut self, key: &'static str) -> Self {
        if let OptionKind::Text { placeholder_key, .. } = &mut self.kind {
            *placeholder_key = Some(key);
        }
        self
    }
    pub fn when(mut self, c: Cond) -> Self {
        self.visible_if = Some(c);
        self
    }
    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
    pub fn common(mut self) -> Self {
        self.common = true;
        self
    }

    /// i18n key of the label for a tool.
    pub fn label_key(&self, tool_id: &str) -> String {
        if self.common { format!("common.opt.{}", self.key) } else { format!("{tool_id}.opt.{}", self.key) }
    }
    /// Localised label of a choice.
    pub fn choice_label(&self, tool_id: &str, c: &Choice) -> String {
        if let Some(l) = c.literal {
            return l.to_string();
        }
        crate::tr(&format!("{}.{}", self.label_key(tool_id), c.value))
    }

    /// Parse a CLI/chain string into a value of this option's type.
    pub fn parse(&self, s: &str) -> crate::Result<Value> {
        Ok(match &self.kind {
            OptionKind::Toggle => Value::Bool(matches!(s, "1" | "true" | "yes" | "on")),
            OptionKind::Int { min, max, .. } => {
                let n: i64 = s.trim().parse()?;
                Value::Int(n.clamp(*min, *max))
            }
            OptionKind::Float { min, max, .. } => {
                let n: f64 = s.trim().parse()?;
                Value::Float(n.clamp(*min, *max))
            }
            OptionKind::Choice(cs) => {
                anyhow::ensure!(
                    cs.iter().any(|c| c.value == s),
                    "'{s}' is not one of {}",
                    cs.iter().map(|c| c.value).collect::<Vec<_>>().join(", ")
                );
                Value::Text(s.into())
            }
            OptionKind::Path { .. } => Value::Path(if s.is_empty() { None } else { Some(PathBuf::from(s)) }),
            OptionKind::Size => {
                let (w, h) = s.split_once(['x', 'X', '×']).ok_or_else(|| anyhow::anyhow!("expected WxH"))?;
                Value::Size(w.trim().parse()?, h.trim().parse()?)
            }
            OptionKind::Anchor => {
                Anchor::parse(s).ok_or_else(|| anyhow::anyhow!("unknown position '{s}'"))?;
                Value::Text(s.into())
            }
            _ => Value::Text(s.into()),
        })
    }
}

/// An option value. Serialised in chains and history (passwords excluded).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Size(u32, u32),
    // Text before Path: with `untagged`, a string deserialises as the first variant that
    // accepts it. Getters below treat Text and Path interchangeably.
    Text(String),
    Path(Option<PathBuf>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// The values of a tool's options, keyed by option key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Values(pub BTreeMap<String, Value>);

impl Values {
    /// Defaults for a schema.
    pub fn defaults(specs: &[OptionSpec]) -> Self {
        Values(specs.iter().map(|s| (s.key.to_string(), s.default.clone())).collect())
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k)
    }
    pub fn set(&mut self, k: &str, v: Value) {
        self.0.insert(k.to_string(), v);
    }
    pub fn bool(&self, k: &str) -> bool {
        matches!(self.0.get(k), Some(Value::Bool(true)))
    }
    pub fn int(&self, k: &str) -> i64 {
        match self.0.get(k) {
            Some(Value::Int(n)) => *n,
            Some(Value::Float(f)) => *f as i64,
            Some(Value::Text(s)) => s.parse().unwrap_or(0),
            _ => 0,
        }
    }
    pub fn float(&self, k: &str) -> f64 {
        match self.0.get(k) {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(n)) => *n as f64,
            Some(Value::Text(s)) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }
    pub fn str(&self, k: &str) -> &str {
        match self.0.get(k) {
            Some(Value::Text(s)) => s,
            _ => "",
        }
    }
    pub fn path(&self, k: &str) -> Option<&std::path::Path> {
        match self.0.get(k) {
            Some(Value::Path(Some(p))) => Some(p),
            Some(Value::Text(s)) if !s.is_empty() => Some(std::path::Path::new(s)),
            _ => None,
        }
    }
    pub fn size(&self, k: &str) -> (u32, u32) {
        match self.0.get(k) {
            Some(Value::Size(w, h)) => (*w, *h),
            _ => (0, 0),
        }
    }
    pub fn anchor(&self, k: &str) -> Anchor {
        Anchor::parse(self.str(k)).unwrap_or(Anchor::Center)
    }
    /// "#rrggbb" or "#rrggbbaa" → [r, g, b, a].
    pub fn color(&self, k: &str) -> [u8; 4] {
        parse_hex_color(self.str(k)).unwrap_or([0, 0, 0, 255])
    }
    /// Copy without secrets, for chains and history.
    pub fn without_secrets(&self, specs: &[OptionSpec]) -> Values {
        let mut out = self.clone();
        for s in specs {
            if matches!(s.kind, OptionKind::Password) {
                out.0.remove(s.key);
            }
        }
        out
    }
}

pub fn parse_hex_color(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([p(0)?, p(2)?, p(4)?, 255]),
        8 => Some([p(0)?, p(2)?, p(4)?, p(6)?]),
        3 => {
            let q = |i: usize| u8::from_str_radix(&h[i..i + 1].repeat(2), 16).ok();
            Some([q(0)?, q(1)?, q(2)?, 255])
        }
        _ => None,
    }
}

/// Parse page ranges "1-3, 5, 8-" (1-based, inclusive) into 0-based page indices < `count`.
/// Empty string = all pages. Supports "last", "odd", "even".
pub fn parse_page_range(s: &str, count: usize) -> crate::Result<Vec<usize>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok((0..count).collect());
    }
    let mut out = Vec::new();
    for part in s.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "odd" => out.extend((0..count).step_by(2)),
            "even" => out.extend((1..count).step_by(2)),
            _ => {
                let num = |t: &str| -> crate::Result<usize> {
                    let t = t.trim();
                    if t == "last" || t.is_empty() {
                        return Ok(count);
                    }
                    Ok(t.parse::<usize>()?)
                };
                let (a, b) = match part.split_once('-') {
                    Some((a, b)) => (if a.trim().is_empty() { 1 } else { num(a)? }, num(b)?),
                    None => {
                        let n = num(part)?;
                        (n, n)
                    }
                };
                anyhow::ensure!(a >= 1 && a <= b, "bad page range '{part}'");
                out.extend((a - 1)..b.min(count));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges() {
        assert_eq!(parse_page_range("", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_page_range("1-2, 5", 6).unwrap(), vec![0, 1, 4]);
        assert_eq!(parse_page_range("4-", 5).unwrap(), vec![3, 4]);
        assert_eq!(parse_page_range("odd", 5).unwrap(), vec![0, 2, 4]);
    }
    #[test]
    fn colors() {
        assert_eq!(parse_hex_color("#ff0080"), Some([255, 0, 128, 255]));
        assert_eq!(parse_hex_color("fff"), Some([255, 255, 255, 255]));
    }
}
