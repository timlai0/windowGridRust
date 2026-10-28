//! Compiles the Slint UI and embeds the overlay and hook DLLs.
//!
//! The components come from `../dist` (or `../dist-iso` for the `isolated` feature), built
//! by `build-hooks.ps1` and `build-overlay.ps1`. If one is missing the app still builds, with
//! that component absent (a warning is printed): at run time it then uses the file next to
//! the executable, if any. That keeps `cargo test` working without building everything first.

use std::fmt::Write;
use std::path::PathBuf;

const COMPONENTS: [(&str, &str); 4] = [
    ("OVERLAY", "WindowGridRustOverlay.exe"),
    ("HOOK_X64", "WindowGridRust64.dll"),
    ("HOOK_X86", "WindowGridRust32.dll"),
    ("HOOK_ARM64", "WindowGridRustARM64.dll"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ISOLATED");

    let style = std::env::var("SLINT_STYLE").unwrap_or_else(|_| "fluent".into());
    slint_build::compile_with_config("ui/app.slint", slint_build::CompilerConfiguration::new().with_style(style))
        .expect("the Slint UI compiles");

    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let isolated = std::env::var_os("CARGO_FEATURE_ISOLATED").is_some();
    let dist = root.join("..").join(if isolated { "dist-iso" } else { "dist" });

    let mut code = String::new();
    for (name, file) in COMPONENTS {
        let path = dist.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        if let Ok(path) = path.canonicalize() {
            let path = path.display().to_string();
            let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
            writeln!(code, "pub const {name}: &[u8] = include_bytes!({path:?});").unwrap();
        } else {
            println!("cargo:warning={file} not found in {}: it will not be embedded", dist.display());
            writeln!(code, "pub const {name}: &[u8] = &[];").unwrap();
        }
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("components.rs");
    std::fs::write(out, code).unwrap();

    // Icon: a failure (no resource compiler on this machine) is not fatal.
    if cfg!(windows) {
        let rc = root.join("assets").join("app.rc");
        if rc.exists() {
            let _ = embed_resource::compile(rc, embed_resource::NONE).manifest_optional();
        }
    }
}
