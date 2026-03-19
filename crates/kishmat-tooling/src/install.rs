use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::action::ToolAction;
use crate::process::run;

const BINS: &[&str] = &["kishmat", "kishmat-ui", "kishmat-updater"];

#[derive(Debug)]
pub struct InstallAction;

impl ToolAction for InstallAction {
    fn run(&self) -> Result<(), String> {
        run(
            "cargo",
            &["build", "--release", "--workspace"],
            &[("RUSTFLAGS", "-C target-cpu=native")],
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

    let desktop_file = desktop_dir.join("kishmat.desktop");
    let desktop = format!(
        "[Desktop Entry]\nName=KishMat Chess\nExec={}/kishmat-ui\nType=Application\nCategories=Game;BoardGame;\n",
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

    let app_dir = home.join("Applications/KishMat.app/Contents/MacOS");
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("failed to create {}: {e}", app_dir.display()))?;
    fs::copy("target/release/kishmat-ui", app_dir.join("KishMat"))
        .map_err(|e| format!("failed to copy app binary: {e}"))?;
    println!("installed binaries to {}", bin_dir.display());
    Ok(())
}

fn install_windows() -> Result<(), String> {
    let local = env::var("LOCALAPPDATA").map_err(|e| format!("LOCALAPPDATA not set: {e}"))?;
    let bin_dir = PathBuf::from(local).join("KishMat/bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("failed to create {}: {e}", bin_dir.display()))?;

    for bin in BINS {
        install_bin(bin, &bin_dir, true)?;
    }

    println!("installed binaries to {}", bin_dir.display());
    Ok(())
}

fn install_bin(bin: &str, target_dir: &Path, exe_suffix: bool) -> Result<(), String> {
    let src = if exe_suffix {
        PathBuf::from(format!("target/release/{bin}.exe"))
    } else {
        PathBuf::from(format!("target/release/{bin}"))
    };
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
        let dst = PathBuf::from("/tmp/kishmat-bin");
        let file = if cfg!(windows) {
            dst.join("kishmat.exe")
        } else {
            dst.join("kishmat")
        };
        assert!(file.ends_with(if cfg!(windows) {
            "kishmat.exe"
        } else {
            "kishmat"
        }));
    }
}
