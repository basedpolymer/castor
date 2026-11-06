//! Compiles the small C shim (csrc/castor_mupdf.c) against the MuPDF headers shipped inside the
//! `mupdf-sys` crate. The headers are found in the cargo registry (pinned version), or in
//! `CASTOR_MUPDF_INCLUDE` when the sources are vendored elsewhere.

use std::path::PathBuf;

const MUPDF_SYS: &str = "mupdf-sys-0.8.0";

fn find_include() -> PathBuf {
    if let Some(p) = std::env::var_os("CASTOR_MUPDF_INCLUDE") {
        return PathBuf::from(p);
    }
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("no home directory");
        PathBuf::from(home).join(".cargo")
    });
    let mut candidates = Vec::new();
    for base in [cargo_home.join("registry").join("src"), cargo_home.join("git").join("checkouts")] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                candidates.push(e.path().join(MUPDF_SYS).join("mupdf").join("include"));
            }
        }
    }
    // Vendored layout: <workspace>/vendor/mupdf-sys/mupdf/include
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    candidates.push(manifest.join("../../vendor/mupdf-sys/mupdf/include"));
    candidates
        .into_iter()
        .find(|p| p.join("mupdf").join("fitz.h").is_file())
        .unwrap_or_else(|| panic!("MuPDF headers of {MUPDF_SYS} not found; set CASTOR_MUPDF_INCLUDE"))
}

fn main() {
    println!("cargo:rerun-if-changed=csrc/castor_mupdf.c");
    println!("cargo:rerun-if-env-changed=CASTOR_MUPDF_INCLUDE");
    let include = find_include();
    cc::Build::new().file("csrc/castor_mupdf.c").include(&include).warnings(false).compile("castor_mupdf");
}
