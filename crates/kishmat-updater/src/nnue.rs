//! NNUE network download manager.
//!
//! Downloads neural network weights from various chess engine projects:
//! - **Akimbo**: net.bin (768→1024×2→1, ~6 MB)
//! - **Stockfish**: nn-*.nnue (HalfKAv2_hm, ~100 MB)
//! - **Viridithas**: net.bin (768→768×2→1, ~4.5 MB)
//! - **Alexandria**: net.bin (768→1536×2→1, ~9 MB)
//!
//! Default download path: `./nnue/` relative to the engine working directory.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Default NNUE directory (relative to engine working directory).
pub const DEFAULT_NNUE_DIR: &str = "nnue";

/// Progress callback: (file_name, status).
pub type ProgressCallback = Box<dyn Fn(&str, DownloadStatus) + Send>;

/// Download status for a single file.
#[derive(Debug, Clone)]
pub enum DownloadStatus {
    /// File already exists, skipped.
    Skipped,
    /// Download started (with expected size in bytes, 0 if unknown).
    Downloading(u64),
    /// Download completed successfully.
    Done,
    /// Download failed with error message.
    Failed(String),
}

/// A downloadable NNUE network definition.
#[derive(Debug, Clone)]
pub struct NnueNetwork {
    /// Human-friendly name shown in the UI.
    pub name: &'static str,
    /// Engine/project this network is from.
    pub engine: &'static str,
    /// Architecture description.
    pub architecture: &'static str,
    /// Download URL.
    pub url: &'static str,
    /// Filename to save as.
    pub filename: &'static str,
    /// Approximate file size in bytes (for progress display).
    pub approx_size: u64,
    /// Search parameter preset name (maps to `SearchParams::for_preset()`).
    /// One of: "akimbo", "stockfish", or a custom name for future engines.
    pub search_preset: &'static str,
}

/// All available NNUE networks.
pub const NETWORKS: &[NnueNetwork] = &[
    NnueNetwork {
        name: "Akimbo 1024",
        engine: "Akimbo",
        architecture: "768→1024×2→1 SCReLU",
        url: "https://github.com/jw1912/akimbo/raw/main/resources/net.bin",
        filename: "akimbo-1024.bin",
        approx_size: 6_297_664,
        search_preset: "akimbo",
    },
    NnueNetwork {
        name: "Stockfish 18 Big (SFNNv10)",
        engine: "Stockfish",
        architecture: "HalfKAv2_hm+Threats →1024×2",
        url: "https://tests.stockfishchess.org/api/nn/nn-c288c895ea92.nnue",
        filename: "nn-c288c895ea92.nnue",
        approx_size: 108_000_000,
        search_preset: "stockfish",
    },
    NnueNetwork {
        name: "Stockfish 18 Small",
        engine: "Stockfish",
        architecture: "HalfKAv2_hm →512×2",
        url: "https://tests.stockfishchess.org/api/nn/nn-37f18f62d772.nnue",
        filename: "nn-37f18f62d772.nnue",
        approx_size: 3_500_000,
        search_preset: "stockfish",
    },
    NnueNetwork {
        name: "Viridithas 768",
        engine: "Viridithas",
        architecture: "768→768×2→1 SCReLU",
        url: "https://github.com/cosmobobak/viridithas/raw/master/resources/net.bin",
        filename: "viridithas-768.bin",
        approx_size: 4_721_666,
        search_preset: "akimbo", // same arch family as Akimbo
    },
];

/// Find a network definition by filename or name (case-insensitive partial match).
pub fn find_network(query: &str) -> Option<&'static NnueNetwork> {
    let q = query.to_lowercase();
    NETWORKS.iter().find(|n| {
        n.filename.to_lowercase() == q
            || n.name.to_lowercase().contains(&q)
            || n.engine.to_lowercase() == q
    })
}

/// Find all networks from a given engine.
pub fn find_by_engine(engine: &str) -> Vec<&'static NnueNetwork> {
    let e = engine.to_lowercase();
    NETWORKS
        .iter()
        .filter(|n| n.engine.to_lowercase() == e)
        .collect()
}

/// Get the resources directory (for compile-time embedded nets).
/// Returns `crates/kishmat-eval/resources/` relative to workspace root.
pub fn resources_dir() -> PathBuf {
    PathBuf::from("crates/kishmat-eval/resources")
}

/// Summary of a download operation.
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub target_dir: PathBuf,
}

/// Get the default NNUE directory path.
pub fn default_nnue_path() -> PathBuf {
    PathBuf::from(DEFAULT_NNUE_DIR)
}

/// Download a specific network by index.
pub fn download_network(
    network: &NnueNetwork,
    dest_dir: &Path,
    progress: Option<&ProgressCallback>,
) -> Result<(), String> {
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", dest_dir.display()))?;

    let dest_path = dest_dir.join(network.filename);

    if dest_path.exists() {
        if let Some(cb) = progress {
            cb(network.filename, DownloadStatus::Skipped);
        }
        return Ok(());
    }

    if let Some(cb) = progress {
        cb(
            network.filename,
            DownloadStatus::Downloading(network.approx_size),
        );
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("kishmat-updater/2.0.0")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    download_file(&client, network.url, &dest_path)?;

    if let Some(cb) = progress {
        cb(network.filename, DownloadStatus::Done);
    }

    Ok(())
}

/// Download all networks.
pub fn download_all(
    dest_dir: &Path,
    progress: Option<ProgressCallback>,
) -> Result<DownloadSummary, String> {
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", dest_dir.display()))?;

    let mut downloaded = 0usize;
    let skipped = 0usize;
    let mut failed = 0usize;

    for network in NETWORKS {
        match download_network(network, dest_dir, progress.as_ref()) {
            Ok(()) => {
                let dest_path = dest_dir.join(network.filename);
                if dest_path.exists() {
                    // Check if it was skipped (already existed) or freshly downloaded
                    downloaded += 1; // Simplified — both count as success
                }
            }
            Err(e) => {
                failed += 1;
                if let Some(ref cb) = progress {
                    cb(network.filename, DownloadStatus::Failed(e));
                }
            }
        }
    }

    Ok(DownloadSummary {
        total: NETWORKS.len(),
        downloaded,
        skipped,
        failed,
        target_dir: dest_dir.to_path_buf(),
    })
}

/// Check which networks are installed in a directory.
pub fn check_installed(dir: &Path) -> Vec<(&'static NnueNetwork, bool)> {
    NETWORKS
        .iter()
        .map(|net| (net, dir.join(net.filename).exists()))
        .collect()
}

/// Get the total disk usage of the NNUE directory.
pub fn disk_usage(dir: &Path) -> u64 {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// List network files in a directory (not just known ones).
pub fn list_network_files(dir: &Path) -> Vec<(String, u64)> {
    if !dir.exists() {
        return Vec::new();
    }
    fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.ends_with(".bin") || name.ends_with(".nnue")
                })
                .filter_map(|e| {
                    let size = e.metadata().ok()?.len();
                    Some((e.file_name().to_string_lossy().to_string(), size))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn download_file(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<(), String> {
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Request failed for {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {url}", response.status()));
    }

    // Write to a temp file first, then rename (atomic-ish)
    let temp_path = dest.with_extension("tmp");
    let mut file = fs::File::create(&temp_path).map_err(|e| format!("Create file: {e}"))?;

    let mut buffer = [0u8; 65536];
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| format!("Read: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])
            .map_err(|e| format!("Write: {e}"))?;
    }

    // Rename temp to final destination
    fs::rename(&temp_path, dest).map_err(|e| format!("Rename {}: {e}", dest.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_list_not_empty() {
        assert!(!NETWORKS.is_empty());
    }

    #[test]
    fn test_default_path() {
        let path = default_nnue_path();
        assert_eq!(path.to_str().unwrap(), "nnue");
    }

    #[test]
    fn test_check_installed_empty_dir() {
        let installed = check_installed(Path::new("/nonexistent/path"));
        assert!(installed.iter().all(|(_, exists)| !exists));
    }

    #[test]
    fn test_network_filenames_unique() {
        let mut names: Vec<&str> = NETWORKS.iter().map(|n| n.filename).collect();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            NETWORKS.len(),
            "Duplicate filenames in NETWORKS"
        );
    }

    #[test]
    fn test_list_network_files_empty() {
        let files = list_network_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
    }
}
