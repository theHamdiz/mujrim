use std::fs;
use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use crate::process::{output, run};

const BINS: &[&str] = &["kishmat", "kishmat-ui", "kishmat-updater"];

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum ReleaseTarget {
    Native,
    Darwin,
    Linux,
    Win,
    Full,
}

#[derive(Debug)]
pub struct ReleaseAction {
    pub target: ReleaseTarget,
}

impl ToolAction for ReleaseAction {
    fn run(&self) -> Result<(), String> {
        match self.target {
            ReleaseTarget::Native => build_native(),
            ReleaseTarget::Darwin => build_darwin(),
            ReleaseTarget::Linux => build_linux(),
            ReleaseTarget::Win => build_windows(),
            ReleaseTarget::Full => {
                build_darwin()?;
                build_linux()?;
                build_windows()?;
                Ok(())
            }
        }
    }
}

fn build_native() -> Result<(), String> {
    run(
        "cargo",
        &["build", "--release", "--workspace"],
        &[("RUSTFLAGS", "-C target-cpu=native")],
    )
}

fn build_darwin() -> Result<(), String> {
    let dist = Path::new("dist/darwin");
    prepare_dist(dist)?;

    run(
        "cargo",
        &[
            "build",
            "--release",
            "--workspace",
            "--target",
            "aarch64-apple-darwin",
        ],
        &[("RUSTFLAGS", "-C target-cpu=apple-m1")],
    )?;
    copy_binaries(
        Path::new("target/aarch64-apple-darwin/release"),
        &dist.join("aarch64"),
    )?;

    let installed = installed_targets()?;
    if installed.contains("x86_64-apple-darwin") {
        run(
            "cargo",
            &[
                "build",
                "--release",
                "--workspace",
                "--target",
                "x86_64-apple-darwin",
            ],
            &[],
        )?;
        copy_binaries(
            Path::new("target/x86_64-apple-darwin/release"),
            &dist.join("x86_64"),
        )?;
    }
    Ok(())
}

fn build_linux() -> Result<(), String> {
    let dist = Path::new("dist/linux");
    prepare_dist(dist)?;

    let installed = installed_targets()?;
    if installed.contains("x86_64-unknown-linux-gnu") {
        run(
            "cargo",
            &[
                "build",
                "--release",
                "--workspace",
                "--target",
                "x86_64-unknown-linux-gnu",
            ],
            &[],
        )?;
        copy_binaries(
            Path::new("target/x86_64-unknown-linux-gnu/release"),
            &dist.join("x86_64"),
        )?;
    }

    if installed.contains("aarch64-unknown-linux-gnu") {
        run(
            "cargo",
            &[
                "build",
                "--release",
                "--workspace",
                "--target",
                "aarch64-unknown-linux-gnu",
            ],
            &[],
        )?;
        copy_binaries(
            Path::new("target/aarch64-unknown-linux-gnu/release"),
            &dist.join("aarch64"),
        )?;
    }

    Ok(())
}

fn build_windows() -> Result<(), String> {
    let dist = Path::new("dist/windows/x86_64");
    if dist.exists() {
        fs::remove_dir_all(dist)
            .map_err(|e| format!("failed to remove {}: {e}", dist.display()))?;
    }
    fs::create_dir_all(dist).map_err(|e| format!("failed to create {}: {e}", dist.display()))?;

    let target = "x86_64-pc-windows-gnu";
    let installed = installed_targets()?;
    if !installed.contains(target) {
        run("rustup", &["target", "add", target], &[])?;
    }

    run(
        "cargo",
        &["build", "--release", "--workspace", "--target", target],
        &[],
    )?;

    for bin in BINS {
        let src = PathBuf::from(format!("target/{target}/release/{bin}.exe"));
        if src.exists() {
            let dst = dist.join(format!("{bin}.exe"));
            fs::copy(&src, &dst).map_err(|e| {
                format!("failed to copy {} to {}: {e}", src.display(), dst.display())
            })?;
        }
    }
    Ok(())
}

fn prepare_dist(dist: &Path) -> Result<(), String> {
    if dist.exists() {
        fs::remove_dir_all(dist)
            .map_err(|e| format!("failed to remove {}: {e}", dist.display()))?;
    }
    fs::create_dir_all(dist.join("aarch64"))
        .map_err(|e| format!("failed to create {}: {e}", dist.join("aarch64").display()))?;
    fs::create_dir_all(dist.join("x86_64"))
        .map_err(|e| format!("failed to create {}: {e}", dist.join("x86_64").display()))?;
    Ok(())
}

fn copy_binaries(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("failed to create {}: {e}", to.display()))?;
    for bin in BINS {
        let src = from.join(bin);
        if src.exists() {
            let dst = to.join(bin);
            fs::copy(&src, &dst).map_err(|e| {
                format!("failed to copy {} to {}: {e}", src.display(), dst.display())
            })?;
        }
    }
    Ok(())
}

fn installed_targets() -> Result<std::collections::HashSet<String>, String> {
    let out = output("rustup", &["target", "list", "--installed"])?;
    Ok(out.lines().map(|s| s.trim().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    #[test]
    fn release_bins_are_stable() {
        assert_eq!(BINS, &["kishmat", "kishmat-ui", "kishmat-updater"]);
    }

    #[test]
    fn release_target_value_enum_roundtrip() {
        assert_eq!(
            ReleaseTarget::from_str("native", true),
            Ok(ReleaseTarget::Native)
        );
        assert_eq!(
            ReleaseTarget::from_str("full", true),
            Ok(ReleaseTarget::Full)
        );
    }
}
