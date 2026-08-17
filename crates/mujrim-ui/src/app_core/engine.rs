//! Engine player configuration. Search always goes through a dedicated binary.

use std::path::PathBuf;

use mujrim_protocols::SearchInfo;
use mujrim_protocols::catalog::{DiscoveredEngine, RuntimeCompatibility, preferred_bundled_engine};

use super::uci_process::{self, ExternalEngineProtocol};

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

    pub fn encode(self) -> &'static str {
        match self {
            Self::HumanVsHuman => "human_vs_human",
            Self::HumanVsEngine => "human_vs_engine",
            Self::EngineVsEngine => "engine_vs_engine",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "human_vs_human" => Self::HumanVsHuman,
            "engine_vs_engine" => Self::EngineVsEngine,
            _ => Self::HumanVsEngine,
        }
    }
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

impl PlayerConfig {
    pub fn encode(&self) -> String {
        match self {
            Self::Human => "human".to_owned(),
            Self::BuiltIn { depth } => format!("builtin:{depth}"),
            Self::External { path, protocol } => {
                format!("{}:{path}", protocol.key())
            }
        }
    }

    pub fn decode(value: &str) -> Self {
        if value == "human" || value.is_empty() {
            return Self::Human;
        }
        if let Some(depth) = value.strip_prefix("builtin:") {
            return Self::BuiltIn {
                depth: depth.parse().unwrap_or(16),
            };
        }
        if let Some(path) = value.strip_prefix("uci:") {
            return Self::External {
                path: path.to_owned(),
                protocol: ExternalEngineProtocol::Uci,
            };
        }
        if let Some(path) = value.strip_prefix("xboard:") {
            return Self::External {
                path: path.to_owned(),
                protocol: ExternalEngineProtocol::Xboard,
            };
        }
        Self::Human
    }
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

fn external_player(engine: &DiscoveredEngine) -> PlayerConfig {
    PlayerConfig::External {
        path: engine.path.to_string_lossy().into_owned(),
        protocol: ExternalEngineProtocol::Uci,
    }
}

pub fn default_engine_player(bundled: &[DiscoveredEngine]) -> PlayerConfig {
    preferred_bundled_engine(bundled)
        .map(external_player)
        .unwrap_or(PlayerConfig::BuiltIn { depth: 16 })
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
            let default_path = match &white {
                PlayerConfig::External { path, .. } => Some(path.as_str()),
                _ => None,
            };
            let black = bundled
                .iter()
                .find(|engine| {
                    default_path.is_none_or(|path| engine.path != std::path::Path::new(path))
                })
                .map(external_player)
                .unwrap_or(PlayerConfig::BuiltIn { depth: 12 });
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
    pub multipv_lines: Vec<(u32, i32, Vec<String>)>,
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
            multipv_lines: info
                .multipv_lines
                .iter()
                .map(|line| (line.multipv, line.score, line.pv.clone()))
                .collect(),
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

/// Dedicated Mujrim binary from `engines/mujrim/`. Prefers `mujrim-v60`.
pub fn discover_default_engine() -> Option<PathBuf> {
    let engines = mujrim_protocols::catalog::discover_bundled_engines_from_environment().ok()?;
    preferred_bundled_engine(&engines).map(|engine| engine.path.clone())
}

pub fn resolve_engine_launch(
    player: &PlayerConfig,
) -> Result<(String, ExternalEngineProtocol), String> {
    match player {
        PlayerConfig::Human => Err("No engine selected for this side.".to_owned()),
        PlayerConfig::External { path, protocol } => Ok((path.clone(), *protocol)),
        PlayerConfig::BuiltIn { .. } => discover_default_engine()
            .map(|path| {
                (
                    path.to_string_lossy().into_owned(),
                    ExternalEngineProtocol::Uci,
                )
            })
            .ok_or_else(|| {
                "Mujrim engine binary not found. Place mujrim-v60 under engines/mujrim/ relative to the UI."
                    .to_owned()
            }),
    }
}

/// Cancel an in-flight GUI engine search (dedicated binary via UCI/XBoard).
pub fn stop_builtin_search() {
    uci_process::cancel_all_pondering();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_and_mode_encode_round_trip() {
        assert_eq!(
            PlayerConfig::decode(&PlayerConfig::Human.encode()),
            PlayerConfig::Human
        );
        assert_eq!(
            PlayerConfig::decode(&PlayerConfig::BuiltIn { depth: 18 }.encode()),
            PlayerConfig::BuiltIn { depth: 18 }
        );
        let external = PlayerConfig::External {
            path: "/opt/lc0".to_owned(),
            protocol: ExternalEngineProtocol::Xboard,
        };
        assert_eq!(PlayerConfig::decode(&external.encode()), external);
        assert_eq!(
            GameMode::decode(GameMode::EngineVsEngine.encode()),
            GameMode::EngineVsEngine
        );
    }

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
    fn builtin_play_resolves_a_dedicated_engine_binary() {
        let src = include_str!("engine.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("discover_default_engine"));
        assert!(production.contains("resolve_engine_launch"));
        assert!(!production.contains("search::SearchEngine"));
        assert!(production.contains("stop_builtin_search"));
        assert!(production.contains("cancel_all_pondering"));
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

    #[test]
    fn default_engine_player_prefers_v60_from_detected_variants() {
        let elite = DiscoveredEngine {
            id: "mujrim-elite",
            display_name: "Mujrim Elite",
            path: PathBuf::from("/opt/mujrim/engines/mujrim/mujrim-elite"),
            target_directory: "engines/mujrim".to_owned(),
            compatibility: RuntimeCompatibility::Native,
            search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
        };
        let v60 = DiscoveredEngine {
            id: "mujrim-v60",
            display_name: "Mujrim v60",
            path: PathBuf::from("/opt/mujrim/engines/mujrim/mujrim-v60"),
            target_directory: "engines/mujrim".to_owned(),
            compatibility: RuntimeCompatibility::Native,
            search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
        };
        let (white, black) =
            players_for_mode(GameMode::EngineVsEngine, &[elite.clone(), v60.clone()]);
        assert!(matches!(
            white,
            PlayerConfig::External { ref path, .. } if path.ends_with("mujrim-v60")
        ));
        assert!(matches!(
            black,
            PlayerConfig::External { ref path, .. } if path.ends_with("mujrim-elite")
        ));
        assert!(matches!(
            default_engine_player(&[elite, v60]),
            PlayerConfig::External { ref path, .. } if path.ends_with("mujrim-v60")
        ));
        let production = include_str!("engine.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("source");
        assert!(production.contains("preferred_bundled_engine"));
        assert!(production.contains("engines/mujrim"));
        assert!(!production.contains("discover_mujrim_cli_from_environment"));
    }
}
