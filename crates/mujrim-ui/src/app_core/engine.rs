//! Engine player configuration and built-in search.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mujrim_protocols::SearchInfo;
use mujrim_protocols::catalog::{DiscoveredEngine, RuntimeCompatibility};

use super::uci_process::ExternalEngineProtocol;

pub const MAX_GUI_HASH_MB: i32 = 512;

pub fn bounded_hash_mb(value: i32) -> usize {
    value.clamp(1, MAX_GUI_HASH_MB) as usize
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub hash_mb: i32,
    pub threads: i32,
    pub max_depth: i32,
    pub time_per_move: i32,
    pub ponder: bool,
    pub use_book: bool,
    pub use_nnue: bool,
    pub eval_file: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            hash_mb: 64,
            threads: 1,
            max_depth: 64,
            time_per_move: 3,
            ponder: false,
            use_book: true,
            use_nnue: true,
            eval_file: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    HumanVsHuman,
    HumanVsEngine,
    EngineVsEngine,
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HumanVsHuman => write!(f, "Human vs Human"),
            Self::HumanVsEngine => write!(f, "Human vs Engine"),
            Self::EngineVsEngine => write!(f, "Engine vs Engine"),
        }
    }
}

impl GameMode {
    pub const ALL: [Self; 3] = [
        Self::HumanVsHuman,
        Self::HumanVsEngine,
        Self::EngineVsEngine,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerConfig {
    Human,
    BuiltIn {
        depth: i32,
    },
    External {
        path: String,
        protocol: ExternalEngineProtocol,
    },
}

impl std::fmt::Display for PlayerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "Human"),
            Self::BuiltIn { depth } => write!(f, "Mujrim (depth {depth})"),
            Self::External { path, protocol } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                write!(f, "{protocol}: {name}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledEngineChoice {
    pub index: usize,
    pub label: String,
}

impl std::fmt::Display for BundledEngineChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

pub fn bundled_engine_label(engine: &DiscoveredEngine) -> String {
    let execution = match engine.compatibility {
        RuntimeCompatibility::Native => "native",
        RuntimeCompatibility::Emulated => "x64 emulation",
    };
    format!("{} ({execution})", engine.display_name)
}

pub fn bundled_engine_choices(engines: &[DiscoveredEngine]) -> Vec<BundledEngineChoice> {
    engines
        .iter()
        .enumerate()
        .map(|(index, engine)| BundledEngineChoice {
            index,
            label: bundled_engine_label(engine),
        })
        .collect()
}

pub fn default_engine_player(bundled: &[DiscoveredEngine]) -> PlayerConfig {
    bundled
        .first()
        .map_or(PlayerConfig::BuiltIn { depth: 16 }, |engine| {
            PlayerConfig::External {
                path: engine.path.to_string_lossy().into_owned(),
                protocol: ExternalEngineProtocol::Uci,
            }
        })
}

pub fn players_for_mode(
    mode: GameMode,
    bundled: &[DiscoveredEngine],
) -> (PlayerConfig, PlayerConfig) {
    match mode {
        GameMode::HumanVsHuman => (PlayerConfig::Human, PlayerConfig::Human),
        GameMode::HumanVsEngine => (PlayerConfig::Human, default_engine_player(bundled)),
        GameMode::EngineVsEngine => {
            let white = default_engine_player(bundled);
            let black = bundled.get(1).map_or_else(
                || PlayerConfig::BuiltIn { depth: 12 },
                |engine| PlayerConfig::External {
                    path: engine.path.to_string_lossy().into_owned(),
                    protocol: ExternalEngineProtocol::Uci,
                },
            );
            (white, black)
        }
    }
}

pub fn selected_bundled_engine(
    engines: &[DiscoveredEngine],
    player: &PlayerConfig,
) -> Option<BundledEngineChoice> {
    let PlayerConfig::External { path, .. } = player else {
        return None;
    };
    engines
        .iter()
        .position(|engine| engine.path == std::path::Path::new(path))
        .map(|index| BundledEngineChoice {
            index,
            label: bundled_engine_label(&engines[index]),
        })
}

#[derive(Clone, Debug)]
pub struct QuickTournamentEngine {
    pub name: String,
    pub path: PathBuf,
    pub search_limits: mujrim_protocols::catalog::SearchLimitSupport,
}

/// Live UCI/search telemetry pushed into the UI without rebuilding views.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    pub depth: i32,
    pub seldepth: i32,
    pub score_cp: i32,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub hashfull: u16,
    pub tablebase_hits: u64,
    pub current_move: Option<String>,
    pub pv: Vec<String>,
    pub best_move: String,
    pub ponder_move: Option<String>,
    pub label: String,
}

impl TelemetrySnapshot {
    pub fn from_search_info(info: &SearchInfo, prefix: &str) -> Self {
        let mut snap = Self {
            depth: info.depth,
            seldepth: info.seldepth,
            score_cp: info.score,
            nodes: info.nodes,
            nps: info.nps,
            time_ms: info.time_ms,
            hashfull: info.hashfull,
            tablebase_hits: info.tablebase_hits,
            current_move: info.current_move.clone(),
            pv: info.pv.clone(),
            best_move: info.best_move.clone(),
            ponder_move: info.ponder_move.clone(),
            label: String::new(),
        };
        snap.label = snap.format_label(prefix);
        snap
    }

    pub fn from_label(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn format_label(&self, prefix: &str) -> String {
        let ponder = self
            .ponder_move
            .as_deref()
            .map_or_else(String::new, |mv| format!(" | ponder {mv}"));
        let current = self
            .current_move
            .as_deref()
            .map_or_else(String::new, |mv| format!(" | current {mv}"));
        let pv = if self.pv.is_empty() {
            String::new()
        } else {
            format!(" | pv {}", self.pv.join(" "))
        };
        let head = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix} ")
        };
        format!(
            "{head}depth {}/{} | score {} cp | {} nodes | {} nps | {} ms | hashfull {}/1000 | tbhits {}{ponder}{current}{pv}",
            self.depth,
            self.seldepth,
            self.score_cp,
            self.nodes,
            self.nps,
            self.time_ms,
            self.hashfull,
            self.tablebase_hits,
        )
    }
}

pub fn apply_search_info(target: &mut TelemetrySnapshot, info: &SearchInfo, prefix: &str) {
    *target = TelemetrySnapshot::from_search_info(info, prefix);
}

pub fn builtin_analysis_line(fen: &str, depth: i32) -> Result<(String, i32, Vec<String>), String> {
    types::init();
    let mut board = types::Board::from_fen(fen)?;
    let (mv, _info) = builtin_engine_search(
        &mut board,
        64,
        1,
        true,
        None,
        Duration::from_millis(250),
        depth.max(1),
    )?;
    Ok((mv.to_uci(), 0, vec![mv.to_uci()]))
}

pub fn builtin_engine_search(
    board: &mut types::Board,
    hash_mb: usize,
    threads: usize,
    use_nnue: bool,
    eval_file: Option<&str>,
    time: Duration,
    max_depth: i32,
) -> Result<(types::Move, String), String> {
    struct BuiltinCache {
        hash_mb: usize,
        threads: usize,
        use_nnue: bool,
        eval_file: Option<String>,
        engine: search::SearchEngine,
    }

    static CACHE: OnceLock<Mutex<Option<BuiltinCache>>> = OnceLock::new();
    let token_slot = builtin_stop_slot();
    let mut guard = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let needs_rebuild = guard.as_ref().is_none_or(|cached| {
        cached.hash_mb != hash_mb
            || cached.threads != threads
            || cached.use_nnue != use_nnue
            || cached.eval_file.as_deref() != eval_file
    });

    if needs_rebuild {
        let mut engine = search::SearchEngine::new(hash_mb, threads);
        engine.set_use_nnue(use_nnue);
        if let Some(path) = eval_file {
            let net = eval::nnue::load_network(std::path::Path::new(path))
                .map_err(|err| format!("EvalFile error: {err}"))?;
            engine.set_nnue_network(net);
        }
        *guard = Some(BuiltinCache {
            hash_mb,
            threads,
            use_nnue,
            eval_file: eval_file.map(str::to_owned),
            engine,
        });
    }

    let cached = guard.as_mut().expect("builtin cache just initialized");
    let token = cached.engine.stop_token();
    *token_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&token));
    let result = cached.engine.search_time(board, time, max_depth);
    *token_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    let note = eval_file
        .map(|_| format!(" | net {}", cached.engine.nnue_info().name))
        .unwrap_or_default();
    Ok((
        result.best_move,
        format!(
            "depth {} | score {} cp | {} nodes | {:.0} nps{}",
            result.depth,
            result.score,
            result.nodes,
            result.nodes as f64 / result.elapsed.as_secs_f64().max(0.001),
            note,
        ),
    ))
}

fn builtin_stop_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Interrupt an in-flight built-in search without waiting on the engine mutex.
pub fn stop_builtin_search() {
    if let Some(flag) = builtin_stop_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
    {
        flag.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_defaults_to_embedded_eval() {
        let cfg = EngineConfig::default();
        assert!(cfg.eval_file.is_none());
        assert!(cfg.use_nnue);
    }

    #[test]
    fn engine_hash_is_bounded_for_low_memory_desktops() {
        assert_eq!(bounded_hash_mb(-1), 1);
        assert_eq!(bounded_hash_mb(64), 64);
        assert_eq!(bounded_hash_mb(4096), 512);
    }

    #[test]
    fn builtin_stop_uses_the_search_engine_token() {
        let src = include_str!("engine.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("stop_builtin_search"));
        assert!(production.contains("stop_token"));
        assert!(production.contains("builtin_stop_slot"));
    }

    #[test]
    fn telemetry_reducer_copies_search_info() {
        let info = SearchInfo {
            depth: 12,
            seldepth: 18,
            score: 34,
            nodes: 1000,
            nps: 50_000,
            time_ms: 20,
            hashfull: 120,
            best_move: "e2e4".into(),
            pv: vec!["e2e4".into(), "e7e5".into()],
            ..SearchInfo::default()
        };
        let snap = TelemetrySnapshot::from_search_info(&info, "UCI");
        assert_eq!(snap.depth, 12);
        assert!(snap.label.contains("UCI"));
        assert!(snap.label.contains("e2e4"));
        let mut target = TelemetrySnapshot::default();
        apply_search_info(&mut target, &info, "UCI");
        assert_eq!(target.best_move, "e2e4");
    }

    #[test]
    fn bundled_engine_choices_expose_execution_mode_and_selection() {
        let engines = vec![DiscoveredEngine {
            id: "obsidian",
            display_name: "Obsidian",
            path: PathBuf::from(r"C:\Mujrim\engines\obsidian.exe"),
            target_directory: "windows-x86_64-avx2".to_owned(),
            compatibility: RuntimeCompatibility::Emulated,
            search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
        }];
        let choices = bundled_engine_choices(&engines);
        assert_eq!(choices[0].label, "Obsidian (x64 emulation)");
        let selected = selected_bundled_engine(
            &engines,
            &PlayerConfig::External {
                path: engines[0].path.to_string_lossy().into_owned(),
                protocol: ExternalEngineProtocol::Uci,
            },
        )
        .expect("bundled engine should be selected");
        assert_eq!(selected, choices[0]);
    }

    #[test]
    fn players_for_mode_assigns_human_and_engine_sides() {
        let (white, black) = players_for_mode(GameMode::HumanVsHuman, &[]);
        assert!(matches!(white, PlayerConfig::Human));
        assert!(matches!(black, PlayerConfig::Human));
        let (white, black) = players_for_mode(GameMode::HumanVsEngine, &[]);
        assert!(matches!(white, PlayerConfig::Human));
        assert!(matches!(black, PlayerConfig::BuiltIn { depth: 16 }));
        let (white, black) = players_for_mode(GameMode::EngineVsEngine, &[]);
        assert!(matches!(white, PlayerConfig::BuiltIn { depth: 16 }));
        assert!(matches!(black, PlayerConfig::BuiltIn { depth: 12 }));
    }
}
