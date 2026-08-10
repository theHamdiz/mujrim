//! NNUE — Mujrim's Neural Network Evaluation module.
//!
//! Architecture: 768→1024×2→1 with king buckets and SCReLU.
//! Binary-compatible with Akimbo's trained net.bin.
//!
//! Supports loading external networks via the adapter system:
//! - `akimbo-nnue` feature: Load Akimbo-family networks (768→H×2→1)
//! - `stockfish-nnue` feature: Load Stockfish .nnue files (HalfKAv2_hm)
//! - `reckless-nnue` feature: Load the Reckless v60 threat-aware raw network
//!
//! Modules:
//! - `network`: Network struct, forward pass, feature indexing
//! - `accumulator`: Incremental accumulator state with cache table
//! - `simd`: AVX2/scalar SIMD operations for SCReLU
//! - `feature`: Utility functions for feature manipulation
//! - `adapter`: Trait-based multi-format network abstraction
//! - `akimbo_format`: Akimbo-family network loader
//! - `stockfish_format`: Stockfish .nnue file parser

pub mod accumulator;
pub mod adapter;
pub mod feature;
pub mod network;
pub mod simd;

#[cfg(any(feature = "reckless-nnue", feature = "stockfish-nnue"))]
mod dirty_threats;

#[cfg(feature = "akimbo-nnue")]
pub mod akimbo_format;

#[cfg(feature = "stockfish-nnue")]
pub mod stockfish_format;

#[cfg(feature = "stockfish-nnue")]
mod stockfish_simd;

#[cfg(feature = "reckless-nnue")]
pub mod reckless_format;

#[cfg(feature = "reckless-nnue")]
mod reckless_simd;

pub use accumulator::NNUEState;
pub use adapter::{
    ActiveNetwork, NetworkFormat, NnueNetworkInfo, NnueNetworkParameters, NnueNetworkSource,
    NnueSearchProfile, auto_detect_network, default_embedded_network, enabled_network_formats,
    load_network,
};
pub use network::{Accumulator, HIDDEN, NUM_BUCKETS, Network, forward, forward_with_network, net};
