//! KishMat Updater Library
//!
//! Provides reusable functionality for:
//! - Syzygy tablebase downloading (3-7 piece)
//! - NNUE network downloading (Akimbo, Stockfish, Viridithas, Alexandria)
//! - Tunable parameter management (params.toml)
//! - GitHub release checking and updates

pub mod syzygy;
pub mod nnue;
pub mod tuning;

