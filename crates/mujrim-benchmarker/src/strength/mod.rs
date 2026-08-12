//! Fast, reproducible engine-strength matches.

pub mod openings;
pub mod runner;
pub mod stats;
pub mod tournament;

pub use runner::{
    EngineSpec, GameProgress, GameProgressEvent, MatchClock, MatchConfig, MatchSummary, run_match,
};
pub use tournament::{
    TournamentConfig, TournamentEngine, TournamentEvent, TournamentGameSnapshot,
    TournamentProgress, TournamentSummary, games_from_match, games_from_summary, run_tournament,
    run_tournament_with_control,
};
