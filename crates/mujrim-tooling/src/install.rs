use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use crate::process::run;

const BINS: &[&str] = &["mujrim", "mujrim-ui", "mujrim-updater"];

#[derive(Debug)]
pub struct InstallAction;

impl ToolAction for InstallAction {
    fn run(&self) -> Result<(), String> {
        let environment = [("CARGO_BUILD_JOBS", "1")];
        run(
            "cargo",
            &[
                "build",
                "--release",
                "--workspace",
                "--exclude",
                "mujrim-ui",
                "--exclude",
                "mujrim-installer",
            ],
            &environment,
        )?;
        run(
            "cargo",
            &["build", "--release", "-p", "mujrim-ui"],
            &environment,
        )?;

        match env::consts::OS {
            "linux" => install_linux(),
            "macos" => install_macos(),
            "windows" => install_windows(),
            other => Err(format!("unsupported OS: {other}")),
        }
    }
}

fn install_linux() -> Result<(), String> {
    let home = home_dir()?;
    let bin_dir = home.join(".local/bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    for bin in BINS {
        install_bin(bin, &bin_dir, false)?;
    }

    let desktop_dir = home.join(".local/share/applications");
    fs::create_dir_all(&desktop_dir)
        .map_err(|e| format!("failed to create {}: {e}", desktop_dir.display()))?;

    let desktop_file = desktop_dir.join("mujrim.desktop");
    let desktop = format!(
        "[Desktop Entry]\nName=Mujrim Chess\nExec={}/mujrim-ui\nType=Application\nCategories=Game;BoardGame;\n",
        bin_dir.display()
    );
    fs::write(&desktop_file, desktop)
        .map_err(|e| format!("failed to write {}: {e}", desktop_file.display()))?;

    println!("installed binaries to {}", bin_dir.display());
    Ok(())
}

fn install_macos() -> Result<(), String> {
    let home = home_dir()?;
    let bin_dir = home.join(".local/bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    for bin in BINS {
        install_bin(bin, &bin_dir, false)?;
    }

    let app_dir = home.join("Applications/Mujrim.app/Contents/MacOS");
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("failed to create {}: {e}", app_dir.display()))?;
    fs::copy("target/release/mujrim-ui", app_dir.join("Mujrim"))
        .map_err(|e| format!("failed to copy app binary: {e}"))?;
    println!("installed binaries to {}", bin_dir.display());
    Ok(())
}

fn install_windows() -> Result<(), String> {
    let local = env::var("LOCALAPPDATA").map_err(|e| format!("LOCALAPPDATA not set: {e}"))?;
    let bin_dir = PathBuf::from(local).join("Mujrim/bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    for bin in BINS {
        install_bin(bin, &bin_dir, true)?;
    }

    println!("installed binaries to {}", bin_dir.display());
    Ok(())
}

fn install_bin(bin: &str, target_dir: &Path, exe_suffix: bool) -> Result<(), String> {
    let src = binary_source(bin, exe_suffix);
    if !src.exists() {
        return Ok(());
    }

    let dst = if exe_suffix {
        target_dir.join(format!("{bin}.exe"))
    } else {
        target_dir.join(bin)
    };

    fs::copy(&src, &dst)
        .map_err(|e| format!("failed to copy {} to {}: {e}", src.display(), dst.display()))?;
    Ok(())
}

fn binary_source(bin: &str, exe_suffix: bool) -> PathBuf {
    let suffix = if exe_suffix { ".exe" } else { "" };
    PathBuf::from(format!("target/release/{bin}{suffix}"))
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|e| format!("HOME not set: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_bin_paths() {
        let dst = PathBuf::from("/tmp/mujrim-bin");
        let file = if cfg!(windows) {
            dst.join("mujrim.exe")
        } else {
            dst.join("mujrim")
        };
        assert!(file.ends_with(if cfg!(windows) {
            "mujrim.exe"
        } else {
            "mujrim"
        }));
    }

    #[test]
    fn all_binaries_use_the_maximally_optimized_release_profile() {
        assert_eq!(
            binary_source("mujrim-ui", true),
            PathBuf::from("target/release/mujrim-ui.exe")
        );
        assert_eq!(
            binary_source("mujrim", true),
            PathBuf::from("target/release/mujrim.exe")
        );
    }
}
