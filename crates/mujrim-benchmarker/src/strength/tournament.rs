//! Resource-bounded paired round-robin engine tournaments.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mujrim_protocols::catalog::SearchLimitSupport;
use mujrim_study::tournament::{
    Entrant, Pairing, Standing, TournamentFormat, TournamentResult, knockout_advancers,
    knockout_round, schedule, standings, swiss_round,
};

use super::runner::{DuelCheckpoint, FailedEngine};
use super::{
    EngineSpec, GameProgressEvent, MatchConfig, MatchSummary, ensure_scored_match,
    forfeit_match_summary, run_match,
};

#[derive(Clone, Debug)]
pub struct TournamentEngine {
    pub engine: EngineSpec,
    pub established_elo: Option<f64>,
    pub search_limits: SearchLimitSupport,
}

#[derive(Clone, Debug)]
pub struct TournamentConfig {
    pub match_config: MatchConfig,
    pub checkpoint_directory: Option<PathBuf>,
    pub format: TournamentFormat,
    pub swiss_rounds: Option<usize>,
    pub completed_pairings: Vec<Pairing>,
}

impl Default for TournamentConfig {
    fn default() -> Self {
        let match_config = MatchConfig {
            pairs: 4,
            concurrency: 1,
            early_stop: false,
            checkpoint_path: None,
            ..MatchConfig::default()
        };
        Self {
            match_config,
            checkpoint_directory: None,
            format: TournamentFormat::RoundRobin,
            swiss_rounds: None,
            completed_pairings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TournamentSummary {
    pub format: TournamentFormat,
    pub engines: Vec<TournamentEngine>,
    pub matches: Vec<MatchSummary>,
    pub standings: Vec<Standing>,
    pub game_results: Vec<TournamentResult>,
    pub cancelled: bool,
    pub error: Option<String>,
}

impl TournamentSummary {
    pub fn to_json_value(&self) -> serde_json::Value {
        let standings = self
            .standings
            .iter()
            .filter_map(|standing| {
                let engine = self.engines.get(standing.entrant)?;
                Some(serde_json::json!({
                    "engine": engine.engine.name,
                    "played": standing.played,
                    "wins": standing.wins,
                    "draws": standing.draws,
                    "losses": standing.losses,
                    "points": standing.points,
                    "seed_elo": engine.established_elo,
                    "performance_elo": standing.performance.map(|rating| rating.elo),
                    "performance_elo_95_low": standing.performance.map(|rating| rating.lower_95),
                    "performance_elo_95_high": standing.performance.map(|rating| rating.upper_95),
                }))
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "format": format_key(self.format),
            "engines": self.engines.len(),
            "matches": self.matches.iter().map(MatchSummary::to_json_value).collect::<Vec<_>>(),
            "standings": standings,
            "cancelled": self.cancelled,
            "error": self.error,
        })
    }
}

/// One concrete game from a finished tournament pairing (UCI move list for board replay).
#[derive(Clone, Debug, PartialEq)]
pub struct TournamentGameSnapshot {
    pub match_index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub white_score: f64,
    pub initial_fen: String,
    pub moves: Vec<String>,
}

/// Live progress events emitted while a tournament is running.
#[derive(Clone, Debug)]
pub enum TournamentEvent {
    Planned {
        total_matches: usize,
        engine_names: Vec<String>,
    },
    MatchStarted {
        index: usize,
        total: usize,
        round: usize,
        white: String,
        black: String,
    },
    GameStarted {
        game_key: String,
        match_index: usize,
        round: usize,
        white: String,
        black: String,
        initial_fen: String,
    },
    PlyPlayed {
        game_key: String,
        ply: usize,
        uci: String,
        score_cp: i32,
        depth: i32,
        nodes: u64,
        moves: Vec<String>,
        white_clock_ms: Option<u64>,
        black_clock_ms: Option<u64>,
    },
    Thinking {
        game_key: String,
        score_cp: i32,
        depth: i32,
        nodes: u64,
        pv: Vec<String>,
        multipv_lines: Vec<mujrim_protocols::MultiPvLine>,
        white_clock_ms: Option<u64>,
        black_clock_ms: Option<u64>,
    },
    GameFinished {
        game_key: String,
        white_score: f64,
        moves: Vec<String>,
    },
    MatchFinished {
        index: usize,
        total: usize,
        round: usize,
        white: String,
        black: String,
        white_points: f64,
        black_points: f64,
        error: Option<String>,
        standings: Vec<Standing>,
        game_results: Vec<TournamentResult>,
        games: Vec<TournamentGameSnapshot>,
    },
    Cancelled {
        standings: Vec<Standing>,
        game_results: Vec<TournamentResult>,
    },
}

/// Flatten every played game in a match for UI replay.
pub fn games_from_match(
    summary: &MatchSummary,
    match_index: usize,
    round: usize,
) -> Vec<TournamentGameSnapshot> {
    let fen = super::openings::START_FEN.to_owned();
    let mut games = Vec::with_capacity(summary.pairs.len().saturating_mul(2));
    for pair in &summary.pairs {
        games.push(TournamentGameSnapshot {
            match_index,
            round,
            white: summary.candidate.clone(),
            black: summary.reference.clone(),
            white_score: pair.candidate_white.outcome.score(),
            initial_fen: fen.clone(),
            moves: pair.candidate_white.moves.clone(),
        });
        games.push(TournamentGameSnapshot {
            match_index,
            round,
            white: summary.reference.clone(),
            black: summary.candidate.clone(),
            white_score: 1.0 - pair.candidate_black.outcome.score(),
            initial_fen: fen.clone(),
            moves: pair.candidate_black.moves.clone(),
        });
    }
    games
}

/// Games, pairing results, and finished encounters recovered from jsonl sidecars.
#[derive(Clone, Debug, Default)]
pub struct ReconstructedTournament {
    pub games: Vec<TournamentGameSnapshot>,
    pub results: Vec<TournamentResult>,
    pub completed_pairings: Vec<Pairing>,
    pub games_per_encounter: usize,
}

/// Rebuild standings inputs from on-disk duel checkpoints.
///
/// A pairing is complete when its sidecar has at least `games_per_encounter`
/// pair records. Incomplete encounters are omitted so resume replays them.
pub fn reconstruct_tournament(
    roster_names: &[String],
    format: TournamentFormat,
    checkpoints: &[DuelCheckpoint],
    games_per_encounter: usize,
) -> ReconstructedTournament {
    let games_per_encounter = games_per_encounter.max(1);
    let plan = schedule(roster_names.len(), format);
    let mut reconstructed = ReconstructedTournament {
        games_per_encounter,
        ..ReconstructedTournament::default()
    };
    for (match_index, pairing) in plan.iter().copied().enumerate() {
        let Some(white) = roster_names.get(pairing.white) else {
            continue;
        };
        let Some(black) = roster_names.get(pairing.black) else {
            continue;
        };
        let Some(checkpoint) = checkpoints.iter().find(|checkpoint| {
            stems_match(&checkpoint.candidate_stem(), white)
                && stems_match(&checkpoint.reference_stem(), black)
        }) else {
            continue;
        };
        if checkpoint.pairs.len() < games_per_encounter {
            continue;
        }
        reconstructed.completed_pairings.push(pairing);
        let summary = MatchSummary {
            candidate: white.clone(),
            reference: black.clone(),
            pairs: checkpoint.pairs.clone(),
            ..empty_match_summary()
        };
        reconstructed
            .games
            .extend(games_from_match(&summary, match_index + 1, pairing.round));
        append_game_results(pairing, &summary, &mut reconstructed.results);
    }
    reconstructed
}

fn empty_match_summary() -> MatchSummary {
    MatchSummary {
        candidate: String::new(),
        reference: String::new(),
        pairs: Vec::new(),
        scores: super::stats::ScoreCount::default(),
        pair_counts: super::stats::PairCount::default(),
        elo_delta: 0.0,
        elo_low: 0.0,
        elo_high: 0.0,
        llr: 0.0,
        sprt_decision: super::stats::SprtDecision::Continue,
        total_nodes: 0,
        elapsed: std::time::Duration::ZERO,
        error: None,
        reference_elo: None,
        config: MatchConfig::default(),
        opening_count: 0,
        opening_fingerprint: String::new(),
        resumed_pairs: 0,
    }
}

fn stems_match(stem: &str, display_name: &str) -> bool {
    sanitized_engine_stem(stem) == sanitized_engine_stem(display_name)
}

/// Infer simultaneous boards from unfinished pairings, capped at `max_slots`.
pub fn infer_event_concurrency(checkpoints: &[DuelCheckpoint], games_per_encounter: usize) -> u32 {
    let needed = games_per_encounter.max(1);
    let incomplete = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.pairs.len() < needed)
        .count();
    if incomplete > 1 {
        return incomplete as u32;
    }
    0
}

/// Infer how many pair records a finished encounter stored.
pub fn infer_games_per_encounter(checkpoints: &[DuelCheckpoint]) -> usize {
    let mut counts = std::collections::BTreeMap::new();
    for checkpoint in checkpoints {
        if !checkpoint.pairs.is_empty() {
            *counts.entry(checkpoint.pairs.len()).or_insert(0usize) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(pairs, seen)| (*seen, *pairs))
        .map(|(pairs, _)| pairs)
        .unwrap_or(1)
}

/// Flatten every game across a finished tournament summary.
pub fn games_from_summary(summary: &TournamentSummary) -> Vec<TournamentGameSnapshot> {
    summary
        .matches
        .iter()
        .enumerate()
        .flat_map(|(index, match_summary)| {
            let round = summary
                .game_results
                .iter()
                .find(|result| {
                    summary
                        .engines
                        .get(result.pairing.white)
                        .map(|engine| engine.engine.name.as_str())
                        == Some(match_summary.candidate.as_str())
                        && summary
                            .engines
                            .get(result.pairing.black)
                            .map(|engine| engine.engine.name.as_str())
                            == Some(match_summary.reference.as_str())
                })
                .map(|result| result.pairing.round)
                .unwrap_or(index + 1);
            games_from_match(match_summary, index + 1, round)
        })
        .collect()
}

pub type TournamentProgress = Arc<dyn Fn(TournamentEvent) + Send + Sync>;

pub fn run_tournament(
    engines: Vec<TournamentEngine>,
    config: TournamentConfig,
) -> TournamentSummary {
    run_tournament_with_control(engines, config, Arc::new(AtomicBool::new(false)), None)
}

pub fn run_tournament_with_control(
    engines: Vec<TournamentEngine>,
    mut config: TournamentConfig,
    cancel: Arc<AtomicBool>,
    on_event: Option<TournamentProgress>,
) -> TournamentSummary {
    config.match_config.concurrency = config.match_config.concurrency.max(1);
    config.match_config.early_stop = false;
    config.match_config.stop_flag = Some(Arc::clone(&cancel));
    ensure_compatible_time_control(&engines, &mut config.match_config);
    let mut matches = Vec::new();
    let mut game_results = Vec::new();
    let mut error = None;
    let mut cancelled = false;

    if let Some(directory) = config.checkpoint_directory.as_ref()
        && let Err(create_error) = std::fs::create_dir_all(directory)
    {
        return TournamentSummary {
            format: config.format,
            engines,
            matches,
            standings: Vec::new(),
            game_results,
            cancelled: false,
            error: Some(format!(
                "failed to create checkpoint directory '{}': {create_error}",
                directory.display()
            )),
        };
    }

    let emit = |event: TournamentEvent| {
        if let Some(callback) = on_event.as_ref() {
            callback(event);
        }
    };

    let engine_names = engines
        .iter()
        .map(|engine| engine.engine.name.clone())
        .collect::<Vec<_>>();

    match config.format {
        TournamentFormat::RoundRobin | TournamentFormat::DoubleRoundRobin => {
            let plan = schedule(engines.len(), config.format);
            emit(TournamentEvent::Planned {
                total_matches: plan.len(),
                engine_names: engine_names.clone(),
            });
            let outcome = execute_plan(
                &engines,
                &config,
                &plan,
                &mut matches,
                &mut game_results,
                &cancel,
                &on_event,
            );
            cancelled = outcome.cancelled;
            error = outcome.error;
        }
        TournamentFormat::Swiss => {
            let rounds = config.swiss_rounds.unwrap_or_else(|| {
                (usize::BITS - engines.len().saturating_sub(1).leading_zeros()) as usize + 1
            });
            let estimated = engines.len() / 2 * rounds.max(1);
            emit(TournamentEvent::Planned {
                total_matches: estimated.max(1),
                engine_names: engine_names.clone(),
            });
            for round in 1..=rounds.max(1) {
                if cancel.load(Ordering::Acquire) {
                    cancelled = true;
                    break;
                }
                let plan = swiss_round(engines.len(), &game_results, round);
                let outcome = execute_plan(
                    &engines,
                    &config,
                    &plan,
                    &mut matches,
                    &mut game_results,
                    &cancel,
                    &on_event,
                );
                cancelled = outcome.cancelled;
                error = outcome.error;
                if cancelled || error.is_some() {
                    break;
                }
            }
        }
        TournamentFormat::Knockout => {
            let mut participants = (0..engines.len()).collect::<Vec<_>>();
            let mut round = 1;
            emit(TournamentEvent::Planned {
                total_matches: engines.len().saturating_sub(1).max(1),
                engine_names: engine_names.clone(),
            });
            while participants.len() > 1 {
                if cancel.load(Ordering::Acquire) {
                    cancelled = true;
                    break;
                }
                let plan = knockout_round(&participants, round);
                let match_start = matches.len();
                let outcome = execute_plan(
                    &engines,
                    &config,
                    &plan,
                    &mut matches,
                    &mut game_results,
                    &cancel,
                    &on_event,
                );
                cancelled = outcome.cancelled;
                error = outcome.error;
                if cancelled || error.is_some() {
                    break;
                }
                let decisive = plan
                    .iter()
                    .zip(&matches[match_start..])
                    .map(|(pairing, summary)| TournamentResult {
                        pairing: *pairing,
                        white_score: knockout_score(summary, &participants, *pairing),
                    })
                    .collect::<Vec<_>>();
                match knockout_advancers(&participants, &plan, &decisive) {
                    Ok(next) => participants = next,
                    Err(knockout_error) => {
                        error = Some(knockout_error);
                        break;
                    }
                }
                round += 1;
            }
        }
    }

    let entrants = engines
        .iter()
        .enumerate()
        .map(|(index, engine)| Entrant {
            id: index.to_string(),
            name: engine.engine.name.clone(),
            seed_elo: engine.established_elo,
        })
        .collect::<Vec<_>>();
    let standings = standings(&entrants, &game_results);
    if cancelled {
        emit(TournamentEvent::Cancelled {
            standings: standings.clone(),
            game_results: game_results.clone(),
        });
    }
    TournamentSummary {
        format: config.format,
        engines,
        matches,
        standings,
        game_results,
        cancelled,
        error,
    }
}

struct PlanOutcome {
    cancelled: bool,
    error: Option<String>,
}

fn match_game_key_prefix(match_index: usize) -> String {
    format!("m{match_index}-")
}

fn execute_plan(
    engines: &[TournamentEngine],
    config: &TournamentConfig,
    plan: &[Pairing],
    matches: &mut Vec<MatchSummary>,
    game_results: &mut Vec<TournamentResult>,
    cancel: &AtomicBool,
    on_event: &Option<TournamentProgress>,
) -> PlanOutcome {
    let emit = |event: TournamentEvent| {
        if let Some(callback) = on_event.as_ref() {
            callback(event);
        }
    };
    let remaining: Vec<Pairing> = plan
        .iter()
        .copied()
        .filter(|pairing| !config.completed_pairings.contains(pairing))
        .collect();
    let total = plan
        .len()
        .max(matches.len().saturating_add(remaining.len()));
    let workers = config
        .match_config
        .concurrency
        .max(1)
        .min(remaining.len().max(1));
    if workers > 1 {
        return execute_plan_parallel(
            engines,
            config,
            &remaining,
            matches,
            game_results,
            cancel,
            on_event,
            total,
            workers,
        );
    }
    for &pairing in &remaining {
        if cancel.load(Ordering::Acquire) {
            return PlanOutcome {
                cancelled: true,
                error: None,
            };
        }
        let candidate = engines[pairing.white].engine.clone();
        let reference = engines[pairing.black].engine.clone();
        let index = matches.len() + 1;
        emit(TournamentEvent::MatchStarted {
            index,
            total: total.max(index),
            round: pairing.round,
            white: candidate.name.clone(),
            black: reference.name.clone(),
        });
        let mut match_config = config.match_config.clone();
        match_config.opening_offset = match_config
            .opening_offset
            .saturating_add(matches.len().saturating_mul(match_config.pairs));
        match_config.reference_elo = engines[pairing.black].established_elo;
        match_config.stop_flag = Some(Arc::new(AtomicBool::new(false)));
        if let Some(flag) = config.match_config.stop_flag.as_ref() {
            match_config.stop_flag = Some(Arc::clone(flag));
        }
        match_config.game_key_prefix = match_game_key_prefix(index);
        match_config.checkpoint_path = config.checkpoint_directory.as_ref().map(|directory| {
            directory.join(format!(
                "{:02}-{}-vs-{}.jsonl",
                matches.len() + 1,
                safe_name(&candidate.name),
                safe_name(&reference.name)
            ))
        });
        let match_index = index;
        let round = pairing.round;
        if let Some(callback) = on_event.clone() {
            match_config.game_progress = Some(Arc::new(move |event| match event {
                GameProgressEvent::Started {
                    game_key,
                    white,
                    black,
                    initial_fen,
                } => callback(TournamentEvent::GameStarted {
                    game_key,
                    match_index,
                    round,
                    white,
                    black,
                    initial_fen,
                }),
                GameProgressEvent::Ply {
                    game_key,
                    ply,
                    uci,
                    score_cp,
                    depth,
                    nodes,
                    moves,
                    white_clock_ms,
                    black_clock_ms,
                } => callback(TournamentEvent::PlyPlayed {
                    game_key,
                    ply,
                    uci,
                    score_cp,
                    depth,
                    nodes,
                    moves,
                    white_clock_ms,
                    black_clock_ms,
                }),
                GameProgressEvent::Thinking {
                    game_key,
                    score_cp,
                    depth,
                    nodes,
                    pv,
                    multipv_lines,
                    white_clock_ms,
                    black_clock_ms,
                } => callback(TournamentEvent::Thinking {
                    game_key,
                    score_cp,
                    depth,
                    nodes,
                    pv,
                    multipv_lines,
                    white_clock_ms,
                    black_clock_ms,
                }),
                GameProgressEvent::Finished {
                    game_key,
                    white_score,
                    moves,
                } => callback(TournamentEvent::GameFinished {
                    game_key,
                    white_score,
                    moves,
                }),
            }));
        }
        // Isolate engine/process failures: never let a panic abort the tournament.
        let summary = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_match(
                candidate.clone(),
                reference.clone(),
                None,
                match_config.clone(),
            )
        })) {
            Ok(summary) => ensure_scored_match(summary, &candidate, &reference),
            Err(_) => forfeit_match_summary(
                &candidate,
                &reference,
                &match_config,
                FailedEngine::Candidate,
                "match panicked; awarded forfeit loss to white/candidate and continued",
            ),
        };
        let white_points = summary.scores.wins as f64 + summary.scores.draws as f64 * 0.5;
        let games = summary.scores.games() as f64;
        let black_points = games - white_points;
        append_game_results(pairing, &summary, game_results);
        // Engine failures become scored forfeits — surface as a note, do not abort.
        let match_note = summary.error.as_ref().map(|error| {
            format!(
                "{} vs {}: {error} (forfeit recorded; tournament continues)",
                summary.candidate, summary.reference
            )
        });
        let entrants = engines
            .iter()
            .enumerate()
            .map(|(idx, engine)| Entrant {
                id: idx.to_string(),
                name: engine.engine.name.clone(),
                seed_elo: engine.established_elo,
            })
            .collect::<Vec<_>>();
        let current_standings = standings(&entrants, game_results);
        let games = games_from_match(&summary, index, pairing.round);
        emit(TournamentEvent::MatchFinished {
            index,
            total: total.max(index),
            round: pairing.round,
            white: summary.candidate.clone(),
            black: summary.reference.clone(),
            white_points,
            black_points,
            error: match_note,
            standings: current_standings,
            game_results: game_results.clone(),
            games,
        });
        matches.push(summary);
        if cancel.load(Ordering::Acquire) {
            return PlanOutcome {
                cancelled: true,
                error: None,
            };
        }
    }
    PlanOutcome {
        cancelled: false,
        error: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_plan_parallel(
    engines: &[TournamentEngine],
    config: &TournamentConfig,
    remaining: &[Pairing],
    matches: &mut Vec<MatchSummary>,
    game_results: &mut Vec<TournamentResult>,
    cancel: &AtomicBool,
    on_event: &Option<TournamentProgress>,
    total: usize,
    workers: usize,
) -> PlanOutcome {
    use std::sync::mpsc;
    let (job_tx, job_rx) = mpsc::channel();
    for (offset, pairing) in remaining.iter().copied().enumerate() {
        let _ = job_tx.send((matches.len() + offset + 1, pairing));
    }
    drop(job_tx);
    let job_rx = Arc::new(std::sync::Mutex::new(job_rx));
    let matches_lock = Arc::new(std::sync::Mutex::new(std::mem::take(matches)));
    let results_lock = Arc::new(std::sync::Mutex::new(std::mem::take(game_results)));
    let engines = engines.to_vec();
    let config = config.clone();
    let cancel = config
        .match_config
        .stop_flag
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire))));
    let on_event = on_event.clone();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let job_rx = Arc::clone(&job_rx);
            let matches_lock = Arc::clone(&matches_lock);
            let results_lock = Arc::clone(&results_lock);
            let engines = engines.clone();
            let config = config.clone();
            let cancel = Arc::clone(&cancel);
            let on_event = on_event.clone();
            scope.spawn(move || {
                loop {
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(guard) = job_rx.lock() else {
                        break;
                    };
                    let Ok((index, pairing)) = guard.recv() else {
                        break;
                    };
                    drop(guard);
                    let emit = |event: TournamentEvent| {
                        if let Some(callback) = on_event.as_ref() {
                            callback(event);
                        }
                    };
                    let candidate = engines[pairing.white].engine.clone();
                    let reference = engines[pairing.black].engine.clone();
                    emit(TournamentEvent::MatchStarted {
                        index,
                        total: total.max(index),
                        round: pairing.round,
                        white: candidate.name.clone(),
                        black: reference.name.clone(),
                    });
                    let mut match_config = config.match_config.clone();
                    match_config.opening_offset = match_config.opening_offset.saturating_add(
                        (index.saturating_sub(1)).saturating_mul(match_config.pairs),
                    );
                    match_config.reference_elo = engines[pairing.black].established_elo;
                    if let Some(flag) = config.match_config.stop_flag.as_ref() {
                        match_config.stop_flag = Some(Arc::clone(flag));
                    }
                    match_config.game_key_prefix = match_game_key_prefix(index);
                    match_config.checkpoint_path =
                        config.checkpoint_directory.as_ref().map(|directory| {
                            directory.join(format!(
                                "{:02}-{}-vs-{}.jsonl",
                                index,
                                safe_name(&candidate.name),
                                safe_name(&reference.name)
                            ))
                        });
                    let match_index = index;
                    let round = pairing.round;
                    if let Some(callback) = on_event.clone() {
                        match_config.game_progress = Some(Arc::new(move |event| match event {
                            GameProgressEvent::Started {
                                game_key,
                                white,
                                black,
                                initial_fen,
                            } => callback(TournamentEvent::GameStarted {
                                game_key,
                                match_index,
                                round,
                                white,
                                black,
                                initial_fen,
                            }),
                            GameProgressEvent::Ply {
                                game_key,
                                ply,
                                uci,
                                score_cp,
                                depth,
                                nodes,
                                moves,
                                white_clock_ms,
                                black_clock_ms,
                            } => callback(TournamentEvent::PlyPlayed {
                                game_key,
                                ply,
                                uci,
                                score_cp,
                                depth,
                                nodes,
                                moves,
                                white_clock_ms,
                                black_clock_ms,
                            }),
                            GameProgressEvent::Thinking {
                                game_key,
                                score_cp,
                                depth,
                                nodes,
                                pv,
                                multipv_lines,
                                white_clock_ms,
                                black_clock_ms,
                            } => callback(TournamentEvent::Thinking {
                                game_key,
                                score_cp,
                                depth,
                                nodes,
                                pv,
                                multipv_lines,
                                white_clock_ms,
                                black_clock_ms,
                            }),
                            GameProgressEvent::Finished {
                                game_key,
                                white_score,
                                moves,
                            } => callback(TournamentEvent::GameFinished {
                                game_key,
                                white_score,
                                moves,
                            }),
                        }));
                    }
                    let summary = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || {
                            run_match(
                                candidate.clone(),
                                reference.clone(),
                                None,
                                match_config.clone(),
                            )
                        },
                    )) {
                        Ok(summary) => ensure_scored_match(summary, &candidate, &reference),
                        Err(_) => forfeit_match_summary(
                            &candidate,
                            &reference,
                            &match_config,
                            FailedEngine::Candidate,
                            "match panicked; awarded forfeit loss to white/candidate and continued",
                        ),
                    };
                    let white_points =
                        summary.scores.wins as f64 + summary.scores.draws as f64 * 0.5;
                    let games = summary.scores.games() as f64;
                    let black_points = games - white_points;
                    let match_note = summary.error.as_ref().map(|error| {
                        format!(
                            "{} vs {}: {error} (forfeit recorded; tournament continues)",
                            summary.candidate, summary.reference
                        )
                    });
                    let games = games_from_match(&summary, index, pairing.round);
                    let mut results_guard = results_lock.lock().unwrap_or_else(|e| e.into_inner());
                    append_game_results(pairing, &summary, &mut results_guard);
                    let current_standings = {
                        let entrants = engines
                            .iter()
                            .enumerate()
                            .map(|(idx, engine)| Entrant {
                                id: idx.to_string(),
                                name: engine.engine.name.clone(),
                                seed_elo: engine.established_elo,
                            })
                            .collect::<Vec<_>>();
                        standings(&entrants, &results_guard)
                    };
                    let snapshot_results = results_guard.clone();
                    drop(results_guard);
                    emit(TournamentEvent::MatchFinished {
                        index,
                        total: total.max(index),
                        round: pairing.round,
                        white: summary.candidate.clone(),
                        black: summary.reference.clone(),
                        white_points,
                        black_points,
                        error: match_note,
                        standings: current_standings,
                        game_results: snapshot_results,
                        games,
                    });
                    matches_lock
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(summary);
                }
            });
        }
    });
    *matches = match Arc::try_unwrap(matches_lock) {
        Ok(mutex) => mutex.into_inner().unwrap_or_else(|e| e.into_inner()),
        Err(arc) => arc.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    };
    *game_results = match Arc::try_unwrap(results_lock) {
        Ok(mutex) => mutex.into_inner().unwrap_or_else(|e| e.into_inner()),
        Err(arc) => arc.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    };
    PlanOutcome {
        cancelled: cancel.load(Ordering::Acquire),
        error: None,
    }
}

fn knockout_score(summary: &MatchSummary, participants: &[usize], pairing: Pairing) -> f64 {
    let games = summary.scores.games() as f64;
    let points = summary.scores.wins as f64 + summary.scores.draws as f64 * 0.5;
    if points > games * 0.5 {
        1.0
    } else if points < games * 0.5 {
        0.0
    } else if participants
        .iter()
        .position(|entrant| *entrant == pairing.white)
        < participants
            .iter()
            .position(|entrant| *entrant == pairing.black)
    {
        1.0
    } else {
        0.0
    }
}

fn format_key(format: TournamentFormat) -> &'static str {
    match format {
        TournamentFormat::RoundRobin => "round_robin",
        TournamentFormat::DoubleRoundRobin => "double_round_robin",
        TournamentFormat::Swiss => "swiss",
        TournamentFormat::Knockout => "knockout",
    }
}

fn ensure_compatible_time_control(engines: &[TournamentEngine], config: &mut MatchConfig) {
    if config.clock.is_some() {
        config.move_time = None;
        config.nodes_per_move = 0;
        return;
    }
    if engines
        .iter()
        .all(|engine| engine.search_limits.fixed_nodes)
        && config.move_time.is_none()
    {
        return;
    }
    if engines.iter().all(|engine| engine.search_limits.move_time) {
        config
            .move_time
            .get_or_insert(std::time::Duration::from_millis(100));
        return;
    }
    config.move_time = None;
    config.nodes_per_move = 0;
    config.max_depth = config.max_depth.clamp(1, 10);
}

fn append_game_results(
    pairing: Pairing,
    summary: &MatchSummary,
    results: &mut Vec<TournamentResult>,
) {
    for pair in &summary.pairs {
        results.push(TournamentResult {
            pairing,
            white_score: pair.candidate_white.outcome.score(),
        });
        results.push(TournamentResult {
            pairing: Pairing {
                round: pairing.round,
                white: pairing.black,
                black: pairing.white,
            },
            white_score: 1.0 - pair.candidate_black.outcome.score(),
        });
    }
}

pub fn sanitized_engine_stem(name: &str) -> String {
    safe_name(name)
}

fn safe_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    sanitized.trim_matches('-').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::super::runner::{EngineTelemetry, GameRecord, PairRecord, Termination};
    use super::super::stats::GameOutcome;
    use super::*;

    #[test]
    fn parallel_pairings_get_distinct_live_board_prefixes() {
        assert_eq!(match_game_key_prefix(1), "m1-");
        assert_ne!(match_game_key_prefix(1), match_game_key_prefix(2));
        assert_ne!(
            crate::strength::progress_game_key(&match_game_key_prefix(1), 0, true),
            crate::strength::progress_game_key(&match_game_key_prefix(2), 0, true)
        );
        let src = include_str!("tournament.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert_eq!(
            production
                .matches("game_key_prefix = match_game_key_prefix")
                .count(),
            2,
            "serial and parallel pairings must both scope live game keys"
        );
    }

    #[test]
    fn tournament_defaults_are_safe_and_complete() {
        let config = TournamentConfig::default();
        assert_eq!(config.match_config.concurrency, 1);
        assert_eq!(config.match_config.max_engine_memory_mb, 384);
        assert_eq!(config.match_config.max_match_memory_mb, 768);
        assert!(!config.match_config.early_stop);
        assert_eq!(config.format, TournamentFormat::RoundRobin);
    }

    #[test]
    fn missing_engine_binaries_forfeit_and_the_event_continues() {
        let engines = vec![
            TournamentEngine {
                engine: EngineSpec::new(PathBuf::from("/no/such/mujrim-tournament-a")),
                established_elo: None,
                search_limits: SearchLimitSupport::STANDARD,
            },
            TournamentEngine {
                engine: EngineSpec::new(PathBuf::from("/no/such/mujrim-tournament-b")),
                established_elo: None,
                search_limits: SearchLimitSupport::STANDARD,
            },
        ];
        let match_config = MatchConfig {
            pairs: 1,
            concurrency: 1,
            hash_mb: 16,
            engine_threads: 1,
            max_engine_memory_mb: 256,
            max_match_memory_mb: 512,
            max_plies: 2,
            nodes_per_move: 1,
            read_timeout: std::time::Duration::from_secs(2),
            early_stop: false,
            ..MatchConfig::default()
        };
        let summary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_tournament(
                engines,
                TournamentConfig {
                    match_config,
                    checkpoint_directory: None,
                    format: TournamentFormat::RoundRobin,
                    swiss_rounds: None,
                    completed_pairings: Vec::new(),
                },
            )
        }))
        .expect("a missing engine must forfeit, not abort the tournament");
        assert_eq!(summary.matches.len(), 1);
        assert!(summary.matches[0].scores.games() >= 1);
        assert!(summary.matches[0].error.is_some());
        assert!(!summary.cancelled);
    }

    #[test]
    fn checkpoint_names_are_filesystem_safe() {
        assert_eq!(safe_name("Stockfish 18 / AVX2"), "stockfish-18---avx2");
    }

    #[test]
    fn six_engines_produce_fifteen_paired_matches() {
        assert_eq!(schedule(6, TournamentFormat::RoundRobin).len(), 15);
    }

    #[test]
    fn depth_only_engines_select_safe_depth_play() {
        let engines = vec![TournamentEngine {
            engine: EngineSpec::new(PathBuf::from("ethereal.exe")),
            established_elo: None,
            search_limits: SearchLimitSupport::DEPTH_ONLY,
        }];
        let mut config = MatchConfig::default();
        ensure_compatible_time_control(&engines, &mut config);
        assert_eq!(config.move_time, None);
        assert_eq!(config.nodes_per_move, 0);
        assert_eq!(config.max_depth, 10);
    }

    #[test]
    fn clock_time_control_is_preserved() {
        let engines = vec![TournamentEngine {
            engine: EngineSpec::new(PathBuf::from("stockfish.exe")),
            established_elo: None,
            search_limits: SearchLimitSupport::STANDARD,
        }];
        let mut config = MatchConfig {
            clock: Some(crate::strength::MatchClock {
                initial: std::time::Duration::from_secs(180),
                increment: std::time::Duration::from_secs(2),
                bonus_after_moves: 40,
                bonus: std::time::Duration::from_secs(180),
            }),
            nodes_per_move: 20_000,
            move_time: Some(std::time::Duration::from_millis(100)),
            ..MatchConfig::default()
        };
        ensure_compatible_time_control(&engines, &mut config);
        assert!(config.clock.is_some());
        assert!(config.move_time.is_none());
        assert_eq!(config.nodes_per_move, 0);
    }

    #[test]
    fn games_from_match_emits_both_color_swapped_boards() {
        use crate::strength::runner::{GameRecord, PairRecord, Termination};
        use crate::strength::stats::{GameOutcome, PairCount, ScoreCount, SprtDecision};
        use std::time::Duration;

        let game = |moves: &[&str], outcome: GameOutcome, candidate_white: bool| GameRecord {
            candidate_white,
            outcome,
            termination: Termination::MaxPlies,
            plies: moves.len(),
            nodes: 0,
            elapsed: Duration::ZERO,
            candidate_telemetry: Default::default(),
            reference_telemetry: Default::default(),
            moves: moves.iter().map(|uci| (*uci).to_owned()).collect(),
        };
        let summary = MatchSummary {
            candidate: "Alpha".into(),
            reference: "Beta".into(),
            pairs: vec![PairRecord {
                index: 0,
                candidate_white: game(&["e2e4", "e7e5"], GameOutcome::Win, true),
                candidate_black: game(&["d2d4", "d7d5"], GameOutcome::Draw, false),
            }],
            scores: ScoreCount {
                wins: 1,
                draws: 1,
                losses: 0,
            },
            pair_counts: PairCount::default(),
            elo_delta: 0.0,
            elo_low: 0.0,
            elo_high: 0.0,
            llr: 0.0,
            sprt_decision: SprtDecision::Continue,
            total_nodes: 0,
            elapsed: Duration::ZERO,
            error: None,
            reference_elo: None,
            config: MatchConfig::default(),
            opening_count: 1,
            opening_fingerprint: "test".into(),
            resumed_pairs: 0,
        };
        let games = games_from_match(&summary, 3, 2);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].white, "Alpha");
        assert_eq!(games[0].black, "Beta");
        assert_eq!(games[0].white_score, 1.0);
        assert_eq!(games[0].moves, vec!["e2e4".to_owned(), "e7e5".to_owned()]);
        assert_eq!(games[1].white, "Beta");
        assert_eq!(games[1].black, "Alpha");
        assert_eq!(games[1].white_score, 0.5);
        assert_eq!(games[0].match_index, 3);
        assert_eq!(games[0].round, 2);
    }

    #[test]
    fn cancel_flag_stops_before_any_match_when_prearmed() {
        let cancel = Arc::new(AtomicBool::new(true));
        let engines = vec![
            TournamentEngine {
                engine: EngineSpec::new(PathBuf::from("alpha.exe")),
                established_elo: None,
                search_limits: SearchLimitSupport::STANDARD,
            },
            TournamentEngine {
                engine: EngineSpec::new(PathBuf::from("beta.exe")),
                established_elo: None,
                search_limits: SearchLimitSupport::STANDARD,
            },
        ];
        let summary = run_tournament_with_control(
            engines,
            TournamentConfig {
                match_config: MatchConfig {
                    pairs: 1,
                    nodes_per_move: 1,
                    ..MatchConfig::default()
                },
                ..TournamentConfig::default()
            },
            cancel,
            None,
        );
        assert!(summary.cancelled);
        assert!(summary.matches.is_empty());
    }

    #[test]
    fn reconstruct_skips_incomplete_pairings_and_maps_stems() {
        let finished = DuelCheckpoint {
            candidate_path: PathBuf::from("/engines/mujrim-elite"),
            reference_path: PathBuf::from("/engines/akimbo"),
            opening_fingerprint: String::new(),
            pairs: vec![PairRecord {
                index: 0,
                candidate_white: GameRecord {
                    candidate_white: true,
                    outcome: GameOutcome::Win,
                    termination: Termination::Checkmate,
                    plies: 2,
                    nodes: 8,
                    elapsed: std::time::Duration::from_millis(4),
                    candidate_telemetry: EngineTelemetry::default(),
                    reference_telemetry: EngineTelemetry::default(),
                    moves: vec!["e2e4".into(), "e7e5".into()],
                },
                candidate_black: GameRecord {
                    candidate_white: false,
                    outcome: GameOutcome::Draw,
                    termination: Termination::DrawRule,
                    plies: 4,
                    nodes: 8,
                    elapsed: std::time::Duration::from_millis(4),
                    candidate_telemetry: EngineTelemetry::default(),
                    reference_telemetry: EngineTelemetry::default(),
                    moves: vec!["d2d4".into(), "d7d5".into()],
                },
            }],
        };
        let incomplete = DuelCheckpoint {
            candidate_path: PathBuf::from("/engines/akimbo"),
            reference_path: PathBuf::from("/engines/mujrim-elite"),
            opening_fingerprint: String::new(),
            pairs: Vec::new(),
        };
        let roster = vec!["Akimbo".into(), "Mujrim Elite".into()];
        assert_eq!(
            infer_games_per_encounter(std::slice::from_ref(&finished)),
            1
        );
        let rebuilt = reconstruct_tournament(
            &roster,
            TournamentFormat::DoubleRoundRobin,
            &[finished.clone(), incomplete.clone()],
            1,
        );
        assert_eq!(rebuilt.completed_pairings.len(), 1);
        assert_eq!(rebuilt.games.len(), 2);
        assert_eq!(rebuilt.results.len(), 2);
        assert_eq!(rebuilt.games[0].white, "Mujrim Elite");
        assert_eq!(
            infer_event_concurrency(&[finished, incomplete.clone()], 1),
            0,
            "a single unfinished pairing is not a simul"
        );
        let live = vec![
            DuelCheckpoint {
                pairs: Vec::new(),
                ..incomplete
            };
            15
        ];
        assert_eq!(infer_event_concurrency(&live, 2), 15);
    }
}
