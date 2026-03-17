//! Syzygy tablebase download manager.
//!
//! Supports downloading 3-7 piece endgame tablebases from the Lichess mirror.
//! Both WDL (.rtbw) and DTZ (.rtbz) files are downloaded.
//!
//! Default download path: `./syzygy/` relative to the engine working directory.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Base URL for Syzygy downloads (Lichess mirror).
const BASE_URL_345: &str = "https://tablebase.lichess.ovh/tables/standard/3-4-5";
const BASE_URL_6: &str = "https://tablebase.lichess.ovh/tables/standard/6";
const BASE_URL_7: &str = "https://tablebase.lichess.ovh/tables/standard/7";

/// Default Syzygy directory (relative to engine working directory).
pub const DEFAULT_SYZYGY_DIR: &str = "syzygy";

/// Progress callback: (current_file_index, total_files, file_name, status).
pub type ProgressCallback = Box<dyn Fn(usize, usize, &str, DownloadStatus) + Send>;

/// Download status for a single file.
#[derive(Debug, Clone)]
pub enum DownloadStatus {
    /// File already exists, skipped.
    Skipped,
    /// Download started.
    Downloading,
    /// Download completed successfully.
    Done,
    /// Download failed with error message.
    Failed(String),
}

/// Which piece counts to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyzygyPieceSet {
    /// 3-4-5 pieces (~1 GB)
    Standard,
    /// 3-4-5-6 pieces (~150 GB)
    Extended,
    /// 3-4-5-6-7 pieces (~140 TB — impractical for most users)
    Full,
}

impl std::fmt::Display for SyzygyPieceSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard => write!(f, "3-4-5 pieces (~1 GB)"),
            Self::Extended => write!(f, "3-4-5-6 pieces (~150 GB)"),
            Self::Full => write!(f, "3-4-5-6-7 pieces (~140 TB)"),
        }
    }
}

/// Summary of a download operation.
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub total_files: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub target_dir: PathBuf,
}

/// Get the default Syzygy directory path.
pub fn default_syzygy_path() -> PathBuf {
    PathBuf::from(DEFAULT_SYZYGY_DIR)
}

/// Get all table names for the requested piece count set.
pub fn table_names(piece_set: SyzygyPieceSet) -> Vec<(&'static str, &'static str)> {
    let mut tables = Vec::new();

    // 3-piece tables
    for name in TABLES_3 {
        tables.push((*name, BASE_URL_345));
    }
    // 4-piece tables
    for name in TABLES_4 {
        tables.push((*name, BASE_URL_345));
    }
    // 5-piece tables
    for name in TABLES_5 {
        tables.push((*name, BASE_URL_345));
    }

    if piece_set == SyzygyPieceSet::Extended || piece_set == SyzygyPieceSet::Full {
        for name in TABLES_6 {
            tables.push((*name, BASE_URL_6));
        }
    }

    if piece_set == SyzygyPieceSet::Full {
        for name in TABLES_7 {
            tables.push((*name, BASE_URL_7));
        }
    }

    tables
}

/// Download Syzygy tablebases to the specified directory.
///
/// Returns a summary of what was downloaded.
pub fn download_tables(
    dest_dir: &Path,
    piece_set: SyzygyPieceSet,
    progress: Option<ProgressCallback>,
) -> Result<DownloadSummary, String> {
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", dest_dir.display()))?;

    let tables = table_names(piece_set);
    // Each table has 2 files: .rtbw and .rtbz
    let total_files = tables.len() * 2;
    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut file_idx = 0usize;

    let client = reqwest::blocking::Client::builder()
        .user_agent("kishmat-updater/2.0.0")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    for (name, base_url) in &tables {
        for ext in &["rtbw", "rtbz"] {
            let file_name = format!("{name}.{ext}");
            let file_path = dest_dir.join(&file_name);
            file_idx += 1;

            if file_path.exists() {
                skipped += 1;
                if let Some(ref cb) = progress {
                    cb(file_idx, total_files, &file_name, DownloadStatus::Skipped);
                }
                continue;
            }

            if let Some(ref cb) = progress {
                cb(file_idx, total_files, &file_name, DownloadStatus::Downloading);
            }

            let url = format!("{base_url}/{file_name}");
            match download_file(&client, &url, &file_path) {
                Ok(()) => {
                    downloaded += 1;
                    if let Some(ref cb) = progress {
                        cb(file_idx, total_files, &file_name, DownloadStatus::Done);
                    }
                }
                Err(e) => {
                    failed += 1;
                    let _ = fs::remove_file(&file_path);
                    if let Some(ref cb) = progress {
                        cb(file_idx, total_files, &file_name, DownloadStatus::Failed(e));
                    }
                }
            }
        }
    }

    Ok(DownloadSummary {
        total_files,
        downloaded,
        skipped,
        failed,
        target_dir: dest_dir.to_path_buf(),
    })
}

/// Check how many tables exist in a directory.
pub fn check_installed(dir: &Path) -> (usize, usize) {
    if !dir.exists() {
        return (0, 0);
    }
    let rtbw = fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "rtbw"))
            .count())
        .unwrap_or(0);
    let rtbz = fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "rtbz"))
            .count())
        .unwrap_or(0);
    (rtbw, rtbz)
}

/// Get the total disk usage of a Syzygy directory.
pub fn disk_usage(dir: &Path) -> u64 {
    if !dir.exists() { return 0; }
    fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum())
        .unwrap_or(0)
}

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    let mut response = client.get(url).send()
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let mut file = fs::File::create(dest)
        .map_err(|e| format!("Create file: {e}"))?;

    let mut buffer = [0u8; 65536];
    loop {
        let n = response.read(&mut buffer).map_err(|e| format!("Read: {e}"))?;
        if n == 0 { break; }
        file.write_all(&buffer[..n]).map_err(|e| format!("Write: {e}"))?;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// Table name lists — complete Syzygy endgame tables
// ═══════════════════════════════════════════════════════════════

const TABLES_3: &[&str] = &[
    "KBvK", "KNvK", "KPvK", "KQvK", "KRvK",
];

const TABLES_4: &[&str] = &[
    "KBBvK", "KBNvK", "KBPvK", "KBvKB", "KBvKN", "KBvKP",
    "KNNvK", "KNPvK", "KNvKN", "KNvKP",
    "KPPvK", "KPvKP",
    "KQBvK", "KQNvK", "KQPvK", "KQvKB", "KQvKN", "KQvKP", "KQvKQ", "KQvKR",
    "KRBvK", "KRNvK", "KRPvK", "KRvKB", "KRvKN", "KRvKP", "KRvKR",
];

const TABLES_5: &[&str] = &[
    // 4v1
    "KBBBvK", "KBBNvK", "KBBPvK", "KBNNvK", "KBNPvK", "KBPPvK",
    "KNNNvK", "KNNPvK", "KNPPvK", "KPPPvK",
    "KQBBvK", "KQBNvK", "KQBPvK", "KQNNvK", "KQNPvK", "KQPPvK",
    "KQQBvK", "KQQNvK", "KQQPvK", "KQQQvK", "KQQRvK",
    "KQRBvK", "KQRNvK", "KQRPvK", "KQRRvK",
    "KRBBvK", "KRBNvK", "KRBPvK", "KRNNvK", "KRNPvK", "KRPPvK",
    "KRRBvK", "KRRNvK", "KRRPvK", "KRRRvK",
    // 3v2
    "KBBvKB", "KBBvKN", "KBBvKP", "KBBvKQ", "KBBvKR",
    "KBNvKB", "KBNvKN", "KBNvKP", "KBNvKQ", "KBNvKR",
    "KBPvKB", "KBPvKN", "KBPvKP", "KBPvKQ", "KBPvKR",
    "KNNvKB", "KNNvKN", "KNNvKP", "KNNvKQ", "KNNvKR",
    "KNPvKB", "KNPvKN", "KNPvKP", "KNPvKQ", "KNPvKR",
    "KPPvKB", "KPPvKN", "KPPvKP", "KPPvKQ", "KPPvKR",
    "KQBvKQ", "KQBvKR",
    "KQNvKQ", "KQNvKR",
    "KQPvKQ", "KQPvKR",
    "KQQvKQ", "KQQvKR",
    "KQRvKQ", "KQRvKR",
    "KRBvKQ", "KRBvKR",
    "KRNvKQ", "KRNvKR",
    "KRPvKB", "KRPvKN", "KRPvKP", "KRPvKQ", "KRPvKR",
    "KRRvKQ", "KRRvKR",
];

// 6-piece tables — representative set (the full set has ~500+ tables)
const TABLES_6: &[&str] = &[
    // 5v1
    "KBBBBvK", "KBBBNvK", "KBBBPvK", "KBBNNvK", "KBBNPvK", "KBBPPvK",
    "KBNNNvK", "KBNNPvK", "KBNPPvK", "KBPPPvK",
    "KNNNNvK", "KNNNPvK", "KNNPPvK", "KNPPPvK", "KPPPPvK",
    "KQBBBvK", "KQBBNvK", "KQBBPvK", "KQBNNvK", "KQBNPvK", "KQBPPvK",
    "KQNNNvK", "KQNNPvK", "KQNPPvK", "KQPPPvK",
    "KQQBBvK", "KQQBNvK", "KQQBPvK", "KQQNNvK", "KQQNPvK", "KQQPPvK",
    "KQQQBvK", "KQQQNvK", "KQQQPvK", "KQQQQvK", "KQQQRvK",
    "KQQRBvK", "KQQRNvK", "KQQRPvK", "KQQRRvK",
    "KQRBBvK", "KQRBNvK", "KQRBPvK", "KQRNNvK", "KQRNPvK", "KQRPPvK",
    "KQRRBvK", "KQRRNvK", "KQRRPvK", "KQRRRvK",
    "KRBBBvK", "KRBBNvK", "KRBBPvK", "KRBNNvK", "KRBNPvK", "KRBPPvK",
    "KRNNNvK", "KRNNPvK", "KRNPPvK", "KRPPPvK",
    "KRRBBvK", "KRRBNvK", "KRRBPvK", "KRRNNvK", "KRRNPvK", "KRRPPvK",
    "KRRRBvK", "KRRRNvK", "KRRRPvK", "KRRRRvK",
    // 4v2 (most common/important)
    "KBBBvKB", "KBBBvKN", "KBBBvKP", "KBBBvKQ", "KBBBvKR",
    "KBBNvKB", "KBBNvKN", "KBBNvKP", "KBBNvKQ", "KBBNvKR",
    "KBBPvKB", "KBBPvKN", "KBBPvKP", "KBBPvKQ", "KBBPvKR",
    "KBNNvKB", "KBNNvKN", "KBNNvKP", "KBNNvKQ", "KBNNvKR",
    "KBNPvKB", "KBNPvKN", "KBNPvKP", "KBNPvKQ", "KBNPvKR",
    "KBPPvKB", "KBPPvKN", "KBPPvKP", "KBPPvKQ", "KBPPvKR",
    "KNNNvKB", "KNNNvKN", "KNNNvKP", "KNNNvKQ", "KNNNvKR",
    "KNNPvKB", "KNNPvKN", "KNNPvKP", "KNNPvKQ", "KNNPvKR",
    "KNPPvKB", "KNPPvKN", "KNPPvKP", "KNPPvKQ", "KNPPvKR",
    "KPPPvKB", "KPPPvKN", "KPPPvKP", "KPPPvKQ", "KPPPvKR",
    "KQBBvKB", "KQBBvKN", "KQBBvKP", "KQBBvKQ", "KQBBvKR",
    "KQBNvKB", "KQBNvKN", "KQBNvKP", "KQBNvKQ", "KQBNvKR",
    "KQBPvKB", "KQBPvKN", "KQBPvKP", "KQBPvKQ", "KQBPvKR",
    "KQNNvKB", "KQNNvKN", "KQNNvKP", "KQNNvKQ", "KQNNvKR",
    "KQNPvKB", "KQNPvKN", "KQNPvKP", "KQNPvKQ", "KQNPvKR",
    "KQPPvKB", "KQPPvKN", "KQPPvKP", "KQPPvKQ", "KQPPvKR",
    "KQQBvKB", "KQQBvKN", "KQQBvKP", "KQQBvKQ", "KQQBvKR",
    "KQQNvKB", "KQQNvKN", "KQQNvKP", "KQQNvKQ", "KQQNvKR",
    "KQQPvKB", "KQQPvKN", "KQQPvKP", "KQQPvKQ", "KQQPvKR",
    "KQQQvKB", "KQQQvKN", "KQQQvKP", "KQQQvKQ", "KQQQvKR",
    "KQQRvKB", "KQQRvKN", "KQQRvKP", "KQQRvKQ", "KQQRvKR",
    "KQRBvKB", "KQRBvKN", "KQRBvKP", "KQRBvKQ", "KQRBvKR",
    "KQRNvKB", "KQRNvKN", "KQRNvKP", "KQRNvKQ", "KQRNvKR",
    "KQRPvKB", "KQRPvKN", "KQRPvKP", "KQRPvKQ", "KQRPvKR",
    "KQRRvKB", "KQRRvKN", "KQRRvKP", "KQRRvKQ", "KQRRvKR",
    "KRBBvKB", "KRBBvKN", "KRBBvKP", "KRBBvKQ", "KRBBvKR",
    "KRBNvKB", "KRBNvKN", "KRBNvKP", "KRBNvKQ", "KRBNvKR",
    "KRBPvKB", "KRBPvKN", "KRBPvKP", "KRBPvKQ", "KRBPvKR",
    "KRNNvKB", "KRNNvKN", "KRNNvKP", "KRNNvKQ", "KRNNvKR",
    "KRNPvKB", "KRNPvKN", "KRNPvKP", "KRNPvKQ", "KRNPvKR",
    "KRPPvKB", "KRPPvKN", "KRPPvKP", "KRPPvKQ", "KRPPvKR",
    "KRRBvKB", "KRRBvKN", "KRRBvKP", "KRRBvKQ", "KRRBvKR",
    "KRRNvKB", "KRRNvKN", "KRRNvKP", "KRRNvKQ", "KRRNvKR",
    "KRRPvKB", "KRRPvKN", "KRRPvKP", "KRRPvKQ", "KRRPvKR",
    "KRRRvKB", "KRRRvKN", "KRRRvKP", "KRRRvKQ", "KRRRvKR",
    // 3v3 (most important)
    "KBBvKBB", "KBBvKBN", "KBBvKBP", "KBBvKNN", "KBBvKNP", "KBBvKPP",
    "KBBvKQB", "KBBvKQN", "KBBvKQP", "KBBvKRB", "KBBvKRN", "KBBvKRP", "KBBvKRR",
    "KBNvKBN", "KBNvKBP", "KBNvKNN", "KBNvKNP", "KBNvKPP",
    "KBNvKQB", "KBNvKQN", "KBNvKQP", "KBNvKRB", "KBNvKRN", "KBNvKRP", "KBNvKRR",
    "KBPvKBP", "KBPvKNN", "KBPvKNP", "KBPvKPP",
    "KBPvKQB", "KBPvKQN", "KBPvKQP", "KBPvKRB", "KBPvKRN", "KBPvKRP", "KBPvKRR",
    "KNNvKNN", "KNNvKNP", "KNNvKPP",
    "KNNvKQB", "KNNvKQN", "KNNvKQP", "KNNvKRB", "KNNvKRN", "KNNvKRP", "KNNvKRR",
    "KNPvKNP", "KNPvKPP",
    "KNPvKQB", "KNPvKQN", "KNPvKQP", "KNPvKRB", "KNPvKRN", "KNPvKRP", "KNPvKRR",
    "KPPvKPP",
    "KPPvKQB", "KPPvKQN", "KPPvKQP", "KPPvKRB", "KPPvKRN", "KPPvKRP", "KPPvKRR",
    "KQBvKQB", "KQBvKQN", "KQBvKQP", "KQBvKRB", "KQBvKRN", "KQBvKRP", "KQBvKRR",
    "KQNvKQN", "KQNvKQP", "KQNvKRB", "KQNvKRN", "KQNvKRP", "KQNvKRR",
    "KQPvKQP", "KQPvKRB", "KQPvKRN", "KQPvKRP", "KQPvKRR",
    "KQQvKQQ", "KQQvKQR", "KQQvKRR",
    "KQRvKQR", "KQRvKRR",
    "KRBvKRB", "KRBvKRN", "KRBvKRP", "KRBvKRR",
    "KRNvKRN", "KRNvKRP", "KRNvKRR",
    "KRPvKRP", "KRPvKRR",
    "KRRvKRR",
];

// 7-piece tables — only the most critical endings (full set is enormous)
const TABLES_7: &[&str] = &[
    // Pawn endings (most practical)
    "KPPPPvKP", "KPPPPvKN", "KPPPPvKB", "KPPPPvKR", "KPPPPvKQ",
    "KPPPvKPP", "KPPPvKNP", "KPPPvKBP",
    "KPPvKPPP",
    // Rook endings (most common in practice)
    "KRPPPvKR", "KRPPvKRP", "KRPvKRPP",
    "KRRPvKRR", "KRRvKRRP",
    // Queen endings  
    "KQPPPvKQ", "KQPPvKQP", "KQPvKQPP",
    // Rook vs minor + pawns
    "KRPPvKBP", "KRPPvKNP", "KRPvKBPP", "KRPvKNPP",
    "KRBPvKRP", "KRNPvKRP",
    // Common practical endings
    "KBPPPvKB", "KBPPPvKN", "KNPPPvKB", "KNPPPvKN",
    "KBPPvKBP", "KBPPvKNP", "KNPPvKBP", "KNPPvKNP",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_counts() {
        let standard = table_names(SyzygyPieceSet::Standard);
        assert!(standard.len() >= 100, "Should have 100+ tables for 3-4-5 piece");

        let extended = table_names(SyzygyPieceSet::Extended);
        assert!(extended.len() > standard.len(), "6-piece set should be larger");

        let full = table_names(SyzygyPieceSet::Full);
        assert!(full.len() > extended.len(), "7-piece set should be largest");
    }

    #[test]
    fn test_default_path() {
        let path = default_syzygy_path();
        assert_eq!(path.to_str().unwrap(), "syzygy");
    }

    #[test]
    fn test_check_installed_empty() {
        let (wdl, dtz) = check_installed(Path::new("/nonexistent/path"));
        assert_eq!(wdl, 0);
        assert_eq!(dtz, 0);
    }
}
