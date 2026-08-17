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

#[cfg(feature = "internal")]
pub mod adapter_gauntlet;
pub mod compare;
#[cfg(feature = "internal")]
pub mod engine_info;
pub mod external;
pub mod hardware;
#[cfg(feature = "internal")]
pub mod internal;
#[cfg(feature = "internal")]
pub mod iterate;
#[cfg(feature = "internal")]
pub mod nnue_bench;
pub mod replay;
pub mod strength;
pub mod suite;

#[cfg(feature = "tui")]
pub mod tui;

#[cfg(test)]
mod feature_contract_tests {
    #[test]
    fn in_process_engine_modules_are_optional() {
        let src = include_str!("lib.rs");
        assert!(src.contains("#[cfg(feature = \"internal\")]"));
        assert!(src.contains("pub mod internal"));
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        assert!(manifest.contains("internal = [\"dep:search\", \"dep:eval\"]"));
        assert!(manifest.contains("optional = true"));
    }
}
