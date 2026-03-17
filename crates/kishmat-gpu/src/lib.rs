//! GPU/NPU auto-detection and compute backends for KishMat.
//!
//! Detects available hardware acceleration and provides a unified
//! interface for matrix operations used in NNUE training.

pub mod detect;

pub use detect::{GpuBackend, detect_best_backend, system_info};
