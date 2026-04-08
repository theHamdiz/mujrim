//! Build script for kishmat-installer.
//!
//! Verifies that release binaries exist before compiling, since they are
//! embedded via `include_bytes!` in `embedded.rs`.

use std::path::Path;

/// Binaries that the installer bundles.
const REQUIRED: &[&str] = &["kishmat", "kishmat-ui", "kishmat-game", "kishmat-updater"];

fn main() {
    let release_dir = Path::new("../../target/release");

    for bin in REQUIRED {
        let path = if cfg!(target_os = "windows") {
            release_dir.join(format!("{bin}.exe"))
        } else {
            release_dir.join(bin)
        };

        // Emit rerun-if-changed so cargo re-embeds when binaries are rebuilt
        println!("cargo:rerun-if-changed={}", path.display());

        if !path.exists() {
            println!(
                "cargo:warning=Missing release binary: {} — run `cargo build --release --workspace --exclude kishmat-installer` first.",
                path.display()
            );
        }
    }

    // Always re-run when logo changes
    println!("cargo:rerun-if-changed=assets/logo.png");
}
