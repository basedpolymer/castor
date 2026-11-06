//! Create tools (QR codes, barcodes, cards, gradients, mockups…). Owner: agent E —
//! see docs/specs/E-create-files.md.

use std::sync::Arc;

use castor_core::Tool;

pub mod common;
pub mod render;

pub mod audiogram;
pub mod barcode;
pub mod brand_sheet;
pub mod carousel;
pub mod data_merge;
pub mod favicon;
pub mod fonts;
pub mod formula;
pub mod gradient;
pub mod html;
pub mod mockup;
pub mod motion;
pub mod pattern;
pub mod placeholder;
pub mod qr;
pub mod shader;
pub mod social_card;
pub mod video_download;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(qr::QrCodeTool::new()),
        Arc::new(barcode::BarcodeTool::new()),
        Arc::new(placeholder::PlaceholderTool::new()),
        Arc::new(favicon::FaviconTool::new()),
        Arc::new(gradient::GradientTool::new()),
        Arc::new(shader::ShaderTool::new()),
        Arc::new(pattern::PatternTool::new()),
        Arc::new(pattern::WatermarkTileTool::new()),
        Arc::new(social_card::SocialCardTool::new()),
        Arc::new(mockup::MockupTool::new()),
        Arc::new(audiogram::AudiogramTool::new()),
        Arc::new(motion::MotionTool::new()),
        Arc::new(brand_sheet::BrandSheetTool::new()),
        Arc::new(carousel::CarouselTool::new()),
        Arc::new(data_merge::DataMergeTool::new()),
        Arc::new(formula::FormulaTool::new()),
        Arc::new(html::HtmlToImage::new()),
        Arc::new(html::HtmlToPdf::new()),
        Arc::new(video_download::VideoDownloadTool::new()),
        Arc::new(fonts::FontsTool::new()),
    ]
}

#[cfg(test)]
mod tests {
    /// Every name, description, option label and choice label exists in English and French.
    #[test]
    fn every_string_is_translated() {
        let mut tools = super::tools();
        tools.extend(tools_files::tools());
        let mut missing = Vec::new();
        for lang in ["en", "fr"] {
            castor_core::i18n::set_language(lang);
            for t in &tools {
                let id = t.meta().id;
                for k in ["name", "desc", "keywords"] {
                    let key = format!("{id}.{k}");
                    if !castor_core::i18n::has(&key) {
                        missing.push(format!("{lang} {key}"));
                    }
                }
                for o in t.options() {
                    let key = o.label_key(id);
                    if !castor_core::i18n::has(&key) {
                        missing.push(format!("{lang} {key}"));
                    }
                    if let castor_core::OptionKind::Choice(cs) = &o.kind {
                        for c in cs.iter().filter(|c| c.literal.is_none()) {
                            let ck = format!("{key}.{}", c.value);
                            if !castor_core::i18n::has(&ck) {
                                missing.push(format!("{lang} {ck}"));
                            }
                        }
                    }
                }
            }
        }
        castor_core::i18n::set_language("en");
        assert!(missing.is_empty(), "missing strings:\n{}", missing.join("\n"));
    }

    #[test]
    fn families() {
        let f = crate::render::typ::bundled_families();
        assert!(f.iter().any(|x| x == "Latin Modern Roman"), "{f:?}");
        let d = crate::render::typ::Doc::new("#set page(width: 4cm, height: 2cm)\n#text(font: \"Latin Modern Roman\")[Hello $x^2$]")
            .compile()
            .unwrap();
        assert_eq!(d.pages().len(), 1);
    }
}
