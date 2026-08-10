//! Fast, reproducible engine-strength matches.

pub mod openings;
pub mod runner;
pub mod stats;
pub mod tournament;

pub use runner::{EngineSpec, MatchConfig, MatchSummary, run_match};
pub use tournament::{TournamentConfig, TournamentEngine, TournamentSummary, run_tournament};
