//! Fast, reproducible engine-strength matches.

pub mod openings;
pub mod runner;
pub mod stats;
pub mod tournament;

pub use runner::{
    EngineSpec, FailedEngine, GameProgress, GameProgressEvent, MatchClock, MatchConfig,
    MatchSummary, bounded_engine_hash_mb, classify_engine_failure, ensure_scored_match,
    forfeit_match_summary, run_match,
};
pub use tournament::{
    TournamentConfig, TournamentEngine, TournamentEvent, TournamentGameSnapshot,
    TournamentProgress, TournamentSummary, games_from_match, games_from_summary, run_tournament,
    run_tournament_with_control,
};
