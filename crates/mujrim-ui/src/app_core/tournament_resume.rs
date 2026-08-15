//! Durable resume checkpoint for a tournament interrupted by UI close.

use std::path::PathBuf;

use mujrim_study::durable;
use mujrim_study::tournament::TournamentFormat;
use mujrim_study::tournament_store::{self, StoredTournament};

use super::settings::AppSettings;
use super::tournament_live::LiveTournamentSnapshot;
use super::tournament_setup::{TimeControlPreset, TournamentSetup};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ActiveTournamentCheckpoint {
    pub id: String,
    pub event: String,
    pub site: String,
    pub format: String,
    pub selected_engine_paths: Vec<PathBuf>,
    pub paused: bool,
    pub white: String,
    pub black: String,
    pub initial_fen: String,
    pub moves: Vec<String>,
    pub games_per_encounter: u32,
    pub concurrency: u32,
    pub hash_mb: u32,
    pub engine_threads: u32,
    pub max_plies: u32,
    pub time_control: String,
    pub played_games: u32,
    pub planned_games: u32,
}

impl Default for ActiveTournamentCheckpoint {
    fn default() -> Self {
        Self {
            id: String::new(),
            event: "Mujrim Tournament".to_owned(),
            site: String::new(),
            format: "round_robin".to_owned(),
            selected_engine_paths: Vec::new(),
            paused: false,
            white: String::new(),
            black: String::new(),
            initial_fen: mujrim_study::opening::START_FEN.to_owned(),
            moves: Vec::new(),
            games_per_encounter: 0,
            concurrency: 0,
            hash_mb: 0,
            engine_threads: 0,
            max_plies: 0,
            time_control: String::new(),
            played_games: 0,
            planned_games: 0,
        }
    }
}

impl ActiveTournamentCheckpoint {
    pub fn path() -> PathBuf {
        let mut path = AppSettings::config_path();
        path.set_file_name("active-tournament.toml");
        path
    }

    pub fn load() -> Option<Self> {
        let contents = durable::read_text(&Self::path())?;
        toml::from_str(&contents).ok()
    }

    pub fn save(&self) {
        let checkpoint = Self::load()
            .map(|existing| existing.merge_richer(self))
            .unwrap_or_else(|| self.clone());
        if let Ok(encoded) = toml::to_string_pretty(&checkpoint) {
            let _ = durable::atomic_write_text(&Self::path(), &encoded);
        }
    }

    fn merge_richer(self, incoming: &Self) -> Self {
        if !self.id.is_empty() && self.id != incoming.id {
            return incoming.clone();
        }
        let mut merged = incoming.clone();
        if self.selected_engine_paths.len() > merged.selected_engine_paths.len() {
            merged.selected_engine_paths = self.selected_engine_paths;
        }
        if self.concurrency > merged.concurrency {
            merged.concurrency = self.concurrency;
        }
        if self.games_per_encounter > merged.games_per_encounter {
            merged.games_per_encounter = self.games_per_encounter;
        }
        if self.played_games > merged.played_games {
            merged.played_games = self.played_games;
        }
        if self.planned_games > merged.planned_games {
            merged.planned_games = self.planned_games;
        }
        if merged.id.is_empty() {
            merged.id = self.id;
        }
        merged
    }

    pub fn clear() {
        durable::remove_file(&Self::path());
    }

    pub fn from_live(id: String, setup: &TournamentSetup, snap: &LiveTournamentSnapshot) -> Self {
        let live = snap.live_games.last();
        Self {
            id,
            event: setup.event.clone(),
            site: setup.site.clone(),
            format: format_key(setup.format).to_owned(),
            selected_engine_paths: setup.selected_engine_paths.clone(),
            paused: snap.paused || snap.running,
            games_per_encounter: setup.games_per_encounter,
            concurrency: setup.concurrency,
            hash_mb: setup.hash_mb,
            engine_threads: setup.engine_threads,
            max_plies: setup.max_plies,
            time_control: time_control_key(setup.time_control).to_owned(),
            played_games: snap.played_games.len() as u32,
            planned_games: snap.planned_games(setup.games_per_encounter) as u32,
            white: live
                .map(|game| game.white.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| snap.current_white.clone()),
            black: live
                .map(|game| game.black.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| snap.current_black.clone()),
            initial_fen: live
                .map(|game| game.initial_fen.clone())
                .filter(|fen| !fen.is_empty())
                .unwrap_or_else(|| mujrim_study::opening::START_FEN.to_owned()),
            moves: live.map(|game| game.moves.clone()).unwrap_or_default(),
        }
    }

    pub fn from_stored(tournament: &StoredTournament, setup: &TournamentSetup) -> Self {
        let last = tournament.games.last();
        Self {
            id: tournament.id.clone(),
            event: tournament.name.clone(),
            site: setup.site.clone(),
            format: format_key(tournament.format).to_owned(),
            selected_engine_paths: setup.selected_engine_paths.clone(),
            paused: tournament_store::is_resumable_status(&tournament.status),
            games_per_encounter: setup.games_per_encounter,
            concurrency: setup.concurrency,
            hash_mb: setup.hash_mb,
            engine_threads: setup.engine_threads,
            max_plies: setup.max_plies,
            time_control: time_control_key(setup.time_control).to_owned(),
            played_games: tournament.games.len() as u32,
            planned_games: crate::app_core::logic::planned_tournament_games_with(
                tournament,
                setup.games_per_encounter,
            ) as u32,
            white: last
                .map(|game| game.white.clone())
                .or_else(|| {
                    tournament
                        .entrants
                        .first()
                        .map(|entrant| entrant.name.clone())
                })
                .unwrap_or_default(),
            black: last
                .map(|game| game.black.clone())
                .or_else(|| {
                    tournament
                        .entrants
                        .get(1)
                        .map(|entrant| entrant.name.clone())
                })
                .unwrap_or_default(),
            initial_fen: last
                .map(|game| game.initial_fen.clone())
                .unwrap_or_else(|| mujrim_study::opening::START_FEN.to_owned()),
            moves: last.map(|game| game.moves.clone()).unwrap_or_default(),
        }
    }

    pub fn apply_setup(&self, setup: &mut TournamentSetup) {
        setup.event = self.event.clone();
        setup.site = self.site.clone();
        setup.format = self.parsed_format();
        if !self.selected_engine_paths.is_empty() {
            setup.selected_engine_paths = self.selected_engine_paths.clone();
        }
        if self.games_per_encounter > 0 {
            setup.games_per_encounter = self.games_per_encounter;
        }
        if self.concurrency > 0 {
            setup.concurrency = self.concurrency;
        }
        if self.hash_mb > 0 {
            setup.hash_mb = self.hash_mb;
        }
        if self.engine_threads > 0 {
            setup.engine_threads = self.engine_threads;
        }
        if self.max_plies > 0 {
            setup.max_plies = self.max_plies;
        }
        if let Some(time_control) = parse_time_control(&self.time_control) {
            setup.time_control = time_control;
        }
        setup.sanitize_for_gui();
    }

    pub fn parsed_format(&self) -> TournamentFormat {
        match self.format.as_str() {
            "double_round_robin" => TournamentFormat::DoubleRoundRobin,
            "swiss" => TournamentFormat::Swiss,
            "knockout" => TournamentFormat::Knockout,
            _ => TournamentFormat::RoundRobin,
        }
    }
}

fn time_control_key(preset: TimeControlPreset) -> &'static str {
    match preset {
        TimeControlPreset::ThreePlusTwo => "three_plus_two",
        TimeControlPreset::FivePlusThree => "five_plus_three",
    }
}

fn parse_time_control(value: &str) -> Option<TimeControlPreset> {
    match value {
        "three_plus_two" => Some(TimeControlPreset::ThreePlusTwo),
        "five_plus_three" => Some(TimeControlPreset::FivePlusThree),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::tournament_live::LiveGameBoard;

    #[test]
    fn checkpoint_captures_first_pairing_and_live_moves() {
        let setup = TournamentSetup {
            event: "Night Swiss".into(),
            selected_engine_paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            ..TournamentSetup::default()
        };
        let mut snap = LiveTournamentSnapshot {
            paused: true,
            current_white: "Alpha".into(),
            current_black: "Beta".into(),
            ..LiveTournamentSnapshot::default()
        };
        snap.upsert_live_game(LiveGameBoard {
            game_key: "g1".into(),
            match_index: 1,
            round: 1,
            white: "Alpha".into(),
            black: "Beta".into(),
            initial_fen: mujrim_study::opening::START_FEN.into(),
            moves: vec!["e2e4".into()],
            last_uci: "e2e4".into(),
            score_cp: 20,
            depth: 8,
            nodes: 100,
            white_clock_ms: Some(180_000),
            black_clock_ms: Some(180_000),
            ..LiveGameBoard::default()
        });
        let checkpoint = ActiveTournamentCheckpoint::from_live("t-1".into(), &setup, &snap);
        assert_eq!(checkpoint.white, "Alpha");
        assert_eq!(checkpoint.black, "Beta");
        assert_eq!(checkpoint.moves, ["e2e4"]);
        assert!(checkpoint.paused);
        assert_eq!(checkpoint.parsed_format(), TournamentFormat::RoundRobin);
    }

    #[test]
    fn apply_setup_restores_clock_and_games_per_encounter() {
        let checkpoint = ActiveTournamentCheckpoint {
            event: "V2".into(),
            format: "double_round_robin".into(),
            games_per_encounter: 2,
            concurrency: 4,
            hash_mb: 128,
            engine_threads: 1,
            max_plies: 400,
            time_control: "five_plus_three".into(),
            selected_engine_paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            ..ActiveTournamentCheckpoint::default()
        };
        let mut setup = TournamentSetup::default();
        checkpoint.apply_setup(&mut setup);
        assert_eq!(setup.event, "V2");
        assert_eq!(setup.format, TournamentFormat::DoubleRoundRobin);
        assert_eq!(setup.games_per_encounter, 2);
        assert_eq!(setup.time_control, TimeControlPreset::FivePlusThree);
        assert_eq!(setup.hash_mb, 128);
        assert_eq!(setup.concurrency, 4);
    }

    #[test]
    fn merge_richer_keeps_the_larger_roster_and_simul() {
        let rich = ActiveTournamentCheckpoint {
            id: "t-1".into(),
            selected_engine_paths: vec![
                PathBuf::from("/tmp/a"),
                PathBuf::from("/tmp/b"),
                PathBuf::from("/tmp/c"),
            ],
            concurrency: 15,
            games_per_encounter: 2,
            ..ActiveTournamentCheckpoint::default()
        };
        let thin = ActiveTournamentCheckpoint {
            id: "t-1".into(),
            selected_engine_paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            concurrency: 1,
            games_per_encounter: 1,
            paused: true,
            ..ActiveTournamentCheckpoint::default()
        };
        let merged = rich.clone().merge_richer(&thin);
        assert_eq!(merged.selected_engine_paths.len(), 3);
        assert_eq!(merged.concurrency, 15);
        assert_eq!(merged.games_per_encounter, 2);
        assert!(merged.paused);
        let richer_counts = ActiveTournamentCheckpoint {
            played_games: 1278,
            planned_games: 1368,
            ..rich
        };
        let merged = richer_counts.merge_richer(&thin);
        assert_eq!(merged.played_games, 1278);
        assert_eq!(merged.planned_games, 1368);
    }
}
