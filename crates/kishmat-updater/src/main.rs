//! KishMat Updater — pulls updates from GitHub releases for all components.
//!
//! Commands:
//! - `check`          — Check for available updates
//! - `update all`     — Update all components
//! - `update <crate>` — Update a specific component (e.g., `kishmat-ui`)
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
const VERSION: &str = "2.0.0";

/// All components (binaries) that can be updated.
const COMPONENTS: &[&str] = &[
    "kishmat",         // Main engine binary (UCI)
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
        .version(VERSION)
        .author("Ahmad Hamdi Emara")
        .about("Updates KishMat chess engine components from GitHub releases")
        .subcommand(Command::new("check").about("Check for available updates"))
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
        .subcommand(Command::new("list").about("List installed components"))
        .subcommand(
            Command::new("syzygy")
                .about("Download Syzygy endgame tablebases")
                .arg(
                    Arg::new("pieces")
                        .short('p')
                        .long("pieces")
                        .help("Max pieces: 5 (standard ~1GB), 6 (extended ~150GB), 7 (full ~140TB)")
                        .default_value("5")
                        .value_parser(["5", "6", "7"]),
                )
                .arg(
                    Arg::new("dir")
                        .short('d')
                        .long("dir")
                        .help("Output directory (default: ./syzygy/)")
                        .default_value(kishmat_updater::syzygy::DEFAULT_SYZYGY_DIR),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("check", _)) => cmd_check(),
        Some(("update", sub)) => {
            let component = sub.get_one::<String>("component").unwrap();
            cmd_update(component);
        }
        Some(("list", _)) => cmd_list(),
        Some(("syzygy", sub)) => {
            let pieces = sub.get_one::<String>("pieces").unwrap();
            let dir = sub.get_one::<String>("dir").unwrap();
            cmd_syzygy(pieces, dir);
        }
        _ => cmd_check(),
    }
}

fn cmd_syzygy(pieces: &str, dir: &str) {
    use kishmat_updater::syzygy::*;

    let piece_set = match pieces {
        "6" => SyzygyPieceSet::Extended,
        "7" => SyzygyPieceSet::Full,
        _ => SyzygyPieceSet::Standard,
    };

    let dest = std::path::PathBuf::from(dir);

    println!("╔══════════════════════════════════════╗");
    println!("║   Syzygy Tablebase Downloader        ║");
    println!("╚══════════════════════════════════════╝");
    println!();
    println!("  Piece set:   {} ", piece_set);
    println!("  Destination: {}", dest.display());

    let tables = table_names(piece_set);
    println!("  Tables:      {} ({} files)", tables.len(), tables.len() * 2);
    println!();

    let progress: Option<ProgressCallback> = Some(Box::new(|idx, total, name, status| {
        match status {
            DownloadStatus::Skipped => println!("  [{idx}/{total}] skip {name}"),
            DownloadStatus::Downloading => print!("  [{idx}/{total}] downloading {name} ..."),
            DownloadStatus::Done => println!(" done"),
            DownloadStatus::Failed(e) => println!(" FAILED: {e}"),
        }
    }));

    match download_tables(&dest, piece_set, progress) {
        Ok(summary) => {
            println!();
            println!("  ✓ Download complete!");
            println!("    Downloaded: {}", summary.downloaded);
            println!("    Skipped:    {}", summary.skipped);
            println!("    Failed:     {}", summary.failed);

            let usage = disk_usage(&dest);
            let usage_mb = usage as f64 / (1024.0 * 1024.0);
            println!("    Disk usage: {:.1} MB", usage_mb);
            println!("    Path:       {}", summary.target_dir.display());
            println!();
            println!("  To use: setoption name SyzygyPath value {}", dest.display());
        }
        Err(e) => {
            eprintln!("  ✗ Download failed: {e}");
        }
    }
}

fn cmd_check() {
    println!("╔══════════════════════════════════════╗");
    println!("║     KishMat Update Checker v{VERSION}    ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    match fetch_latest_release() {
        Ok(release) => {
            println!("  Latest release: {} ({})", release.tag_name, release.published_at);
            if let Some(ref name) = release.name {
                println!("  Name: {name}");
            }
            println!();

            let platform = current_platform_tag();
            println!("  Your platform: {platform}");

            let matching: Vec<_> = release.assets.iter()
                .filter(|a| a.name.contains(&platform) || a.name.contains("universal"))
                .collect();

            if matching.is_empty() {
                println!("  No pre-built binaries for your platform.");
                println!("  Build from source: cargo build --release --workspace");
            } else {
                println!("  Available for your platform:");
                for asset in &matching {
                    let size_mb = asset.size as f64 / (1024.0 * 1024.0);
                    println!("    • {} ({:.1} MB)", asset.name, size_mb);
                }
            }

            println!();
            println!("  Current version: {VERSION}");
            if release.tag_name.contains(VERSION) {
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

fn cmd_update(component: &str) {
    println!("╔══════════════════════════════════════╗");
    println!("║       KishMat Updater v{VERSION}         ║");
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
    println!("  Platform: {}", current_platform_tag());
    println!();

    if component == "all" {
        // Try platform archive first (e.g., kishmat-v2.0.0-x86_64-apple-darwin.tar.gz)
        let platform = current_platform_tag();
        let archive = release.assets.iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains(&platform) && (n.ends_with(".tar.gz") || n.ends_with(".zip"))
        });

        if let Some(archive_asset) = archive {
            println!("  Found platform archive: {}", archive_asset.name);
            match download_and_extract(archive_asset, &install_dir) {
                Ok(count) => println!("  ✓ Extracted {count} files"),
                Err(e) => {
                    eprintln!("  ✗ Archive extraction failed: {e}");
                    eprintln!("  Falling back to individual downloads...");
                    for comp in COMPONENTS {
                        update_component(&release, comp, &install_dir);
                    }
                }
            }
        } else {
            println!("  No platform archive found, downloading individually...");
            for comp in COMPONENTS {
                update_component(&release, comp, &install_dir);
            }
        }
    } else if COMPONENTS.contains(&component) {
        update_component(&release, component, &install_dir);
    } else {
        eprintln!("  ✗ Unknown component: {component}");
        eprintln!("  Available: {}", COMPONENTS.join(", "));
        return;
    }

    println!();
    println!("  ✓ Update complete!");
}

fn cmd_list() {
    println!("╔══════════════════════════════════════╗");
    println!("║     KishMat Components v{VERSION}        ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    let install_dir = get_install_dir();
    println!("  Install directory: {}", install_dir.display());
    println!();

    for comp in COMPONENTS {
        let binary_name = if cfg!(windows) {
            format!("{comp}.exe")
        } else {
            comp.to_string()
        };
        let path = install_dir.join(&binary_name);
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

/// Build platform tag, e.g. "x86_64-apple-darwin" or "x86_64-pc-windows-msvc".
fn current_platform_tag() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match os {
        "macos" => format!("{arch}-apple-darwin"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        other => format!("{arch}-{other}"),
    }
}

/// Fetch the latest release from GitHub API.
fn fetch_latest_release() -> Result<Release, String> {
    let url = format!(
        "https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest"
    );

    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("kishmat-updater/{VERSION}"))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client.get(&url).send()
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("GitHub API error: {}", response.status()));
    }

    response.json::<Release>().map_err(|e| format!("JSON parse error: {e}"))
}

/// Update a single component from release assets.
fn update_component(release: &Release, component: &str, install_dir: &PathBuf) {
    let platform = current_platform_tag();
    let ext = if cfg!(windows) { ".exe" } else { "" };

    let matches: Vec<&Asset> = release.assets.iter()
        .filter(|a| {
            let name = a.name.to_lowercase();
            name.contains(component) &&
            (name.contains(&platform) || name.contains("universal")) &&
            !name.ends_with(".tar.gz") && !name.ends_with(".zip")
        })
        .collect();

    let asset = match matches.first() {
        Some(a) => a,
        None => {
            println!("    ⊘ {component}: no matching asset for {platform}");
            return;
        }
    };

    let dest_name = format!("{component}{ext}");
    println!("    ↓ Downloading {}...", asset.name);

    match download_to_file(asset, &install_dir.join(&dest_name)) {
        Ok(_) => {
            // Self-update: if updating the updater, rename current → .old, new → current
            if component == "kishmat-updater" {
                let current_exe = std::env::current_exe().ok();
                if let Some(ref exe_path) = current_exe {
                    let old_path = exe_path.with_extension("old");
                    let _ = fs::rename(exe_path, &old_path);
                    let _ = fs::rename(install_dir.join(&dest_name), exe_path);
                    println!("    ✓ {component}: self-updated (restart to use new version)");
                    return;
                }
            }
            println!("    ✓ {component}: installed");
        }
        Err(e) => eprintln!("    ✗ {component}: {e}"),
    }
}

/// Download and extract a platform archive (.tar.gz or .zip).
fn download_and_extract(asset: &Asset, install_dir: &PathBuf) -> Result<usize, String> {
    let temp_path = install_dir.join(&asset.name);
    download_to_file(asset, &temp_path)?;

    let count = if asset.name.ends_with(".tar.gz") || asset.name.ends_with(".tgz") {
        extract_tar_gz(&temp_path, install_dir)?
    } else if asset.name.ends_with(".zip") {
        extract_zip(&temp_path, install_dir)?
    } else {
        return Err(format!("Unknown archive format: {}", asset.name));
    };

    let _ = fs::remove_file(&temp_path);
    Ok(count)
}

/// Download an asset to a specific file path with progress bar.
fn download_to_file(asset: &Asset, dest: &PathBuf) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("kishmat-updater/{VERSION}"))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut response = client.get(&asset.browser_download_url).send()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download error: {}", response.status()));
    }

    let pb = ProgressBar::new(asset.size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );

    let mut file = fs::File::create(dest)
        .map_err(|e| format!("Failed to create file: {e}"))?;

    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 8192];

    loop {
        let n = response.read(&mut buffer).map_err(|e| format!("Read error: {e}"))?;
        if n == 0 { break; }
        file.write_all(&buffer[..n]).map_err(|e| format!("Write error: {e}"))?;
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
        if !asset.name.ends_with(".tar.gz") && !asset.name.ends_with(".zip")
            && !asset.name.ends_with(".tgz")
        {
            let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o755));
        }
    }

    Ok(())
}

/// Extract a .tar.gz archive into the target directory.
fn extract_tar_gz(archive: &PathBuf, dest: &PathBuf) -> Result<usize, String> {
    use std::io::BufReader;
    let file = fs::File::open(archive).map_err(|e| format!("Open failed: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    let mut count = 0usize;

    for entry in tar.entries().map_err(|e| format!("Tar error: {e}"))? {
        let mut entry = entry.map_err(|e| format!("Entry error: {e}"))?;
        let path = entry.path().map_err(|e| format!("Path error: {e}"))?.into_owned();

        // Only extract files (skip directories), flatten into dest
        if entry.header().entry_type().is_file() {
            let file_name = path.file_name()
                .ok_or_else(|| "No file name".to_string())?;
            let out_path = dest.join(file_name);
            entry.unpack(&out_path).map_err(|e| format!("Unpack error: {e}"))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(0o755));
            }

            println!("    ✓ extracted: {}", file_name.to_string_lossy());
            count += 1;
        }
    }
    Ok(count)
}

/// Extract a .zip archive into the target directory.
fn extract_zip(archive: &PathBuf, dest: &PathBuf) -> Result<usize, String> {
    let file = fs::File::open(archive).map_err(|e| format!("Open failed: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Zip error: {e}"))?;
    let mut count = 0usize;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("Zip entry error: {e}"))?;
        if entry.is_file() {
            let file_name = entry.name().rsplit('/').next().unwrap_or(entry.name()).to_string();
            let out_path = dest.join(&file_name);
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("Create error: {e}"))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Copy error: {e}"))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(0o755));
            }

            println!("    ✓ extracted: {file_name}");
            count += 1;
        }
    }
    Ok(count)
}

/// Get the installation directory for the current platform.
fn get_install_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let dir = PathBuf::from(local_app_data).join("KishMat").join("bin");
            if dir.exists() || fs::create_dir_all(&dir).is_ok() {
                return dir;
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local").join("bin");
        if local_bin.exists() || fs::create_dir_all(&local_bin).is_ok() {
            return local_bin;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
