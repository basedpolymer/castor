//! `create.formula`: typeset maths as SVG, PNG or PDF with Typst's math engine and New Computer
//! Modern Math. Accepts common LaTeX (translated: \frac, \sqrt, Greek, operators, accents,
//! \mathbb…, matrices, cases, aligned) or native Typst math.

use anyhow::{Result, bail};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common;
use crate::render::{png, style, typ};

pub struct FormulaTool {
    meta: ToolMeta,
}

impl FormulaTool {
    pub fn new() -> Self {
        FormulaTool {
            meta: common::meta(
                "create.formula",
                InputSpec::none(),
                "SVG · PNG · PDF",
                &["formula", "equation", "latex", "math", "typst", "maths", "tex"],
                "formula",
            ),
        }
    }
}

impl Tool for FormulaTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::multiline("formula", r"e^{i\pi} + 1 = 0 \quad \int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}"),
            OptionSpec::choice("syntax", ["auto", "latex", "typst"].map(Choice::key).to_vec(), "auto"),
            OptionSpec::toggle("display", true),
            OptionSpec::int("font_size", 6, 200, 24).unit("pt"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("background", "#00000000"),
            OptionSpec::int("padding", 0, 100, 6).unit("pt"),
            OptionSpec::choice("format", vec![Choice::lit("svg", "SVG"), Choice::lit("png", "PNG"), Choice::lit("pdf", "PDF")], "svg"),
            OptionSpec::int("dpi", 72, 1200, 300).unit("dpi").when(Cond::Eq("format", "png")),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let doc = document(v)?.compile()?;
        let (w, h) = typ::page_size(&doc, 0);
        let ppt = (max_side as f64 * 0.8 / w.max(h)).min(4.0) as f32;
        let pm = typ::pixmap(&doc, 0, ppt)?;
        // Show transparent formulas on paper so they can be seen.
        let img = png::flatten(&png::pixmap_to_rgba(&pm), [0xfc, 0xfb, 0xf9]);
        Ok(Some(common::preview(png::encode_rgb(&img)?, common::figure(&format!("{:.0} × {:.0} pt", w, h)))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let doc = document(v)?.compile()?;
        let fmt = v.str("format");
        let out = common::out_named(cx, v.str("file_name"), fmt);
        match fmt {
            "pdf" => common::write(&out, &typ::pdf(&doc)?)?,
            "png" => {
                let pm = typ::pixmap(&doc, 0, v.int("dpi").clamp(72, 1200) as f32 / 72.0)?;
                common::write(&out, &png::encode_pixmap(&pm)?)?;
            }
            _ => common::write(&out, typ::svg(&doc, 0)?.as_bytes())?,
        }
        let (w, h) = typ::page_size(&doc, 0);
        Ok(vec![Output::new(out).with_note(format!("{:.0} × {:.0} pt", w, h))])
    }
}

/// The Typst document for the formula.
pub fn document(v: &Values) -> Result<typ::Doc> {
    let src = v.str("formula").trim();
    anyhow::ensure!(!src.is_empty(), "{}", tr("create.formula.err.empty"));
    let latex = match v.str("syntax") {
        "latex" => true,
        "typst" => false,
        _ => src.contains('\\') || src.contains('{'),
    };
    let math = if latex { latex_to_typst(src)? } else { src.to_string() };
    let bg = v.color("background");
    let fill = if bg[3] == 0 { "none".to_string() } else { typ::color(bg) };
    let body = if v.bool("display") { format!("$ {math} $") } else { format!("${math}$") };
    let source = format!(
        "#set page(width: auto, height: auto, margin: {}pt, fill: {fill})\n#set text(font: \"Latin Modern Roman\", size: {}pt, fill: {})\n#show math.equation: set text(font: \"New Computer Modern Math\")\n{body}\n",
        v.int("padding").clamp(0, 100),
        v.int("font_size").clamp(6, 200),
        typ::color(v.color("color")),
    );
    let _ = style::INK;
    Ok(typ::Doc::new(source))
}

// ---------------------------------------------------------------- LaTeX → Typst

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Cmd(String),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    Row,
    Letter(char),
    Num(String),
    Char(char),
    Space,
}

fn lex(s: &str) -> Vec<Tok> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\\' => {
                i += 1;
                if i < c.len() && c[i].is_ascii_alphabetic() {
                    let st = i;
                    while i < c.len() && c[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    out.push(Tok::Cmd(c[st..i].iter().collect()));
                    continue;
                }
                if i < c.len() {
                    if c[i] == '\\' {
                        out.push(Tok::Row);
                    } else {
                        out.push(Tok::Cmd(c[i].to_string()));
                    }
                    i += 1;
                }
                continue;
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            c0 if c0.is_whitespace() => {
                if out.last() != Some(&Tok::Space) {
                    out.push(Tok::Space);
                }
            }
            c0 if c0.is_ascii_digit() || (c0 == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let st = i;
                while i < c.len() && (c[i].is_ascii_digit() || (c[i] == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()))) {
                    i += 1;
                }
                out.push(Tok::Num(c[st..i].iter().collect()));
                continue;
            }
            c0 if c0.is_alphabetic() => out.push(Tok::Letter(c0)),
            c0 => out.push(Tok::Char(c0)),
        }
        i += 1;
    }
    out
}

struct P {
    t: Vec<Tok>,
    i: usize,
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
        "leq" | "le" => "<=",
        "geq" | "ge" => ">=",
        "neq" | "ne" => "!=",
        "ll" => "<<",
        "gg" => ">>",
        "approx" => "approx",
        "equiv" => "equiv",
        "sim" => "tilde.op",
        "simeq" => "tilde.eq",
        "cong" => "tilde.equiv",
        "propto" => "prop",
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
        "emptyset" | "varnothing" => "emptyset",
        "forall" => "forall",
        "exists" => "exists",
        "neg" | "lnot" => "not",
        "land" | "wedge" => "and",
        "lor" | "vee" => "or",
        "to" | "rightarrow" => "arrow.r",
        "leftarrow" | "gets" => "arrow.l",
        "leftrightarrow" => "arrow.l.r",
        "Rightarrow" => "arrow.r.double",
        "Leftarrow" => "arrow.l.double",
        "Leftrightarrow" => "arrow.l.r.double",
        "implies" => "arrow.r.double.long",
        "impliedby" => "arrow.l.double.long",
        "iff" => "arrow.l.r.double.long",
        "mapsto" => "arrow.r.bar",
        "longrightarrow" => "arrow.r.long",
        "pm" => "plus.minus",
        "mp" => "minus.plus",
        "times" => "times",
        "cdot" => "dot.op",
        "cdots" => "dots.c",
        "ldots" | "dots" => "dots",
        "vdots" => "dots.v",
        "ddots" => "dots.down",
        "div" => "div",
        "circ" => "compose",
        "ast" => "ast",
        "star" => "star",
        "oplus" => "plus.circle",
        "otimes" => "times.circle",
        "langle" => "angle.l",
        "rangle" => "angle.r",
        "lfloor" => "floor.l",
        "rfloor" => "floor.r",
        "lceil" => "ceil.l",
        "rceil" => "ceil.r",
        "mid" => "divides",
        "parallel" => "parallel",
        "perp" => "perp",
        "angle" => "angle",
        "degree" => "degree",
        "prime" => "prime",
        "hbar" => "planck.reduce",
        "ell" => "ell",
        "Re" => "Re",
        "Im" => "Im",
        "aleph" => "aleph",
        "quad" => "quad",
        "qquad" => "wide",
        "," => "thin",
        ":" | ">" => "med",
        ";" => "thick",
        " " => "space",
        "%" => "%",
        "{" => "\\{",
        "}" => "\\}",
        "|" => "||",
        "#" => "\\#",
        "$" => "\\$",
        "&" => "\\&",
        "_" => "\\_",
        "lvert" | "rvert" | "vert" => "|",
        "lVert" | "rVert" | "Vert" => "||",
        _ => return None,
    })
}

const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "sinh", "cosh", "tanh", "coth", "arcsin", "arccos", "arctan", "log", "ln", "lg", "exp", "lim",
    "liminf", "limsup", "max", "min", "sup", "inf", "det", "arg", "deg", "dim", "gcd", "hom", "ker", "mod", "Pr",
];

fn esc_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn skip_space(&mut self) {
        while self.peek() == Some(&Tok::Space) {
            self.i += 1;
        }
    }

    /// Raw text of a {group} (for \text, \operatorname, \begin names).
    fn raw_group(&mut self) -> Result<String> {
        self.skip_space();
        if self.peek() != Some(&Tok::Open) {
            return Ok(match self.next() {
                Some(Tok::Letter(c)) | Some(Tok::Char(c)) => c.to_string(),
                Some(Tok::Num(n)) => n,
                _ => String::new(),
            });
        }
        self.i += 1;
        let mut s = String::new();
        let mut depth = 0;
        while let Some(t) = self.next() {
            match t {
                Tok::Open => {
                    depth += 1;
                    s.push('{');
                }
                Tok::Close if depth == 0 => return Ok(s),
                Tok::Close => {
                    depth -= 1;
                    s.push('}');
                }
                Tok::Letter(c) | Tok::Char(c) => s.push(c),
                Tok::Num(n) => s.push_str(&n),
                Tok::Space => s.push(' '),
                Tok::Cmd(c) => {
                    s.push('\\');
                    s.push_str(&c);
                }
                Tok::Sup => s.push('^'),
                Tok::Sub => s.push('_'),
                Tok::Amp => s.push('&'),
                Tok::Row => s.push_str("\\\\"),
            }
        }
        bail!("{}", tr("create.formula.err.brace"))
    }

    /// One argument: a {group} or a single token, as Typst (to be wrapped by the caller).
    fn arg(&mut self) -> Result<String> {
        self.skip_space();
        if self.peek() == Some(&Tok::Open) {
            self.i += 1;
            let s = self.seq(&[Tok::Close])?;
            if self.next() != Some(Tok::Close) {
                bail!("{}", tr("create.formula.err.brace"));
            }
            return Ok(s);
        }
        self.atom()
    }

    /// Optional [n] argument (\sqrt[3]{x}).
    fn opt_arg(&mut self) -> Result<Option<String>> {
        self.skip_space();
        if self.peek() == Some(&Tok::Char('[')) {
            self.i += 1;
            let s = self.seq(&[Tok::Char(']')])?;
            self.i += 1;
            return Ok(Some(s));
        }
        Ok(None)
    }

    fn atom(&mut self) -> Result<String> {
        let Some(t) = self.next() else { return Ok(String::new()) };
        Ok(match t {
            Tok::Letter(c) => c.to_string(),
            Tok::Num(n) => n,
            Tok::Open => {
                // An invisible LaTeX group: Typst needs no grouping outside scripts.
                let s = self.seq(&[Tok::Close])?;
                if self.next() != Some(Tok::Close) {
                    bail!("{}", tr("create.formula.err.brace"));
                }
                s
            }
            Tok::Char(c) => match c {
                '/' => "slash".into(),
                '"' => "\\\"".into(),
                '#' => "\\#".into(),
                '$' => "\\$".into(),
                '\'' => "'".into(),
                ',' => ",".into(),
                c => c.to_string(),
            },
            Tok::Space => " ".into(),
            Tok::Cmd(name) => self.command(&name)?,
            Tok::Sup | Tok::Sub => String::new(),
            Tok::Amp => "&".into(),
            Tok::Row => " \\ ".into(),
            Tok::Close => bail!("{}", tr("create.formula.err.brace")),
        })
    }

    fn command(&mut self, name: &str) -> Result<String> {
        if let Some(s) = symbol(name) {
            return Ok(s.to_string());
        }
        if FUNCS.contains(&name) {
            return Ok(name.to_string());
        }
        let wrap = |f: &str, a: String| format!("{f}({})", protect(&a));
        Ok(match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg()?;
                let b = self.arg()?;
                format!("frac({}, {})", protect(&a), protect(&b))
            }
            "binom" | "dbinom" | "tbinom" => {
                let a = self.arg()?;
                let b = self.arg()?;
                format!("binom({}, {})", protect(&a), protect(&b))
            }
            "sqrt" => match self.opt_arg()? {
                Some(n) => {
                    let x = self.arg()?;
                    format!("root({}, {})", protect(&n), protect(&x))
                }
                None => wrap("sqrt", self.arg()?),
            },
            "hat" | "widehat" => wrap("hat", self.arg()?),
            "bar" | "overline" => wrap("overline", self.arg()?),
            "underline" => wrap("underline", self.arg()?),
            "vec" => wrap("arrow", self.arg()?),
            "dot" => wrap("dot", self.arg()?),
            "ddot" => wrap("dot.double", self.arg()?),
            "tilde" | "widetilde" => wrap("tilde", self.arg()?),
            "overbrace" => wrap("overbrace", self.arg()?),
            "underbrace" => wrap("underbrace", self.arg()?),
            "mathbb" => wrap("bb", self.arg()?),
            "mathbf" | "boldsymbol" | "bm" => wrap("bold", self.arg()?),
            "mathrm" => wrap("upright", self.arg()?),
            "mathit" => wrap("italic", self.arg()?),
            "mathcal" => wrap("cal", self.arg()?),
            "mathfrak" => wrap("frak", self.arg()?),
            "mathsf" => wrap("sans", self.arg()?),
            "mathtt" => wrap("mono", self.arg()?),
            "text" | "textrm" | "textit" | "textbf" | "mbox" | "mathtext" => esc_str(&self.raw_group()?),
            "operatorname" => format!("op({})", esc_str(&self.raw_group()?)),
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "middle" => {
                // Typst scales delimiters by itself; "\left." means "no delimiter".
                self.skip_space();
                if self.peek() == Some(&Tok::Char('.')) {
                    self.i += 1;
                }
                String::new()
            }
            "displaystyle" | "textstyle" | "limits" | "nolimits" | "!" => String::new(),
            "begin" => {
                let env = self.raw_group()?;
                self.environment(env.trim())?
            }
            "end" => {
                let _ = self.raw_group()?;
                String::new()
            }
            other => bail!("{}", tr_args("create.formula.err.command", &[("command", &format!("\\{other}"))])),
        })
    }

    fn environment(&mut self, env: &str) -> Result<String> {
        let env = env.trim_end_matches('*');
        if env == "array" {
            let _ = self.raw_group()?; // column spec
        }
        // Collect rows of cells until \end{env}.
        let mut rows: Vec<Vec<String>> = vec![vec![]];
        let mut cell = String::new();
        loop {
            match self.peek() {
                None => bail!("{}", tr_args("create.formula.err.end", &[("env", env)])),
                Some(Tok::Cmd(c)) if c == "end" => {
                    self.i += 1;
                    let _ = self.raw_group()?;
                    break;
                }
                Some(Tok::Amp) => {
                    self.i += 1;
                    rows.last_mut().unwrap().push(std::mem::take(&mut cell));
                }
                Some(Tok::Row) => {
                    self.i += 1;
                    rows.last_mut().unwrap().push(std::mem::take(&mut cell));
                    rows.push(vec![]);
                }
                _ => {
                    let a = self.item()?;
                    cell.push_str(&a);
                }
            }
        }
        rows.last_mut().unwrap().push(cell);
        rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
        let delim = match env {
            "pmatrix" => Some("\"(\""),
            "bmatrix" => Some("\"[\""),
            "Bmatrix" => Some("\"{\""),
            "vmatrix" => Some("\"|\""),
            "Vmatrix" => Some("\"||\""),
            "matrix" | "array" | "smallmatrix" => Some("#none"),
            _ => None,
        };
        Ok(match (env, delim) {
            (_, Some(d)) => format!(
                "mat(delim: {d}, {})",
                rows.iter().map(|r| r.iter().map(|c| protect(c.trim())).collect::<Vec<_>>().join(", ")).collect::<Vec<_>>().join("; ")
            ),
            ("cases", _) => format!(
                "cases({})",
                rows.iter().map(|r| r.iter().map(|c| protect_semi(c.trim())).collect::<Vec<_>>().join(" & ")).collect::<Vec<_>>().join(", ")
            ),
            // aligned, align, gather, split, equation…: Typst equations take & and \ directly.
            _ => rows.iter().map(|r| r.join(" & ")).collect::<Vec<_>>().join(" \\ "),
        })
    }

    /// An atom with its scripts.
    fn item(&mut self) -> Result<String> {
        let mut base = self.atom()?;
        loop {
            let save = self.i;
            self.skip_space();
            match self.peek() {
                Some(Tok::Sup) => {
                    self.i += 1;
                    let a = self.arg()?;
                    base = format!("{}^({})", if base.trim().is_empty() { "\"\"".into() } else { base }, a.trim());
                }
                Some(Tok::Sub) => {
                    self.i += 1;
                    let a = self.arg()?;
                    base = format!("{}_({})", if base.trim().is_empty() { "\"\"".into() } else { base }, a.trim());
                }
                _ => {
                    self.i = save;
                    return Ok(base);
                }
            }
        }
    }

    /// A sequence until one of `stop` (not consumed) or the end.
    fn seq(&mut self, stop: &[Tok]) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();
        while let Some(t) = self.peek() {
            if stop.contains(t) {
                break;
            }
            let it = self.item()?;
            parts.push(it);
        }
        // Separate everything with spaces so "ab" stays "a b" (two variables), then tidy.
        let s = parts.iter().filter(|p| !p.is_empty()).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        Ok(s)
    }
}

/// Inside function arguments, commas and semicolons would split arguments: escape them.
fn protect(s: &str) -> String {
    let s = s.trim();
    let mut o = String::new();
    let mut depth = 0i32;
    let mut in_str = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                o.push(c);
            }
            '(' if !in_str => {
                depth += 1;
                o.push(c);
            }
            ')' if !in_str => {
                depth -= 1;
                o.push(c);
            }
            ',' | ';' if !in_str && depth <= 0 => {
                o.push('\\');
                o.push(c);
            }
            c => o.push(c),
        }
    }
    if o.is_empty() { "\"\"".into() } else { o }
}

fn protect_semi(s: &str) -> String {
    protect(s)
}

/// Translate common LaTeX maths into Typst maths.
pub fn latex_to_typst(src: &str) -> Result<String> {
    let src = src.trim().trim_start_matches("$$").trim_end_matches("$$").trim_matches('$');
    let src = src.trim_start_matches("\\[").trim_end_matches("\\]");
    let mut p = P { t: lex(src), i: 0 };
    let mut out = String::new();
    while p.peek().is_some() {
        if p.peek() == Some(&Tok::Close) {
            bail!("{}", tr("create.formula.err.brace"));
        }
        let s = p.seq(&[Tok::Close])?;
        out.push_str(&s);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiles(latex: &str) {
        let t = latex_to_typst(latex).unwrap_or_else(|e| panic!("{latex}: {e:#}"));
        typ::Doc::new(format!("$ {t} $")).compile().unwrap_or_else(|e| panic!("{latex}\n→ {t}\n{e:#}"));
    }

    #[test]
    fn translations() {
        assert_eq!(latex_to_typst(r"\frac{a}{b}").unwrap(), "frac(a, b)");
        assert_eq!(latex_to_typst(r"x^{2n}").unwrap(), "x^(2 n)");
        assert_eq!(latex_to_typst(r"\sqrt[3]{x}").unwrap(), "root(3, x)");
        assert_eq!(latex_to_typst(r"\alpha\beta").unwrap(), "alpha beta");
        for f in [
            r"e^{i\pi} + 1 = 0",
            r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
            r"\sum_{k=1}^{n} k = \frac{n(n+1)}{2}",
            r"\mathbb{R}^n \to \mathbb{R}, \quad f(x, y) = \lVert x - y \rVert",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            r"f(x) = \begin{cases} x^2 & \text{if } x \geq 0 \\ -x & \text{otherwise} \end{cases}",
            r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
            r"\hat{H}\psi = E\psi, \quad \vec{F} = m\vec{a}",
            r"\left( \frac{a}{b} \right)^2 \leq \binom{n}{k}",
            r"\begin{aligned} a &= b + c \\ d &= e \end{aligned}",
            r"\operatorname{tr}(A) = \sum_i A_{ii}",
            r"a/b + \{1, 2\}",
        ] {
            compiles(f);
        }
        assert!(latex_to_typst(r"\frac{a}{b").is_err());
        assert!(latex_to_typst(r"\nosuchcommand").is_err());
    }
}
