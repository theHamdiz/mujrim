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

pub mod network;
pub mod accumulator;
pub mod simd;
pub mod feature;
pub mod adapter;

#[cfg(feature = "akimbo-nnue")]
pub mod akimbo_format;

#[cfg(feature = "stockfish-nnue")]
pub mod stockfish_format;

pub use network::{Accumulator, Network, HIDDEN, NUM_BUCKETS, forward, net};
pub use accumulator::NNUEState;
pub use adapter::{ActiveNetwork, NetworkFormat, NnueNetworkInfo, load_network};

