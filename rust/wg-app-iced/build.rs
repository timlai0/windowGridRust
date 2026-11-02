//! Embeds the application icon (shared with `wg-app`); the tray icon is loaded from it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../wg-app/assets");

    // A failure (no resource compiler on this machine) is not fatal.
    if cfg!(windows) {
        let rc = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..").join("wg-app").join("assets").join("app.rc");
        if rc.exists() {
            let _ = embed_resource::compile(rc, embed_resource::NONE).manifest_optional();
        }
    }
}
