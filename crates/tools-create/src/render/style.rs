//! House style (docs/DESIGN.md): paper, ink, pastel family tints, the diamond motif.

pub const PAPER: [u8; 4] = [0xfc, 0xfb, 0xf9, 255];
pub const SURFACE: [u8; 4] = [0xf4, 0xf2, 0xee, 255];
pub const INK: [u8; 4] = [0x1b, 0x16, 0x16, 255];
pub const MUTED: [u8; 4] = [0x5c, 0x5a, 0x55, 255];
pub const FAINT: [u8; 4] = [0x8c, 0x8a, 0x84, 255];
pub const RULE: [u8; 4] = [0xd9, 0xd6, 0xcf, 255];
pub const LINK: [u8; 4] = [0x2f, 0x6d, 0xb5, 255];

/// Pastel tints of the families, in the order PDF, Image, Video, Audio, Create, Files.
pub const TINTS: [[u8; 4]; 6] = [
    [0xfb, 0xe2, 0xff, 255],
    [0xcc, 0xfa, 0xff, 255],
    [0xad, 0xe1, 0xff, 255],
    [0xaa, 0xfa, 0xba, 255],
    [0xff, 0xec, 0xa4, 255],
    [0xff, 0xdc, 0xc8, 255],
];
pub const CREATE_TINT: [u8; 4] = TINTS[4];

/// "#rrggbb".
pub fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// "#rrggbb" or "rgba(…)" for SVG fills, plus the opacity separately.
pub fn svg_paint(c: [u8; 4]) -> (String, f32) {
    (hex(c), c[3] as f32 / 255.0)
}

/// Relative luminance (sRGB, 0..1).
pub fn luminance(c: [u8; 4]) -> f32 {
    let f = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
}

/// WCAG contrast ratio.
pub fn contrast(a: [u8; 4], b: [u8; 4]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Ink or paper, whichever reads best on `bg`.
pub fn text_on(bg: [u8; 4]) -> [u8; 4] {
    if contrast(INK, bg) >= contrast(PAPER, bg) { INK } else { PAPER }
}

/// Linear interpolation in OKLab (smooth, perceptual gradients).
pub fn mix_oklab(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let la = to_oklab(a);
    let lb = to_oklab(b);
    let l = [la[0] + (lb[0] - la[0]) * t, la[1] + (lb[1] - la[1]) * t, la[2] + (lb[2] - la[2]) * t];
    let mut c = from_oklab(l);
    c[3] = (a[3] as f32 + (b[3] as f32 - a[3] as f32) * t).round() as u8;
    c
}

pub fn srgb_to_linear(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

pub fn to_oklab(c: [u8; 4]) -> [f32; 3] {
    let (r, g, b) = (srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

pub fn from_oklab(lab: [f32; 3]) -> [u8; 4] {
    let lin = oklab_to_linear(lab);
    [linear_to_srgb(lin[0]), linear_to_srgb(lin[1]), linear_to_srgb(lin[2]), 255]
}

pub fn oklab_to_linear(lab: [f32; 3]) -> [f32; 3] {
    let l = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    let m = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    let s = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

/// Typst prelude shared by every generated document: colours, fonts, the diamond.
pub const TYPST_PRELUDE: &str = r##"
#let paper = rgb("#fcfbf9")
#let surface = rgb("#f4f2ee")
#let ink = rgb("#1b1616")
#let muted = rgb("#5c5a55")
#let faint = rgb("#8c8a84")
#let rulec = rgb("#d9d6cf")
#let tint = rgb("#ffeca4")
#let serif = "Latin Modern Roman"
#let sans = "Latin Modern Sans"
#let mono = "Latin Modern Mono"
#let chorus = "TeX Gyre Chorus"
#let diamond(size: 7pt, stroke: 0.9pt + ink, fill: none) = box(width: size * 1.42, height: size * 1.42, align(center + horizon, rotate(45deg, rect(width: size, height: size, stroke: stroke, fill: fill))))
#let pagerule(width: 100%, stroke: 0.6pt + ink, size: 5pt) = box(width: width, height: size * 1.5, {
  place(horizon + left, line(length: 100% - size * 1.6, stroke: stroke))
  place(horizon + right, diamond(size: size, stroke: stroke))
})
"##;
