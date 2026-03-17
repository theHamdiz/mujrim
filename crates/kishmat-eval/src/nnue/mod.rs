//! NNUE — KishMat's Neural Network Evaluation module.
//!
//! Architecture: 768→1024×2→1 with king buckets and SCReLU.
//! Binary-compatible with Akimbo's trained net.bin.
//!
//! Modules:
//! - `network`: Network struct, forward pass, feature indexing
//! - `accumulator`: Incremental accumulator state with cache table
//! - `simd`: AVX2/scalar SIMD operations for SCReLU
//! - `feature`: Utility functions for feature manipulation

pub mod network;
pub mod accumulator;
pub mod simd;
pub mod feature;

pub use network::{Accumulator, Network, HIDDEN, NUM_BUCKETS, forward, net};
pub use accumulator::NNUEState;
