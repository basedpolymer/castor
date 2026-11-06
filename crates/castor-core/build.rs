// The translations in /locales are compiled into this crate by `rust_i18n::i18n!`.
// Tell Cargo to rebuild when any of them changes.
fn main() {
    println!("cargo:rerun-if-changed=../../locales");
    if let Ok(entries) = std::fs::read_dir("../../locales") {
        for e in entries.flatten() {
            println!("cargo:rerun-if-changed={}", e.path().display());
        }
    }
}
