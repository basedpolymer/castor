//! Fonts compiled into the engine: Latin Modern (GUST font licence), TeX Gyre Chorus
//! (GUST font licence) and Tesseract's glyph-less font (Apache-2.0) for invisible OCR text.

pub const CHORUS: &[u8] = include_bytes!("../../../assets/fonts/texgyrechorus-mediumitalic.otf");
pub const LM_ROMAN: &[u8] = include_bytes!("../../../assets/fonts/lmroman10-regular.otf");
pub const LM_ROMAN_BOLD: &[u8] = include_bytes!("../../../assets/fonts/lmroman10-bold.otf");
pub const LM_ROMAN_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/lmroman10-italic.otf");
pub const LM_SANS: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-regular.otf");
pub const LM_SANS_BOLD: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-bold.otf");
pub const LM_MONO: &[u8] = include_bytes!("../../../assets/fonts/lmmono10-regular.otf");
/// Every glyph is empty; used with Identity-H + a UCS ToUnicode map, as Tesseract does.
pub const GLYPHLESS: &[u8] = include_bytes!("../data/glyphless.ttf");

/// Font choices offered by text-stamping tools. Base-14 names are understood by MuPDF directly;
/// the others are embedded from the bytes above.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Helvetica,
    HelveticaBold,
    Times,
    TimesBold,
    Courier,
    LatinModern,
    LatinModernBold,
    LatinModernSans,
    Chorus,
}

impl Face {
    pub fn parse(s: &str) -> Face {
        match s {
            "helvetica-bold" => Face::HelveticaBold,
            "times" => Face::Times,
            "times-bold" => Face::TimesBold,
            "courier" => Face::Courier,
            "latin-modern" => Face::LatinModern,
            "latin-modern-bold" => Face::LatinModernBold,
            "latin-modern-sans" => Face::LatinModernSans,
            "chorus" => Face::Chorus,
            _ => Face::Helvetica,
        }
    }
    /// Embedded font file, or None for base-14 faces.
    pub fn bytes(self) -> Option<&'static [u8]> {
        match self {
            Face::LatinModern => Some(LM_ROMAN),
            Face::LatinModernBold => Some(LM_ROMAN_BOLD),
            Face::LatinModernSans => Some(LM_SANS),
            Face::Chorus => Some(CHORUS),
            _ => None,
        }
    }
    /// Base-14 PostScript name (or a stable name for embedded faces).
    pub fn name(self) -> &'static str {
        match self {
            Face::Helvetica => "Helvetica",
            Face::HelveticaBold => "Helvetica-Bold",
            Face::Times => "Times-Roman",
            Face::TimesBold => "Times-Bold",
            Face::Courier => "Courier",
            Face::LatinModern => "LMRoman10-Regular",
            Face::LatinModernBold => "LMRoman10-Bold",
            Face::LatinModernSans => "LMSans10-Regular",
            Face::Chorus => "TeXGyreChorus-MediumItalic",
        }
    }
    /// A `mupdf::Font` for measuring and encoding.
    pub fn load(self) -> anyhow::Result<mupdf::Font> {
        Ok(match self.bytes() {
            Some(b) => mupdf::Font::from_bytes(self.name(), b)?,
            None => mupdf::Font::new(self.name())?,
        })
    }
}

/// Choice values for a font option (kept in sync with [`Face::parse`]).
pub const FACE_CHOICES: &[&str] = &[
    "helvetica",
    "helvetica-bold",
    "times",
    "times-bold",
    "courier",
    "latin-modern",
    "latin-modern-bold",
    "latin-modern-sans",
    "chorus",
];
