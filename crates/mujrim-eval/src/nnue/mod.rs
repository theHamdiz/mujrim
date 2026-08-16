//! NNUE — Mujrim's Neural Network Evaluation module.
//!
//! Architecture: 768→1024×2→1 with king buckets and SCReLU.
//! Binary-compatible with Akimbo's trained net.bin.
//!
//! Supports loading external networks via the adapter system:
//! - `akimbo-nnue` feature: Load Akimbo-family networks (768→H×2→1)
//! - `stockfish-nnue` feature: Load Stockfish .nnue files (HalfKAv2_hm)
//! - `reckless-nnue` feature: Load the Reckless v60 threat-aware raw network
//! - `viridithas-nnue` feature: Load Viridithas `.nnue.zst` simple and velarised nets
//! - `obsidian-nnue` feature: Load Obsidian layered `net89perm.bin` nets
//! - `plentychess-nnue` feature: Load PlentyChess SLEB128 `0179r.bin` nets
//! - `ateed-nnue` feature: Load Ateed MoE nets (`ATEED001`)
//!
//! Modules:
//! - `network`: Network struct, forward pass, feature indexing
//! - `accumulator`: Incremental accumulator state with ply stack + Finny cache
//! - `simd`: AVX2/AVX-512/scalar SIMD operations for SCReLU
//! - `feature`: Utility functions for feature manipulation
//! - `adapter`: Trait-based multi-format network abstraction
//! - `akimbo_format`: Akimbo-family network loader
//! - `stockfish_format`: Stockfish .nnue file parser

pub mod accumulator;
pub mod adapter;
mod akimbo_state;
pub mod feature;
pub mod network;
pub mod simd;

#[cfg(any(
    feature = "reckless-nnue",
    feature = "stockfish-nnue",
    feature = "viridithas-nnue",
    feature = "plentychess-nnue"
))]
mod dirty_threats;

#[cfg(feature = "viridithas-nnue")]
mod bit_rays;

#[cfg(feature = "akimbo-nnue")]
pub mod akimbo_format;

#[cfg(feature = "stockfish-nnue")]
pub mod stockfish_format;

#[cfg(any(
    feature = "stockfish-nnue",
    feature = "obsidian-nnue",
    feature = "akimbo-nnue"
))]
mod stockfish_simd;

#[cfg(any(
    feature = "stockfish-nnue",
    feature = "obsidian-nnue",
    feature = "plentychess-nnue",
    feature = "viridithas-nnue"
))]
mod layered_forward;

#[cfg(feature = "reckless-nnue")]
pub mod reckless_format;

#[cfg(feature = "reckless-nnue")]
mod reckless_simd;

#[cfg(feature = "viridithas-nnue")]
pub mod viridithas_format;

#[cfg(feature = "obsidian-nnue")]
pub mod obsidian_format;

#[cfg(feature = "plentychess-nnue")]
pub mod plentychess_format;

#[cfg(feature = "ateed-nnue")]
pub mod ateed_format;

pub use accumulator::NNUEState;
pub use adapter::{
    ActiveNetwork, LC0_BUNDLED_WEIGHTS_NAME, LC0_WEIGHT_FILENAMES, NetworkFormat, NnueNetworkInfo,
    NnueNetworkParameters, NnueNetworkSource, NnueSearchProfile, auto_detect_from_search_roots,
    auto_detect_network, default_embedded_network, discover_lc0_weights, discover_named_network,
    embedded_network_for_preset, enabled_network_formats, load_network, load_network_for_preset,
    nnue_search_directories,
};
#[cfg(feature = "ateed-nnue")]
pub use ateed_format::{AteedEval, AteedExpert, AteedExpertUpdate, AteedNetwork, wdl_variance};
pub use network::{Accumulator, HIDDEN, NUM_BUCKETS, Network, forward, forward_with_network, net};
