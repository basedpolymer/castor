//! Rendering pages on one thread while the C shim rewrites images on another used to crash
//! (access violation): the shim created unlocked MuPDF contexts next to mupdf-rs's locked ones.
//! Slow (needs the 1890 scan fixture): `cargo test -p engine-pdf --test concurrency -- --ignored`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
#[ignore]
fn thumbnails_while_optimizing() {
    let scan = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/generated/pdf/yearbook-1890-scan.pdf");
    if !scan.is_file() {
        eprintln!("fixture missing: {} — skipped", scan.display());
        return;
    }
    let out = std::env::temp_dir().join("castor-concurrency-test.pdf");
    let done = Arc::new(AtomicBool::new(false));
    let renders = {
        let (scan, done) = (scan.clone(), done.clone());
        std::thread::spawn(move || {
            let mut n = 0;
            while !done.load(Ordering::SeqCst) {
                for page in 0..4 {
                    engine_pdf::thumbnail(&scan, page, 160).expect("thumbnail");
                    n += 1;
                }
            }
            n
        })
    };
    let opts = engine_pdf::optimize::Optimize { rewrite_images: true, ..Default::default() };
    engine_pdf::optimize::optimize(&scan, &out, "", &opts).expect("optimize");
    done.store(true, Ordering::SeqCst);
    let n = renders.join().expect("render thread panicked");
    assert!(n > 0);
    let _ = std::fs::remove_file(out);
}
