//! Fast, reproducible engine-strength matches.

pub mod openings;
pub mod runner;
pub mod stats;
pub mod tournament;

pub use runner::{
    DuelCheckpoint, EngineSpec, FailedEngine, GameProgress, GameProgressEvent, MatchClock,
    MatchConfig, MatchSummary, SearchControl, bounded_engine_hash_mb, classify_engine_failure,
    ensure_scored_match, forfeit_match_summary, match_search_control, progress_game_key,
    read_duel_checkpoint, run_match, scan_duel_checkpoints,
};
pub use tournament::{
    ReconstructedTournament, TournamentConfig, TournamentEngine, TournamentEvent,
    TournamentGameSnapshot, TournamentProgress, TournamentSummary, games_from_match,
    games_from_summary, infer_event_concurrency, infer_games_per_encounter, reconstruct_tournament,
    run_tournament, run_tournament_with_control, sanitized_engine_stem,
};
