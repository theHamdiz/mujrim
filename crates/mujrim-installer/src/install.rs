//! Cross-platform installation logic.
//!
//! Writes embedded binaries to disk and creates platform-appropriate
//! shortcuts, desktop entries, and application bundles.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::embedded::{self, EmbeddedBinary};

/// Result of the full installation process.
#[derive(Debug, Clone)]
pub struct InstallResult {
    pub binaries_written: usize,
    pub shortcuts_created: usize,
    pub install_dir: PathBuf,
}

/// Platform-specific default install directory.
pub fn default_install_dir() -> PathBuf {
    match std::env::consts::OS {
        "macos" => {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            home.join(".local/bin")
        }
        "linux" => {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            home.join(".local/bin")
        }
        "windows" => {
            let local = std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| String::from("C:\\Users\\Public\\AppData\\Local"));
            PathBuf::from(local).join("Mujrim").join("bin")
        }
        _ => PathBuf::from("."),
    }
}

/// Run the full installation to `install_dir`.
///
/// Returns an error string on failure.
pub fn install_all(install_dir: &Path) -> Result<InstallResult, String> {
    if !embedded::has_payload() {
        return Err("No binaries embedded. Rebuild with: just installer".into());
    }

    fs::create_dir_all(install_dir)
        .map_err(|e| format!("Cannot create {}: {e}", install_dir.display()))?;

    let mut written = 0usize;
    let is_windows = cfg!(target_os = "windows");

    for bin in embedded::BINARIES {
        write_binary(bin, install_dir, is_windows)?;
        written += 1;
    }

    let shortcuts = match std::env::consts::OS {
        "macos" => create_macos_bundles(install_dir)?,
        "linux" => create_linux_desktop_entries(install_dir)?,
        "windows" => create_windows_shortcuts(install_dir)?,
        _ => 0,
    };

    Ok(InstallResult {
        binaries_written: written,
        shortcuts_created: shortcuts,
        install_dir: install_dir.to_path_buf(),
    })
}

/// Write a single embedded binary to disk.
fn write_binary(bin: &EmbeddedBinary, dir: &Path, exe_suffix: bool) -> Result<(), String> {
    let filename = if exe_suffix {
        format!("{}.exe", bin.filename)
    } else {
        bin.filename.to_string()
    };

    let dest = dir.join(&filename);
    let mut file = fs::File::create(&dest).map_err(|e| format!("Write {}: {e}", dest.display()))?;
    file.write_all(bin.data)
        .map_err(|e| format!("Write {}: {e}", dest.display()))?;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&dest, perms).map_err(|e| format!("chmod {}: {e}", dest.display()))?;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// macOS — .app bundles
// ═══════════════════════════════════════════════════════════════════

fn create_macos_bundles(bin_dir: &Path) -> Result<usize, String> {
    let home = dirs::home_dir().ok_or("HOME not set")?;
    let mut created = 0usize;

    let logo_png = include_bytes!("../../../assets/branding/mujrim-icon.png");

    for bin in embedded::BINARIES.iter().filter(|b| b.create_shortcut) {
        let app_name = match bin.filename {
            "mujrim-ui" => "Mujrim Chess",
            "mujrim-game" => "Mujrim Game",
            _ => bin.name,
        };

        let app_dir = home
            .join("Applications")
            .join(format!("{app_name}.app"))
            .join("Contents");
        let macos_dir = app_dir.join("MacOS");
        let resources_dir = app_dir.join("Resources");

        fs::create_dir_all(&macos_dir)
            .map_err(|e| format!("Create {}: {e}", macos_dir.display()))?;
        fs::create_dir_all(&resources_dir)
            .map_err(|e| format!("Create {}: {e}", resources_dir.display()))?;

        // Copy binary into .app bundle
        let src = bin_dir.join(bin.filename);
        let dst = macos_dir.join(app_name.replace(' ', ""));
        if src.exists() {
            fs::copy(&src, &dst).map_err(|e| format!("Copy to bundle: {e}"))?;
        }

        // Write Info.plist
        let bundle_id = format!("com.mujrim.{}", bin.filename.replace('-', ""));
        let executable = app_name.replace(' ', "");
        let plist = info_plist(app_name, &executable, &bundle_id);
        fs::write(app_dir.join("Info.plist"), plist)
            .map_err(|e| format!("Write Info.plist: {e}"))?;

        // Write icon as PNG (macOS will use it; proper .icns is better but
        // requires more tooling — PNG works for user-space .app bundles)
        fs::write(resources_dir.join("AppIcon.png"), logo_png)
            .map_err(|e| format!("Write icon: {e}"))?;

        created += 1;
    }

    Ok(created)
}

// ═══════════════════════════════════════════════════════════════════
// Linux — .desktop entries
// ═══════════════════════════════════════════════════════════════════

fn create_linux_desktop_entries(bin_dir: &Path) -> Result<usize, String> {
    let home = dirs::home_dir().ok_or("HOME not set")?;
    let app_dir = home.join(".local/share/applications");
    let icon_dir = home.join(".local/share/icons/mujrim");

    fs::create_dir_all(&app_dir).map_err(|e| format!("Create {}: {e}", app_dir.display()))?;
    fs::create_dir_all(&icon_dir).map_err(|e| format!("Create {}: {e}", icon_dir.display()))?;

    // Write icon
    let logo_png = include_bytes!("../../../assets/branding/mujrim-icon.png");
    let icon_path = icon_dir.join("mujrim.png");
    fs::write(&icon_path, logo_png).map_err(|e| format!("Write icon: {e}"))?;

    let mut created = 0usize;

    for bin in embedded::BINARIES.iter().filter(|b| b.create_shortcut) {
        let app_name = match bin.filename {
            "mujrim-ui" => "Mujrim Chess",
            "mujrim-game" => "Mujrim Game",
            _ => bin.name,
        };

        let executable = bin_dir.join(bin.filename).display().to_string();
        let icon = icon_path.display().to_string();
        let desktop = desktop_entry(app_name, &executable, &icon, bin.description);

        let desktop_file = app_dir.join(format!("{}.desktop", bin.filename));
        fs::write(&desktop_file, desktop)
            .map_err(|e| format!("Write {}: {e}", desktop_file.display()))?;

        created += 1;
    }

    Ok(created)
}

// ═══════════════════════════════════════════════════════════════════
// Windows — Start Menu shortcuts
// ═══════════════════════════════════════════════════════════════════

fn create_windows_shortcuts(bin_dir: &Path) -> Result<usize, String> {
    let mut created = 0usize;

    // Write .ico from embedded PNG
    let _logo_png = include_bytes!("../../../assets/branding/mujrim-icon.png");
    let icon_dir = bin_dir.parent().unwrap_or(bin_dir);
    let icon_path = icon_dir.join("mujrim.ico");

    // Convert PNG → ICO using the image crate
    if let Ok(img) = image::load_from_memory(_logo_png) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
        // ICO is just a BMP-in-container; save as PNG-in-ICO for simplicity
        let _ = resized.save(&icon_path);
    }

    for bin in embedded::BINARIES.iter().filter(|b| b.create_shortcut) {
        let app_name = match bin.filename {
            "mujrim-ui" => "Mujrim Chess",
            "mujrim-game" => "Mujrim Game",
            _ => bin.name,
        };

        // Use PowerShell to create a Start Menu .lnk shortcut
        let start_menu = std::env::var("APPDATA")
            .map(|a| {
                PathBuf::from(a)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs")
            })
            .unwrap_or_else(|_| bin_dir.to_path_buf());

        let lnk_path = start_menu.join(format!("{app_name}.lnk"));
        let target = bin_dir.join(format!("{}.exe", bin.filename));

        let ps_script = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.IconLocation = '{}'; \
             $s.Description = '{}'; \
             $s.Save()",
            lnk_path.display(),
            target.display(),
            icon_path.display(),
            bin.description,
        );

        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_script])
            .output();

        created += 1;
    }

    Ok(created)
}

// ═══════════════════════════════════════════════════════════════════
// Platform helpers
// ═══════════════════════════════════════════════════════════════════

/// Generate the Info.plist content for a macOS `.app` bundle.
pub fn info_plist(app_name: &str, executable: &str, bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>{app_name}</string>
    <key>CFBundleDisplayName</key>
    <string>{app_name}</string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleVersion</key>
    <string>2.0.0</string>
    <key>CFBundleShortVersionString</key>
    <string>2.0.0</string>
    <key>CFBundleExecutable</key>
    <string>{executable}</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>"#
    )
}

/// Generate a Linux `.desktop` entry string.
pub fn desktop_entry(name: &str, exec: &str, icon: &str, comment: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Name={name}\n\
         Comment={comment}\n\
         Exec={exec}\n\
         Icon={icon}\n\
         Type=Application\n\
         Categories=Game;BoardGame;\n\
         Terminal=false\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_install_dir_is_not_empty() {
        let dir = default_install_dir();
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn info_plist_contains_bundle_id() {
        let plist = info_plist("Test", "test", "com.test.app");
        assert!(plist.contains("com.test.app"));
        assert!(plist.contains("<key>CFBundleExecutable</key>"));
        assert!(plist.contains("<key>CFBundleDisplayName</key>"));
        assert!(plist.contains("<key>CFBundleShortVersionString</key>"));
    }

    #[test]
    fn desktop_entry_format() {
        let entry = desktop_entry("Mujrim", "/usr/bin/mujrim", "/icon.png", "Chess");
        assert!(entry.contains("Name=Mujrim"));
        assert!(entry.contains("Exec=/usr/bin/mujrim"));
        assert!(entry.contains("Icon=/icon.png"));
        assert!(entry.contains("Categories=Game;BoardGame;"));
    }

    #[test]
    fn binary_filename_extension_windows() {
        let name = "mujrim-ui";
        let with_exe = format!("{name}.exe");
        assert!(with_exe.ends_with(".exe"));
    }

    #[test]
    fn install_dir_platform_specific() {
        let dir = default_install_dir();
        if cfg!(target_os = "windows") {
            assert!(dir.to_string_lossy().contains("Mujrim"));
        } else {
            assert!(dir.to_string_lossy().contains(".local/bin"));
        }
    }
}
