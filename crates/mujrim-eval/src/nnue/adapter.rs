//! Runtime NNUE network selection.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

#[cfg(any(
    not(feature = "embedded-networks"),
    feature = "stockfish-nnue",
    feature = "reckless-nnue"
))]
use sha2::{Digest, Sha256};
#[cfg(any(
    not(feature = "embedded-networks"),
    feature = "stockfish-nnue",
    feature = "reckless-nnue"
))]
use std::fs::File;
#[cfg(any(
    not(feature = "embedded-networks"),
    feature = "stockfish-nnue",
    feature = "reckless-nnue"
))]
use std::io::Read;

use super::network::{HIDDEN, NUM_BUCKETS, Network, net};
#[cfg(feature = "reckless-nnue")]
use super::reckless_format::{
    FILE_SIZE as RECKLESS_FILE_SIZE, HIDDEN_SIZE as RECKLESS_HIDDEN,
    INPUT_BUCKETS as RECKLESS_INPUT_BUCKETS, OUTPUT_BUCKETS as RECKLESS_OUTPUT_BUCKETS,
    RecklessNetwork,
};
#[cfg(feature = "stockfish-nnue")]
use super::stockfish_format::{
    FILE_SIZE as STOCKFISH_FILE_SIZE, L1 as STOCKFISH_L1, LAYER_STACKS as STOCKFISH_BUCKETS,
    StockfishNetwork,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkFormat {
    Embedded,
    Akimbo,
    Stockfish,
    Reckless,
    Viridithas,
    Obsidian,
    PlentyChess,
    Ateed,
}

/// Search-stack family required by an evaluator architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NnueSearchProfile {
    Akimbo,
    Stockfish,
    Reckless,
    Viridithas,
    Obsidian,
    PlentyChess,
    Ateed,
    Lc0,
}

impl NnueSearchProfile {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Akimbo => "akimbo",
            Self::Stockfish => "stockfish",
            Self::Reckless => "reckless",
            Self::Viridithas => "viridithas",
            Self::Obsidian => "obsidian",
            Self::PlentyChess => "plentychess",
            Self::Ateed => "ateed",
            Self::Lc0 => "lc0",
        }
    }
}

impl Display for NetworkFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embedded => f.write_str("Embedded"),
            Self::Akimbo => f.write_str("Akimbo"),
            Self::Stockfish => f.write_str("Stockfish"),
            Self::Reckless => f.write_str("Reckless"),
            Self::Viridithas => f.write_str("Viridithas"),
            Self::Obsidian => f.write_str("Obsidian"),
            Self::PlentyChess => f.write_str("PlentyChess"),
            Self::Ateed => f.write_str("Ateed"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NnueNetworkInfo {
    pub name: String,
    pub format: NetworkFormat,
    pub architecture: String,
    pub hidden_size: usize,
    pub num_buckets: usize,
    pub qa: i32,
    pub qb: i32,
    pub scale: i32,
    pub file_size: u64,
}

pub enum NnueNetworkParameters<'a> {
    Akimbo(&'a Network),
    #[cfg(feature = "stockfish-nnue")]
    Stockfish(&'a StockfishNetwork),
    #[cfg(feature = "reckless-nnue")]
    Reckless(&'a RecklessNetwork),
    #[cfg(feature = "viridithas-nnue")]
    Viridithas(&'a super::viridithas_format::ViridithasNetwork),
    #[cfg(feature = "obsidian-nnue")]
    Obsidian(&'a super::obsidian_format::ObsidianNetwork),
    #[cfg(feature = "plentychess-nnue")]
    PlentyChess(&'a super::plentychess_format::PlentyChessNetwork),
    #[cfg(feature = "ateed-nnue")]
    Ateed(&'a super::ateed_format::AteedNetwork),
}

pub trait NnueNetworkSource {
    fn parameters(&self) -> NnueNetworkParameters<'_>;
    fn info(&self) -> NnueNetworkInfo;
    fn search_profile(&self) -> NnueSearchProfile;

    #[inline]
    fn preset_hint(&self) -> &'static str {
        self.search_profile().as_str()
    }
}

pub enum ActiveNetwork {
    Embedded,
    #[cfg(feature = "stockfish-nnue")]
    EmbeddedStockfish,
    #[cfg(feature = "reckless-nnue")]
    EmbeddedReckless,
    ExternalAkimbo {
        network: Box<Network>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "stockfish-nnue")]
    ExternalStockfish {
        network: Box<StockfishNetwork>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "reckless-nnue")]
    ExternalReckless {
        network: Box<RecklessNetwork>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "viridithas-nnue")]
    ExternalViridithas {
        network: Box<super::viridithas_format::ViridithasNetwork>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "obsidian-nnue")]
    ExternalObsidian {
        network: Box<super::obsidian_format::ObsidianNetwork>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "plentychess-nnue")]
    ExternalPlentyChess {
        network: Box<super::plentychess_format::PlentyChessNetwork>,
        info: NnueNetworkInfo,
    },
    #[cfg(feature = "ateed-nnue")]
    ExternalAteed {
        network: Box<super::ateed_format::AteedNetwork>,
        info: NnueNetworkInfo,
    },
}

/// Default network selected by the engine.
#[inline]
pub fn default_embedded_network() -> ActiveNetwork {
    #[cfg(feature = "reckless-nnue")]
    {
        ActiveNetwork::EmbeddedReckless
    }
    #[cfg(all(not(feature = "reckless-nnue"), feature = "stockfish-nnue"))]
    {
        ActiveNetwork::EmbeddedStockfish
    }
    #[cfg(all(not(feature = "stockfish-nnue"), not(feature = "reckless-nnue")))]
    {
        ActiveNetwork::Embedded
    }
}

/// Embedded network that matches an explicit EvalPreset / search-profile name.
///
/// Returns `None` for `"auto"` or unknown names. Callers must not apply a preset's
/// search parameters without also installing this network — otherwise Stockfish
/// pruning runs on a Reckless (or Akimbo) evaluator.
#[inline]
pub fn embedded_network_for_preset(preset: &str) -> Option<ActiveNetwork> {
    match preset {
        "akimbo" => Some(ActiveNetwork::Embedded),
        #[cfg(feature = "stockfish-nnue")]
        "stockfish" => Some(ActiveNetwork::EmbeddedStockfish),
        #[cfg(feature = "reckless-nnue")]
        "reckless" => Some(ActiveNetwork::EmbeddedReckless),
        "viridithas" | "obsidian" | "plentychess" | "ateed" | "lc0" => None,
        _ => None,
    }
}

impl NnueNetworkSource for ActiveNetwork {
    #[inline(always)]
    fn parameters(&self) -> NnueNetworkParameters<'_> {
        match self {
            Self::Embedded => NnueNetworkParameters::Akimbo(net()),
            #[cfg(feature = "stockfish-nnue")]
            Self::EmbeddedStockfish => {
                NnueNetworkParameters::Stockfish(super::stockfish_format::embedded())
            }
            #[cfg(feature = "reckless-nnue")]
            Self::EmbeddedReckless => {
                NnueNetworkParameters::Reckless(super::reckless_format::embedded())
            }
            Self::ExternalAkimbo { network, .. } => NnueNetworkParameters::Akimbo(network),
            #[cfg(feature = "stockfish-nnue")]
            Self::ExternalStockfish { network, .. } => NnueNetworkParameters::Stockfish(network),
            #[cfg(feature = "reckless-nnue")]
            Self::ExternalReckless { network, .. } => NnueNetworkParameters::Reckless(network),
            #[cfg(feature = "viridithas-nnue")]
            Self::ExternalViridithas { network, .. } => NnueNetworkParameters::Viridithas(network),
            #[cfg(feature = "obsidian-nnue")]
            Self::ExternalObsidian { network, .. } => NnueNetworkParameters::Obsidian(network),
            #[cfg(feature = "plentychess-nnue")]
            Self::ExternalPlentyChess { network, .. } => {
                NnueNetworkParameters::PlentyChess(network)
            }
            #[cfg(feature = "ateed-nnue")]
            Self::ExternalAteed { network, .. } => NnueNetworkParameters::Ateed(network),
        }
    }

    fn info(&self) -> NnueNetworkInfo {
        match self {
            Self::Embedded => embedded_info(),
            #[cfg(feature = "stockfish-nnue")]
            Self::EmbeddedStockfish => embedded_stockfish_info(),
            #[cfg(feature = "reckless-nnue")]
            Self::EmbeddedReckless => embedded_reckless_info(),
            Self::ExternalAkimbo { info, .. } => info.clone(),
            #[cfg(feature = "stockfish-nnue")]
            Self::ExternalStockfish { info, .. } => info.clone(),
            #[cfg(feature = "reckless-nnue")]
            Self::ExternalReckless { info, .. } => info.clone(),
            #[cfg(feature = "viridithas-nnue")]
            Self::ExternalViridithas { info, .. } => info.clone(),
            #[cfg(feature = "obsidian-nnue")]
            Self::ExternalObsidian { info, .. } => info.clone(),
            #[cfg(feature = "plentychess-nnue")]
            Self::ExternalPlentyChess { info, .. } => info.clone(),
            #[cfg(feature = "ateed-nnue")]
            Self::ExternalAteed { info, .. } => info.clone(),
        }
    }

    fn search_profile(&self) -> NnueSearchProfile {
        match self {
            Self::Embedded | Self::ExternalAkimbo { .. } => NnueSearchProfile::Akimbo,
            #[cfg(feature = "stockfish-nnue")]
            Self::EmbeddedStockfish | Self::ExternalStockfish { .. } => {
                NnueSearchProfile::Stockfish
            }
            #[cfg(feature = "reckless-nnue")]
            Self::EmbeddedReckless => NnueSearchProfile::Reckless,
            #[cfg(feature = "reckless-nnue")]
            Self::ExternalReckless { .. } => NnueSearchProfile::Reckless,
            #[cfg(feature = "viridithas-nnue")]
            Self::ExternalViridithas { .. } => NnueSearchProfile::Viridithas,
            #[cfg(feature = "obsidian-nnue")]
            Self::ExternalObsidian { .. } => NnueSearchProfile::Obsidian,
            #[cfg(feature = "plentychess-nnue")]
            Self::ExternalPlentyChess { .. } => NnueSearchProfile::PlentyChess,
            #[cfg(feature = "ateed-nnue")]
            Self::ExternalAteed { .. } => NnueSearchProfile::Ateed,
        }
    }
}

#[cfg(feature = "stockfish-nnue")]
fn embedded_stockfish_info() -> NnueNetworkInfo {
    NnueNetworkInfo {
        name: "Embedded Stockfish nn-ab28990d4ea3".to_string(),
        format: NetworkFormat::Stockfish,
        architecture: format!(
            "HalfKAv2_hm+FullThreats+PP_3Wide 86896->1024->32->32->1 [{}]",
            super::stockfish_simd::selected_backend(),
        ),
        hidden_size: STOCKFISH_L1,
        num_buckets: STOCKFISH_BUCKETS,
        qa: 255,
        qb: 64,
        scale: 600,
        file_size: STOCKFISH_FILE_SIZE as u64,
    }
}

#[cfg(feature = "reckless-nnue")]
fn embedded_reckless_info() -> NnueNetworkInfo {
    NnueNetworkInfo {
        name: "Embedded threat-aware v60".to_string(),
        format: NetworkFormat::Reckless,
        architecture: format!(
            "piece+threat FT→{RECKLESS_HIDDEN}→{RECKLESS_HIDDEN}×{RECKLESS_OUTPUT_BUCKETS}→16→32→1 [{}]",
            super::reckless_simd::selected_backend().name(),
        ),
        hidden_size: RECKLESS_HIDDEN,
        num_buckets: RECKLESS_INPUT_BUCKETS,
        qa: 255,
        qb: 64,
        scale: 380,
        file_size: RECKLESS_FILE_SIZE,
    }
}

fn embedded_info() -> NnueNetworkInfo {
    NnueNetworkInfo {
        name: "Embedded Akimbo 1024".to_string(),
        format: NetworkFormat::Embedded,
        architecture: format!("768→{HIDDEN}×2→1 SCReLU"),
        hidden_size: HIDDEN,
        num_buckets: NUM_BUCKETS,
        qa: 255,
        qb: 64,
        scale: 400,
        file_size: std::mem::size_of::<Network>() as u64,
    }
}

pub fn enabled_network_formats() -> Vec<NetworkFormat> {
    vec![
        #[cfg(feature = "stockfish-nnue")]
        NetworkFormat::Stockfish,
        #[cfg(feature = "reckless-nnue")]
        NetworkFormat::Reckless,
        #[cfg(feature = "akimbo-nnue")]
        NetworkFormat::Akimbo,
        #[cfg(feature = "viridithas-nnue")]
        NetworkFormat::Viridithas,
        #[cfg(feature = "obsidian-nnue")]
        NetworkFormat::Obsidian,
        #[cfg(feature = "plentychess-nnue")]
        NetworkFormat::PlentyChess,
        #[cfg(feature = "ateed-nnue")]
        NetworkFormat::Ateed,
    ]
}

pub fn load_network(path: &Path) -> Result<ActiveNetwork, String> {
    if !path.is_file() {
        return Err(format!("NNUE file not found: {}", path.display()));
    }

    #[cfg(feature = "reckless-nnue")]
    let file_size = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect NNUE file '{}': {error}", path.display()))?
        .len();

    #[cfg(feature = "reckless-nnue")]
    if file_size == RECKLESS_FILE_SIZE {
        return load_reckless_network(path, file_size);
    }

    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read NNUE file '{}': {error}", path.display()))?;

    #[cfg(feature = "ateed-nnue")]
    if super::ateed_format::looks_like_ateed(path, &bytes) {
        return load_ateed_network(path, &bytes);
    }

    #[cfg(feature = "viridithas-nnue")]
    if super::viridithas_format::looks_like_viridithas(path, &bytes) {
        return load_viridithas_network(path, &bytes);
    }

    #[cfg(feature = "plentychess-nnue")]
    if super::plentychess_format::is_plentychess_path(path) {
        return load_plentychess_network(path, &bytes);
    }

    #[cfg(feature = "obsidian-nnue")]
    if super::obsidian_format::is_obsidian_path(path)
        || bytes.len() as u64 == super::obsidian_format::FILE_SIZE
    {
        return load_obsidian_network(path, &bytes);
    }

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "nnue" {
        #[cfg(feature = "stockfish-nnue")]
        return load_stockfish_network(path);
        #[cfg(not(feature = "stockfish-nnue"))]
        return Err(format!(
            "Stockfish NNUE '{}' is not compatible with Mujrim's native evaluator",
            path.display()
        ));
    }

    load_native_network(path)
}

#[cfg(feature = "stockfish-nnue")]
fn load_stockfish_network(path: &Path) -> Result<ActiveNetwork, String> {
    let file_size = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect '{}': {error}", path.display()))?
        .len();
    let network = super::stockfish_format::load(path)?;
    let name = path.file_stem().map_or_else(
        || "External Stockfish network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalStockfish {
        network,
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Stockfish,
            architecture: format!(
                "HalfKAv2_hm+FullThreats+PP_3Wide 86896->1024->32->32->1 [{}]",
                super::stockfish_simd::selected_backend(),
            ),
            hidden_size: STOCKFISH_L1,
            num_buckets: STOCKFISH_BUCKETS,
            qa: 255,
            qb: 64,
            scale: 600,
            file_size,
        },
    })
}

#[cfg(feature = "akimbo-nnue")]
fn load_native_network(path: &Path) -> Result<ActiveNetwork, String> {
    let network = super::akimbo_format::load(path)?;
    let file_size = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect '{}': {error}", path.display()))?
        .len();
    let name = path.file_stem().map_or_else(
        || "External Akimbo network".to_string(),
        |name| name.to_string_lossy().into(),
    );
    Ok(ActiveNetwork::ExternalAkimbo {
        network,
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Akimbo,
            architecture: format!("768→{HIDDEN}×2→1 SCReLU"),
            hidden_size: HIDDEN,
            num_buckets: NUM_BUCKETS,
            qa: 255,
            qb: 64,
            scale: 400,
            file_size,
        },
    })
}

#[cfg(feature = "reckless-nnue")]
fn load_reckless_network(path: &Path, file_size: u64) -> Result<ActiveNetwork, String> {
    let network = super::reckless_format::load(path)?;
    let name = path.file_stem().map_or_else(
        || "External Reckless network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalReckless {
        network,
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Reckless,
            architecture: format!(
                "piece+threat FT→{RECKLESS_HIDDEN}→{RECKLESS_HIDDEN}×{RECKLESS_OUTPUT_BUCKETS}→16→32→1"
            ),
            hidden_size: RECKLESS_HIDDEN,
            num_buckets: RECKLESS_INPUT_BUCKETS,
            qa: 255,
            qb: 64,
            scale: 380,
            file_size,
        },
    })
}

#[cfg(feature = "viridithas-nnue")]
fn load_viridithas_network(path: &Path, bytes: &[u8]) -> Result<ActiveNetwork, String> {
    let network = super::viridithas_format::ViridithasNetwork::from_bytes(bytes)?;
    let hidden = network.hidden();
    let scale = network.scale();
    let architecture = network.architecture();
    types::init();
    let startpos = network.evaluate(&types::Board::new());
    if startpos.abs() > 2_500 {
        return Err(format!(
            "Viridithas net startpos eval {startpos} is outside a sane opening range; keeping the search profile and the implemented fallback net"
        ));
    }
    let name = path.file_stem().map_or_else(
        || "External Viridithas network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalViridithas {
        network: Box::new(network),
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Viridithas,
            architecture,
            hidden_size: hidden,
            num_buckets: super::viridithas_format::KING_BUCKETS,
            qa: super::viridithas_format::QA,
            qb: 64,
            scale,
            file_size: bytes.len() as u64,
        },
    })
}

#[cfg(feature = "plentychess-nnue")]
fn load_plentychess_network(path: &Path, bytes: &[u8]) -> Result<ActiveNetwork, String> {
    let network = super::plentychess_format::PlentyChessNetwork::from_compressed_bytes(bytes)?;
    let name = path.file_stem().map_or_else(
        || "External PlentyChess network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalPlentyChess {
        network: Box::new(network),
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::PlentyChess,
            architecture:
                "768×12 + 4560 pawn-pair + 59808 threat → 1024 pairwise-CReLU → 16 → 32 → 1 ×8 (0179r)"
                    .to_string(),
            hidden_size: super::plentychess_format::L1,
            num_buckets: super::plentychess_format::KING_BUCKETS,
            qa: super::plentychess_format::NETWORK_QA,
            qb: super::plentychess_format::NETWORK_QB,
            scale: super::plentychess_format::NETWORK_SCALE,
            file_size: bytes.len() as u64,
        },
    })
}

#[cfg(feature = "obsidian-nnue")]
fn load_obsidian_network(path: &Path, bytes: &[u8]) -> Result<ActiveNetwork, String> {
    let network = super::obsidian_format::ObsidianNetwork::from_bytes(bytes)?;
    let name = path.file_stem().map_or_else(
        || "External Obsidian network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalObsidian {
        network: Box::new(network),
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Obsidian,
            architecture: "768→1536→16→32→1 (13 king buckets, 8 output buckets)".to_string(),
            hidden_size: super::obsidian_format::L1,
            num_buckets: super::obsidian_format::KING_BUCKETS,
            qa: super::obsidian_format::NETWORK_QA,
            qb: super::obsidian_format::NETWORK_QB,
            scale: super::obsidian_format::NETWORK_SCALE,
            file_size: bytes.len() as u64,
        },
    })
}

#[cfg(feature = "ateed-nnue")]
fn load_ateed_network(path: &Path, bytes: &[u8]) -> Result<ActiveNetwork, String> {
    let network = super::ateed_format::AteedNetwork::from_bytes(bytes)?;
    let name = path.file_stem().map_or_else(
        || "External Ateed network".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(ActiveNetwork::ExternalAteed {
        network: Box::new(network),
        info: NnueNetworkInfo {
            name,
            format: NetworkFormat::Ateed,
            architecture:
                "768×8hm i16 + 4560 pawn-pair i8 → 1024 CReLU → 4-expert MoE (16→32→eval+WDL)"
                    .to_string(),
            hidden_size: super::ateed_format::L1,
            num_buckets: super::ateed_format::KING_BUCKETS,
            qa: super::ateed_format::QA,
            qb: super::ateed_format::QB,
            scale: super::ateed_format::SCALE,
            file_size: bytes.len() as u64,
        },
    })
}

#[cfg(not(feature = "akimbo-nnue"))]
fn load_native_network(path: &Path) -> Result<ActiveNetwork, String> {
    Err(format!(
        "Akimbo NNUE loading is disabled in this build: {}",
        path.display()
    ))
}

pub fn auto_detect_network(dir: &Path) -> (Option<ActiveNetwork>, String) {
    auto_detect_networks(&[dir.to_path_buf()])
}

/// Scan `nnue/` plus `dist/nnue` (and other search roots) for a compatible net.
pub fn auto_detect_from_search_roots() -> (Option<ActiveNetwork>, String) {
    auto_detect_networks(&nnue_search_directories())
}

fn auto_detect_networks(dirs: &[PathBuf]) -> (Option<ActiveNetwork>, String) {
    let mut paths = Vec::new();
    for dir in dirs {
        match candidate_paths(dir) {
            Ok(found) => paths.extend(found),
            Err(message) => return (None, message),
        }
    }
    let mut paths = paths
        .into_iter()
        .map(|path| (network_priority(&path), path))
        .collect::<Vec<_>>();
    paths.sort_unstable_by_key(|(priority, _)| std::cmp::Reverse(*priority));

    let mut failures = Vec::new();
    for (_, path) in paths {
        match load_network(&path) {
            Ok(network) => {
                let name = network.info().name;
                return (Some(network), format!("Auto-loaded NNUE network: {name}"));
            }
            Err(error) => failures.push(error),
        }
    }

    let scanned = dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let message = if failures.is_empty() {
        format!("No compatible NNUE files found in [{scanned}]; using embedded network")
    } else {
        format!(
            "No compatible NNUE files in [{scanned}]; using embedded network ({})",
            failures.join("; ")
        )
    };
    (None, message)
}

/// Directories searched for on-disk nets: `MUJRIM_NNUE`, `nnue/`, `dist/<os-arch>/nnue`.
pub fn nnue_search_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(explicit) = std::env::var_os("MUJRIM_NNUE") {
        let explicit = PathBuf::from(explicit);
        if explicit.is_dir() {
            dirs.push(explicit);
        } else if let Some(parent) = explicit.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable
            .parent()
            .into_iter()
            .flat_map(|path| path.ancestors())
            .take(7)
        {
            dirs.push(ancestor.join("nnue"));
            dirs.push(ancestor.join("dist").join("nnue"));
            dirs.push(
                ancestor
                    .join("dist")
                    .join(format!(
                        "{}-{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ))
                    .join("nnue"),
            );
        }
    }
    if let Ok(current) = std::env::current_dir() {
        dirs.push(current.join("nnue"));
        dirs.push(current.join("dist").join("nnue"));
        dirs.push(
            current
                .join("dist")
                .join(format!(
                    "{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ))
                .join("nnue"),
        );
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"));
    dirs.sort();
    dirs.dedup();
    dirs.retain(|dir| dir.is_dir());
    dirs
}

pub fn discover_named_network(filename: &str) -> Option<PathBuf> {
    nnue_search_directories()
        .into_iter()
        .map(|dir| dir.join(filename))
        .find(|path| path.is_file())
}

/// Canonical on-disk Ateed artifact shared by train, eval, and search.
pub const ATEED_NETWORK_FILENAME: &str = "ateed_default.bin";

/// First writable `nnue/` next to the binary or CWD (created if needed).
pub fn writable_nnue_directory() -> PathBuf {
    for dir in nnue_search_directories() {
        let name = dir.file_name().and_then(|name| name.to_str());
        if name == Some("nnue") && !dir.ends_with("resources") && dir.is_dir() {
            return dir;
        }
    }
    let dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("nnue");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Existing `ateed_default.bin` if discovered, otherwise `nnue/ateed_default.bin`.
pub fn ateed_artifact_path() -> PathBuf {
    discover_named_network(ATEED_NETWORK_FILENAME)
        .unwrap_or_else(|| writable_nnue_directory().join(ATEED_NETWORK_FILENAME))
}

/// Map a user/CLI output path onto the shared Ateed artifact when they asked
/// for the default filename.
pub fn resolve_ateed_output_path(path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let file_name = path.file_name().and_then(|name| name.to_str());
    let parent_empty = path
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty());
    if file_name == Some(ATEED_NETWORK_FILENAME) && parent_empty {
        return ateed_artifact_path();
    }
    if parent_empty {
        return writable_nnue_directory().join(path);
    }
    path.to_path_buf()
}

/// Bundled official Lc0 transformer (BT4-it332). Evaluated by official lc0, not in-process.
pub const LC0_BUNDLED_WEIGHTS_NAME: &str = "lc0_bt4.pb.gz";

/// Filenames searched for official lc0 `--weights`, strongest bundled net first.
pub const LC0_WEIGHT_FILENAMES: &[&str] = &[
    LC0_BUNDLED_WEIGHTS_NAME,
    "weights.pb.gz",
    "lc0_t1_512.pb.gz",
    "lc0_default.pb.gz",
    "192x15-2024.pb.gz",
    "lc0.pb.gz",
];

fn is_usable_lc0_weights(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .is_ok_and(|metadata| metadata.len() > 1_000_000)
}

/// Locate the bundled or downloaded official Lc0 `.pb.gz` weights.
pub fn discover_lc0_weights() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("MUJRIM_LC0_WEIGHTS") {
        let path = PathBuf::from(explicit);
        if is_usable_lc0_weights(&path) {
            return Some(path);
        }
    }
    LC0_WEIGHT_FILENAMES
        .iter()
        .find_map(|name| discover_named_network(name).filter(|path| is_usable_lc0_weights(path)))
}

/// Load the embedded net for a preset, or the matching file from `nnue/` / `dist/nnue`.
pub fn load_network_for_preset(preset: &str) -> Result<ActiveNetwork, String> {
    if let Some(embedded) = embedded_network_for_preset(preset) {
        return Ok(embedded);
    }
    let names: &[&str] = match preset {
        "viridithas" => &["sandhi-s2-b200.nnue.zst", "viri_default.nnue.zst"],
        "obsidian" => &["obs_default.bin", "net89perm.bin"],
        "plentychess" | "plenty" => &["plenty_default.bin", "0179r.bin"],
        "ateed" => &["ateed_default.bin"],
        "lc0" => {
            return Err(
                "Lc0 transformer nets (.pb.gz) are not an in-process NNUE; use the official lc0 binary"
                    .to_string(),
            );
        }
        _ => {
            return Err(format!(
                "preset '{preset}' has no embedded or on-disk network mapping"
            ));
        }
    };
    for name in names {
        if let Some(path) = discover_named_network(name) {
            return load_network(&path);
        }
    }
    Err(format!(
        "no {preset} network found in nnue/ or dist/nnue (looked for {})",
        names.join(", ")
    ))
}

fn candidate_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let paths = std::fs::read_dir(dir)
        .map_err(|error| format!("failed to read NNUE directory '{}': {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "bin" | "net" | "nnue" | "zst"
                    )
                })
        })
        .collect();
    Ok(paths)
}

#[cfg(not(feature = "embedded-networks"))]
pub(crate) fn discover_network_file(
    expected_size: u64,
    expected_sha256: &str,
) -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    if let Some(explicit) = std::env::var_os("MUJRIM_NNUE") {
        let explicit = PathBuf::from(explicit);
        if explicit.is_file() {
            paths.push(explicit);
        } else {
            collect_network_files(&explicit, 0, &mut paths);
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        for ancestor in executable
            .parent()
            .into_iter()
            .flat_map(|path| path.ancestors())
            .take(7)
        {
            collect_network_files(&ancestor.join("nnue"), 0, &mut paths);
        }
    }
    if let Ok(current) = std::env::current_dir() {
        collect_network_files(&current.join("nnue"), 0, &mut paths);
        collect_network_files(&current.join("dist").join("nnue"), 0, &mut paths);
    }
    // Checked-in eval payloads (CI + workspace tests of UI/search dependents).
    collect_network_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"),
        0,
        &mut paths,
    );
    paths.sort();
    paths.dedup();

    let mut rejected = Vec::new();
    for path in paths {
        let size = path.metadata().ok().map(|metadata| metadata.len());
        let hash = (size == Some(expected_size))
            .then(|| file_sha256(&path))
            .flatten();
        if size == Some(expected_size) && hash.as_deref() == Some(expected_sha256) {
            return Ok(path);
        }
        if rejected.len() < 8 {
            rejected.push(format!(
                "{} (size={}, sha256={})",
                path.display(),
                size.map_or_else(|| "unreadable".to_string(), |value| value.to_string()),
                hash.as_deref().unwrap_or("not-computed")
            ));
        }
    }
    Err(format!(
        "no NNUE with SHA-256 {expected_sha256} and size {expected_size} was found in an nnue/ directory; inspected: {}",
        if rejected.is_empty() {
            "no candidate files".to_string()
        } else {
            rejected.join(", ")
        }
    ))
}

#[cfg(not(feature = "embedded-networks"))]
fn collect_network_files(directory: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            output.push(path);
        } else if path.is_dir() {
            collect_network_files(&path, depth + 1, output);
        }
    }
}

#[cfg(any(
    not(feature = "embedded-networks"),
    feature = "stockfish-nnue",
    feature = "reckless-nnue"
))]
fn file_sha256(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
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

fn network_priority(path: &Path) -> u8 {
    #[cfg(feature = "reckless-nnue")]
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() == RECKLESS_FILE_SIZE)
        && file_sha256(path).as_deref()
            == Some("7f587dfb1fe5d74d53909328afa6fd51650c8c7f45907602db7fbb1e52948c61")
    {
        return 5;
    }
    #[cfg(feature = "stockfish-nnue")]
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() == STOCKFISH_FILE_SIZE as u64)
        && file_sha256(path).as_deref()
            == Some("ab28990d4ea3d5c97f7d3918bc5dd5061609330369fe00c2d93a34d4777b5552")
    {
        return 4;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("zst") => 3,
        Some(extension) if extension.eq_ignore_ascii_case("nnue") => 3,
        Some(extension) if extension.eq_ignore_ascii_case("bin") => 2,
        Some(extension) if extension.eq_ignore_ascii_case("net") => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_network_metadata_matches_runtime_layout() {
        let active = ActiveNetwork::Embedded;
        let info = active.info();
        assert_eq!(info.hidden_size, HIDDEN);
        assert_eq!(info.file_size as usize, std::mem::size_of::<Network>());
        let network = match active.parameters() {
            NnueNetworkParameters::Akimbo(network) => network,
            #[cfg(feature = "reckless-nnue")]
            NnueNetworkParameters::Reckless(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
            #[cfg(feature = "stockfish-nnue")]
            NnueNetworkParameters::Stockfish(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
            #[cfg(feature = "viridithas-nnue")]
            NnueNetworkParameters::Viridithas(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
            #[cfg(feature = "obsidian-nnue")]
            NnueNetworkParameters::Obsidian(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
            #[cfg(feature = "plentychess-nnue")]
            NnueNetworkParameters::PlentyChess(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
            #[cfg(feature = "ateed-nnue")]
            NnueNetworkParameters::Ateed(_) => {
                panic!("embedded network must use the Akimbo evaluator")
            }
        };
        assert!(std::ptr::eq(network, net()));
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn embedded_reckless_network_exposes_static_parameters() {
        let active = ActiveNetwork::EmbeddedReckless;
        let info = active.info();
        assert_eq!(info.format, NetworkFormat::Reckless);
        assert_eq!(info.file_size, RECKLESS_FILE_SIZE);
        let NnueNetworkParameters::Reckless(network) = active.parameters() else {
            panic!("embedded threat-aware network must use its native evaluator");
        };
        assert!(std::ptr::eq(
            network,
            super::super::reckless_format::embedded()
        ));
    }

    #[test]
    fn missing_network_is_rejected() {
        let error = load_network(Path::new("missing-mujrim-network.bin"))
            .err()
            .unwrap();
        assert!(error.contains("not found"));
    }

    #[test]
    fn invalid_stockfish_format_is_rejected() {
        let path = std::env::temp_dir().join("mujrim-adapter-test.nnue");
        std::fs::write(&path, b"not a network").unwrap();
        let error = load_network(&path).err().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(error.contains("Stockfish NNUE"));
    }

    #[cfg(feature = "stockfish-nnue")]
    #[test]
    fn embedded_stockfish_network_exposes_current_static_parameters() {
        let active = ActiveNetwork::EmbeddedStockfish;
        let info = active.info();
        assert_eq!(info.format, NetworkFormat::Stockfish);
        assert_eq!(info.file_size, STOCKFISH_FILE_SIZE as u64);
        let NnueNetworkParameters::Stockfish(network) = active.parameters() else {
            panic!("embedded Stockfish network must use its native evaluator");
        };
        assert!(std::ptr::eq(
            network,
            super::super::stockfish_format::embedded()
        ));
    }

    #[test]
    fn embedded_network_for_preset_matches_named_profiles() {
        assert!(matches!(
            embedded_network_for_preset("akimbo"),
            Some(ActiveNetwork::Embedded)
        ));
        #[cfg(feature = "stockfish-nnue")]
        assert!(matches!(
            embedded_network_for_preset("stockfish"),
            Some(ActiveNetwork::EmbeddedStockfish)
        ));
        #[cfg(feature = "reckless-nnue")]
        assert!(matches!(
            embedded_network_for_preset("reckless"),
            Some(ActiveNetwork::EmbeddedReckless)
        ));
        assert!(embedded_network_for_preset("auto").is_none());
        assert!(embedded_network_for_preset("unknown").is_none());
    }

    #[test]
    fn strongest_compatible_embedded_network_is_the_default() {
        let active = default_embedded_network();
        #[cfg(feature = "reckless-nnue")]
        {
            assert_eq!(active.info().format, NetworkFormat::Reckless);
            assert_eq!(active.preset_hint(), "reckless");
        }
        #[cfg(all(not(feature = "reckless-nnue"), feature = "stockfish-nnue"))]
        {
            assert_eq!(active.info().format, NetworkFormat::Stockfish);
            assert_eq!(active.preset_hint(), "stockfish");
        }
        #[cfg(all(not(feature = "stockfish-nnue"), not(feature = "reckless-nnue")))]
        {
            assert_eq!(active.info().format, NetworkFormat::Embedded);
            assert_eq!(active.preset_hint(), "akimbo");
        }
    }

    #[cfg(feature = "stockfish-nnue")]
    #[test]
    fn stockfish_file_remains_available_but_does_not_override_v60() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(super::super::stockfish_format::NETWORK_FILENAME);
        assert_eq!(network_priority(&path), 4);
    }

    #[cfg(feature = "viridithas-nnue")]
    #[test]
    fn load_network_parses_a_viridithas_piece_feature_file() {
        let path = std::env::temp_dir().join("viri_default.bin");
        let bytes = vec![0u8; super::super::viridithas_format::simple_size(256)];
        std::fs::write(&path, bytes).unwrap();
        let active = load_network(&path).expect("viridithas layout");
        let _ = std::fs::remove_file(&path);
        assert_eq!(active.info().format, NetworkFormat::Viridithas);
        assert_eq!(active.search_profile(), NnueSearchProfile::Viridithas);
    }

    #[test]
    fn obsidian_nnue_feature_does_not_enable_stockfish() {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        let line = manifest
            .lines()
            .find(|line| line.starts_with("obsidian-nnue"))
            .expect("obsidian-nnue feature");
        assert_eq!(line, "obsidian-nnue = [\"nnue\"]");
        assert!(!line.contains("stockfish-nnue"));
    }

    #[cfg(feature = "obsidian-nnue")]
    #[test]
    fn obsidian_nnue_reuses_i16_feature_simd() {
        assert!(!super::super::stockfish_simd::selected_backend().is_empty());
    }

    #[cfg(all(feature = "obsidian-nnue", not(feature = "stockfish-nnue")))]
    #[test]
    fn obsidian_only_build_excludes_stockfish_format() {
        assert!(!enabled_network_formats().contains(&NetworkFormat::Stockfish));
        assert!(enabled_network_formats().contains(&NetworkFormat::Obsidian));
        let path = std::env::temp_dir().join("mujrim-obs-only.nnue");
        std::fs::write(&path, [0u8; 16]).unwrap();
        let error = load_network(&path).err().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            error.contains("Stockfish") && error.contains("not compatible"),
            "{error}"
        );
    }

    #[cfg(feature = "obsidian-nnue")]
    #[test]
    fn load_network_identifies_obsidian_by_filename() {
        let path = std::env::temp_dir().join("obs_default.bin");
        std::fs::write(&path, [0u8; 16]).unwrap();
        let error = load_network(&path).err().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("Obsidian"), "{error}");
    }

    #[test]
    fn auto_detect_accepts_zst_and_bin_extensions() {
        let zst = Path::new("viri_default.nnue.zst");
        let bin = Path::new("obs_default.bin");
        assert_eq!(zst.extension().and_then(|ext| ext.to_str()), Some("zst"));
        assert!(matches!(
            bin.extension().and_then(|ext| ext.to_str()),
            Some("bin")
        ));
        assert!(network_priority(zst) >= 3);
    }

    #[test]
    fn search_roots_include_dist_nnue_when_present() {
        let cwd = std::env::current_dir().expect("cwd");
        let platform = cwd
            .join("dist")
            .join(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
            .join("nnue");
        let legacy = cwd.join("dist").join("nnue");
        let roots = nnue_search_directories();
        if platform.is_dir() {
            assert!(
                roots.iter().any(|dir| dir == &platform),
                "dist/<os-arch>/nnue must be a discovery root"
            );
        }
        if legacy.is_dir() {
            assert!(
                roots.iter().any(|dir| dir == &legacy),
                "legacy dist/nnue must stay a discovery root"
            );
        }
    }

    #[cfg(feature = "viridithas-nnue")]
    #[test]
    fn preset_loader_finds_downloaded_viridithas_file() {
        let Some(path) = discover_named_network("sandhi-s2-b200.nnue.zst")
            .or_else(|| discover_named_network("viri_default.nnue.zst"))
        else {
            return;
        };
        let net = load_network(&path).expect("official Viridithas 20 sandhi net must load");
        assert_eq!(net.search_profile(), NnueSearchProfile::Viridithas);
        assert_eq!(net.info().format, NetworkFormat::Viridithas);
        assert!(
            net.info().architecture.contains("sandhi"),
            "viri_default must load sandhi, got {}",
            net.info().architecture
        );
        if let ActiveNetwork::ExternalViridithas { network, .. } = &net {
            types::init();
            let startpos = network.evaluate(&types::Board::new());
            assert_eq!(startpos, 43, "official Viridithas 20 startpos eval");
        }
    }

    #[cfg(feature = "obsidian-nnue")]
    #[test]
    fn preset_loader_finds_downloaded_obsidian_net() {
        if discover_named_network("obs_default.bin").is_none() {
            return;
        }
        let net = load_network_for_preset("obsidian").expect("obs net");
        assert_eq!(net.search_profile(), NnueSearchProfile::Obsidian);
        assert_eq!(net.info().format, NetworkFormat::Obsidian);
        assert_ne!(net.info().format, NetworkFormat::Stockfish);
        assert!(
            !net.info().architecture.contains("HalfKA"),
            "obsidian preset must not load a Stockfish net, got {}",
            net.info().architecture
        );
    }

    #[cfg(feature = "ateed-nnue")]
    #[test]
    fn load_network_identifies_ateed_by_magic() {
        let path = std::env::temp_dir().join("mujrim-ateed-magic.bin");
        let bytes = super::super::ateed_format::AteedNetwork::zero().to_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let net = load_network(&path).expect("ATEED001 must decode as Ateed");
        let _ = std::fs::remove_file(&path);
        assert_eq!(net.search_profile(), NnueSearchProfile::Ateed);
        assert_eq!(net.info().format, NetworkFormat::Ateed);
        assert_eq!(net.info().hidden_size, super::super::ateed_format::L1);
        assert_eq!(
            net.info().file_size,
            super::super::ateed_format::FILE_SIZE as u64
        );
    }

    #[cfg(feature = "ateed-nnue")]
    #[test]
    fn preset_loader_finds_downloaded_ateed_net() {
        let Some(path) = discover_named_network("ateed_default.bin") else {
            return;
        };
        let net = load_network(&path).expect("ateed_default must decode as Ateed");
        assert_eq!(net.search_profile(), NnueSearchProfile::Ateed);
        assert_eq!(net.info().format, NetworkFormat::Ateed);
    }

    #[cfg(feature = "plentychess-nnue")]
    #[test]
    fn preset_loader_finds_downloaded_plentychess_net() {
        let Some(path) = discover_named_network("plenty_default.bin")
            .or_else(|| discover_named_network("0179r.bin"))
        else {
            return;
        };
        let net = load_network(&path).expect("0179r must decode as PlentyChess");
        assert_eq!(net.search_profile(), NnueSearchProfile::PlentyChess);
        assert_eq!(net.info().format, NetworkFormat::PlentyChess);
        if let ActiveNetwork::ExternalPlentyChess { network, .. } = &net {
            types::init();
            let startpos = network.evaluate(&types::Board::new());
            assert!(
                startpos.abs() < 250,
                "loaded PlentyChess startpos should be a quiet opening score, got {startpos}"
            );
        }
    }

    #[test]
    fn lc0_preset_does_not_silently_load_an_nnue() {
        let Err(error) = load_network_for_preset("lc0") else {
            panic!("lc0 is not an in-process NNUE");
        };
        assert!(error.contains("official lc0"), "{error}");
    }

    #[test]
    fn lc0_weight_discovery_prefers_bundled_bt4() {
        assert_eq!(LC0_WEIGHT_FILENAMES[0], LC0_BUNDLED_WEIGHTS_NAME);
        assert!(LC0_WEIGHT_FILENAMES.contains(&"lc0_default.pb.gz"));
        if let Some(path) = discover_lc0_weights() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("lc0 weights filename");
            assert!(
                LC0_WEIGHT_FILENAMES.contains(&name),
                "unexpected lc0 weights {name}"
            );
            assert!(
                path.metadata().expect("lc0 weights metadata").len() > 1_000_000,
                "lc0 weights must be a real downloaded .pb.gz"
            );
        }
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(LC0_BUNDLED_WEIGHTS_NAME);
        if bundled.is_file()
            && bundled
                .metadata()
                .is_ok_and(|metadata| metadata.len() > 1_000_000)
        {
            let discovered = discover_lc0_weights().expect("bundled BT4 must be discoverable");
            assert_eq!(
                discovered.file_name(),
                Some(std::ffi::OsStr::new(LC0_BUNDLED_WEIGHTS_NAME))
            );
        }
    }

    #[test]
    fn resolve_ateed_output_path_maps_default_filename_into_nnue() {
        let resolved = resolve_ateed_output_path(ATEED_NETWORK_FILENAME);
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some(ATEED_NETWORK_FILENAME)
        );
        assert!(
            resolved
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                == Some("nnue")
                || resolved
                    .components()
                    .any(|component| component.as_os_str() == "nnue")
        );
        let custom = resolve_ateed_output_path("scratch.bin");
        assert_eq!(
            custom.file_name().and_then(|name| name.to_str()),
            Some("scratch.bin")
        );
        let absolute = resolve_ateed_output_path("/tmp/ateed_default.bin");
        assert_eq!(absolute, PathBuf::from("/tmp/ateed_default.bin"));
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn v60_has_highest_auto_detect_priority() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("reckless_v60.nnue");
        assert_eq!(network_priority(&path), 5);
    }
}
