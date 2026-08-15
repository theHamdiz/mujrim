//! Build-time payload generation for the self-contained installer.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

include!("../../build/windows_manifest.rs");

const PAYLOADS: &[(&str, &str, &str, bool, &str)] = &[
    (
        "Mujrim Elite Engine (external networks)",
        "mujrim-external",
        "release/mujrim-external",
        false,
        "Lean Mujrim architecture-aware UCI engine with runtime NNUE discovery",
    ),
    (
        "Mujrim Elite Engine (embedded networks)",
        "mujrim-embedded",
        "release/mujrim-embedded",
        false,
        "Self-contained Mujrim engine with Akimbo, Stockfish, and v60 NNUE payloads",
    ),
    (
        "Mujrim v60 Engine (external network)",
        "mujrim-v60-external",
        "release/mujrim-v60-external",
        false,
        "Lean Mujrim v60 engine with runtime fingerprint discovery",
    ),
    (
        "Mujrim v60 Engine (embedded network)",
        "mujrim-v60-embedded",
        "release/mujrim-v60-embedded",
        false,
        "Self-contained Mujrim v60 tournament engine",
    ),
    (
        "Mujrim UI",
        "mujrim-ui",
        "release/mujrim-ui",
        true,
        "Mujrim Chess GUI",
    ),
    (
        "Mujrim Updater",
        "mujrim-updater",
        "release/mujrim-updater",
        false,
        "Mujrim update manager",
    ),
];

fn artifact_base(out_dir: &Path) -> PathBuf {
    out_dir
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == "release" || name == "desktop-release")
        })
        .and_then(Path::parent)
        .expect("OUT_DIR must be nested below a release profile")
        .to_path_buf()
}

fn main() {
    println!("cargo:rerun-if-changed=../../assets/branding/mujrim-icon.png");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_EMBED");
    embed_as_invoker();
    if env::var_os("CARGO_FEATURE_EMBED").is_none() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let base = artifact_base(&out_dir);
    let windows = env::var("TARGET").is_ok_and(|target| target.contains("windows"));
    let suffix = if windows { ".exe" } else { "" };
    let mut generated = String::from("pub const BINARIES: &[EmbeddedBinary] = &[\n");

    for &(name, filename, relative, shortcut, description) in PAYLOADS {
        let payload = base.join(format!("{relative}{suffix}"));
        println!("cargo:rerun-if-changed={}", payload.display());
        assert!(
            payload.is_file(),
            "missing installer payload {}; build release engines/updater and desktop clients first",
            payload.display()
        );
        writeln!(
            generated,
            "    EmbeddedBinary {{ name: {name:?}, filename: {filename:?}, data: include_bytes!({path:?}), create_shortcut: {shortcut}, description: {description:?} }},",
            path = payload.to_string_lossy(),
        )
        .expect("write generated payload table");
    }
    generated.push_str("];\n");
    fs::write(out_dir.join("embedded_payload.rs"), generated)
        .expect("write generated installer payload table");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_target_base_for_native_and_explicit_targets() {
        assert_eq!(
            artifact_base(Path::new(
                r"C:\repo\target\release\build\installer-hash\out"
            )),
            PathBuf::from(r"C:\repo\target")
        );
        assert_eq!(
            artifact_base(Path::new(
                r"C:\repo\target\desktop-release\build\installer-hash\out"
            )),
            PathBuf::from(r"C:\repo\target")
        );
        assert_eq!(
            artifact_base(Path::new(
                r"C:\repo\target\aarch64-pc-windows-msvc\desktop-release\build\installer-hash\out"
            )),
            PathBuf::from(r"C:\repo\target\aarch64-pc-windows-msvc")
        );
    }

    #[test]
    fn installer_payloads_use_the_maximally_optimized_release_profile() {
        assert!(
            PAYLOADS
                .iter()
                .all(|(_, _, relative, _, _)| relative.starts_with("release/"))
        );
        assert!(PAYLOADS.iter().any(|(_, filename, relative, _, _)| {
            *filename == "mujrim-v60-external" && *relative == "release/mujrim-v60-external"
        }));
        assert!(PAYLOADS.iter().any(|(_, filename, relative, _, _)| {
            *filename == "mujrim-v60-embedded" && *relative == "release/mujrim-v60-embedded"
        }));
    }
}
