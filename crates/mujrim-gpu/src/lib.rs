//! GPU/NPU auto-detection and compute backends for Mujrim.
//!
//! Detects available hardware acceleration and provides a unified
//! interface for matrix operations used in NNUE training.

pub mod compute;
pub mod detect;

pub use compute::{CpuCompute, TrainCompute, training_compute};
pub use detect::{GpuBackend, detect_best_backend, system_info};
