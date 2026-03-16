//! KishMat Updater — a binary batch updater that pulls updates from GitHub.
//!
//! Commands:
//! - `check`          — Check for available updates
//! - `update all`     — Update all components
//! - `update <crate>` — Update a specific component (e.g., `kishmat-search`)
//! - `list`           — List installed components and versions

use clap::{Arg, Command};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

/// GitHub repository owner and name.
const REPO_OWNER: &str = "theHamdiz";
const REPO_NAME: &str = "kishmat";

/// Components that can be individually updated.
const COMPONENTS: &[&str] = &[
    "kishmat",         // Main engine binary
    "kishmat-ui",      // GUI application
    "kishmat-updater", // This updater itself
];

/// A GitHub release asset.
#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// A GitHub release.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    name: Option<String>,
    assets: Vec<Asset>,
    published_at: String,
}

fn main() {
    let matches = Command::new("KishMat Updater")
        .version("2.0.0")
        .author("Ahmad Hamdi Emara")
        .about("Updates KishMat chess engine components from GitHub releases")
        .subcommand(
            Command::new("check")
                .about("Check for available updates"),
        )
        .subcommand(
            Command::new("update")
                .about("Update components")
                .arg(
                    Arg::new("component")
                        .help("Component to update (or 'all')")
                        .default_value("all")
                        .index(1),
                ),
        )
        .subcommand(
            Command::new("list")
                .about("List installed components"),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("check", _)) => cmd_check(),
        Some(("update", sub)) => {
            let component = sub.get_one::<String>("component").unwrap();
            cmd_update(component);
        }
        Some(("list", _)) => cmd_list(),
        _ => {
            cmd_check();
        }
    }
}

/// Check for the latest release on GitHub.
fn cmd_check() {
    println!("╔══════════════════════════════════════╗");
    println!("║     KishMat Update Checker v2.0.0    ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    match fetch_latest_release() {
        Ok(release) => {
            println!("  Latest release: {} ({})", release.tag_name, release.published_at);
            if let Some(ref name) = release.name {
                println!("  Name: {name}");
            }
            println!();

            if release.assets.is_empty() {
                println!("  No binary assets available for download.");
                println!("  You may need to build from source.");
            } else {
                println!("  Available assets:");
                for asset in &release.assets {
                    let size_mb = asset.size as f64 / (1024.0 * 1024.0);
                    println!("    • {} ({:.1} MB)", asset.name, size_mb);
                }
            }

            println!();
            println!("  Current version: 2.0.0");
            if release.tag_name.contains("2.0.0") {
                println!("  ✓ You are up to date!");
            } else {
                println!("  ⚡ Update available! Run: kishmat-updater update all");
            }
        }
        Err(e) => {
            eprintln!("  ✗ Failed to check for updates: {e}");
            eprintln!("  Check your internet connection and try again.");
        }
    }
}

/// Update one or all components.
fn cmd_update(component: &str) {
    println!("╔══════════════════════════════════════╗");
    println!("║       KishMat Updater v2.0.0         ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    let release = match fetch_latest_release() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  ✗ Failed to fetch release: {e}");
            return;
        }
    };

    println!("  Latest release: {}", release.tag_name);

    let install_dir = get_install_dir();
    println!("  Install directory: {}", install_dir.display());

    if component == "all" {
        println!("  Updating all components...");
        for comp in COMPONENTS {
            update_component(&release, comp, &install_dir);
        }
    } else {
        update_component(&release, component, &install_dir);
    }

    println!();
    println!("  ✓ Update complete!");
}

/// List installed components.
fn cmd_list() {
    println!("╔══════════════════════════════════════╗");
    println!("║     KishMat Components v2.0.0        ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    let install_dir = get_install_dir();
    println!("  Install directory: {}", install_dir.display());
    println!();

    for comp in COMPONENTS {
        let path = install_dir.join(comp);
        if path.exists() {
            let metadata = fs::metadata(&path).ok();
            let size = metadata.map(|m| m.len()).unwrap_or(0);
            let size_mb = size as f64 / (1024.0 * 1024.0);
            println!("  ✓ {comp:<20} ({:.1} MB)", size_mb);
        } else {
            println!("  ✗ {comp:<20} (not installed)");
        }
    }
}

/// Fetch the latest release from GitHub API.
fn fetch_latest_release() -> Result<Release, String> {
    let url = format!(
        "https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest"
    );

    let client = reqwest::blocking::Client::builder()
        .user_agent("kishmat-updater/2.0.0")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("GitHub API error: {}", response.status()));
    }

    response
        .json::<Release>()
        .map_err(|e| format!("JSON parse error: {e}"))
}

/// Update a single component from the release assets.
fn update_component(release: &Release, component: &str, install_dir: &PathBuf) {
    // Look for an asset matching this component name
    let target_os = std::env::consts::OS;
    let target_arch = std::env::consts::ARCH;

    let matches: Vec<&Asset> = release.assets.iter()
        .filter(|a| {
            let name = a.name.to_lowercase();
            name.contains(component) &&
            (name.contains(target_os) || name.contains("universal")) &&
            (name.contains(target_arch) || name.contains("universal"))
        })
        .collect();

    let asset = match matches.first() {
        Some(a) => a,
        None => {
            println!("    ⊘ {component}: no matching asset for {target_os}-{target_arch}");
            return;
        }
    };

    println!("    ↓ Downloading {}...", asset.name);

    match download_asset(asset, install_dir) {
        Ok(path) => println!("    ✓ {component}: installed to {}", path.display()),
        Err(e) => eprintln!("    ✗ {component}: {e}"),
    }
}

/// Download an asset and save it to the install directory.
fn download_asset(asset: &Asset, install_dir: &PathBuf) -> Result<PathBuf, String> {
    fs::create_dir_all(install_dir)
        .map_err(|e| format!("Failed to create directory: {e}"))?;

    let dest_path = install_dir.join(&asset.name);

    let client = reqwest::blocking::Client::builder()
        .user_agent("kishmat-updater/2.0.0")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download error: {}", response.status()));
    }

    let total_size = asset.size;
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );

    let mut file = fs::File::create(&dest_path)
        .map_err(|e| format!("Failed to create file: {e}"))?;

    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 8192];

    loop {
        let n = response.read(&mut buffer)
            .map_err(|e| format!("Read error: {e}"))?;
        if n == 0 { break; }

        file.write_all(&buffer[..n])
            .map_err(|e| format!("Write error: {e}"))?;
        hasher.update(&buffer[..n]);

        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_and_clear();

    let hash = format!("{:x}", hasher.finalize());
    println!("    ⊕ SHA256: {}", &hash[..16]);

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !asset.name.ends_with(".zip") && !asset.name.ends_with(".tar.gz") {
            let _ = fs::set_permissions(&dest_path, fs::Permissions::from_mode(0o755));
        }
    }

    Ok(dest_path)
}

/// Get the installation directory.
fn get_install_dir() -> PathBuf {
    // Try $HOME/.local/bin first, then fall back to current directory
    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local").join("bin");
        if local_bin.exists() || fs::create_dir_all(&local_bin).is_ok() {
            return local_bin;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
