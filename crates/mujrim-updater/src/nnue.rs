//! NNUE network download manager.
//!
//! Downloads neural network weights that are known to be compatible with
//! Mujrim's currently supported runtime loader.
//!
//! ## Canonical Naming Convention
//!
//! Every network gets a **stable identifier** (`id`) following the pattern
//! `{engine}_{variant}` (e.g., `sf_current`, `ak_default`).
//!
//! The **local filename** is derived from the `id` as `{id}.{ext}` where the
//! extension matches the file format (`.bin` for Akimbo-family, `.nnue` for
//! Stockfish-family).
//!
//! ## Update Detection
//!
//! A `networks.json` manifest lives alongside the downloaded nets. Each entry
//! records which `upstream_name` and `url` were used when the file was last
//! downloaded. When the registry's `url`/`upstream_name` differ from the
//! manifest's, we know a newer network is available.
//!
//! Default download path: `./nnue/` relative to the engine working directory.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Default NNUE directory (relative to engine working directory).
pub const DEFAULT_NNUE_DIR: &str = "nnue";

/// Manifest filename stored alongside network files.
pub const MANIFEST_FILE: &str = "networks.json";

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

/// Outcome for downloading one network file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadOutcome {
    Downloaded,
    Skipped,
}

// ═══════════════════════════════════════════════════════════════════
// Network definition
// ═══════════════════════════════════════════════════════════════════

/// A downloadable NNUE network definition.
///
/// Each entry has a **canonical `id`** that stays constant even when the
/// upstream file hash changes (e.g. `sf_current` for Stockfish's current net).
#[derive(Debug, Clone)]
pub struct NnueNetwork {
    // ── Identity ──
    /// Stable canonical id: `{engine}_{variant}`, e.g. `"sf_current"`.
    /// Used as the primary lookup key. Never changes.
    pub id: &'static str,
    /// Human-friendly display name shown in the UI.
    pub name: &'static str,
    /// Engine family (for grouping), e.g. `"Stockfish"`.
    pub engine: &'static str,

    // ── Technical ──
    /// Architecture description, e.g. `"HalfKAv2_hm SFNNv10 (big net)"`.
    pub architecture: &'static str,
    /// Download URL (may change when a new net is released).
    pub url: &'static str,
    /// Canonical local filename derived from `id`, e.g. `"sf_current.nnue"`.
    /// This is what gets saved to disk and referenced everywhere.
    pub filename: &'static str,
    /// Original upstream filename for provenance, e.g. `"nn-ab28990d4ea3.nnue"`.
    /// Updated alongside the URL when the engine ships a new net.
    pub upstream_name: &'static str,
    /// Approximate file size in bytes (for progress display).
    pub approx_size: u64,
    /// Search parameter preset name (maps to `SearchParams::for_preset()`).
    pub search_preset: &'static str,
    /// Approximate ELO strength rating (from CCRL or engine author estimates).
    /// Used to prioritize network loading: the strongest available net is loaded first.
    pub elo: u32,
}

// ═══════════════════════════════════════════════════════════════════
// Network registry
// ═══════════════════════════════════════════════════════════════════

/// All available NNUE networks.
///
/// ```text
/// ID               Engine      Filename              Upstream
/// ─────────────    ──────────  ────────────────────  ────────────────────────
/// ak_default       Akimbo      ak_default.bin        net.bin
/// threat_v60       Threat      threat_v60.nnue       v60-7f587dfb.nnue
/// sf_current       Stockfish   sf_current.nnue       nn-ab28990d4ea3.nnue
/// viri_default     Viridithas  viri_default.nnue.zst sandhi-s2-b200.nnue.zst
/// viri_velarised   Viridithas  viri_velarised.nnue.zst velarised-2-b800.nnue.zst
/// plenty_default   PlentyChess plenty_default.bin    0179r.bin
/// ateed_default    Ateed       ateed_default.bin     ateed_default.bin
/// lc0_bt4          Lc0         lc0_bt4.pb.gz         BT4-1024x15x32h-swa-6147500-policytune-332.pb.gz
/// lc0_default      Lc0         lc0_default.pb.gz     t1-256x10-distilled-swa-2432500.pb.gz
/// alex_default     Alexandria  alex_default.net      nn.net
/// ```
pub const NETWORKS: &[NnueNetwork] = &[
    // ── Akimbo ──────────────────────────────────────────────────────
    NnueNetwork {
        id: "ak_default",
        name: "Akimbo 1024",
        engine: "Akimbo",
        architecture: "768→1024×2→1 SCReLU",
        url: "https://github.com/jw1912/akimbo/raw/main/resources/net.bin",
        filename: "ak_default.bin",
        upstream_name: "net.bin",
        approx_size: 6_297_664,
        search_preset: "akimbo",
        elo: 3200,
    },
    NnueNetwork {
        id: "threat_v60",
        name: "Threat-aware v60",
        engine: "Reckless",
        architecture: "piece+threat FT→768→768×8→16→32→1",
        url: "https://github.com/codedeliveryservice/RecklessNetworks/releases/download/networks/v60-7f587dfb.nnue",
        filename: "threat_v60.nnue",
        upstream_name: "v60-7f587dfb.nnue",
        approx_size: 63_266_880,
        search_preset: "reckless",
        elo: 3612,
    },
    // ── Stockfish ───────────────────────────────────────────────────
    NnueNetwork {
        id: "sf_current",
        name: "Stockfish current nn-ab28990d4ea3",
        engine: "Stockfish",
        architecture: "HalfKAv2_hm+FullThreats+PP_3Wide 86896→1024→32→32→1",
        url: "https://tests.stockfishchess.org/api/nn/nn-ab28990d4ea3.nnue",
        filename: "sf_current.nnue",
        upstream_name: "nn-ab28990d4ea3.nnue",
        approx_size: 95_144_073,
        search_preset: "stockfish",
        elo: 3642,
    },
    // ── Viridithas ──────────────────────────────────────────────────
    // Upstream latest is viridithas-networks v109 `sandhi`
    // (`sandhi-s2-b200.nnue.zst`, 2026-06-26).
    NnueNetwork {
        id: "viri_default",
        name: "Viridithas v109 sandhi",
        engine: "Viridithas",
        architecture: "(704×16hm + (59808+4560)hm → 1024)×2 → (32 → 32×2 → 1)×8",
        url: "https://github.com/cosmobobak/viridithas-networks/releases/download/v109/sandhi-s2-b200.nnue.zst",
        filename: "viri_default.nnue.zst",
        upstream_name: "sandhi-s2-b200.nnue.zst",
        approx_size: 52_442_657,
        search_preset: "viridithas",
        elo: 3550,
    },
    NnueNetwork {
        id: "viri_velarised",
        name: "Viridithas v104.1 velarised-2",
        engine: "Viridithas",
        architecture: "704×16hm → 2560 pairwise-CReLU → 16 HardSwish6 → 32 SwiGLU → 1 ×8 (velarised-2)",
        url: "https://github.com/cosmobobak/viridithas-networks/releases/download/v104.1/velarised-2-b800.nnue.zst",
        filename: "viri_velarised.nnue.zst",
        upstream_name: "velarised-2-b800.nnue.zst",
        approx_size: 29_564_785,
        search_preset: "viridithas",
        elo: 3350,
    },
    NnueNetwork {
        id: "obs_default",
        name: "Obsidian net89perm",
        engine: "Obsidian",
        architecture: "768→1536→16→32→1 (13 king buckets, 8 output buckets)",
        url: "https://github.com/gab8192/Obsidian-nets/releases/download/nets/net89perm.bin",
        filename: "obs_default.bin",
        upstream_name: "net89perm.bin",
        approx_size: 30_905_888,
        search_preset: "obsidian",
        elo: 3600,
    },
    NnueNetwork {
        id: "plenty_default",
        name: "PlentyChess 0179r",
        engine: "PlentyChess",
        architecture: "768×12 + 4560 pawn-pair + 59808 threat → 1024 → 16 → 32 → 1 ×8",
        url: "https://github.com/Yoshie2000/PlentyNetworks/releases/download/0179r/0179r.bin",
        filename: "plenty_default.bin",
        upstream_name: "0179r.bin",
        approx_size: 76_368_704,
        search_preset: "plentychess",
        elo: 3600,
    },
    NnueNetwork {
        id: "ateed_default",
        name: "Ateed MoE v1",
        engine: "Ateed",
        architecture: "768×8hm i16 + 4560 pawn-pair i8 → 1024 CReLU → 4-expert MoE (16→32→eval+WDL)",
        url: "https://github.com/theHamdiz/mujrim/releases/download/ateed-v1/ateed_default.bin",
        filename: "ateed_default.bin",
        upstream_name: "ateed_default.bin",
        approx_size: 17_327_452,
        search_preset: "ateed",
        elo: 0,
    },
    NnueNetwork {
        id: "lc0_bt4",
        name: "Lc0 BT4-it332",
        engine: "Lc0",
        architecture: "BT4 1024×15×32h transformer (official TCEC/CCC .pb.gz, not in-process NNUE)",
        url: "https://storage.lczero.org/files/networks-contrib/BT4-1024x15x32h-swa-6147500-policytune-332.pb.gz",
        filename: "lc0_bt4.pb.gz",
        upstream_name: "BT4-1024x15x32h-swa-6147500-policytune-332.pb.gz",
        approx_size: 382_645_315,
        search_preset: "lc0",
        elo: 3750,
    },
    NnueNetwork {
        id: "lc0_default",
        name: "Lc0 T1-256x10 distilled SWA",
        engine: "Lc0",
        architecture: "T1-256x10 transformer (official lc0 .pb.gz, not in-process NNUE)",
        url: "https://storage.lczero.org/files/networks-contrib/t1-256x10-distilled-swa-2432500.pb.gz",
        filename: "lc0_default.pb.gz",
        upstream_name: "t1-256x10-distilled-swa-2432500.pb.gz",
        approx_size: 37_000_000,
        search_preset: "lc0",
        elo: 3600,
    },
    NnueNetwork {
        id: "lc0_t1_512",
        name: "Lc0 T1-512x15x8h distilled SWA",
        engine: "Lc0",
        architecture: "T1-512x15x8h transformer (official lc0 .pb.gz, not in-process NNUE)",
        url: "https://storage.lczero.org/files/networks-contrib/t1-512x15x8h-distilled-swa-3395000.pb.gz",
        filename: "lc0_t1_512.pb.gz",
        upstream_name: "t1-512x15x8h-distilled-swa-3395000.pb.gz",
        approx_size: 149_758_071,
        search_preset: "lc0",
        elo: 3640,
    },
    // ── Alexandria ──────────────────────────────────────────────────
    // Nets are published at PGG106/Alexandria-networks/releases.
    NnueNetwork {
        id: "alex_default",
        name: "Alexandria (latest)",
        engine: "Alexandria",
        architecture: "(768→1024)×2→1 SCReLU",
        url: "https://github.com/PGG106/Alexandria-networks/releases/latest/download/nn.net",
        filename: "alex_default.net",
        upstream_name: "nn.net",
        approx_size: 6_300_000,
        search_preset: "akimbo",
        elo: 3380,
    },
    // NOTE: Ethereal is excluded — its NNUE networks are commercial/paid.
];

// ═══════════════════════════════════════════════════════════════════
// Manifest — tracks what was downloaded for update detection
// ═══════════════════════════════════════════════════════════════════

/// Record of a single downloaded network, persisted in `networks.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstalledNetwork {
    /// Canonical id matching the registry entry.
    pub id: String,
    /// The URL used when this file was last downloaded.
    pub url: String,
    /// The upstream_name at the time of download.
    pub upstream_name: String,
    /// Canonical filename on disk.
    pub filename: String,
    /// ISO 8601 timestamp of when the download occurred.
    pub downloaded_at: String,
    /// File size in bytes.
    pub file_size: u64,
}

/// The full manifest: a map from canonical id → installed record.
pub type Manifest = HashMap<String, InstalledNetwork>;

/// Load the manifest from a directory, or return an empty one.
pub fn load_manifest(dir: &Path) -> Manifest {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return Manifest::new();
    }
    match fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Manifest::new(),
    }
}

/// Save the manifest to a directory.
pub fn save_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = dir.join(MANIFEST_FILE);
    let json =
        serde_json::to_string_pretty(manifest).map_err(|e| format!("JSON serialize error: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Write manifest: {e}"))?;
    Ok(())
}

/// Check whether a registered network has a newer version available.
///
/// Compares the manifest's `url` and `upstream_name` against the current
/// registry. If they differ (or the file isn't in the manifest), the net
/// needs updating.
pub fn needs_update(network: &NnueNetwork, manifest: &Manifest) -> bool {
    match manifest.get(network.id) {
        None => true, // not installed via manifest
        Some(entry) => entry.url != network.url || entry.upstream_name != network.upstream_name,
    }
}

/// Build an `InstalledNetwork` record for a freshly downloaded network.
fn make_manifest_entry(network: &NnueNetwork, file_size: u64) -> InstalledNetwork {
    InstalledNetwork {
        id: network.id.to_string(),
        url: network.url.to_string(),
        upstream_name: network.upstream_name.to_string(),
        filename: network.filename.to_string(),
        downloaded_at: now_iso8601(),
        file_size,
    }
}

/// Current time as ISO 8601 string (no chrono dependency).
fn now_iso8601() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple epoch-seconds representation, good enough without chrono
    format!("epoch:{}", dur.as_secs())
}

// ═══════════════════════════════════════════════════════════════════
// Lookup functions
// ═══════════════════════════════════════════════════════════════════

/// Find a network by its canonical id (e.g. `"sf_current"`).
///
/// This is the **preferred lookup** — IDs are stable and unambiguous.
pub fn find_by_id(id: &str) -> Option<&'static NnueNetwork> {
    NETWORKS.iter().find(|n| n.id == id)
}

/// Find a network by filename, name, id, upstream name, or engine
/// (case-insensitive partial match). Tries exact matches first.
pub fn find_network(query: &str) -> Option<&'static NnueNetwork> {
    let q = query.to_lowercase();
    // Exact id match first (fast path)
    if let Some(net) = NETWORKS.iter().find(|n| n.id == q) {
        return Some(net);
    }
    // Exact filename / upstream_name match
    NETWORKS.iter().find(|n| {
        n.filename.to_lowercase() == q
            || n.upstream_name.to_lowercase() == q
            || n.name.to_lowercase().contains(&q)
            || n.engine.to_lowercase() == q
    })
}

/// Find all networks from a given engine (case-insensitive).
pub fn find_by_engine(engine: &str) -> Vec<&'static NnueNetwork> {
    let e = engine.to_lowercase();
    NETWORKS
        .iter()
        .filter(|n| n.engine.to_lowercase() == e)
        .collect()
}

/// Return the sorted list of distinct engine families.
pub fn list_engines() -> Vec<&'static str> {
    let mut engines: Vec<&str> = NETWORKS.iter().map(|n| n.engine).collect();
    engines.sort_unstable();
    engines.dedup();
    engines
}

// ═══════════════════════════════════════════════════════════════════
// Paths
// ═══════════════════════════════════════════════════════════════════

/// Get the resources directory (for compile-time embedded nets).
/// Returns `crates/mujrim-eval/resources/` relative to workspace root.
pub fn resources_dir() -> PathBuf {
    PathBuf::from("crates/mujrim-eval/resources")
}

/// Get the default NNUE directory path.
pub fn default_nnue_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MUJRIM_NNUE") {
        return PathBuf::from(path);
    }
    if let Ok(executable) = std::env::current_exe()
        && executable.ancestors().any(|part| {
            part.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("dist"))
        })
        && let Some(parent) = executable.parent()
    {
        return parent.join(DEFAULT_NNUE_DIR);
    }
    PathBuf::from(DEFAULT_NNUE_DIR)
}

/// Find a network by its complete content fingerprint, independent of its
/// filename or directory nesting. Scanning is bounded to protect GUI startup.
pub fn find_by_fingerprint(dir: &Path, expected_sha256: &str) -> Option<PathBuf> {
    fn visit(dir: &Path, depth: usize, expected: &str) -> Option<PathBuf> {
        if depth > 3 {
            return None;
        }
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = visit(&path, depth + 1, expected) {
                    return Some(found);
                }
            } else if path.is_file() && sha256_file(&path).as_deref() == Some(expected) {
                return Some(path);
            }
        }
        None
    }

    visit(dir, 0, expected_sha256)
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

// ═══════════════════════════════════════════════════════════════════
// Downloading
// ═══════════════════════════════════════════════════════════════════

/// Summary of a download operation.
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub updated: usize,
    pub target_dir: PathBuf,
}

/// Download a specific network to `dest_dir`.
///
/// The file is saved as `network.filename` (the canonical name, not the
/// upstream name). If the file already exists **and** the manifest shows
/// the same upstream version, it is skipped. If the upstream version
/// changed, the file is re-downloaded.
pub fn download_network(
    network: &NnueNetwork,
    dest_dir: &Path,
    progress: Option<&ProgressCallback>,
) -> Result<DownloadOutcome, String> {
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", dest_dir.display()))?;

    let dest_path = dest_dir.join(network.filename);
    let mut manifest = load_manifest(dest_dir);

    // Skip if file exists AND manifest says it's the same upstream version
    if dest_path.exists() && !needs_update(network, &manifest) {
        if let Some(cb) = progress {
            cb(network.filename, DownloadStatus::Skipped);
        }
        return Ok(DownloadOutcome::Skipped);
    }

    if let Some(cb) = progress {
        cb(
            network.filename,
            DownloadStatus::Downloading(network.approx_size),
        );
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("mujrim-updater/1.0.0")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let expected_size = (network.id == "ateed_default").then_some(network.approx_size);
    crate::download::download_resumable(&client, network.url, &dest_path, expected_size)?;

    // Record in manifest
    let file_size = fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
    manifest.insert(
        network.id.to_string(),
        make_manifest_entry(network, file_size),
    );
    let _ = save_manifest(dest_dir, &manifest); // best-effort

    if let Some(cb) = progress {
        cb(network.filename, DownloadStatus::Done);
    }

    Ok(DownloadOutcome::Downloaded)
}

/// Download all networks.
pub fn download_all(
    dest_dir: &Path,
    progress: Option<ProgressCallback>,
) -> Result<DownloadSummary, String> {
    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", dest_dir.display()))?;

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for network in NETWORKS {
        match download_network(network, dest_dir, progress.as_ref()) {
            Ok(DownloadOutcome::Downloaded) => {
                downloaded += 1;
            }
            Ok(DownloadOutcome::Skipped) => {
                skipped += 1;
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
        updated: 0,
        target_dir: dest_dir.to_path_buf(),
    })
}

// ═══════════════════════════════════════════════════════════════════
// Status / inspection
// ═══════════════════════════════════════════════════════════════════

/// Per-network installation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetStatus {
    /// Not downloaded.
    Missing,
    /// Downloaded, up to date with registry.
    Current,
    /// Downloaded, but registry has a newer upstream version.
    UpdateAvailable,
}

/// Check which networks are installed in a directory, with update status.
pub fn check_installed(dir: &Path) -> Vec<(&'static NnueNetwork, NetStatus)> {
    let manifest = load_manifest(dir);
    NETWORKS
        .iter()
        .map(|net| {
            let path = dir.join(net.filename);
            if !path.exists() {
                (net, NetStatus::Missing)
            } else if needs_update(net, &manifest) {
                (net, NetStatus::UpdateAvailable)
            } else {
                (net, NetStatus::Current)
            }
        })
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
                    name.ends_with(".bin")
                        || name.ends_with(".nnue")
                        || name.ends_with(".nnue.zst")
                        || name.ends_with(".net")
                })
                .filter_map(|e| {
                    let size = e.metadata().ok()?.len();
                    Some((e.file_name().to_string_lossy().to_string(), size))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ═══════════════════════════════════════════════════════════════════
// Internal helpers
// ═══════════════════════════════════════════════════════════════════

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
        let statuses = check_installed(Path::new("/nonexistent/path"));
        assert!(statuses.iter().all(|(_, s)| *s == NetStatus::Missing));
    }

    #[test]
    fn test_network_ids_unique() {
        let mut ids: Vec<&str> = NETWORKS.iter().map(|n| n.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), NETWORKS.len(), "Duplicate IDs in NETWORKS");
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
    fn test_find_by_id() {
        assert!(find_by_id("sf_current").is_some());
        assert!(find_by_id("ak_default").is_some());
        assert!(find_by_id("viri_default").is_some());
        assert!(find_by_id("obs_default").is_some());
        assert!(find_by_id("plenty_default").is_some());
        assert!(find_by_id("ateed_default").is_some());
        assert!(find_by_id("lc0_bt4").is_some());
        assert!(find_by_id("lc0_default").is_some());
        assert!(find_by_id("lc0_t1_512").is_some());
        assert!(find_by_id("alex_default").is_some());
        assert!(find_by_id("nonexistent").is_none());
    }

    #[test]
    fn viridithas_and_obsidian_catalog_use_matching_search_presets() {
        let viri = find_by_id("viri_default").expect("viri_default");
        assert_eq!(viri.search_preset, "viridithas");
        assert_eq!(viri.upstream_name, "sandhi-s2-b200.nnue.zst");
        assert!(viri.architecture.contains("59808"));
        assert!(viri.architecture.contains("4560"));
        let velarised = find_by_id("viri_velarised").expect("viri_velarised");
        assert_eq!(velarised.upstream_name, "velarised-2-b800.nnue.zst");
        let obs = find_by_id("obs_default").expect("obs_default");
        assert_eq!(obs.search_preset, "obsidian");
        assert_eq!(obs.upstream_name, "net89perm.bin");
        assert_eq!(obs.approx_size, 30_905_888);
        let plenty = find_by_id("plenty_default").expect("plenty_default");
        assert_eq!(plenty.search_preset, "plentychess");
        assert_eq!(plenty.upstream_name, "0179r.bin");
        let ateed = find_by_id("ateed_default").expect("ateed_default");
        assert_eq!(ateed.search_preset, "ateed");
        assert_eq!(ateed.filename, "ateed_default.bin");
        assert_eq!(ateed.approx_size, 17_327_452);
        assert_eq!(
            crate::download::validate_size(ateed.approx_size, Some(17_327_452)).ok(),
            Some(())
        );
        let lc0 = find_by_id("lc0_bt4").expect("lc0_bt4");
        assert_eq!(lc0.search_preset, "lc0");
        assert_eq!(lc0.filename, "lc0_bt4.pb.gz");
        assert_eq!(
            lc0.upstream_name,
            "BT4-1024x15x32h-swa-6147500-policytune-332.pb.gz"
        );
        assert_eq!(lc0.approx_size, 382_645_315);
        let lc0_small = find_by_id("lc0_default").expect("lc0_default");
        assert!(lc0_small.filename.ends_with(".pb.gz"));
    }

    #[test]
    fn test_find_network_by_id() {
        let net = find_network("sf_current").unwrap();
        assert_eq!(net.engine, "Stockfish");
    }

    #[test]
    fn test_find_network_by_upstream_name() {
        let net = find_network("nn-ab28990d4ea3.nnue").unwrap();
        assert_eq!(net.id, "sf_current");
    }

    #[test]
    fn test_find_network_by_canonical_filename() {
        let net = find_network("ak_default.bin").unwrap();
        assert_eq!(net.id, "ak_default");
    }

    #[test]
    fn test_find_by_engine() {
        let sf = find_by_engine("stockfish");
        assert_eq!(sf.len(), 1);
        assert!(sf.iter().all(|n| n.engine == "Stockfish"));
    }

    #[test]
    fn test_list_engines() {
        let engines = list_engines();
        assert!(engines.contains(&"Akimbo"));
        assert!(engines.contains(&"Stockfish"));
        assert!(engines.contains(&"Viridithas"));
        assert!(engines.contains(&"Obsidian"));
        assert!(engines.contains(&"Alexandria"));
        assert!(engines.contains(&"Lc0"));
    }

    #[test]
    fn test_canonical_filename_matches_id() {
        for net in NETWORKS {
            // Filename must start with the id
            let stem = net.filename.split('.').next().unwrap();
            assert_eq!(
                stem, net.id,
                "Filename stem '{}' does not match id '{}'",
                stem, net.id
            );
        }
    }

    #[test]
    fn test_list_network_files_empty() {
        let files = list_network_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
    }

    #[test]
    fn test_manifest_round_trip() {
        let temp_dir = std::env::temp_dir().join("mujrim-manifest-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let mut manifest = Manifest::new();
        manifest.insert(
            "sf_big".to_string(),
            InstalledNetwork {
                id: "sf_big".to_string(),
                url: "https://example.com/net.nnue".to_string(),
                upstream_name: "nn-abc123.nnue".to_string(),
                filename: "sf_big.nnue".to_string(),
                downloaded_at: "epoch:0".to_string(),
                file_size: 100,
            },
        );
        save_manifest(&temp_dir, &manifest).unwrap();

        let loaded = load_manifest(&temp_dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["sf_big"].upstream_name, "nn-abc123.nnue");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_needs_update_missing() {
        let manifest = Manifest::new();
        let net = &NETWORKS[0];
        assert!(needs_update(net, &manifest));
    }

    #[test]
    fn test_needs_update_current() {
        let net = &NETWORKS[0];
        let mut manifest = Manifest::new();
        manifest.insert(
            net.id.to_string(),
            InstalledNetwork {
                id: net.id.to_string(),
                url: net.url.to_string(),
                upstream_name: net.upstream_name.to_string(),
                filename: net.filename.to_string(),
                downloaded_at: "epoch:0".to_string(),
                file_size: net.approx_size,
            },
        );
        assert!(!needs_update(net, &manifest));
    }

    #[test]
    fn test_needs_update_stale() {
        let net = &NETWORKS[0];
        let mut manifest = Manifest::new();
        manifest.insert(
            net.id.to_string(),
            InstalledNetwork {
                id: net.id.to_string(),
                url: "https://old-url.example.com".to_string(),
                upstream_name: "old-name.bin".to_string(),
                filename: net.filename.to_string(),
                downloaded_at: "epoch:0".to_string(),
                file_size: net.approx_size,
            },
        );
        assert!(needs_update(net, &manifest));
    }

    #[test]
    fn test_download_network_returns_skipped_for_existing_file() {
        let temp_dir = std::env::temp_dir().join("mujrim-nnue-skip-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let net = &NETWORKS[0];
        let existing = temp_dir.join(net.filename);
        fs::write(&existing, b"already here").unwrap();

        // Write a matching manifest entry so it sees no update needed
        let mut manifest = Manifest::new();
        manifest.insert(net.id.to_string(), make_manifest_entry(net, 12));
        save_manifest(&temp_dir, &manifest).unwrap();

        let outcome = download_network(net, &temp_dir, None).unwrap();
        assert_eq!(outcome, DownloadOutcome::Skipped);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
