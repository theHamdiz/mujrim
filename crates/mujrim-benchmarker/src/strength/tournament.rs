//! Resource-bounded paired round-robin engine tournaments.

use std::path::PathBuf;

use mujrim_protocols::catalog::SearchLimitSupport;
use mujrim_study::tournament::{
    Entrant, Pairing, Standing, TournamentFormat, TournamentResult, knockout_advancers,
    knockout_round, schedule, standings, swiss_round,
};

use super::{EngineSpec, MatchConfig, MatchSummary, run_match};

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
        }
    }
}

#[derive(Clone, Debug)]
pub struct TournamentSummary {
    pub format: TournamentFormat,
    pub engines: Vec<TournamentEngine>,
    pub matches: Vec<MatchSummary>,
    pub standings: Vec<Standing>,
    pub error: Option<String>,
}

impl TournamentSummary {
    pub fn to_json_value(&self) -> serde_json::Value {
        let standings = self
            .standings
            .iter()
            .map(|standing| {
                let engine = &self.engines[standing.entrant];
                serde_json::json!({
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
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "format": format_key(self.format),
            "engines": self.engines.len(),
            "matches": self.matches.iter().map(MatchSummary::to_json_value).collect::<Vec<_>>(),
            "standings": standings,
            "error": self.error,
        })
    }
}

pub fn run_tournament(
    engines: Vec<TournamentEngine>,
    mut config: TournamentConfig,
) -> TournamentSummary {
    config.match_config.concurrency = config.match_config.concurrency.max(1);
    config.match_config.early_stop = false;
    ensure_compatible_time_control(&engines, &mut config.match_config);
    let mut matches = Vec::new();
    let mut game_results = Vec::new();
    let mut error = None;

    if let Some(directory) = config.checkpoint_directory.as_ref()
        && let Err(create_error) = std::fs::create_dir_all(directory)
    {
        return TournamentSummary {
            format: config.format,
            engines,
            matches,
            standings: Vec::new(),
            error: Some(format!(
                "failed to create checkpoint directory '{}': {create_error}",
                directory.display()
            )),
        };
    }

    match config.format {
        TournamentFormat::RoundRobin | TournamentFormat::DoubleRoundRobin => {
            let plan = schedule(engines.len(), config.format);
            error = execute_plan(&engines, &config, &plan, &mut matches, &mut game_results);
        }
        TournamentFormat::Swiss => {
            let rounds = config.swiss_rounds.unwrap_or_else(|| {
                (usize::BITS - engines.len().saturating_sub(1).leading_zeros()) as usize + 1
            });
            for round in 1..=rounds.max(1) {
                let plan = swiss_round(engines.len(), &game_results, round);
                error = execute_plan(&engines, &config, &plan, &mut matches, &mut game_results);
                if error.is_some() {
                    break;
                }
            }
        }
        TournamentFormat::Knockout => {
            let mut participants = (0..engines.len()).collect::<Vec<_>>();
            let mut round = 1;
            while participants.len() > 1 {
                let plan = knockout_round(&participants, round);
                let match_start = matches.len();
                error = execute_plan(&engines, &config, &plan, &mut matches, &mut game_results);
                if error.is_some() {
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
    TournamentSummary {
        format: config.format,
        engines,
        matches,
        standings,
        error,
    }
}

fn execute_plan(
    engines: &[TournamentEngine],
    config: &TournamentConfig,
    plan: &[Pairing],
    matches: &mut Vec<MatchSummary>,
    game_results: &mut Vec<TournamentResult>,
) -> Option<String> {
    for &pairing in plan {
        let candidate = engines[pairing.white].engine.clone();
        let reference = engines[pairing.black].engine.clone();
        let mut match_config = config.match_config.clone();
        match_config.opening_offset = match_config
            .opening_offset
            .saturating_add(matches.len().saturating_mul(match_config.pairs));
        match_config.reference_elo = engines[pairing.black].established_elo;
        match_config.checkpoint_path = config.checkpoint_directory.as_ref().map(|directory| {
            directory.join(format!(
                "{:02}-{}-vs-{}.jsonl",
                matches.len() + 1,
                safe_name(&candidate.name),
                safe_name(&reference.name)
            ))
        });
        let summary = run_match(candidate, reference, None, match_config);
        append_game_results(pairing, &summary, game_results);
        let match_error = summary.error.as_ref().map(|error| {
            format!(
                "{} vs {} failed: {error}",
                summary.candidate, summary.reference
            )
        });
        matches.push(summary);
        if match_error.is_some() {
            return match_error;
        }
    }
    None
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
    use super::*;

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
}
