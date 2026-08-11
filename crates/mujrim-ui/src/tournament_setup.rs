//! CuteChess-inspired tournament setup state (Mujrim theme).

use mujrim_benchmarker::strength::MatchConfig;
use mujrim_study::tournament::TournamentFormat;
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
    pub concurrency: u32,
    /// Reserved for paired color-swap UI (always on in runner today).
    #[allow(dead_code)]
    pub swap_sides: bool,
    pub time_mode: TimeMode,
    pub nodes_per_move: u32,
    pub move_time_ms: u32,
    pub max_depth: i32,
    pub hash_mb: u32,
    pub engine_threads: u32,
    pub max_plies: u32,
    pub selected_engine_paths: Vec<PathBuf>,
    /// Optional PGN export path after the event finishes (wired in a follow-up).
    #[allow(dead_code)]
    pub pgn_output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeMode {
    Nodes,
    MoveTime,
    Depth,
}

impl TimeMode {
    pub const ALL: [Self; 3] = [Self::Nodes, Self::MoveTime, Self::Depth];
}

impl std::fmt::Display for TimeMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nodes => write!(formatter, "Nodes per move"),
            Self::MoveTime => write!(formatter, "Move time"),
            Self::Depth => write!(formatter, "Fixed depth"),
        }
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
            time_mode: TimeMode::Nodes,
            nodes_per_move: 2_000,
            move_time_ms: 100,
            max_depth: 12,
            hash_mb: 16,
            engine_threads: 1,
            max_plies: 160,
            selected_engine_paths: Vec::new(),
            pgn_output: String::new(),
        }
    }
}

impl TournamentSetup {
    pub fn validate(&self) -> Result<(), String> {
        if self.selected_engine_paths.len() < 2 {
            return Err("Select at least two host-native engines.".to_owned());
        }
        if !(1..=4).contains(&self.concurrency) {
            return Err("Concurrency must be between 1 and 4.".to_owned());
        }
        if self.games_per_encounter == 0 {
            return Err("Games per encounter must be at least 1.".to_owned());
        }
        Ok(())
    }

    pub fn to_match_config(&self) -> MatchConfig {
        let (nodes_per_move, move_time, max_depth) = match self.time_mode {
            TimeMode::Nodes => (self.nodes_per_move.max(1) as u64, None, 128),
            TimeMode::MoveTime => (
                0,
                Some(Duration::from_millis(self.move_time_ms.max(1) as u64)),
                128,
            ),
            TimeMode::Depth => (0, None, self.max_depth.max(1)),
        };
        MatchConfig {
            pairs: self.games_per_encounter.max(1) as usize,
            concurrency: self.concurrency.clamp(1, 4) as usize,
            hash_mb: self.hash_mb.max(1) as usize,
            engine_threads: self.engine_threads.max(1) as usize,
            max_engine_memory_mb: 384,
            max_match_memory_mb: 768,
            session_pairs: 1,
            max_plies: self.max_plies.max(1) as usize,
            early_stop: false,
            checkpoint_path: None,
            stop_flag: None,
            game_progress: None,
            nodes_per_move,
            move_time,
            max_depth,
            ..MatchConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_two_engines_and_sane_concurrency() {
        let mut setup = TournamentSetup::default();
        assert!(setup.validate().is_err());
        setup.selected_engine_paths = vec![PathBuf::from("a.exe"), PathBuf::from("b.exe")];
        assert!(setup.validate().is_ok());
        setup.concurrency = 8;
        assert!(setup.validate().is_err());
    }

    #[test]
    fn time_mode_maps_into_match_config() {
        let mut setup = TournamentSetup {
            selected_engine_paths: vec![PathBuf::from("a"), PathBuf::from("b")],
            time_mode: TimeMode::MoveTime,
            move_time_ms: 250,
            ..TournamentSetup::default()
        };
        let config = setup.to_match_config();
        assert_eq!(config.move_time, Some(Duration::from_millis(250)));
        assert_eq!(config.nodes_per_move, 0);
        setup.time_mode = TimeMode::Depth;
        setup.max_depth = 9;
        let depth = setup.to_match_config();
        assert_eq!(depth.max_depth, 9);
    }
}
