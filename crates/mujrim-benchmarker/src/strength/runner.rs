//! Resource-bounded paired-game runner with node, time, or depth controls.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

use mujrim_protocols::{EngineOptions, EngineSession, ProtocolKind, SearchInfo, SearchRequest};
use types::Color;

use super::openings::{Opening, default_openings, openings_fingerprint, resolve_legal_move};
use super::stats::{GameOutcome, PairCount, ScoreCount, Sprt, SprtDecision, paired_elo_interval};

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Debug)]
pub struct EngineSpec {
    pub name: String,
    pub path: PathBuf,
    pub args: Vec<String>,
    pub uci_options: Vec<(String, String)>,
}

impl EngineSpec {
    pub fn new(path: PathBuf) -> Self {
        let name = path.file_stem().map_or_else(
            || "engine".to_string(),
            |name| name.to_string_lossy().into(),
        );
        Self {
            name,
            path,
            args: Vec::new(),
            uci_options: Vec::new(),
        }
    }
}

/// Live callbacks while a single game is being played.
pub type GameProgress = Arc<dyn Fn(GameProgressEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub enum GameProgressEvent {
    Started {
        game_key: String,
        white: String,
        black: String,
        initial_fen: String,
    },
    Ply {
        game_key: String,
        ply: usize,
        uci: String,
        score_cp: i32,
        depth: i32,
        nodes: u64,
        moves: Vec<String>,
    },
    Finished {
        game_key: String,
        white_score: f64,
        moves: Vec<String>,
    },
}

#[derive(Clone)]
pub struct MatchConfig {
    pub pairs: usize,
    pub opening_offset: usize,
    pub concurrency: usize,
    pub nodes_per_move: u64,
    /// Optional wall-clock budget. When present it replaces fixed nodes and depth.
    pub move_time: Option<Duration>,
    pub max_depth: i32,
    pub hash_mb: usize,
    pub engine_threads: usize,
    pub max_engine_memory_mb: usize,
    pub max_match_memory_mb: usize,
    pub session_pairs: usize,
    pub max_plies: usize,
    pub read_timeout: Duration,
    pub resign_cp: i32,
    pub resign_plies: usize,
    pub draw_cp: i32,
    pub draw_plies: usize,
    pub draw_min_ply: usize,
    pub sprt: Sprt,
    pub reference_elo: Option<f64>,
    pub checkpoint_path: Option<PathBuf>,
    /// Stop as soon as SPRT accepts a hypothesis. Tournament matches disable
    /// this so every scheduled pairing has an equal sample size.
    pub early_stop: bool,
    /// Optional external cancel flag observed by match workers.
    pub stop_flag: Option<Arc<AtomicBool>>,
    /// Optional ply-by-ply progress hook for live tournament boards.
    pub game_progress: Option<GameProgress>,
}

impl std::fmt::Debug for MatchConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatchConfig")
            .field("pairs", &self.pairs)
            .field("concurrency", &self.concurrency)
            .field("nodes_per_move", &self.nodes_per_move)
            .field("move_time", &self.move_time)
            .field("max_depth", &self.max_depth)
            .field("hash_mb", &self.hash_mb)
            .field("engine_threads", &self.engine_threads)
            .field("early_stop", &self.early_stop)
            .field("game_progress", &self.game_progress.as_ref().map(|_| "set"))
            .finish_non_exhaustive()
    }
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            pairs: 32,
            opening_offset: 0,
            concurrency: 1,
            nodes_per_move: 20_000,
            move_time: None,
            max_depth: 128,
            hash_mb: 32,
            engine_threads: 1,
            max_engine_memory_mb: 384,
            max_match_memory_mb: 768,
            session_pairs: 1,
            max_plies: 300,
            read_timeout: Duration::from_secs(30),
            resign_cp: 1_000,
            resign_plies: 8,
            draw_cp: 10,
            draw_plies: 16,
            draw_min_ply: 80,
            sprt: Sprt::default(),
            reference_elo: None,
            checkpoint_path: None,
            early_stop: true,
            stop_flag: None,
            game_progress: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Termination {
    Checkmate,
    DrawRule,
    AdjudicatedWin,
    AdjudicatedDraw,
    MaxPlies,
    Forfeit(String),
}

#[derive(Clone, Debug)]
pub struct EngineTelemetry {
    pub searches: u64,
    pub nodes: u64,
    pub search_time: Duration,
    pub depth_sum: u64,
    pub max_depth: i32,
    pub max_seldepth: i32,
}

impl Default for EngineTelemetry {
    fn default() -> Self {
        Self {
            searches: 0,
            nodes: 0,
            search_time: Duration::ZERO,
            depth_sum: 0,
            max_depth: 0,
            max_seldepth: 0,
        }
    }
}

impl EngineTelemetry {
    fn observe(&mut self, info: &SearchInfo, wall_time: Duration) {
        self.searches = self.searches.saturating_add(1);
        self.nodes = self.nodes.saturating_add(info.nodes);
        let reported_time = Duration::from_millis(info.time_ms);
        self.search_time = self.search_time.saturating_add(if reported_time.is_zero() {
            wall_time
        } else {
            reported_time
        });
        self.depth_sum = self.depth_sum.saturating_add(info.depth.max(0) as u64);
        self.max_depth = self.max_depth.max(info.depth);
        self.max_seldepth = self.max_seldepth.max(info.seldepth);
    }

    fn merge(&mut self, other: &Self) {
        self.searches = self.searches.saturating_add(other.searches);
        self.nodes = self.nodes.saturating_add(other.nodes);
        self.search_time = self.search_time.saturating_add(other.search_time);
        self.depth_sum = self.depth_sum.saturating_add(other.depth_sum);
        self.max_depth = self.max_depth.max(other.max_depth);
        self.max_seldepth = self.max_seldepth.max(other.max_seldepth);
    }

    fn nps(&self) -> Option<u64> {
        let millis = self.search_time.as_millis();
        (millis > 0).then(|| self.nodes.saturating_mul(1_000) / millis as u64)
    }

    fn average_depth(&self) -> Option<f64> {
        (self.searches > 0).then(|| self.depth_sum as f64 / self.searches as f64)
    }
}

#[derive(Clone, Debug)]
pub struct GameRecord {
    pub candidate_white: bool,
    pub outcome: GameOutcome,
    pub termination: Termination,
    pub plies: usize,
    pub nodes: u64,
    pub elapsed: Duration,
    pub candidate_telemetry: EngineTelemetry,
    pub reference_telemetry: EngineTelemetry,
    pub moves: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PairRecord {
    pub index: usize,
    pub candidate_white: GameRecord,
    pub candidate_black: GameRecord,
}

#[derive(Clone, Debug)]
pub struct MatchSummary {
    pub candidate: String,
    pub reference: String,
    pub pairs: Vec<PairRecord>,
    pub scores: ScoreCount,
    pub pair_counts: PairCount,
    pub elo_delta: f64,
    pub elo_low: f64,
    pub elo_high: f64,
    pub llr: f64,
    pub sprt_decision: SprtDecision,
    pub total_nodes: u64,
    pub elapsed: Duration,
    pub error: Option<String>,
    pub reference_elo: Option<f64>,
    pub config: MatchConfig,
    pub opening_count: usize,
    pub opening_fingerprint: String,
    pub resumed_pairs: usize,
}

impl MatchSummary {
    pub fn candidate_elo(&self) -> Option<f64> {
        self.reference_elo.map(|elo| elo + self.elo_delta)
    }

    pub fn to_json_value(&self) -> serde_json::Value {
        let (candidate_telemetry, reference_telemetry) = self.aggregate_telemetry();
        let _decision = match self.sprt_decision {
            SprtDecision::Continue => "continue",
            SprtDecision::AcceptH0 => "accept_h0",
            SprtDecision::AcceptH1 => "accept_h1",
        };
        let _pairs: Vec<_> = self
            .pairs
            .iter()
            .map(|pair| {
                serde_json::json!({
                    "index": pair.index,
                    "candidate_white": game_json(&pair.candidate_white),
                    "candidate_black": game_json(&pair.candidate_black),
                })
            })
            .collect();
        serde_json::json!({
            "candidate": self.candidate,
            "reference": self.reference,
            "pairs": self.pairs.len(),
            "games": self.scores.games(),
            "wins": self.scores.wins,
            "draws": self.scores.draws,
            "losses": self.scores.losses,
            "pentanomial": self.pair_counts.bins,
            "score_rate": self.scores.score_rate(),
            "elo_delta": self.elo_delta,
            "elo_95_low": finite_or_null(self.elo_low),
            "elo_95_high": finite_or_null(self.elo_high),
            "reference_elo": self.reference_elo,
            "candidate_elo": self.candidate_elo(),
            "llr": self.llr,
            "sprt_decision": _decision,
            "total_nodes": self.total_nodes,
            "elapsed_ms": self.elapsed.as_millis(),
            "resumed_pairs": self.resumed_pairs,
            "aggregate_nps": if self.resumed_pairs > 0 || self.elapsed.as_millis() == 0 {
                None
            } else {
                Some(self.total_nodes.saturating_mul(1000) / self.elapsed.as_millis() as u64)
            },
            "telemetry": {
                "candidate": telemetry_json(&candidate_telemetry),
                "reference": telemetry_json(&reference_telemetry),
            },
            "error": self.error,
            "config": match_config_json(
                &self.config,
                self.opening_count,
                &self.opening_fingerprint,
            ),
            "results": _pairs,
        })
    }

    fn aggregate_telemetry(&self) -> (EngineTelemetry, EngineTelemetry) {
        let mut candidate = EngineTelemetry::default();
        let mut reference = EngineTelemetry::default();
        for game in self
            .pairs
            .iter()
            .flat_map(|pair| [&pair.candidate_white, &pair.candidate_black])
        {
            candidate.merge(&game.candidate_telemetry);
            reference.merge(&game.reference_telemetry);
        }
        (candidate, reference)
    }
}

fn match_config_json(
    config: &MatchConfig,
    opening_count: usize,
    opening_fingerprint: &str,
) -> serde_json::Value {
    serde_json::json!({
        "requested_pairs": config.pairs,
        "opening_offset": config.opening_offset,
        "concurrency": config.concurrency,
        "opening_count": opening_count,
        "opening_fingerprint": opening_fingerprint,
        "nodes_per_move": config.nodes_per_move,
        "move_time_ms": config.move_time.map(|duration| duration.as_millis()),
        "max_depth": config.max_depth,
        "hash_mb": config.hash_mb,
        "engine_threads": config.engine_threads,
        "max_engine_memory_mb": config.max_engine_memory_mb,
        "max_match_memory_mb": config.max_match_memory_mb,
        "session_pairs": config.session_pairs,
        "max_plies": config.max_plies,
        "read_timeout_ms": config.read_timeout.as_millis(),
        "checkpoint_path": config
            .checkpoint_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "early_stop": config.early_stop,
        "adjudication": {
            "resign_cp": config.resign_cp,
            "resign_plies": config.resign_plies,
            "draw_cp": config.draw_cp,
            "draw_plies": config.draw_plies,
            "draw_min_ply": config.draw_min_ply,
        },
        "sprt": {
            "elo0": config.sprt.elo0,
            "elo1": config.sprt.elo1,
            "alpha": config.sprt.alpha,
            "beta": config.sprt.beta,
        },
    })
}

fn finite_or_null(value: f64) -> serde_json::Value {
    if value.is_finite() {
        serde_json::json!(value)
    } else {
        serde_json::Value::Null
    }
}

fn telemetry_json(telemetry: &EngineTelemetry) -> serde_json::Value {
    serde_json::json!({
        "searches": telemetry.searches,
        "nodes": telemetry.nodes,
        "search_time_ms": telemetry.search_time.as_millis(),
        "nps": telemetry.nps(),
        "depth_sum": telemetry.depth_sum,
        "average_depth": telemetry.average_depth(),
        "max_depth": telemetry.max_depth,
        "max_seldepth": telemetry.max_seldepth,
    })
}

fn game_json(game: &GameRecord) -> serde_json::Value {
    let _outcome = match game.outcome {
        GameOutcome::Win => "win",
        GameOutcome::Draw => "draw",
        GameOutcome::Loss => "loss",
    };
    let _termination = match &game.termination {
        Termination::Checkmate => "checkmate",
        Termination::DrawRule => "draw_rule",
        Termination::AdjudicatedWin => "adjudicated_win",
        Termination::AdjudicatedDraw => "adjudicated_draw",
        Termination::MaxPlies => "max_plies",
        Termination::Forfeit(_) => "forfeit",
    };
    let _detail = match &game.termination {
        Termination::Forfeit(message) => Some(message.as_str()),
        _ => None,
    };
    serde_json::json!({
        "outcome": _outcome,
        "termination": _termination,
        "detail": _detail,
        "plies": game.plies,
        "nodes": game.nodes,
        "elapsed_ms": game.elapsed.as_millis(),
        "telemetry": {
            "candidate": telemetry_json(&game.candidate_telemetry),
            "reference": telemetry_json(&game.reference_telemetry),
        },
        "moves": game.moves,
    })
}

struct CheckpointWriter {
    writer: BufWriter<File>,
}

impl CheckpointWriter {
    fn open(
        path: &Path,
        identity: &serde_json::Value,
        requested_pairs: usize,
    ) -> Result<(Self, Vec<PairRecord>), String> {
        let mut resumed = Vec::new();
        let mut truncate_to = None;
        if path.exists() {
            let contents = std::fs::read_to_string(path).map_err(|error| {
                format!("failed to read checkpoint '{}': {error}", path.display())
            })?;
            let lines: Vec<_> = contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            let Some(header_line) = lines.first() else {
                return Err(format!("checkpoint '{}' is empty", path.display()));
            };
            let header: serde_json::Value = serde_json::from_str(header_line).map_err(|error| {
                format!("invalid checkpoint header '{}': {error}", path.display())
            })?;
            if &header != identity {
                return Err(format!(
                    "checkpoint '{}' does not match this duel",
                    path.display()
                ));
            }

            let mut pairs = BTreeMap::new();
            for (line_index, line) in lines.iter().enumerate().skip(1) {
                match serde_json::from_str::<serde_json::Value>(line)
                    .map_err(|error| error.to_string())
                    .and_then(|value| pair_from_checkpoint(&value))
                {
                    Ok(pair) if pair.index < requested_pairs => {
                        pairs.insert(pair.index, pair);
                    }
                    Ok(_) => {}
                    Err(_) if line_index + 1 == lines.len() => {
                        // A controller can die between writing the JSON payload and newline.
                        truncate_to = contents.rfind(line).map(|offset| offset as u64);
                        break;
                    }
                    Err(error) => {
                        return Err(format!(
                            "invalid checkpoint record {} in '{}': {error}",
                            line_index + 1,
                            path.display()
                        ));
                    }
                }
            }
            resumed.extend(pairs.into_values());

            if let Some(length) = truncate_to {
                let file = OpenOptions::new().write(true).open(path).map_err(|error| {
                    format!("failed to repair checkpoint '{}': {error}", path.display())
                })?;
                file.set_len(length).map_err(|error| {
                    format!("failed to repair checkpoint '{}': {error}", path.display())
                })?;
                file.sync_data().map_err(|error| {
                    format!(
                        "failed to sync repaired checkpoint '{}': {error}",
                        path.display()
                    )
                })?;
            }
        } else {
            let mut file = File::create(path).map_err(|error| {
                format!("failed to create checkpoint '{}': {error}", path.display())
            })?;
            serde_json::to_writer(&mut file, identity).map_err(|error| {
                format!("failed to write checkpoint '{}': {error}", path.display())
            })?;
            writeln!(file).map_err(|error| {
                format!("failed to write checkpoint '{}': {error}", path.display())
            })?;
            file.sync_data().map_err(|error| {
                format!("failed to sync checkpoint '{}': {error}", path.display())
            })?;
        }

        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|error| {
                format!("failed to append checkpoint '{}': {error}", path.display())
            })?;
        Ok((
            Self {
                writer: BufWriter::new(file),
            },
            resumed,
        ))
    }

    fn append(&mut self, pair: &PairRecord) -> Result<(), String> {
        let value = serde_json::json!({
            "type": "pair",
            "index": pair.index,
            "candidate_white": game_json(&pair.candidate_white),
            "candidate_black": game_json(&pair.candidate_black),
        });
        serde_json::to_writer(&mut self.writer, &value)
            .map_err(|error| format!("failed to encode checkpoint pair: {error}"))?;
        writeln!(self.writer)
            .map_err(|error| format!("failed to write checkpoint pair: {error}"))?;
        self.writer
            .flush()
            .map_err(|error| format!("failed to flush checkpoint pair: {error}"))?;
        self.writer
            .get_ref()
            .sync_data()
            .map_err(|error| format!("failed to sync checkpoint pair: {error}"))
    }
}

fn checkpoint_identity(
    candidate: &EngineSpec,
    reference: &EngineSpec,
    config: &MatchConfig,
    opening_fingerprint: &str,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "type": "mujrim-duel-checkpoint",
        "version": 1,
        "candidate": engine_identity(candidate)?,
        "reference": engine_identity(reference)?,
        "opening_fingerprint": opening_fingerprint,
        "opening_offset": config.opening_offset,
        "nodes_per_move": config.nodes_per_move,
        "move_time_ms": config.move_time.map(|duration| duration.as_millis()),
        "max_depth": config.max_depth,
        "hash_mb": config.hash_mb,
        "engine_threads": config.engine_threads,
        "session_pairs": config.session_pairs,
        "max_plies": config.max_plies,
        "adjudication": {
            "resign_cp": config.resign_cp,
            "resign_plies": config.resign_plies,
            "draw_cp": config.draw_cp,
            "draw_plies": config.draw_plies,
            "draw_min_ply": config.draw_min_ply,
        },
    }))
}

fn engine_identity(engine: &EngineSpec) -> Result<serde_json::Value, String> {
    let resolved = resolve_executable_path(&engine.path);
    let path = std::fs::canonicalize(&resolved).unwrap_or_else(|_| resolved.clone());
    let metadata = std::fs::metadata(&resolved).map_err(|error| {
        format!(
            "failed to inspect engine '{}': {error}",
            engine.path.display()
        )
    })?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis());
    Ok(serde_json::json!({
        "path": path,
        "args": &engine.args,
        "uci_options": &engine.uci_options,
        "length": metadata.len(),
        "modified_ms": modified_ms,
    }))
}

fn resolve_executable_path(path: &Path) -> PathBuf {
    if path.components().count() != 1 || path.is_file() {
        return path.to_path_buf();
    }
    let Some(search_path) = std::env::var_os("PATH") else {
        return path.to_path_buf();
    };
    let extensions = executable_extensions();
    resolve_executable_in(path, &search_path, &extensions).unwrap_or_else(|| path.to_path_buf())
}

fn resolve_executable_in(
    path: &Path,
    search_path: &std::ffi::OsStr,
    extensions: &[String],
) -> Option<PathBuf> {
    for directory in std::env::split_paths(search_path) {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Some(candidate);
        }
        if path.extension().is_none() {
            for extension in extensions {
                let mut candidate = directory.join(path);
                candidate.set_extension(extension);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var_os("PATHEXT").map_or_else(
            || vec!["exe".to_string()],
            |value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .map(|extension| extension.trim().trim_start_matches('.').to_string())
                    .filter(|extension| !extension.is_empty())
                    .collect()
            },
        )
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn pair_from_checkpoint(value: &serde_json::Value) -> Result<PairRecord, String> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("pair") {
        return Err("record type is not 'pair'".to_string());
    }
    let index = value
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or("pair index is missing or invalid")?;
    Ok(PairRecord {
        index,
        candidate_white: game_from_checkpoint(
            value
                .get("candidate_white")
                .ok_or("candidate_white is missing")?,
            true,
        )?,
        candidate_black: game_from_checkpoint(
            value
                .get("candidate_black")
                .ok_or("candidate_black is missing")?,
            false,
        )?,
    })
}

fn game_from_checkpoint(
    value: &serde_json::Value,
    candidate_white: bool,
) -> Result<GameRecord, String> {
    let outcome = match value.get("outcome").and_then(serde_json::Value::as_str) {
        Some("win") => GameOutcome::Win,
        Some("draw") => GameOutcome::Draw,
        Some("loss") => GameOutcome::Loss,
        _ => return Err("game outcome is missing or invalid".to_string()),
    };
    let termination = match value.get("termination").and_then(serde_json::Value::as_str) {
        Some("checkmate") => Termination::Checkmate,
        Some("draw_rule") => Termination::DrawRule,
        Some("adjudicated_win") => Termination::AdjudicatedWin,
        Some("adjudicated_draw") => Termination::AdjudicatedDraw,
        Some("max_plies") => Termination::MaxPlies,
        Some("forfeit") => Termination::Forfeit(
            value
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("checkpointed forfeit")
                .to_string(),
        ),
        _ => return Err("game termination is missing or invalid".to_string()),
    };
    let plies = value
        .get("plies")
        .and_then(serde_json::Value::as_u64)
        .and_then(|plies| usize::try_from(plies).ok())
        .ok_or("game plies are missing or invalid")?;
    let nodes = value
        .get("nodes")
        .and_then(serde_json::Value::as_u64)
        .ok_or("game nodes are missing or invalid")?;
    let elapsed_ms = value
        .get("elapsed_ms")
        .and_then(serde_json::Value::as_u64)
        .ok_or("game elapsed_ms is missing or invalid")?;
    let moves = value
        .get("moves")
        .and_then(serde_json::Value::as_array)
        .map(|moves| {
            moves
                .iter()
                .map(|mv| {
                    mv.as_str()
                        .map(ToOwned::to_owned)
                        .ok_or("game move is not a string")
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let telemetry = value.get("telemetry");
    let candidate_telemetry = telemetry
        .and_then(|value| value.get("candidate"))
        .map(telemetry_from_json)
        .transpose()?
        .unwrap_or_default();
    let reference_telemetry = telemetry
        .and_then(|value| value.get("reference"))
        .map(telemetry_from_json)
        .transpose()?
        .unwrap_or_default();
    Ok(GameRecord {
        candidate_white,
        outcome,
        termination,
        plies,
        nodes,
        elapsed: Duration::from_millis(elapsed_ms),
        candidate_telemetry,
        reference_telemetry,
        moves,
    })
}

fn telemetry_from_json(value: &serde_json::Value) -> Result<EngineTelemetry, String> {
    let read_u64 = |name| {
        value
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("telemetry {name} is missing or invalid"))
    };
    let read_i32 = |name| {
        value
            .get(name)
            .and_then(serde_json::Value::as_i64)
            .and_then(|number| i32::try_from(number).ok())
            .ok_or_else(|| format!("telemetry {name} is missing or invalid"))
    };
    Ok(EngineTelemetry {
        searches: read_u64("searches")?,
        nodes: read_u64("nodes")?,
        search_time: Duration::from_millis(read_u64("search_time_ms")?),
        depth_sum: read_u64("depth_sum")?,
        max_depth: read_i32("max_depth")?,
        max_seldepth: read_i32("max_seldepth")?,
    })
}

fn validate_resource_budget(config: &MatchConfig) -> Result<(), String> {
    if config.move_time.is_none() && config.nodes_per_move == 0 && config.max_depth <= 0 {
        return Err("matches require a positive node, time, or depth limit".to_owned());
    }
    if config.move_time.is_some_and(|duration| duration.is_zero()) {
        return Err("timed matches require a positive move time".to_owned());
    }
    if config.max_match_memory_mb == 0 {
        return Err("maximum match memory must be positive".to_string());
    }
    let engine_count = config
        .concurrency
        .checked_mul(2)
        .ok_or("configured engine count overflows")?;
    let requested_mb = engine_count
        .checked_mul(config.max_engine_memory_mb)
        .ok_or("configured match memory ceiling overflows")?;
    if requested_mb > config.max_match_memory_mb {
        return Err(format!(
            "configured match can reserve {requested_mb} MiB across {engine_count} engines, exceeding the {} MiB aggregate limit; lower --concurrency or --max-engine-memory, or explicitly raise --max-match-memory on a monitored host",
            config.max_match_memory_mb
        ));
    }
    Ok(())
}

pub fn run_match(
    candidate: EngineSpec,
    reference: EngineSpec,
    openings: Option<Vec<Opening>>,
    config: MatchConfig,
) -> MatchSummary {
    let started = Instant::now();
    let openings = Arc::new(
        openings
            .filter(|openings| !openings.is_empty())
            .unwrap_or_else(default_openings),
    );
    let opening_count = openings.len();
    let opening_fingerprint = openings_fingerprint(&openings);
    let checkpoint_setup: Result<(Option<CheckpointWriter>, Vec<PairRecord>), String> =
        validate_resource_budget(&config).and_then(|()| {
            config.checkpoint_path.as_deref().map_or_else(
                || Ok((None, Vec::new())),
                |path| {
                    let identity =
                        checkpoint_identity(&candidate, &reference, &config, &opening_fingerprint)?;
                    let (writer, pairs) = CheckpointWriter::open(path, &identity, config.pairs)?;
                    Ok((Some(writer), pairs))
                },
            )
        });
    let (checkpoint_writer, resumed, checkpoint_error) = match checkpoint_setup {
        Ok((writer, pairs)) => (writer, pairs, None),
        Err(message) => (None, Vec::new(), Some(message)),
    };
    let resumed_pairs = resumed.len();
    let completed = Arc::new(
        resumed
            .iter()
            .map(|pair| pair.index)
            .collect::<HashSet<_>>(),
    );
    let resumed_decision = config.sprt.paired_decision(pair_count_records(&resumed));
    let next_pair = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(
        checkpoint_error.is_some()
            || resumed_pairs >= config.pairs
            || resumed_decision != SprtDecision::Continue
            || config
                .stop_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire)),
    ));
    let mut initial_results = Vec::with_capacity(config.pairs);
    initial_results.extend(resumed);
    let results = Arc::new(Mutex::new(initial_results));
    let checkpoint_writer = Arc::new(Mutex::new(checkpoint_writer));
    let error = Arc::new(Mutex::new(checkpoint_error));
    let workers = config.concurrency.max(1).min(config.pairs.max(1));
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let candidate = candidate.clone();
        let reference = reference.clone();
        let openings = Arc::clone(&openings);
        let next_pair = Arc::clone(&next_pair);
        let completed = Arc::clone(&completed);
        let stopped = Arc::clone(&stopped);
        let results = Arc::clone(&results);
        let checkpoint_writer = Arc::clone(&checkpoint_writer);
        let error = Arc::clone(&error);
        let config = config.clone();
        handles.push(std::thread::spawn(move || {
            let mut sessions = None;
            let mut session_pairs = 0usize;

            loop {
                if config
                    .stop_flag
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Acquire))
                {
                    stopped.store(true, Ordering::Release);
                }
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let index = next_pair.fetch_add(1, Ordering::Relaxed);
                if index >= config.pairs {
                    break;
                }
                if completed.contains(&index) {
                    continue;
                }
                if sessions.is_none() || session_pairs >= config.session_pairs.max(1) {
                    drop(sessions.take());
                    sessions = match spawn_sessions(&candidate, &reference, &config) {
                        Ok(sessions) => Some(sessions),
                        Err(message) => {
                            *lock_recover(&error) = Some(message);
                            stopped.store(true, Ordering::Release);
                            break;
                        }
                    };
                    session_pairs = 0;
                }
                let opening = &openings[(config.opening_offset + index) % openings.len()];
                let white = {
                    let (candidate_session, reference_session) =
                        sessions.as_mut().expect("sessions initialized above");
                    play_game(
                        candidate_session,
                        reference_session,
                        opening,
                        true,
                        &config,
                        &candidate.name,
                        &reference.name,
                        &format!("pair{index}-cw"),
                    )
                };
                if let Some(message) = resource_limit_detail(&white) {
                    *lock_recover(&error) = Some(message.to_string());
                    stopped.store(true, Ordering::Release);
                    break;
                }
                if requires_session_recycle(&white) {
                    drop(sessions.take());
                    sessions = match spawn_sessions(&candidate, &reference, &config) {
                        Ok(sessions) => Some(sessions),
                        Err(message) => {
                            *lock_recover(&error) = Some(message);
                            stopped.store(true, Ordering::Release);
                            break;
                        }
                    };
                    session_pairs = 0;
                }
                let black = {
                    let (candidate_session, reference_session) =
                        sessions.as_mut().expect("sessions initialized above");
                    play_game(
                        candidate_session,
                        reference_session,
                        opening,
                        false,
                        &config,
                        &candidate.name,
                        &reference.name,
                        &format!("pair{index}-cb"),
                    )
                };
                if let Some(message) = resource_limit_detail(&black) {
                    *lock_recover(&error) = Some(message.to_string());
                    stopped.store(true, Ordering::Release);
                    break;
                }
                let recycle_after_black = requires_session_recycle(&black);
                let pair = PairRecord {
                    index,
                    candidate_white: white,
                    candidate_black: black,
                };
                if let Some(writer) = lock_recover(&checkpoint_writer).as_mut()
                    && let Err(message) = writer.append(&pair)
                {
                    *lock_recover(&error) = Some(message);
                    stopped.store(true, Ordering::Release);
                    break;
                }
                let mut locked = lock_recover(&results);
                locked.push(pair);
                if recycle_after_black {
                    drop(sessions.take());
                    session_pairs = 0;
                } else {
                    session_pairs += 1;
                }
                let pair_counts = pair_count_records(&locked);
                drop(locked);
                if config.early_stop
                    && config.sprt.paired_decision(pair_counts) != SprtDecision::Continue
                {
                    stopped.store(true, Ordering::Release);
                }
            }
        }));
    }

    for handle in handles {
        if handle.join().is_err() {
            *lock_recover(&error) = Some("match worker panicked".to_string());
        }
    }

    let mut pairs = Arc::try_unwrap(results)
        .expect("match results still shared")
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pairs.sort_unstable_by_key(|pair| pair.index);
    let scores = score_records(&pairs);
    let pair_counts = pair_count_records(&pairs);
    let outcomes: Vec<_> = pairs
        .iter()
        .map(|pair| (pair.candidate_white.outcome, pair.candidate_black.outcome))
        .collect();
    let (elo_low, elo_high) = paired_elo_interval(&outcomes);
    let total_nodes = pairs
        .iter()
        .flat_map(|pair| [&pair.candidate_white, &pair.candidate_black])
        .map(|game| game.nodes)
        .sum();

    MatchSummary {
        candidate: candidate.name,
        reference: reference.name,
        pairs,
        scores,
        pair_counts,
        elo_delta: scores.elo(),
        elo_low,
        elo_high,
        llr: config.sprt.paired_llr(pair_counts),
        sprt_decision: config.sprt.paired_decision(pair_counts),
        total_nodes,
        elapsed: started.elapsed(),
        error: Arc::try_unwrap(error)
            .expect("match error still shared")
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        reference_elo: config.reference_elo,
        config,
        opening_count,
        opening_fingerprint,
        resumed_pairs,
    }
}

fn spawn_sessions(
    candidate: &EngineSpec,
    reference: &EngineSpec,
    config: &MatchConfig,
) -> Result<(EngineSession, EngineSession), String> {
    let memory_limit = Some((config.max_engine_memory_mb as u64).saturating_mul(1024 * 1024));
    let mut candidate_session = EngineSession::spawn_with_args_and_memory_limit(
        &candidate.path,
        &candidate.args,
        ProtocolKind::Uci,
        memory_limit,
    )?;
    let mut reference_session = EngineSession::spawn_with_args_and_memory_limit(
        &reference.path,
        &reference.args,
        ProtocolKind::Uci,
        memory_limit,
    )?;
    let common_options = EngineOptions {
        hash_mb: Some(config.hash_mb),
        threads: Some(config.engine_threads),
        own_book: Some(false),
        custom: Vec::new(),
    };
    let mut candidate_options = common_options.clone();
    candidate_options.custom.clone_from(&candidate.uci_options);
    let mut reference_options = common_options;
    reference_options.custom.clone_from(&reference.uci_options);
    candidate_session.configure(&candidate_options)?;
    reference_session.configure(&reference_options)?;
    candidate_session.set_read_timeout(config.read_timeout);
    reference_session.set_read_timeout(config.read_timeout);
    candidate_session.set_memory_limit_bytes(memory_limit);
    reference_session.set_memory_limit_bytes(memory_limit);
    Ok((candidate_session, reference_session))
}

fn resource_limit_detail(game: &GameRecord) -> Option<&str> {
    match &game.termination {
        Termination::Forfeit(message)
            if message.starts_with("engine working set exceeded limit:")
                || (message.contains("memory allocation of") && message.contains("failed")) =>
        {
            Some(message)
        }
        _ => None,
    }
}

fn requires_session_recycle(game: &GameRecord) -> bool {
    matches!(game.termination, Termination::Forfeit(_))
}

fn engine_color(candidate_white: bool, candidate_engine: bool) -> Color {
    if candidate_white == candidate_engine {
        Color::White
    } else {
        Color::Black
    }
}

#[allow(clippy::too_many_arguments)]
fn play_game(
    candidate: &mut EngineSession,
    reference: &mut EngineSession,
    opening: &Opening,
    candidate_white: bool,
    config: &MatchConfig,
    candidate_name: &str,
    reference_name: &str,
    game_key: &str,
) -> GameRecord {
    let started = Instant::now();
    let (white_name, black_name) = if candidate_white {
        (candidate_name, reference_name)
    } else {
        (reference_name, candidate_name)
    };
    let emit = |event: GameProgressEvent| {
        if let Some(callback) = config.game_progress.as_ref() {
            callback(event);
        }
    };
    let mut board = match opening.board() {
        Ok(board) => board,
        Err(message) => return forfeit(candidate_white, Color::White, message, started.elapsed()),
    };
    let mut moves = opening.moves.clone();
    let mut nodes = 0;
    let mut candidate_telemetry = EngineTelemetry::default();
    let mut reference_telemetry = EngineTelemetry::default();
    let mut adjudicator = Adjudicator::new(config);
    emit(GameProgressEvent::Started {
        game_key: game_key.to_owned(),
        white: white_name.to_owned(),
        black: black_name.to_owned(),
        initial_fen: opening.initial_fen.clone(),
    });
    let emit_done = |record: GameRecord| -> GameRecord {
        let white_score = if candidate_white {
            record.outcome.score()
        } else {
            1.0 - record.outcome.score()
        };
        emit(GameProgressEvent::Finished {
            game_key: game_key.to_owned(),
            white_score,
            moves: record.moves.clone(),
        });
        record
    };

    if let Err(message) = candidate.new_game() {
        return emit_done(with_moves(
            forfeit(
                candidate_white,
                engine_color(candidate_white, true),
                format!("candidate new-game initialization failed: {message}"),
                started.elapsed(),
            ),
            &moves,
        ));
    }
    if let Err(message) = reference.new_game() {
        return emit_done(with_moves(
            forfeit(
                candidate_white,
                engine_color(candidate_white, false),
                format!("reference new-game initialization failed: {message}"),
                started.elapsed(),
            ),
            &moves,
        ));
    }

    for ply in moves.len()..config.max_plies {
        let side = board.side_to_move;
        let candidate_turn = (side == Color::White) == candidate_white;
        let session = if candidate_turn {
            &mut *candidate
        } else {
            &mut *reference
        };
        let request = SearchRequest {
            fen: opening.initial_fen.clone(),
            moves: moves.clone(),
            depth: config.max_depth,
            movetime: config.move_time,
            node_limit: (config.move_time.is_none() && config.nodes_per_move > 0)
                .then_some(config.nodes_per_move),
        };
        let search_started = Instant::now();
        let info = match session.search(&request) {
            Ok(info) => info,
            Err(message) => {
                return emit_done(with_moves(
                    with_telemetry(
                        forfeit_with_progress(
                            candidate_white,
                            side,
                            message,
                            ply,
                            nodes,
                            started.elapsed(),
                        ),
                        &candidate_telemetry,
                        &reference_telemetry,
                    ),
                    &moves,
                ));
            }
        };
        let search_elapsed = search_started.elapsed();
        if candidate_turn {
            candidate_telemetry.observe(&info, search_elapsed);
        } else {
            reference_telemetry.observe(&info, search_elapsed);
        }
        nodes += info.nodes;
        let white_eval = if side == Color::White {
            info.score
        } else {
            -info.score
        };
        if let Some(winner) = adjudicator.observe(white_eval, ply) {
            return emit_done(with_moves(
                with_telemetry(
                    finish(
                        candidate_white,
                        winner,
                        Termination::AdjudicatedWin,
                        ply,
                        nodes,
                        started.elapsed(),
                    ),
                    &candidate_telemetry,
                    &reference_telemetry,
                ),
                &moves,
            ));
        }
        if adjudicator.is_draw() {
            return emit_done(with_moves(
                with_telemetry(
                    draw_record(
                        candidate_white,
                        Termination::AdjudicatedDraw,
                        ply,
                        nodes,
                        started.elapsed(),
                    ),
                    &candidate_telemetry,
                    &reference_telemetry,
                ),
                &moves,
            ));
        }

        let Some(mv) = resolve_legal_move(&mut board, &info.best_move) else {
            return emit_done(with_moves(
                with_telemetry(
                    forfeit_with_progress(
                        candidate_white,
                        side,
                        illegal_bestmove_detail(&info.best_move, ply, &opening.initial_fen, &moves),
                        ply,
                        nodes,
                        started.elapsed(),
                    ),
                    &candidate_telemetry,
                    &reference_telemetry,
                ),
                &moves,
            ));
        };
        moves.push(mv.to_uci());
        board.make_move(mv);
        emit(GameProgressEvent::Ply {
            game_key: game_key.to_owned(),
            ply: moves.len(),
            uci: mv.to_uci(),
            score_cp: white_eval,
            depth: info.depth,
            nodes: info.nodes,
            moves: moves.clone(),
        });
        let played = ply + 1;

        if board.is_checkmate() {
            return emit_done(with_moves(
                with_telemetry(
                    finish(
                        candidate_white,
                        board.side_to_move.opponent(),
                        Termination::Checkmate,
                        played,
                        nodes,
                        started.elapsed(),
                    ),
                    &candidate_telemetry,
                    &reference_telemetry,
                ),
                &moves,
            ));
        }
        if board.is_stalemate() || board.is_draw() {
            return emit_done(with_moves(
                with_telemetry(
                    draw_record(
                        candidate_white,
                        Termination::DrawRule,
                        played,
                        nodes,
                        started.elapsed(),
                    ),
                    &candidate_telemetry,
                    &reference_telemetry,
                ),
                &moves,
            ));
        }
    }

    emit_done(with_moves(
        with_telemetry(
            draw_record(
                candidate_white,
                Termination::MaxPlies,
                config.max_plies,
                nodes,
                started.elapsed(),
            ),
            &candidate_telemetry,
            &reference_telemetry,
        ),
        &moves,
    ))
}

fn score_records(pairs: &[PairRecord]) -> ScoreCount {
    let mut scores = ScoreCount::default();
    for pair in pairs {
        scores.push(pair.candidate_white.outcome);
        scores.push(pair.candidate_black.outcome);
    }
    scores
}

fn pair_count_records(pairs: &[PairRecord]) -> PairCount {
    let mut counts = PairCount::default();
    for pair in pairs {
        counts.push(pair.candidate_white.outcome, pair.candidate_black.outcome);
    }
    counts
}

fn finish(
    candidate_white: bool,
    winner: Color,
    termination: Termination,
    plies: usize,
    nodes: u64,
    elapsed: Duration,
) -> GameRecord {
    let candidate_won = (winner == Color::White) == candidate_white;
    GameRecord {
        candidate_white,
        outcome: if candidate_won {
            GameOutcome::Win
        } else {
            GameOutcome::Loss
        },
        termination,
        plies,
        nodes,
        elapsed,
        candidate_telemetry: EngineTelemetry::default(),
        reference_telemetry: EngineTelemetry::default(),
        moves: Vec::new(),
    }
}

fn forfeit(
    candidate_white: bool,
    forfeiting_side: Color,
    message: String,
    elapsed: Duration,
) -> GameRecord {
    forfeit_with_progress(candidate_white, forfeiting_side, message, 0, 0, elapsed)
}

fn forfeit_with_progress(
    candidate_white: bool,
    forfeiting_side: Color,
    message: String,
    plies: usize,
    nodes: u64,
    elapsed: Duration,
) -> GameRecord {
    finish(
        candidate_white,
        forfeiting_side.opponent(),
        Termination::Forfeit(message),
        plies,
        nodes,
        elapsed,
    )
}

fn illegal_bestmove_detail(
    best_move: &str,
    ply: usize,
    initial_fen: &str,
    moves: &[String],
) -> String {
    let move_list = moves.join(" ");
    format!(
        "illegal bestmove '{best_move}' at ply {ply}; position fen {initial_fen} moves {move_list}"
    )
}

fn draw_record(
    candidate_white: bool,
    termination: Termination,
    plies: usize,
    nodes: u64,
    elapsed: Duration,
) -> GameRecord {
    GameRecord {
        candidate_white,
        outcome: GameOutcome::Draw,
        termination,
        plies,
        nodes,
        elapsed,
        candidate_telemetry: EngineTelemetry::default(),
        reference_telemetry: EngineTelemetry::default(),
        moves: Vec::new(),
    }
}

fn with_moves(mut game: GameRecord, moves: &[String]) -> GameRecord {
    game.moves.extend_from_slice(moves);
    game
}

fn with_telemetry(
    mut game: GameRecord,
    candidate: &EngineTelemetry,
    reference: &EngineTelemetry,
) -> GameRecord {
    debug_assert_eq!(game.nodes, candidate.nodes.saturating_add(reference.nodes));
    game.candidate_telemetry = candidate.clone();
    game.reference_telemetry = reference.clone();
    game
}

struct Adjudicator {
    resign_cp: i32,
    resign_plies: usize,
    draw_cp: i32,
    draw_plies: usize,
    draw_min_ply: usize,
    decisive_side: Option<Color>,
    decisive_count: usize,
    draw_count: usize,
}

impl Adjudicator {
    fn new(config: &MatchConfig) -> Self {
        Self {
            resign_cp: config.resign_cp,
            resign_plies: config.resign_plies,
            draw_cp: config.draw_cp,
            draw_plies: config.draw_plies,
            draw_min_ply: config.draw_min_ply,
            decisive_side: None,
            decisive_count: 0,
            draw_count: 0,
        }
    }

    fn observe(&mut self, white_eval: i32, ply: usize) -> Option<Color> {
        if white_eval.abs() >= self.resign_cp {
            let side = if white_eval > 0 {
                Color::White
            } else {
                Color::Black
            };
            if self.decisive_side == Some(side) {
                self.decisive_count += 1;
            } else {
                self.decisive_side = Some(side);
                self.decisive_count = 1;
            }
        } else {
            self.decisive_side = None;
            self.decisive_count = 0;
        }

        if ply >= self.draw_min_ply && white_eval.abs() <= self.draw_cp {
            self.draw_count += 1;
        } else {
            self.draw_count = 0;
        }

        if self.decisive_count >= self.resign_plies {
            self.decisive_side
        } else {
            None
        }
    }

    fn is_draw(&self) -> bool {
        self.draw_count >= self.draw_plies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint_test_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock predates Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mujrim-duel-checkpoint-{}-{nonce}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn adjudication_requires_stable_scores() {
        let config = MatchConfig {
            resign_cp: 500,
            resign_plies: 3,
            ..MatchConfig::default()
        };
        let mut adjudicator = Adjudicator::new(&config);
        assert_eq!(adjudicator.observe(600, 10), None);
        assert_eq!(adjudicator.observe(700, 11), None);
        assert_eq!(adjudicator.observe(-800, 12), None);
        assert_eq!(adjudicator.observe(-900, 13), None);
        assert_eq!(adjudicator.observe(-1_000, 14), Some(Color::Black));
    }

    #[test]
    fn finish_maps_colors_to_candidate_outcome() {
        let white = finish(
            true,
            Color::White,
            Termination::Checkmate,
            1,
            1,
            Duration::ZERO,
        );
        let black = finish(
            false,
            Color::White,
            Termination::Checkmate,
            1,
            1,
            Duration::ZERO,
        );
        assert_eq!(white.outcome, GameOutcome::Win);
        assert_eq!(black.outcome, GameOutcome::Loss);
    }

    #[test]
    fn engine_colors_and_forfeit_recycling_follow_the_pair_assignment() {
        assert_eq!(engine_color(true, true), Color::White);
        assert_eq!(engine_color(true, false), Color::Black);
        assert_eq!(engine_color(false, true), Color::Black);
        assert_eq!(engine_color(false, false), Color::White);

        let forfeit = forfeit(
            true,
            Color::White,
            "disconnected".to_string(),
            Duration::ZERO,
        );
        let draw = draw_record(true, Termination::MaxPlies, 1, 1, Duration::ZERO);
        assert!(requires_session_recycle(&forfeit));
        assert!(!requires_session_recycle(&draw));
    }

    #[test]
    fn forfeits_preserve_progress_and_replay_context() {
        let moves = vec!["e2e4".to_string(), "e7e5".to_string()];
        let detail = illegal_bestmove_detail("0000", 2, "test-fen", &moves);
        assert_eq!(
            detail,
            "illegal bestmove '0000' at ply 2; position fen test-fen moves e2e4 e7e5"
        );

        let game = forfeit_with_progress(true, Color::White, detail, 2, 40_000, Duration::ZERO);
        assert_eq!(game.plies, 2);
        assert_eq!(game.nodes, 40_000);
        assert_eq!(game.outcome, GameOutcome::Loss);
    }

    #[test]
    fn engine_telemetry_tracks_search_cost_and_round_trips() {
        let mut candidate = EngineTelemetry::default();
        candidate.observe(
            &SearchInfo {
                depth: 12,
                seldepth: 19,
                nodes: 50_000,
                time_ms: 20,
                ..SearchInfo::default()
            },
            Duration::from_millis(25),
        );
        candidate.observe(
            &SearchInfo {
                depth: 14,
                seldepth: 22,
                nodes: 70_000,
                time_ms: 30,
                ..SearchInfo::default()
            },
            Duration::from_millis(35),
        );
        assert_eq!(candidate.searches, 2);
        assert_eq!(candidate.nodes, 120_000);
        assert_eq!(candidate.search_time, Duration::from_millis(50));
        assert_eq!(candidate.average_depth(), Some(13.0));
        assert_eq!(candidate.max_depth, 14);
        assert_eq!(candidate.max_seldepth, 22);
        assert_eq!(candidate.nps(), Some(2_400_000));

        let game = with_telemetry(
            finish(
                true,
                Color::White,
                Termination::Checkmate,
                2,
                120_000,
                Duration::from_millis(60),
            ),
            &candidate,
            &EngineTelemetry::default(),
        );
        let restored = game_from_checkpoint(&game_json(&game), true).expect("telemetry checkpoint");
        assert_eq!(restored.candidate_telemetry.nodes, candidate.nodes);
        assert_eq!(restored.candidate_telemetry.depth_sum, candidate.depth_sum);
        assert_eq!(restored.candidate_telemetry.max_seldepth, 22);
    }

    #[test]
    fn legacy_checkpoint_without_telemetry_remains_readable() {
        let game = finish(
            true,
            Color::White,
            Termination::Checkmate,
            1,
            10,
            Duration::from_millis(1),
        );
        let mut json = game_json(&game);
        json.as_object_mut()
            .expect("game object")
            .remove("telemetry");

        let restored = game_from_checkpoint(&json, true).expect("legacy checkpoint");
        assert_eq!(restored.nodes, 10);
        assert_eq!(restored.candidate_telemetry.searches, 0);
        assert_eq!(restored.reference_telemetry.searches, 0);
    }

    #[test]
    fn pair_counts_preserve_paired_outcomes() {
        let pair = PairRecord {
            index: 0,
            candidate_white: finish(
                true,
                Color::White,
                Termination::Checkmate,
                1,
                1,
                Duration::ZERO,
            ),
            candidate_black: draw_record(false, Termination::DrawRule, 2, 1, Duration::ZERO),
        };
        assert_eq!(pair_count_records(&[pair]).bins, [0, 0, 0, 1, 0]);
    }

    #[test]
    fn config_json_captures_reproducibility_inputs() {
        let config = MatchConfig {
            pairs: 12,
            opening_offset: 64,
            nodes_per_move: 4_000,
            engine_threads: 2,
            ..MatchConfig::default()
        };
        let json = match_config_json(&config, 512, "0123456789abcdef");
        assert_eq!(json["requested_pairs"], 12);
        assert_eq!(json["opening_offset"], 64);
        assert_eq!(json["nodes_per_move"], 4_000);
        assert_eq!(json["engine_threads"], 2);
        assert_eq!(json["max_engine_memory_mb"], 384);
        assert_eq!(json["max_match_memory_mb"], 768);
        assert_eq!(json["session_pairs"], 1);
        assert_eq!(json["opening_count"], 512);
        assert_eq!(json["opening_fingerprint"], "0123456789abcdef");
        assert_eq!(json["sprt"]["elo0"], config.sprt.elo0);
    }

    #[test]
    fn match_defaults_to_one_pair_worker() {
        assert_eq!(MatchConfig::default().concurrency, 1);
        assert_eq!(MatchConfig::default().max_engine_memory_mb, 384);
        assert_eq!(MatchConfig::default().max_match_memory_mb, 768);
        assert_eq!(MatchConfig::default().session_pairs, 1);
    }

    #[test]
    fn aggregate_memory_budget_rejects_unsafe_parallelism() {
        let config = MatchConfig {
            concurrency: 2,
            ..MatchConfig::default()
        };

        let error = validate_resource_budget(&config).expect_err("1.5 GiB must exceed default cap");

        assert!(error.contains("1536 MiB"));
        assert!(error.contains("768 MiB aggregate limit"));
    }

    #[test]
    fn positive_depth_is_a_valid_hard_limit() {
        let config = MatchConfig {
            nodes_per_move: 0,
            move_time: None,
            max_depth: 10,
            ..MatchConfig::default()
        };
        assert!(validate_resource_budget(&config).is_ok());
    }

    #[test]
    fn checkpoint_identity_allows_extension_and_sprt_retargeting() {
        let engine = EngineSpec::new(std::env::current_exe().expect("test executable path"));
        let initial = MatchConfig::default();
        let retargeted = MatchConfig {
            pairs: 256,
            sprt: Sprt {
                elo0: -122.0,
                elo1: -102.0,
                ..Sprt::default()
            },
            reference_elo: Some(3_612.0),
            ..initial.clone()
        };

        let initial_identity = checkpoint_identity(&engine, &engine, &initial, "opening-set")
            .expect("initial checkpoint identity");
        let retargeted_identity = checkpoint_identity(&engine, &engine, &retargeted, "opening-set")
            .expect("retargeted checkpoint identity");

        assert_eq!(initial_identity, retargeted_identity);
    }

    #[test]
    fn executable_identity_resolves_a_bare_name_from_search_path() {
        let executable = std::env::current_exe().expect("test executable path");
        let directory = executable.parent().expect("test executable directory");
        let filename = executable.file_name().expect("test executable filename");
        let resolved = resolve_executable_in(Path::new(filename), directory.as_os_str(), &[])
            .expect("resolve test executable");

        assert_eq!(resolved, executable);

        #[cfg(windows)]
        {
            let stem = executable.file_stem().expect("test executable stem");
            let extension = executable
                .extension()
                .expect("test executable extension")
                .to_string_lossy()
                .into_owned();
            let resolved =
                resolve_executable_in(Path::new(stem), directory.as_os_str(), &[extension])
                    .expect("resolve extensionless test executable");
            assert_eq!(resolved, executable);
        }
    }

    #[test]
    fn resource_limit_forfeit_is_not_strength_data() {
        for detail in [
            "engine working set exceeded limit: 257 MiB > 256 MiB",
            "engine closed stdout: memory allocation of 11952 bytes failed",
        ] {
            let game = GameRecord {
                candidate_white: true,
                outcome: GameOutcome::Loss,
                termination: Termination::Forfeit(detail.to_string()),
                plies: 0,
                nodes: 0,
                elapsed: Duration::ZERO,
                candidate_telemetry: EngineTelemetry::default(),
                reference_telemetry: EngineTelemetry::default(),
                moves: Vec::new(),
            };

            assert!(resource_limit_detail(&game).is_some());
        }
    }

    #[test]
    fn checkpoint_round_trip_repairs_partial_tail() {
        let path = checkpoint_test_path();
        let engine = EngineSpec::new(std::env::current_exe().expect("test executable path"));
        let config = MatchConfig {
            pairs: 4,
            checkpoint_path: Some(path.clone()),
            ..MatchConfig::default()
        };
        let identity = checkpoint_identity(&engine, &engine, &config, "opening-set")
            .expect("checkpoint identity");
        let (mut writer, resumed) =
            CheckpointWriter::open(&path, &identity, config.pairs).expect("new checkpoint");
        assert!(resumed.is_empty());

        let first = PairRecord {
            index: 2,
            candidate_white: with_moves(
                finish(
                    true,
                    Color::White,
                    Termination::Checkmate,
                    31,
                    1_024,
                    Duration::from_millis(7),
                ),
                &["e2e4".to_owned(), "e7e5".to_owned()],
            ),
            candidate_black: draw_record(
                false,
                Termination::DrawRule,
                48,
                2_048,
                Duration::from_millis(9),
            ),
        };
        writer.append(&first).expect("checkpoint first pair");
        drop(writer);

        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append partial record");
        write!(file, "{{\"type\":").expect("write partial record");
        file.sync_data().expect("sync partial record");
        drop(file);

        let (mut writer, resumed) =
            CheckpointWriter::open(&path, &identity, config.pairs).expect("resume checkpoint");
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].index, 2);
        assert_eq!(resumed[0].candidate_white.outcome, GameOutcome::Win);
        assert_eq!(resumed[0].candidate_black.outcome, GameOutcome::Draw);
        assert_eq!(resumed[0].candidate_white.moves, ["e2e4", "e7e5"]);

        let second = PairRecord {
            index: 3,
            candidate_white: draw_record(
                true,
                Termination::AdjudicatedDraw,
                80,
                3_072,
                Duration::from_millis(11),
            ),
            candidate_black: finish(
                false,
                Color::Black,
                Termination::AdjudicatedWin,
                52,
                4_096,
                Duration::from_millis(13),
            ),
        };
        writer.append(&second).expect("checkpoint second pair");
        drop(writer);

        let (writer, resumed) =
            CheckpointWriter::open(&path, &identity, config.pairs).expect("reopen checkpoint");
        drop(writer);
        assert_eq!(
            resumed.iter().map(|pair| pair.index).collect::<Vec<_>>(),
            [2, 3]
        );

        std::fs::remove_file(&path).expect("remove checkpoint fixture");
    }
}
