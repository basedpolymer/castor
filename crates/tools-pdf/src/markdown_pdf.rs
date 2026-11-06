//! pdf.from-markdown — Markdown → PDF in the house ("TeXtured") style, typeset with Typst:
//! Latin Modern (embedded, extractable text), numbered headings, "Figure n / …" captions,
//! booktabs tables, code in Latin Modern Mono, LaTeX-style math (`$…$`, `$$…$$`) converted to
//! Typst math (see [`latex_to_typst`] for the supported subset), optional title page and table
//! of contents, page numbers, paper and font size.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta};
use pulldown_cmark::{Alignment, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd};
use typst::diag::{FileError, FileResult, Severity};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

use crate::common::*;

pub struct FromMarkdown {
    meta: ToolMeta,
}

impl FromMarkdown {
    pub fn new() -> Self {
        FromMarkdown {
            meta: meta(
                "pdf.from-markdown",
                Family::Pdf,
                InputSpec::kinds(&[FileKind::Markdown]).with_extensions(&["md", "markdown", "txt"]),
                "PDF",
                &["markdown", "md", "notes", "typeset", "latex", "report", "pretty", "typst", "math"],
                "",
            ),
        }
    }
}

// ------------------------------------------------------------------------------------------
// Typst world: bundled fonts, in-memory files, no network, no system fonts.

const MATH_FONT: &[u8] = include_bytes!("../data/NewCMMath-Regular.otf");

fn fonts() -> &'static Vec<Font> {
    static F: OnceLock<Vec<Font>> = OnceLock::new();
    F.get_or_init(|| {
        use engine_pdf::fonts as f;
        [f::LM_ROMAN, f::LM_ROMAN_BOLD, f::LM_ROMAN_ITALIC, f::LM_SANS, f::LM_SANS_BOLD, f::LM_SANS_OBLIQUE, f::LM_MONO, MATH_FONT]
            .iter()
            .flat_map(|d| Font::iter(Bytes::new(*d)))
            .collect()
    })
}

fn library() -> &'static LazyHash<Library> {
    static L: OnceLock<LazyHash<Library>> = OnceLock::new();
    L.get_or_init(|| LazyHash::new(Library::default()))
}

fn book() -> &'static LazyHash<FontBook> {
    static B: OnceLock<LazyHash<FontBook>> = OnceLock::new();
    B.get_or_init(|| LazyHash::new(FontBook::from_fonts(fonts().iter())))
}

struct MdWorld {
    main: FileId,
    source: Source,
    files: HashMap<FileId, Bytes>,
}

fn file_id(path: &str) -> Result<FileId> {
    let vp = VirtualPath::new(path).map_err(|e| anyhow::anyhow!("bad path {path}: {e:?}"))?;
    Ok(FileId::new(RootedPath::new(VirtualRoot::Project, vp)))
}

impl World for MdWorld {
    fn library(&self) -> &LazyHash<Library> {
        library()
    }
    fn book(&self) -> &LazyHash<FontBook> {
        book()
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            return Ok(self.source.clone());
        }
        Err(FileError::NotFound(PathBuf::from(id.vpath().get_without_slash())))
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or_else(|| FileError::NotFound(PathBuf::from(id.vpath().get_without_slash())))
    }
    fn font(&self, index: usize) -> Option<Font> {
        fonts().get(index).cloned()
    }
    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        Datetime::from_ymd(now.year(), now.month() as u8, now.day())
    }
}

/// Compile Typst source with in-memory files into a PDF.
pub fn typst_pdf(source: &str, files: &[(String, Vec<u8>)]) -> Result<(Vec<u8>, usize)> {
    let main = file_id("/main.typ")?;
    let mut map = HashMap::new();
    for (p, d) in files {
        map.insert(file_id(p)?, Bytes::new(d.clone()));
    }
    let world = MdWorld { main, source: Source::new(main, source.to_string()), files: map };
    let warned = typst::compile::<PagedDocument>(&world);
    typst::comemo::evict(10);
    let doc = warned.output.map_err(|errs| {
        let msgs: Vec<String> = errs
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| {
                let line = d
                    .span
                    .id()
                    .and_then(|id| world.source(id).ok().zip(typst::WorldExt::range(&world, d.span)))
                    .and_then(|(src, r)| src.lines().byte_to_line(r.start))
                    .map(|l| format!("line {}: ", l + 1))
                    .unwrap_or_default();
                format!("{line}{}", d.message)
            })
            .collect();
        anyhow::anyhow!("{}", msgs.join("; "))
    })?;
    let pages = doc.pages().len();
    let opts = typst_pdf::PdfOptions { creator: typst::foundations::Smart::Custom(Some("Castor".into())), ..Default::default() };
    let pdf = typst_pdf::pdf(&doc, &opts)
        .map_err(|e| anyhow::anyhow!("PDF export failed: {}", e.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))?;
    Ok((pdf, pages))
}

// ------------------------------------------------------------------------------------------
// Escaping.

/// Escape text for Typst markup.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '$' | '#' | '@' | '<' | '>' | '[' | ']' | '~' | '=' | '+' | '-' | '/') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// A Typst string literal.
pub fn lit(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            '\t' => o.push_str("    "),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ------------------------------------------------------------------------------------------
// LaTeX math → Typst math.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Cmd(String),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    NewLine,
    Ch(char),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        match c {
            '\\' => {
                i += 1;
                if i < cs.len() && cs[i].is_ascii_alphabetic() {
                    let st = i;
                    while i < cs.len() && cs[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    out.push(Tok::Cmd(cs[st..i].iter().collect()));
                    continue;
                } else if i < cs.len() {
                    if cs[i] == '\\' {
                        out.push(Tok::NewLine);
                    } else {
                        out.push(Tok::Cmd(cs[i].to_string()));
                    }
                }
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            c => out.push(Tok::Ch(c)),
        }
        i += 1;
    }
    out
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "alpha",
        "beta" => "beta",
        "gamma" => "gamma",
        "delta" => "delta",
        "epsilon" => "epsilon.alt",
        "varepsilon" => "epsilon",
        "zeta" => "zeta",
        "eta" => "eta",
        "theta" => "theta",
        "vartheta" => "theta.alt",
        "iota" => "iota",
        "kappa" => "kappa",
        "lambda" => "lambda",
        "mu" => "mu",
        "nu" => "nu",
        "xi" => "xi",
        "pi" => "pi",
        "varpi" => "pi.alt",
        "rho" => "rho",
        "varrho" => "rho.alt",
        "sigma" => "sigma",
        "varsigma" => "sigma.alt",
        "tau" => "tau",
        "upsilon" => "upsilon",
        "phi" => "phi.alt",
        "varphi" => "phi",
        "chi" => "chi",
        "psi" => "psi",
        "omega" => "omega",
        "Gamma" => "Gamma",
        "Delta" => "Delta",
        "Theta" => "Theta",
        "Lambda" => "Lambda",
        "Xi" => "Xi",
        "Pi" => "Pi",
        "Sigma" => "Sigma",
        "Upsilon" => "Upsilon",
        "Phi" => "Phi",
        "Psi" => "Psi",
        "Omega" => "Omega",
        "sum" => "sum",
        "prod" => "product",
        "coprod" => "product.co",
        "int" => "integral",
        "iint" => "integral.double",
        "iiint" => "integral.triple",
        "oint" => "integral.cont",
        "infty" => "infinity",
        "partial" => "diff",
        "nabla" => "nabla",
        "cdot" => "dot.op",
        "times" => "times",
        "div" => "div",
        "pm" => "plus.minus",
        "mp" => "minus.plus",
        "leq" | "le" => "lt.eq",
        "geq" | "ge" => "gt.eq",
        "neq" | "ne" => "eq.not",
        "ll" => "lt.double",
        "gg" => "gt.double",
        "approx" => "approx",
        "equiv" => "equiv",
        "sim" => "tilde.op",
        "simeq" => "tilde.eq",
        "cong" => "tilde.equiv",
        "propto" => "prop",
        "to" | "rightarrow" => "arrow.r",
        "leftarrow" | "gets" => "arrow.l",
        "Rightarrow" | "implies" => "arrow.r.double",
        "Leftarrow" => "arrow.l.double",
        "Leftrightarrow" | "iff" => "arrow.l.r.double",
        "leftrightarrow" => "arrow.l.r",
        "mapsto" => "arrow.r.bar",
        "longrightarrow" => "arrow.r.long",
        "uparrow" => "arrow.t",
        "downarrow" => "arrow.b",
        "in" => "in",
        "notin" => "in.not",
        "ni" => "in.rev",
        "subset" => "subset",
        "subseteq" => "subset.eq",
        "supset" => "supset",
        "supseteq" => "supset.eq",
        "cup" => "union",
        "cap" => "sect",
        "bigcup" => "union.big",
        "bigcap" => "sect.big",
        "setminus" => "without",
        "forall" => "forall",
        "exists" => "exists",
        "nexists" => "exists.not",
        "neg" | "lnot" => "not",
        "land" | "wedge" => "and",
        "lor" | "vee" => "or",
        "emptyset" | "varnothing" => "emptyset",
        "ldots" | "dots" => "dots.h",
        "cdots" => "dots.c",
        "vdots" => "dots.v",
        "ddots" => "dots.down",
        "langle" => "angle.l",
        "rangle" => "angle.r",
        "lfloor" => "floor.l",
        "rfloor" => "floor.r",
        "lceil" => "ceil.l",
        "rceil" => "ceil.r",
        "circ" => "compose",
        "bullet" => "bullet",
        "star" => "star.op",
        "ast" => "ast.op",
        "oplus" => "plus.circle",
        "otimes" => "times.circle",
        "perp" => "perp",
        "parallel" => "parallel",
        "mid" => "divides",
        "angle" => "angle",
        "hbar" => "planck.reduce",
        "ell" => "ell",
        "Re" => "Re",
        "Im" => "Im",
        "aleph" => "aleph",
        "prime" => "prime",
        "degree" => "degree",
        "dagger" => "dagger",
        "top" => "top",
        "bot" => "bot",
        "quad" => "quad",
        "qquad" => "wide",
        "," => "thin",
        ":" | ">" => "med",
        ";" => "thick",
        " " => "space",
        "|" => "bar.v.double",
        "{" => "\\{",
        "}" => "\\}",
        "%" => "%",
        "$" => "\\$",
        "#" => "\\#",
        "_" => "\\_",
        "&" => "\\&",
        "!" => "",
        _ => return None,
    })
}

/// Functions named like LaTeX operators that Typst knows as text operators.
const OPS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "log", "ln", "lg", "exp", "lim",
    "liminf", "limsup", "max", "min", "sup", "inf", "det", "dim", "ker", "deg", "gcd", "arg", "Pr", "mod",
];

struct MathConv {
    toks: Vec<Tok>,
    i: usize,
}

impl MathConv {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }
    /// One argument: a `{group}` or a single token.
    fn arg(&mut self) -> String {
        while matches!(self.peek(), Some(Tok::Ch(' '))) {
            self.i += 1;
        }
        match self.peek() {
            Some(Tok::Open) => {
                self.i += 1;
                self.until_close()
            }
            Some(_) => self.atom(),
            None => String::new(),
        }
    }
    /// Raw text of a `{group}` (for \text, \operatorname).
    fn raw_arg(&mut self) -> String {
        while matches!(self.peek(), Some(Tok::Ch(' '))) {
            self.i += 1;
        }
        if self.peek() != Some(&Tok::Open) {
            return match self.next() {
                Some(Tok::Ch(c)) => c.to_string(),
                _ => String::new(),
            };
        }
        self.i += 1;
        let mut depth = 0;
        let mut s = String::new();
        while let Some(t) = self.next() {
            match t {
                Tok::Open => {
                    depth += 1;
                    s.push('{');
                }
                Tok::Close if depth == 0 => break,
                Tok::Close => {
                    depth -= 1;
                    s.push('}');
                }
                Tok::Ch(c) => s.push(c),
                Tok::Cmd(c) => {
                    s.push('\\');
                    s.push_str(&c);
                }
                Tok::Sup => s.push('^'),
                Tok::Sub => s.push('_'),
                Tok::Amp => s.push('&'),
                Tok::NewLine => s.push(' '),
            }
        }
        s
    }
    fn until_close(&mut self) -> String {
        let mut out = String::new();
        while let Some(t) = self.peek() {
            if *t == Tok::Close {
                self.i += 1;
                break;
            }
            out.push_str(&self.atom());
        }
        format!("({})", out.trim())
    }
    /// Read an `\begin{env}…\end{env}` body as rows of cells.
    fn env_rows(&mut self, env: &str) -> Vec<Vec<String>> {
        let mut rows = vec![vec![String::new()]];
        while self.peek().is_some() {
            if let Some(Tok::Cmd(c)) = self.peek() {
                if c == "end" {
                    self.i += 1;
                    let name = self.raw_arg();
                    if name == env {
                        break;
                    }
                    continue;
                }
            }
            match self.peek() {
                Some(Tok::Amp) => {
                    self.i += 1;
                    rows.last_mut().unwrap().push(String::new());
                }
                Some(Tok::NewLine) => {
                    self.i += 1;
                    rows.push(vec![String::new()]);
                }
                _ => {
                    let a = self.atom();
                    rows.last_mut().unwrap().last_mut().unwrap().push_str(&a);
                }
            }
        }
        rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
        rows
    }
    fn atom(&mut self) -> String {
        let Some(t) = self.next() else { return String::new() };
        match t {
            Tok::Open => self.until_close(),
            Tok::Close => String::new(),
            Tok::Sup => format!("^{} ", self.arg()),
            Tok::Sub => format!("_{} ", self.arg()),
            Tok::Amp => "& ".into(),
            Tok::NewLine => "\\ ".into(),
            Tok::Ch(c) if c.is_ascii_alphabetic() => format!("{c} "),
            Tok::Ch(c) if c.is_ascii_digit() || c == '.' => c.to_string(),
            Tok::Ch(c) if c.is_whitespace() => " ".into(),
            Tok::Ch('/') => "\\/ ".into(),
            Tok::Ch('"') => "\\\" ".into(),
            Tok::Ch('#') => "\\# ".into(),
            Tok::Ch('@') => "\\@ ".into(),
            Tok::Ch('$') => "\\$ ".into(),
            Tok::Ch('~') => " ".into(),
            Tok::Ch('\'') => "'".into(),
            Tok::Ch(c) => format!("{c} "),
            Tok::Cmd(name) => self.command(&name),
        }
    }
    fn command(&mut self, name: &str) -> String {
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg();
                let b = self.arg();
                format!("frac({}, {}) ", strip(&a), strip(&b))
            }
            "binom" | "dbinom" | "tbinom" => {
                let a = self.arg();
                let b = self.arg();
                format!("binom({}, {}) ", strip(&a), strip(&b))
            }
            "sqrt" => {
                if matches!(self.peek(), Some(Tok::Ch('['))) {
                    self.i += 1;
                    let mut idx = String::new();
                    while let Some(t) = self.peek() {
                        if *t == Tok::Ch(']') {
                            self.i += 1;
                            break;
                        }
                        idx.push_str(&self.atom());
                    }
                    let a = self.arg();
                    format!("root({}, {}) ", idx.trim(), strip(&a))
                } else {
                    let a = self.arg();
                    format!("sqrt({}) ", strip(&a))
                }
            }
            "mathbb" => format!("bb({}) ", strip(&self.arg())),
            "mathcal" => format!("cal({}) ", strip(&self.arg())),
            "mathfrak" => format!("frak({}) ", strip(&self.arg())),
            "mathbf" | "boldsymbol" | "bm" => format!("bold({}) ", strip(&self.arg())),
            "mathit" => format!("italic({}) ", strip(&self.arg())),
            "mathsf" => format!("sans({}) ", strip(&self.arg())),
            "mathtt" => format!("mono({}) ", strip(&self.arg())),
            "mathrm" => format!("upright({}) ", strip(&self.arg())),
            "text" | "textrm" | "textit" | "textbf" | "mbox" => format!("{} ", lit(&self.raw_arg())),
            "operatorname" => format!("op({}) ", lit(&self.raw_arg())),
            "hat" | "widehat" => format!("hat({}) ", strip(&self.arg())),
            "bar" => format!("macron({}) ", strip(&self.arg())),
            "overline" => format!("overline({}) ", strip(&self.arg())),
            "underline" => format!("underline({}) ", strip(&self.arg())),
            "vec" => format!("arrow({}) ", strip(&self.arg())),
            "tilde" | "widetilde" => format!("tilde({}) ", strip(&self.arg())),
            "dot" => format!("dot({}) ", strip(&self.arg())),
            "ddot" => format!("dot.double({}) ", strip(&self.arg())),
            "overbrace" => format!("overbrace({}) ", strip(&self.arg())),
            "underbrace" => format!("underbrace({}) ", strip(&self.arg())),
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "displaystyle" | "textstyle"
            | "limits" | "nolimits" => {
                // Typst sizes delimiters by itself; `\left.` means "no delimiter".
                if matches!(self.peek(), Some(Tok::Ch('.'))) {
                    self.i += 1;
                }
                String::new()
            }
            "begin" => {
                let env = self.raw_arg();
                let rows = self.env_rows(&env);
                let cells = |sep: &str| rows.iter().map(|r| r.iter().map(|c| c.trim().to_string()).collect::<Vec<_>>().join(sep)).collect::<Vec<_>>();
                match env.trim_end_matches('*') {
                    "cases" => format!("cases({}) ", cells(" & quad ").join(", ")),
                    "matrix" => format!("mat(delim: #none, {}) ", cells(", ").join("; ")),
                    "pmatrix" => format!("mat({}) ", cells(", ").join("; ")),
                    "bmatrix" => format!("mat(delim: \"[\", {}) ", cells(", ").join("; ")),
                    "vmatrix" => format!("mat(delim: \"|\", {}) ", cells(", ").join("; ")),
                    _ => cells(" & ").join(" \\ "),
                }
            }
            n if OPS.contains(&n) => format!("{n} "),
            n => match symbol(n) {
                Some(s) => format!("{s} "),
                None => format!("{} ", lit(&format!("\\{n}"))),
            },
        }
    }
}

/// Remove one level of outer parentheses produced for groups.
fn strip(s: &str) -> String {
    let t = s.trim();
    if t.starts_with('(') && t.ends_with(')') {
        // Only if the parentheses match each other.
        let mut depth = 0;
        for (i, c) in t.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i != t.len() - 1 {
                        return t.to_string();
                    }
                }
                _ => {}
            }
        }
        return t[1..t.len() - 1].to_string();
    }
    t.to_string()
}

/// Convert LaTeX math to Typst math (inside `$…$`).
///
/// Supported: letters, digits, `^`/`_` with groups, `\frac \dfrac \binom \sqrt[n]`, Greek
/// letters (`\epsilon`/`\varepsilon` and `\phi`/`\varphi` keep their LaTeX shapes), big
/// operators (`\sum \prod \int \iint \oint`), `\lim \sin \log …`, relations and arrows
/// (`\leq \neq \approx \in \subseteq \to \Rightarrow …`), dots, `\mathbb \mathcal \mathbf
/// \mathrm \text \operatorname`, accents (`\hat \bar \vec \tilde \dot \overline`),
/// `\left … \right` (auto-sized), spacing (`\, \; \quad`), and the environments `cases`,
/// `matrix`, `pmatrix`, `bmatrix`, `vmatrix`, `aligned`/`align`/`gathered` (`&` and `\\`).
/// Unknown commands are shown as text.
pub fn latex_to_typst(src: &str) -> String {
    let mut c = MathConv { toks: tokenize(src), i: 0 };
    let mut out = String::new();
    while c.peek().is_some() {
        out.push_str(&c.atom());
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------------------------------
// Markdown → Typst markup.

struct Front {
    title: String,
    author: String,
    date: String,
}

fn front_matter(src: &str) -> (Front, &str) {
    let mut f = Front { title: String::new(), author: String::new(), date: String::new() };
    let s = src.trim_start_matches('\u{feff}');
    if let Some(rest) = s.strip_prefix("---\n").or_else(|| s.strip_prefix("---\r\n")) {
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    match k.trim().to_lowercase().as_str() {
                        "title" => f.title = v,
                        "author" | "authors" => f.author = v,
                        "date" => f.date = v,
                        _ => {}
                    }
                }
            }
            let after = &rest[end + 4..];
            let after = after.split_once('\n').map(|x| x.1).unwrap_or("");
            return (f, after);
        }
    }
    (f, s)
}

struct Conv<'a> {
    base: &'a Path,
    files: Vec<(String, Vec<u8>)>,
    footnotes: HashMap<String, String>,
    math_as_text: bool,
    first_h1: Option<String>,
}

/// Image bytes in a format Typst reads (PNG, JPEG, GIF, WebP, SVG); others converted to PNG.
fn image_file(p: &Path) -> Option<(String, Vec<u8>)> {
    let data = std::fs::read(p).ok()?;
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") {
        return Some((ext, data));
    }
    let img = image::load_from_memory(&data).ok()?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(("png".into(), png))
}

fn image_width_pt(ext: &str, data: &[u8]) -> f32 {
    if ext == "svg" {
        return 360.0;
    }
    let w = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .map(|(w, _)| w)
        .unwrap_or(1000);
    // Natural size at 150 dpi, at most the text width.
    (w as f32 * 72.0 / 150.0).clamp(36.0, 1000.0)
}

impl Conv<'_> {
    /// Convert events[i..] until the end tag matching the start at `i - 1` (or the end).
    fn run(&mut self, ev: &[Event], i: &mut usize, stop: Option<&TagEnd>) -> String {
        let mut o = String::new();
        while *i < ev.len() {
            let e = ev[*i].clone();
            *i += 1;
            match e {
                Event::End(ref end) => {
                    if Some(end) == stop {
                        return o;
                    }
                }
                Event::Start(tag) => o.push_str(&self.tag(tag, ev, i)),
                Event::Text(t) => o.push_str(&esc(&t)),
                Event::Code(t) => o.push_str(&format!("#raw({})", lit(&t))),
                Event::InlineMath(m) => o.push_str(&self.math(&m, false)),
                Event::DisplayMath(m) => o.push_str(&self.math(&m, true)),
                // A space, not a newline: a wrapped line starting with "2." must not become a list.
                Event::SoftBreak => o.push(' '),
                Event::HardBreak => o.push_str("#linebreak()\n"),
                Event::Rule => o.push_str("\n#align(center, line(length: 30%, stroke: 0.4pt))\n\n"),
                Event::FootnoteReference(name) => {
                    let body = self.footnotes.get(name.as_ref()).cloned().unwrap_or_default();
                    o.push_str(&format!("#footnote[{body}]"));
                }
                Event::TaskListMarker(done) => {
                    o.push_str(if done { "#box(baseline: 0.1em, $ballot.check$) " } else { "#box(baseline: 0.1em, $ballot$) " });
                }
                Event::Html(h) | Event::InlineHtml(h) => {
                    let t = h.trim().to_ascii_lowercase();
                    if t == "<br>" || t == "<br/>" || t == "<br />" {
                        o.push_str("#linebreak()");
                    }
                }
            }
        }
        o
    }

    fn math(&self, m: &str, display: bool) -> String {
        if self.math_as_text {
            return format!("#raw({})", lit(m));
        }
        let t = latex_to_typst(m);
        if display { format!("\n$ {t} $\n") } else { format!("${t}$") }
    }

    fn tag(&mut self, tag: Tag, ev: &[Event], i: &mut usize) -> String {
        match tag {
            Tag::Paragraph => format!("{}\n\n", self.run(ev, i, Some(&TagEnd::Paragraph))),
            Tag::Heading { level, .. } => {
                let n = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                let body = self.run(ev, i, Some(&TagEnd::Heading(level)));
                if n == 1 && self.first_h1.is_none() {
                    self.first_h1 = Some(body.clone());
                }
                format!("\n{} {}\n\n", "=".repeat(n), body.replace('\n', " "))
            }
            Tag::Emphasis => format!("#emph[{}]", self.run(ev, i, Some(&TagEnd::Emphasis))),
            Tag::Strong => format!("#strong[{}]", self.run(ev, i, Some(&TagEnd::Strong))),
            Tag::Strikethrough => format!("#strike[{}]", self.run(ev, i, Some(&TagEnd::Strikethrough))),
            Tag::Superscript => format!("#super[{}]", self.run(ev, i, Some(&TagEnd::Superscript))),
            Tag::Subscript => format!("#sub[{}]", self.run(ev, i, Some(&TagEnd::Subscript))),
            Tag::BlockQuote(k) => format!("#quote(block: true)[{}]\n\n", self.run(ev, i, Some(&TagEnd::BlockQuote(k)))),
            Tag::CodeBlock(kind) => {
                let mut code = String::new();
                while *i < ev.len() {
                    let e = ev[*i].clone();
                    *i += 1;
                    match e {
                        Event::Text(t) => code.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                // No syntax colours: black ink, as in the rest of the page.
                let _ = kind;
                let lang = String::new();
                format!("#raw(block: true, {lang}{})\n\n", lit(code.trim_end_matches('\n')))
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let mut items = Vec::new();
                while *i < ev.len() {
                    match &ev[*i] {
                        Event::Start(Tag::Item) => {
                            *i += 1;
                            items.push(self.run(ev, i, Some(&TagEnd::Item)));
                        }
                        Event::End(TagEnd::List(_)) => {
                            *i += 1;
                            break;
                        }
                        _ => *i += 1,
                    }
                }
                let body = items.iter().map(|it| format!("[{}]", it.trim())).collect::<Vec<_>>().join(", ");
                match start {
                    Some(n) if ordered => format!("#enum(start: {n}, {body})\n\n"),
                    _ => format!("#list({body})\n\n"),
                }
            }
            Tag::Item => self.run(ev, i, Some(&TagEnd::Item)),
            Tag::Link { dest_url, .. } => {
                let text = self.run(ev, i, Some(&TagEnd::Link));
                if dest_url.starts_with('#') || dest_url.is_empty() {
                    text
                } else {
                    format!("#link({})[{}]", lit(&dest_url), text)
                }
            }
            Tag::Image { dest_url, .. } => {
                // Alt text = the caption.
                let mut alt = String::new();
                while *i < ev.len() {
                    let e = ev[*i].clone();
                    *i += 1;
                    match e {
                        Event::End(TagEnd::Image) => break,
                        Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                        _ => {}
                    }
                }
                let url = dest_url.replace("%20", " ");
                let local = (!url.contains("://")).then(|| self.base.join(&url)).filter(|p| p.is_file());
                match local.and_then(|p| image_file(&p)) {
                    Some((ext, data)) => {
                        let w = image_width_pt(&ext, &data);
                        let vpath = format!("/img{}.{ext}", self.files.len() + 1);
                        self.files.push((vpath.clone(), data));
                        // Natural size, but never wider than the text block.
                        let img = format!("layout(size => image({}, width: calc.min({w:.0}pt, size.width)))", lit(&vpath));
                        if alt.trim().is_empty() {
                            format!("\n#align(center, block(width: 100%, align(center, {img})))\n\n")
                        } else {
                            format!("\n#figure({img}, caption: [{}])\n\n", esc(alt.trim()))
                        }
                    }
                    None => format!("#emph[{}]", esc(if alt.is_empty() { &url } else { &alt })),
                }
            }
            Tag::Table(aligns) => {
                let cols = aligns.len().max(1);
                let al: Vec<&str> = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::Right => "right",
                        Alignment::Center => "center",
                        _ => "left",
                    })
                    .collect();
                let mut head: Vec<String> = Vec::new();
                let mut rows: Vec<String> = Vec::new();
                let mut in_head = false;
                while *i < ev.len() {
                    let e = ev[*i].clone();
                    *i += 1;
                    match e {
                        Event::Start(Tag::TableHead) => in_head = true,
                        Event::End(TagEnd::TableHead) => in_head = false,
                        Event::Start(Tag::TableCell) => {
                            let c = self.run(ev, i, Some(&TagEnd::TableCell));
                            if in_head { head.push(format!("[{}]", c.trim())) } else { rows.push(format!("[{}]", c.trim())) }
                        }
                        Event::End(TagEnd::Table) => break,
                        _ => {}
                    }
                }
                format!(
                    "\n#align(center, table(columns: {cols}, align: ({},), stroke: none, inset: (x: 0.6em, y: 0.35em), table.hline(stroke: 1.2pt), table.header({}), table.hline(stroke: 0.6pt), {} table.hline(stroke: 1.2pt)))\n\n",
                    al.join(", "),
                    head.iter().map(|h| format!("strong{h}")).collect::<Vec<_>>().join(", "),
                    if rows.is_empty() { String::new() } else { format!("{},", rows.join(", ")) }
                )
            }
            Tag::FootnoteDefinition(name) => {
                // Already collected; skip it here.
                let _ = self.run(ev, i, Some(&TagEnd::FootnoteDefinition));
                let _ = name;
                String::new()
            }
            Tag::HtmlBlock => {
                let _ = self.run(ev, i, Some(&TagEnd::HtmlBlock));
                String::new()
            }
            _ => String::new(),
        }
    }
}

/// Markdown → (Typst body, in-memory files, first H1 text).
fn convert(md: &str, base: &Path, math_as_text: bool) -> (String, Vec<(String, Vec<u8>)>, Option<String>) {
    let mut opts = MdOptions::empty();
    for o in [
        MdOptions::ENABLE_TABLES,
        MdOptions::ENABLE_FOOTNOTES,
        MdOptions::ENABLE_STRIKETHROUGH,
        MdOptions::ENABLE_TASKLISTS,
        MdOptions::ENABLE_MATH,
        MdOptions::ENABLE_SMART_PUNCTUATION,
        MdOptions::ENABLE_SUPERSCRIPT,
        MdOptions::ENABLE_SUBSCRIPT,
    ] {
        opts.insert(o);
    }
    // Math is taken verbatim from the source: the parser applies backslash escapes inside it,
    // which would turn LaTeX's `\` (new row) into a single backslash.
    let ev: Vec<Event> = Parser::new_ext(md, opts)
        .into_offset_iter()
        .map(|(e, r)| match e {
            Event::InlineMath(_) | Event::DisplayMath(_) => {
                let raw = md.get(r).unwrap_or("").trim();
                let inner = raw.trim_start_matches('$').trim_end_matches('$').to_string();
                if matches!(e, Event::InlineMath(_)) { Event::InlineMath(inner.into()) } else { Event::DisplayMath(inner.into()) }
            }
            e => e,
        })
        .collect();
    let mut c = Conv { base, files: Vec::new(), footnotes: HashMap::new(), math_as_text, first_h1: None };
    // Footnote bodies first (references may come before definitions).
    let mut k = 0;
    while k < ev.len() {
        if let Event::Start(Tag::FootnoteDefinition(name)) = &ev[k] {
            let name = name.to_string();
            let mut j = k + 1;
            let body = c.run(&ev, &mut j, Some(&TagEnd::FootnoteDefinition));
            c.footnotes.insert(name, body.trim().to_string());
            k = j;
        } else {
            k += 1;
        }
    }
    let mut i = 0;
    let body = c.run(&ev, &mut i, None);
    (body, c.files, c.first_h1)
}

/// The house style preamble.
fn preamble(v: &Values, title: &str, author: &str) -> String {
    let paper = match v.str("paper") {
        "letter" => "us-letter",
        "a5" => "a5",
        _ => "a4",
    };
    let size = v.float("font_size").clamp(8.0, 14.0);
    let lang = {
        let l = v.str("language").trim().to_lowercase();
        if l.len() == 2 && l.chars().all(|c| c.is_ascii_lowercase()) { l } else { "en".into() }
    };
    let numbering = if v.bool("numbered") { "\"1.1\"" } else { "none" };
    let page_numbering = if v.bool("page_numbers") { "\"1\"" } else { "none" };
    let margin = if paper == "a5" { "(x: 1.8cm, top: 2cm, bottom: 2.2cm)" } else { "(x: 2.8cm, top: 2.8cm, bottom: 3cm)" };
    format!(
        r##"#set document(title: {title}{author})
#set page(paper: "{paper}", margin: {margin}, numbering: {page_numbering}, number-align: center + bottom)
#set text(font: "Latin Modern Roman", size: {size}pt, lang: "{lang}", hyphenate: true)
#set par(justify: true, leading: 0.62em, spacing: 0.95em)
#show raw: set text(font: "Latin Modern Mono", size: 1em)
#show raw.where(block: true): it => block(width: 100%, inset: (left: 1.2em, y: 0.4em), it)
#show math.equation: set text(font: "New Computer Modern Math")
#set heading(numbering: {numbering})
#show heading: set text(font: "Latin Modern Sans", weight: "bold")
#show heading: set block(above: 1.5em, below: 0.8em)
#show heading.where(level: 1): set text(size: 1.45em)
#show heading.where(level: 2): set text(size: 1.2em)
#show heading.where(level: 3): set text(size: 1.05em)
#show heading: it => {{
  if it.numbering == none {{ it }} else {{
    block(text(font: "Latin Modern Sans", weight: "bold", [#counter(heading).display(it.numbering)#h(0.9em)#it.body]))
  }}
}}
#set figure(gap: 0.8em)
#show figure.caption: it => context text(size: 0.92em, fill: rgb("#5c5a55"))[#strong[#it.supplement #it.counter.display(it.numbering)] / #it.body]
#show quote.where(block: true): it => pad(x: 2em, emph(it.body))
#set list(marker: ([•], [–]), indent: 0.6em)
#set enum(indent: 0.6em)
#show link: set text(fill: rgb("#2f6db5"))
#show outline.entry.where(level: 1): set text(weight: "bold")
"##,
        title = lit(title),
        author = if author.trim().is_empty() { String::new() } else { format!(", author: {}", lit(author)) },
    )
}

fn title_block(front: &Front, title: &str, page: bool) -> String {
    let mut s = String::new();
    let t = format!("#text(font: \"Latin Modern Roman\", size: {}em, weight: \"bold\", {})", if page { "2.2" } else { "1.8" }, lit(title));
    let a = if front.author.is_empty() { String::new() } else { format!("#v(0.8em) #text(size: 1.2em, {})", lit(&front.author)) };
    let d = if front.date.is_empty() { String::new() } else { format!("#v(0.4em) #text(size: 1.05em, {})", lit(&front.date)) };
    if page {
        s.push_str(&format!(
            "#page(numbering: none)[#v(1fr) #align(center)[{t} {a} {d} #v(1.5em) #text(size: 1.4em, sym.diamond.stroked)] #v(2fr)]\n#counter(page).update(1)\n"
        ));
    } else {
        s.push_str(&format!("#align(center)[{t} {a} {d}]\n#v(1.5em)\n"));
    }
    s
}

impl Tool for FromMarkdown {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("paper", vec![Choice::lit("a4", "A4"), Choice::lit("letter", "Letter"), Choice::lit("a5", "A5")], "a4"),
            OptionSpec::float("font_size", 8.0, 14.0, 11.0).unit("pt"),
            OptionSpec::toggle("numbered", true),
            OptionSpec::toggle("toc", false),
            OptionSpec::toggle("title_page", false),
            OptionSpec::toggle("page_numbers", true),
            OptionSpec::text("language", "en").advanced(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let src = std::fs::read_to_string(&input.path).map_err(|e| anyhow::anyhow!("{}: {e}", input.name()))?;
        let (front, md) = front_matter(&src);
        let base = input.path.parent().unwrap_or(Path::new("."));
        cx.message(tr("pdf.from-markdown.msg"));
        let build = |math_as_text: bool| -> Result<(Vec<u8>, usize, String)> {
            let (body, files, first_h1) = convert(md, base, math_as_text);
            let title = if !front.title.is_empty() {
                front.title.clone()
            } else {
                first_h1.map(|h| h.replace('\\', "")).unwrap_or_else(|| input.stem())
            };
            let mut s = preamble(v, &title, &front.author);
            if v.bool("title_page") {
                s.push_str(&title_block(&front, &title, true));
            } else if !front.title.is_empty() {
                s.push_str(&title_block(&front, &title, false));
            }
            if v.bool("toc") {
                s.push_str(&format!(
                    "#outline(title: [{}], indent: auto)\n#pagebreak(weak: true)\n",
                    esc(&tr("pdf.from-markdown.contents"))
                ));
            }
            s.push_str(&body);
            if let Some(dir) = std::env::var_os("CASTOR_DEBUG_TYPST") {
                let _ = std::fs::write(std::path::Path::new(&dir).join("main.typ"), &s);
            }
            let (pdf, pages) = typst_pdf(&s, &files)?;
            Ok((pdf, pages, title))
        };
        cx.progress(0.2);
        let (pdf, pages, _title, note) = match build(false) {
            Ok((p, n, t)) => (p, n, t, None),
            Err(e) => {
                // A formula Typst cannot read must not cost the whole document: retry with
                // formulas shown as code, and say so.
                let (p, n, t) = build(true).map_err(|_| e)?;
                (p, n, t, Some(tr("pdf.from-markdown.note.math")))
            }
        };
        cx.check_cancel()?;
        cx.progress(0.9);
        let out = cx.output_named(&format!("{}.pdf", input.stem()));
        std::fs::write(&out, &pdf)?;
        let mut n = tr_args("pdf.note.pages", &[("count", &pages.to_string())]);
        if let Some(extra) = note {
            n.push_str(" · ");
            n.push_str(&extra);
        }
        Ok(vec![Output::new(out).with_note(n)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latex_math() {
        assert_eq!(latex_to_typst(r"\frac{a}{b}"), "frac(a, b)");
        assert_eq!(latex_to_typst(r"x^{2} + y_i"), "x ^(2) + y _i");
        assert_eq!(latex_to_typst(r"\sqrt[3]{x}"), "root(3, x)");
        assert_eq!(latex_to_typst(r"\alpha \leq \beta"), "alpha lt.eq beta");
        assert_eq!(latex_to_typst(r"\mathbb{R}"), "bb(R)");
        assert_eq!(latex_to_typst(r"\text{if } x"), "\"if \" x");
        assert!(latex_to_typst(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}").starts_with("mat(a, b; c, d)"));
        assert_eq!(latex_to_typst("ab"), "a b");
    }
    #[test]
    fn escaping() {
        assert_eq!(esc("a*b_c #1 http://x"), "a\\*b\\_c \\#1 http:\\/\\/x");
    }
}
