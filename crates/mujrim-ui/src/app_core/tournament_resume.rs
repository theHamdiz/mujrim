//! Durable resume checkpoint for a tournament interrupted by UI close.

use std::path::PathBuf;

use mujrim_study::tournament::TournamentFormat;
use mujrim_study::tournament_store::{self, StoredTournament};

use super::settings::AppSettings;
use super::tournament_live::LiveTournamentSnapshot;
use super::tournament_setup::TournamentSetup;

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
        let contents = std::fs::read_to_string(Self::path()).ok()?;
        toml::from_str(&contents).ok()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(encoded) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, encoded);
        }
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
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

    pub fn parsed_format(&self) -> TournamentFormat {
        match self.format.as_str() {
            "double_round_robin" => TournamentFormat::DoubleRoundRobin,
            "swiss" => TournamentFormat::Swiss,
            "knockout" => TournamentFormat::Knockout,
            _ => TournamentFormat::RoundRobin,
        }
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
        });
        let checkpoint = ActiveTournamentCheckpoint::from_live("t-1".into(), &setup, &snap);
        assert_eq!(checkpoint.white, "Alpha");
        assert_eq!(checkpoint.black, "Beta");
        assert_eq!(checkpoint.moves, ["e2e4"]);
        assert!(checkpoint.paused);
        assert_eq!(checkpoint.parsed_format(), TournamentFormat::RoundRobin);
    }
}
