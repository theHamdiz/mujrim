//! NNUE — KishMat's Neural Network Evaluation module.
//!
//! Architecture: 768→1024×2→1 with king buckets and SCReLU.
//! Binary-compatible with Akimbo's trained net.bin.
//!
//! Supports loading external networks via the adapter system:
//! - `akimbo-nnue` feature: Load Akimbo-family networks (768→H×2→1)
//! - `stockfish-nnue` feature: Load Stockfish .nnue files (HalfKAv2_hm)
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

#[cfg(feature = "akimbo-nnue")]
pub mod akimbo_format;

#[cfg(feature = "stockfish-nnue")]
pub mod stockfish_format;

pub use accumulator::NNUEState;
pub use adapter::{
    ActiveNetwork, NetworkFormat, NnueNetworkInfo, NnueNetworkSource, auto_detect_network,
    enabled_network_formats, load_network, load_network_str, network_strength,
    scan_network_files,
};
pub use network::{Accumulator, HIDDEN, NUM_BUCKETS, Network, forward, forward_with_network, net};
