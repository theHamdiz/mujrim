//! Mujrim Benchmarker — benchmark suite for chess engines.
//!
//! Supports two modes:
//! - **Internal**: Benchmarks Mujrim directly (using `SearchEngine`)
//! - **External**: Benchmarks any UCI-compatible engine binary via subprocess
//!
//! Features:
//! - Bratko-Kopec test suite (24 positions)
//! - Custom FEN position files
//! - NNUE network info display
//! - Search technique introspection
//! - Hardware detection (CPU, SIMD, GPU)
//! - Optional TUI with live progress (ratatui)

pub mod compare;
pub mod engine_info;
pub mod external;
pub mod hardware;
pub mod internal;
pub mod iterate;
pub mod nnue_bench;
pub mod replay;
pub mod strength;
pub mod suite;

#[cfg(feature = "tui")]
pub mod tui;
