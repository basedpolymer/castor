//! Network test (ignored by default): fetch the official yt-dlp binary, verify it against the
//! release checksums and run `--version`.
//! `cargo test -p engine-media --test ytdlp -- --ignored`

#[test]
#[ignore]
fn ensure_and_version() {
    let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/generated/castor-home");
    unsafe { std::env::set_var("CASTOR_HOME", &home) };
    let tmp = tempfile::tempdir().unwrap();
    let cx = castor_core::RunCx::new(tmp.path().to_path_buf(), tmp.path().to_path_buf(), "", castor_core::CancelToken::new());
    let p = engine_media::ytdlp::ensure(&cx).unwrap();
    assert!(p.is_file());
    let v = engine_media::ytdlp::version(&cx).unwrap();
    assert!(v.starts_with("20"), "version {v}");
    eprintln!("yt-dlp {v} at {}", p.display());
}
