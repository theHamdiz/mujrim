//! CuteChess-inspired tournament setup state (Mujrim theme).

use mujrim_benchmarker::hardware::safe_simultaneous_games;
use mujrim_benchmarker::strength::{MatchClock, MatchConfig};
use mujrim_study::tournament::{Pairing, TournamentFormat};
use std::path::PathBuf;
use std::time::Duration;

/// User-editable tournament configuration for the Setup pane.
#[derive(Clone, Debug)]
pub struct TournamentSetup {
    pub event: String,
    pub site: String,
    pub format: TournamentFormat,
    pub swiss_rounds: u32,
    pub games_per_encounter: u32,
    /// Simultaneous games; clamped to host-safe cores.
    pub concurrency: u32,
    pub completed_pairings: Vec<Pairing>,
    /// Reserved for paired color-swap UI (always on in runner today).
    #[allow(dead_code)]
    pub swap_sides: bool,
    pub time_control: TimeControlPreset,
    pub hash_mb: u32,
    pub engine_threads: u32,
    pub max_plies: u32,
    pub selected_engine_paths: Vec<PathBuf>,
    /// Optional PGN export path after the event finishes (wired in a follow-up).
    #[allow(dead_code)]
    pub pgn_output: String,
}

pub const GUI_TOURNAMENT_MAX_HASH_MB: u32 = 256;
pub const GUI_TOURNAMENT_MAX_THREADS: u32 = 2;
pub const GUI_TOURNAMENT_ENGINE_MEMORY_MB: u32 = 1024;
pub const GUI_TOURNAMENT_MATCH_MEMORY_MB: u32 = 2048;

pub fn detected_safe_games() -> u32 {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    safe_simultaneous_games(cores) as u32
}
pub const GUI_TOURNAMENT_DEFAULT_ENGINES: usize = 2;

/// Real-time Fischer clocks with a secondary control after move 40.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeControlPreset {
    /// 3 minutes + 2s/move, then +3 minutes after 40 moves.
    ThreePlusTwo,
    /// 5 minutes + 3s/move, then +5 minutes after 40 moves.
    FivePlusThree,
}

impl TimeControlPreset {
    pub const ALL: [Self; 2] = [Self::ThreePlusTwo, Self::FivePlusThree];

    pub fn match_clock(self) -> MatchClock {
        match self {
            Self::ThreePlusTwo => MatchClock {
                initial: Duration::from_secs(3 * 60),
                increment: Duration::from_secs(2),
                bonus_after_moves: 40,
                bonus: Duration::from_secs(3 * 60),
            },
            Self::FivePlusThree => MatchClock {
                initial: Duration::from_secs(5 * 60),
                increment: Duration::from_secs(3),
                bonus_after_moves: 40,
                bonus: Duration::from_secs(5 * 60),
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ThreePlusTwo => "3+2 (+3 after 40)",
            Self::FivePlusThree => "5+3 (+5 after 40)",
        }
    }
}

impl std::fmt::Display for TimeControlPreset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.label())
    }
}

impl Default for TournamentSetup {
    fn default() -> Self {
        Self {
            event: "Mujrim Tournament".to_owned(),
            site: String::new(),
            format: TournamentFormat::RoundRobin,
            swiss_rounds: 4,
            games_per_encounter: 1,
            concurrency: 1,
            swap_sides: true,
            time_control: TimeControlPreset::ThreePlusTwo,
            hash_mb: 128,
            engine_threads: 1,
            max_plies: 400,
            selected_engine_paths: Vec::new(),
            pgn_output: String::new(),
            completed_pairings: Vec::new(),
        }
    }
}

impl TournamentSetup {
    pub fn validate(&self) -> Result<(), String> {
        if self.selected_engine_paths.len() < 2 {
            return Err("Select at least two host-native engines.".to_owned());
        }
        if self.concurrency == 0 {
            return Err("Concurrency must be at least 1.".to_owned());
        }
        if self.games_per_encounter == 0 {
            return Err("Games per encounter must be at least 1.".to_owned());
        }
        Ok(())
    }

    pub fn sanitize_for_gui(&mut self) {
        let max_games = detected_safe_games();
        self.concurrency = self.concurrency.clamp(1, max_games);
        self.hash_mb = self.hash_mb.clamp(16, GUI_TOURNAMENT_MAX_HASH_MB);
        self.engine_threads = self.engine_threads.clamp(1, GUI_TOURNAMENT_MAX_THREADS);
        self.games_per_encounter = self.games_per_encounter.clamp(1, 4);
        self.max_plies = self.max_plies.clamp(1, 400);
    }

    pub fn to_match_config(&self) -> MatchConfig {
        let mut setup = self.clone();
        setup.sanitize_for_gui();
        let clock = setup.time_control.match_clock();
        let read_timeout = clock.initial + clock.bonus + Duration::from_secs(90);
        let hash_mb = mujrim_benchmarker::strength::bounded_engine_hash_mb(
            setup.hash_mb as usize,
            GUI_TOURNAMENT_ENGINE_MEMORY_MB as usize,
        );
        MatchConfig {
            pairs: setup.games_per_encounter as usize,
            concurrency: setup.concurrency.max(1) as usize,
            hash_mb,
            engine_threads: setup.engine_threads.max(1) as usize,
            max_engine_memory_mb: GUI_TOURNAMENT_ENGINE_MEMORY_MB as usize,
            max_match_memory_mb: GUI_TOURNAMENT_MATCH_MEMORY_MB as usize
                * setup.concurrency.max(1) as usize,
            session_pairs: 1,
            max_plies: setup.max_plies as usize,
            early_stop: false,
            checkpoint_path: None,
            stop_flag: None,
            game_progress: None,
            nodes_per_move: 0,
            move_time: None,
            clock: Some(clock),
            max_depth: 128,
            read_timeout,
            ..MatchConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_two_engines() {
        let mut setup = TournamentSetup::default();
        assert!(setup.validate().is_err());
        setup.selected_engine_paths = vec![PathBuf::from("a.exe"), PathBuf::from("b.exe")];
        assert!(setup.validate().is_ok());
        setup.concurrency = 0;
        assert!(setup.validate().is_err());
        setup.concurrency = 2;
        assert!(setup.validate().is_ok());
    }

    #[test]
    fn time_control_maps_into_match_clock() {
        let setup = TournamentSetup {
            selected_engine_paths: vec![PathBuf::from("a"), PathBuf::from("b")],
            time_control: TimeControlPreset::FivePlusThree,
            ..TournamentSetup::default()
        };
        let config = setup.to_match_config();
        let clock = config.clock.expect("clock TC");
        assert_eq!(clock.initial, Duration::from_secs(5 * 60));
        assert_eq!(clock.increment, Duration::from_secs(3));
        assert_eq!(clock.bonus_after_moves, 40);
        assert_eq!(clock.bonus, Duration::from_secs(5 * 60));
        assert!(config.move_time.is_none());
        assert_eq!(config.nodes_per_move, 0);
        assert_eq!(config.concurrency, 1);
        assert!(
            config.read_timeout >= Duration::from_secs(6 * 60),
            "handshake/search timeout must outlast NNUE and Lc0 weight load"
        );

        let three = TimeControlPreset::ThreePlusTwo.match_clock();
        assert_eq!(three.initial, Duration::from_secs(3 * 60));
        assert_eq!(three.increment, Duration::from_secs(2));
        assert_eq!(three.bonus, Duration::from_secs(3 * 60));
    }

    #[test]
    fn gui_match_config_never_overcommits_host_memory() {
        let setup = TournamentSetup {
            selected_engine_paths: vec![PathBuf::from("a"), PathBuf::from("b")],
            hash_mb: 512,
            engine_threads: 8,
            concurrency: 1,
            ..TournamentSetup::default()
        };
        let config = setup.to_match_config();
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.engine_threads, GUI_TOURNAMENT_MAX_THREADS as usize);
        assert!(config.hash_mb <= GUI_TOURNAMENT_MAX_HASH_MB as usize);
        assert_eq!(
            config.max_engine_memory_mb,
            GUI_TOURNAMENT_ENGINE_MEMORY_MB as usize
        );
        assert_eq!(
            config.max_match_memory_mb,
            GUI_TOURNAMENT_MATCH_MEMORY_MB as usize
        );
        assert!(
            config
                .concurrency
                .saturating_mul(2)
                .saturating_mul(config.max_engine_memory_mb)
                <= config.max_match_memory_mb
        );
        assert!(config.hash_mb + 128 <= config.max_engine_memory_mb);
    }

    #[test]
    fn sanitize_for_gui_clamps_hash_threads_and_concurrency() {
        let mut setup = TournamentSetup {
            hash_mb: 512,
            engine_threads: 8,
            concurrency: 4,
            games_per_encounter: 99,
            max_plies: 9_000,
            ..TournamentSetup::default()
        };
        setup.sanitize_for_gui();
        assert!(setup.concurrency >= 1);
        assert!(setup.concurrency <= detected_safe_games());
        assert_eq!(setup.engine_threads, GUI_TOURNAMENT_MAX_THREADS);
        assert_eq!(setup.hash_mb, GUI_TOURNAMENT_MAX_HASH_MB);
        assert_eq!(setup.games_per_encounter, 4);
        assert_eq!(setup.max_plies, 400);
    }

    #[test]
    fn match_config_scales_memory_with_concurrency() {
        let setup = TournamentSetup {
            selected_engine_paths: vec![PathBuf::from("a"), PathBuf::from("b")],
            concurrency: detected_safe_games().clamp(1, 2),
            ..TournamentSetup::default()
        };
        let config = setup.to_match_config();
        assert_eq!(
            config.max_match_memory_mb,
            GUI_TOURNAMENT_MATCH_MEMORY_MB as usize * config.concurrency
        );
        assert_eq!(config.engine_threads, 1);
        assert!(config.concurrency >= 1);
        assert!(config.concurrency <= detected_safe_games() as usize);
    }
}
