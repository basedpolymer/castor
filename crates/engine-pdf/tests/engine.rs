//! Engine tests: metadata strings, HTML layout with embedded fonts, font subsetting, thumbnails,
//! page count, OCR of a rendered image.

use std::path::Path;

use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::mupdf::Size;

fn text_of(p: &Path) -> String {
    let doc = engine_pdf::open_document(p, "").unwrap();
    (0..doc.page_count().unwrap()).map(|i| engine_pdf::text::page_text(&doc.load_page(i).unwrap()).unwrap()).collect()
}

#[test]
fn info_strings_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let mut doc = PdfDocument::new();
    doc.new_page(Size::A4).unwrap();
    engine_pdf::set_info(&mut doc, "Title", "Quarterly — été 日本").unwrap();
    engine_pdf::set_info(&mut doc, "Author", "basedpolymer").unwrap();
    let p = d.path().join("info.pdf");
    engine_pdf::save_pdf(&doc, &p, &engine_pdf::Save::default()).unwrap();
    let doc = engine_pdf::open_pdf(&p, "").unwrap();
    assert_eq!(engine_pdf::get_info(&doc, "Title"), "Quarterly — été 日本");
    assert_eq!(engine_pdf::get_info(&doc, "Author"), "basedpolymer");
    assert_eq!(engine_pdf::page_count(&p).unwrap(), 1);
    let png = engine_pdf::thumbnail(&p, 0, 128).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!(img.height(), 128);
}

#[test]
fn html_layout_keeps_text_after_subsetting() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("lmr.otf"), engine_pdf::fonts::LM_ROMAN).unwrap();
    let html = d.path().join("t.xhtml");
    std::fs::write(
        &html,
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><style>@font-face { font-family: LM; src: url(\"lmr.otf\"); } body { font-family: LM; }</style></head><body><p>Hello Latin Modern world</p></body></html>",
    )
    .unwrap();
    let mut doc = engine_pdf::open_document(&html, "").unwrap();
    doc.layout(595.0, 842.0, 11.0).unwrap();
    let pdf = doc.convert_to_pdf(0, -1, 0).unwrap();
    let a = d.path().join("a.pdf");
    engine_pdf::save_pdf(&pdf, &a, &engine_pdf::Save::default()).unwrap();
    assert!(text_of(&a).contains("Hello Latin Modern world"), "before subsetting: {:?}", text_of(&a));
    let b = d.path().join("b.pdf");
    let o = engine_pdf::optimize::Optimize { subset_fonts: true, ..Default::default() };
    engine_pdf::optimize::optimize(&a, &b, "", &o).unwrap();
    assert!(text_of(&b).contains("Hello Latin Modern world"), "after subsetting: {:?}", text_of(&b));
}

