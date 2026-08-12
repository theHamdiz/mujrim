//! CuteChess-inspired tournament setup state (Mujrim theme).

use mujrim_benchmarker::strength::{MatchClock, MatchConfig};
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
    /// Always 1 — one full board like Engine vs Engine / CuteChess.
    pub concurrency: u32,
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
            hash_mb: 64,
            engine_threads: 1,
            max_plies: 400,
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
        if self.concurrency != 1 {
            return Err("Tournament games run one at a time on a full board.".to_owned());
        }
        if self.games_per_encounter == 0 {
            return Err("Games per encounter must be at least 1.".to_owned());
        }
        Ok(())
    }

    pub fn to_match_config(&self) -> MatchConfig {
        let clock = self.time_control.match_clock();
        let read_timeout = clock.initial + clock.bonus + Duration::from_secs(90);
        MatchConfig {
            pairs: self.games_per_encounter.max(1) as usize,
            concurrency: 1,
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
    fn validate_requires_two_engines_and_single_board() {
        let mut setup = TournamentSetup::default();
        assert!(setup.validate().is_err());
        setup.selected_engine_paths = vec![PathBuf::from("a.exe"), PathBuf::from("b.exe")];
        assert!(setup.validate().is_ok());
        setup.concurrency = 2;
        assert!(setup.validate().is_err());
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

        let three = TimeControlPreset::ThreePlusTwo.match_clock();
        assert_eq!(three.initial, Duration::from_secs(3 * 60));
        assert_eq!(three.increment, Duration::from_secs(2));
        assert_eq!(three.bonus, Duration::from_secs(3 * 60));
    }
}
