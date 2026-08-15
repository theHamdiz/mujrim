//! Contracts for GitHub workflow efficiency and required CI coverage.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn read(rel: impl AsRef<Path>) -> String {
        let path = workspace_root().join(rel);
        fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read {}: {err}", path.display());
        })
    }

    fn ci() -> String {
        read(".github/workflows/ci.yml")
    }

    fn release() -> String {
        read(".github/workflows/release.yml")
    }

    fn packaging_script() -> String {
        read("scripts/package-dist-windows.ps1")
    }

    fn pins_serial_codegen(src: &str) -> bool {
        src.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == "CARGO_BUILD_JOBS: \"1\""
                || trimmed.contains("$env:CARGO_BUILD_JOBS = \"1\"")
        })
    }

    #[test]
    fn ci_and_release_do_not_serialize_codegen() {
        assert!(
            !pins_serial_codegen(&ci()),
            "ci.yml must not set CARGO_BUILD_JOBS=1"
        );
        assert!(
            !pins_serial_codegen(&release()),
            "release.yml must not set CARGO_BUILD_JOBS=1"
        );
        assert!(
            !pins_serial_codegen(&packaging_script()),
            "package-dist-windows.ps1 must not pin CARGO_BUILD_JOBS=1"
        );
    }

    #[test]
    fn ci_runs_fmt_once_on_linux_x86_64() {
        let src = ci();
        assert!(src.contains("if: matrix.fmt"));
        assert_eq!(
            src.matches("fmt: true").count(),
            1,
            "formatting should run on exactly one native job"
        );
        assert!(src.contains("cargo fmt --all -- --check"));
    }

    #[test]
    fn ci_clippy_is_dev_profile_deny_warnings() {
        let src = ci();
        assert!(src.contains("cargo clippy --workspace --all-targets -- -D warnings"));
        assert!(
            !src.contains("cargo clippy --release"),
            "clippy --release uses fat LTO and dominates wall time"
        );
    }

    #[test]
    fn ci_tests_match_quality_gate_profile() {
        let src = ci();
        assert!(src.contains("cargo test --workspace"));
        assert!(
            !src.contains("release-test"),
            "CI should not rebuild the workspace under release-test"
        );
        assert!(
            !src.contains("cargo build --release --workspace"),
            "CI check must not fat-LTO the whole workspace"
        );
    }

    #[test]
    fn ci_native_matrix_covers_required_hosts() {
        let src = ci();
        for os in [
            "ubuntu-latest",
            "ubuntu-24.04-arm",
            "macos-latest",
            "windows-latest",
            "windows-11-arm",
        ] {
            assert!(src.contains(os), "missing native runner {os}");
        }
    }

    #[test]
    fn ci_smoke_checks_uci_handshake() {
        let src = ci();
        assert!(src.contains("uciok"));
        assert!(src.contains("Mujrim 1.0.0"));
        assert!(src.contains("--backend"));
        assert!(src.contains("universal"));
    }

    #[test]
    fn ci_cross_uses_prebuilt_cross_and_required_targets() {
        let src = ci();
        assert!(src.contains("taiki-e/install-action@v2"));
        assert!(src.contains("tool: cross"));
        assert!(
            !src.contains("cargo install cross"),
            "installing cross from git on every job is too slow"
        );
        for target in [
            "aarch64-unknown-linux-gnu",
            "armv7-unknown-linux-gnueabihf",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
        ] {
            assert!(src.contains(target), "missing cross target {target}");
        }
    }

    #[test]
    fn release_artifacts_use_fat_lto_release_profile() {
        let src = release();
        assert!(src.contains("cargo build --release"));
        assert!(src.contains("CARGO_PROFILE_RELEASE_LTO: fat"));
        assert!(
            !src.contains("--profile"),
            "release.yml must not select a weaker Cargo profile for dist artifacts"
        );
        assert!(!src.contains("desktop-release"));
        let justfile = read("justfile");
        assert!(justfile.contains("CARGO_PROFILE_RELEASE_LTO=fat"));
        assert!(justfile.contains("dist: release"));
        assert!(justfile.contains("dist_cargo_env"));
        let manifest = read("Cargo.toml");
        assert!(manifest.contains("lto = \"fat\""));
        assert!(manifest.contains("codegen-units = 1"));
        assert!(manifest.contains("opt-level = 3"));
    }

    #[test]
    fn release_cross_matches_ci_install_strategy() {
        let src = release();
        assert!(src.contains("taiki-e/install-action@v2"));
        assert!(src.contains("tool: cross"));
        assert!(!src.contains("cargo install cross"));
        assert!(src.contains("armv7-unknown-linux-gnueabihf"));
        assert!(src.contains("x86_64-unknown-linux-musl"));
        assert!(src.contains("aarch64-unknown-linux-musl"));
        assert!(src.contains("uciok"));
        assert!(src.contains("Mujrim 1.0.0"));
    }

    #[test]
    fn shared_actions_exist() {
        let root = workspace_root();
        assert!(root.join(".github/actions/fetch-nnue/action.yml").is_file());
        assert!(
            root.join(".github/actions/linux-gui-deps/action.yml")
                .is_file()
        );
        assert!(ci().contains("./.github/actions/fetch-nnue"));
        assert!(ci().contains("./.github/actions/linux-gui-deps"));
        assert!(release().contains("./.github/actions/fetch-nnue"));
        assert!(release().contains("./.github/actions/linux-gui-deps"));
    }
}
