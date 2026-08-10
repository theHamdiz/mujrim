//! Download management for NNUE networks and Syzygy tablebases.
//!
//! Wraps `mujrim-updater` download functions for use in the async iced UI.

use std::path::{Path, PathBuf};

use updater::nnue::{self, NnueNetwork};
use updater::syzygy::{self, SyzygyPieceSet};

/// Which NNUE network the user has selected for download.
#[derive(Debug, Clone)]
pub struct NnueSelection {
    pub network: &'static NnueNetwork,
    pub selected: bool,
}

/// Build the default selection list — Akimbo is pre-checked.
pub fn default_nnue_selections() -> Vec<NnueSelection> {
    nnue::NETWORKS
        .iter()
        .map(|net| NnueSelection {
            network: net,
            selected: net.id == "ak_default",
        })
        .collect()
}

/// Default tablebase tier.
pub fn default_syzygy_tier() -> SyzygyPieceSet {
    SyzygyPieceSet::Extended
}

/// Estimated download size for a Syzygy tier (bytes).
pub fn syzygy_estimated_size(tier: SyzygyPieceSet) -> u64 {
    match tier {
        SyzygyPieceSet::Standard => 1_000_000_000,
        SyzygyPieceSet::Extended => 150_000_000_000,
        SyzygyPieceSet::Full => 140_000_000_000_000,
    }
}

/// Format bytes as a human-readable string.
pub fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        format!("{:.0} TB", bytes as f64 / 1_000_000_000_000.0)
    } else if bytes >= 1_000_000_000 {
        format!("{:.0} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

/// Download selected NNUE networks (blocking — run in a `Task::perform`).
pub fn download_nnue_blocking(
    selections: &[NnueSelection],
    dest_dir: &Path,
) -> Result<usize, String> {
    let mut count = 0usize;
    for sel in selections.iter().filter(|s| s.selected) {
        nnue::download_network(sel.network, dest_dir, None)?;
        count += 1;
    }
    Ok(count)
}

/// Download Syzygy tablebases (blocking — run in a `Task::perform`).
pub fn download_syzygy_blocking(tier: SyzygyPieceSet, dest_dir: &Path) -> Result<usize, String> {
    let summary = syzygy::download_tables(dest_dir, tier, None)?;
    Ok(summary.downloaded)
}

/// Resolve the NNUE download directory relative to the install dir.
pub fn nnue_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("nnue")
}

/// Resolve the Syzygy download directory relative to the install dir.
pub fn syzygy_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("syzygy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_selections_include_akimbo() {
        let sels = default_nnue_selections();
        let ak = sels.iter().find(|s| s.network.id == "ak_default");
        assert!(ak.is_some(), "Akimbo should be in the list");
        assert!(ak.unwrap().selected, "Akimbo should be pre-selected");
    }

    #[test]
    fn default_selections_others_unchecked() {
        let sels = default_nnue_selections();
        for s in &sels {
            if s.network.id != "ak_default" {
                assert!(!s.selected, "{} should not be pre-selected", s.network.id);
            }
        }
    }

    #[test]
    fn default_syzygy_is_extended() {
        assert_eq!(default_syzygy_tier(), SyzygyPieceSet::Extended);
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(500), "500 B");
        assert_eq!(human_bytes(12_000), "12 KB");
        assert!(human_bytes(17_000_000).contains("MB"));
        assert!(human_bytes(2_000_000_000).contains("GB"));
        assert!(human_bytes(5_000_000_000_000).contains("TB"));
    }

    #[test]
    fn nnue_dir_is_subdir() {
        let dir = nnue_dir(Path::new("/opt/mujrim"));
        assert_eq!(dir, PathBuf::from("/opt/mujrim/nnue"));
    }

    #[test]
    fn syzygy_dir_is_subdir() {
        let dir = syzygy_dir(Path::new("/opt/mujrim"));
        assert_eq!(dir, PathBuf::from("/opt/mujrim/syzygy"));
    }
}
