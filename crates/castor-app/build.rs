// Embed the Windows icon (resource id 1, which GPUI uses as the window icon).
fn main() {
    println!("cargo:rerun-if-changed=resources/castor.rc");
    println!("cargo:rerun-if-changed=../../assets/brand/castor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/castor.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
