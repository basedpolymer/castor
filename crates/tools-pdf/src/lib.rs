//! PDF tools (and the image tools that are really PDF/OCR work). Owner: agent A, see
//! docs/specs/A-pdf.md. Strings: locales/pdf.yml.

use std::sync::Arc;

use castor_core::Tool;

pub mod common;
mod compare;
mod compress;
mod convert;
mod docs;
mod forms;
pub mod images;
mod markdown_pdf;
mod merge;
pub mod ocrtools;
mod overlay;
mod pagetools;
mod redact;
mod split;
pub mod structure;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(compress::Compress::new()),
        Arc::new(merge::Merge::new()),
        Arc::new(split::Split::new()),
        Arc::new(ocrtools::OcrPdf::new()),
        Arc::new(redact::Redact::new()),
        Arc::new(overlay::Sign::new()),
        Arc::new(docs::Encrypt::new()),
        Arc::new(docs::Unlock::new()),
        Arc::new(images::ToImages::new()),
        Arc::new(pagetools::Organize::new()),
        Arc::new(pagetools::Rotate::new()),
        Arc::new(pagetools::ExtractPages::new()),
        Arc::new(overlay::PageNumbers::new()),
        Arc::new(overlay::Stamp::new()),
        Arc::new(pagetools::Crop::new()),
        Arc::new(convert::ToMarkdown::new()),
        Arc::new(pagetools::AttachToEach::new()),
        Arc::new(docs::Flatten::new()),
        Arc::new(docs::Repair::new()),
        Arc::new(convert::FromOffice::new()),
        Arc::new(convert::ToEpub::new()),
        Arc::new(convert::FromEbook::new()),
        Arc::new(compare::Compare::new()),
        Arc::new(docs::Metadata::new()),
        Arc::new(forms::FillForm::new()),
        Arc::new(markdown_pdf::FromMarkdown::new()),
        Arc::new(images::ExtractImages::new()),
        Arc::new(docs::Grayscale::new()),
        // Image family, built on the PDF/OCR engine.
        Arc::new(images::ScanToPdf::new()),
        Arc::new(images::ImagesToPdf::new()),
        Arc::new(ocrtools::ExtractText::new()),
    ]
}
