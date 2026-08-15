//! Live tournament progress snapshot shared between the worker and UI.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use mujrim_benchmarker::strength::TournamentGameSnapshot;
use mujrim_study::tournament::{Standing, TournamentFormat, TournamentResult};

#[derive(Clone, Debug)]
pub struct FinishedMatchRow {
    pub index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub white_points: f64,
    pub black_points: f64,
    pub error: Option<String>,
}

impl FinishedMatchRow {
    pub fn label(&self) -> String {
        let score = score_label(self.white_points, self.black_points);
        let detail = self
            .error
            .as_deref()
            .map(|error| format!(" · {error}"))
            .unwrap_or_default();
        format!(
            "#{}/R{} · {} {} {}{detail}",
            self.index, self.round, self.white, score, self.black
        )
    }
}

#[derive(Clone, Debug)]
pub struct StandingRow {
    pub rank: usize,
    pub name: String,
    #[allow(dead_code)]
    pub played: usize,
    pub wins: usize,
    pub draws: usize,
    pub losses: usize,
    pub points: f64,
    pub performance: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodiumTier {
    Gold,
    Silver,
    Bronze,
}

impl PodiumTier {
    pub const fn from_rank(rank: usize) -> Option<Self> {
        match rank {
            1 => Some(Self::Gold),
            2 => Some(Self::Silver),
            3 => Some(Self::Bronze),
            _ => None,
        }
    }

    pub const fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Gold => (212, 175, 55),
            Self::Silver => (176, 176, 186),
            Self::Bronze => (184, 115, 51),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Gold => "Gold",
            Self::Silver => "Silver",
            Self::Bronze => "Bronze",
        }
    }
}

impl StandingRow {
    pub fn elo_label(&self) -> String {
        match self.performance {
            Some(elo) => format!("{elo:.0}"),
            None => "—".to_owned(),
        }
    }

    pub const fn podium(&self) -> Option<PodiumTier> {
        PodiumTier::from_rank(self.rank)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkingLine {
    pub multipv: u32,
    pub score_cp: i32,
    pub pv: Vec<String>,
}

/// In-progress game board for the live arena.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveGameBoard {
    pub game_key: String,
    pub match_index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub initial_fen: String,
    pub moves: Vec<String>,
    pub last_uci: String,
    pub score_cp: i32,
    pub depth: i32,
    pub nodes: u64,
    pub white_clock_ms: Option<u64>,
    pub black_clock_ms: Option<u64>,
    pub clock_synced_ms: Option<u64>,
    pub pv: Vec<String>,
    pub multipv_lines: Vec<ThinkingLine>,
}

impl LiveGameBoard {
    pub fn is_placeholder(&self) -> bool {
        self.game_key.starts_with("pending-")
    }
}

/// Replayable tournament game for the hub board viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayedGame {
    pub id: usize,
    pub match_index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub white_score: f64,
    pub initial_fen: String,
    pub moves: Vec<String>,
}

impl PlayedGame {
    pub fn from_snapshot(id: usize, snapshot: TournamentGameSnapshot) -> Self {
        Self {
            id,
            match_index: snapshot.match_index,
            round: snapshot.round,
            white: snapshot.white,
            black: snapshot.black,
            white_score: snapshot.white_score,
            initial_fen: snapshot.initial_fen,
            moves: snapshot.moves,
        }
    }

    pub fn result_label(&self) -> &'static str {
        result_label(self.white_score)
    }

    pub fn title(&self) -> String {
        format!(
            "R{} · {} {} {}",
            self.round,
            self.white,
            self.result_label(),
            self.black
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct LiveTournamentSnapshot {
    pub running: bool,
    pub format_label: String,
    pub engine_names: Vec<String>,
    pub total_matches: usize,
    pub completed_matches: usize,
    pub current_white: String,
    pub current_black: String,
    pub current_round: usize,
    pub finished_matches: Vec<FinishedMatchRow>,
    pub standings: Vec<StandingRow>,
    pub game_results: Vec<TournamentResult>,
    pub played_games: Vec<PlayedGame>,
    pub live_games: Vec<LiveGameBoard>,
    pub cancelled: bool,
    pub finished: bool,
    pub paused: bool,
    pub status_line: String,
    pub error: Option<String>,
    pub show_results_panel: bool,
}

/// Each encounter plays `games_per_encounter` color-swapped pairs (two games each).
pub const GAMES_PER_ENCOUNTER_PAIR: usize = 2;

pub fn games_per_match(games_per_encounter: u32) -> usize {
    (games_per_encounter.max(1) as usize).saturating_mul(GAMES_PER_ENCOUNTER_PAIR)
}

impl LiveTournamentSnapshot {
    pub fn progress_fraction(&self) -> f32 {
        if self.total_matches == 0 {
            return if self.finished { 1.0 } else { 0.0 };
        }
        (self.completed_matches as f32 / self.total_matches as f32).clamp(0.0, 1.0)
    }

    pub fn planned_games(&self, games_per_encounter: u32) -> usize {
        self.total_matches
            .saturating_mul(games_per_match(games_per_encounter))
    }

    pub fn remaining_matches(&self) -> usize {
        self.total_matches.saturating_sub(self.completed_matches)
    }

    pub fn remaining_games(&self, games_per_encounter: u32) -> usize {
        if self.finished && !self.cancelled {
            return 0;
        }
        self.planned_games(games_per_encounter)
            .saturating_sub(self.played_games.len())
    }

    pub fn game_progress_fraction(&self, games_per_encounter: u32) -> f32 {
        let total = self.planned_games(games_per_encounter);
        if total == 0 {
            return self.progress_fraction();
        }
        (self.played_games.len() as f32 / total as f32).clamp(0.0, 1.0)
    }

    pub fn remaining_games_label(&self, games_per_encounter: u32) -> String {
        let remaining = self.remaining_games(games_per_encounter);
        let planned = self.planned_games(games_per_encounter);
        if planned == 0 {
            return "No games scheduled".to_owned();
        }
        match remaining {
            0 => "0 games remaining".to_owned(),
            1 => "1 game remaining".to_owned(),
            n => format!("{n} games remaining"),
        }
    }

    pub fn phase_label(&self) -> &'static str {
        if self.cancelled {
            "Stopped"
        } else if self.finished {
            "Finished"
        } else if self.paused {
            "Paused"
        } else if self.running {
            "Live"
        } else {
            "Ready"
        }
    }

    pub fn current_match_label(&self) -> String {
        if !self.current_white.is_empty() {
            format!(
                "Round {} · {} vs {}",
                self.current_round.max(1),
                self.current_white,
                self.current_black
            )
        } else if self.finished {
            "No active pairing".to_owned()
        } else {
            "Waiting for first pairing…".to_owned()
        }
    }

    pub fn append_games(&mut self, games: Vec<TournamentGameSnapshot>) {
        for game in games {
            let id = self.played_games.len();
            self.played_games.push(PlayedGame::from_snapshot(id, game));
        }
    }

    pub fn game(&self, id: usize) -> Option<&PlayedGame> {
        self.played_games.get(id)
    }

    pub fn latest_game_id(&self) -> Option<usize> {
        self.played_games.last().map(|game| game.id)
    }

    pub fn upsert_live_game(&mut self, board: LiveGameBoard) {
        if !board.is_placeholder() {
            self.drop_placeholder_games();
        }
        if let Some(existing) = self
            .live_games
            .iter_mut()
            .find(|game| game.game_key == board.game_key)
        {
            *existing = board;
        } else {
            self.live_games.push(board);
        }
    }

    pub fn drop_placeholder_games(&mut self) {
        self.live_games.retain(|game| !game.is_placeholder());
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_ply(
        &mut self,
        game_key: &str,
        ply: usize,
        uci: String,
        score_cp: i32,
        depth: i32,
        nodes: u64,
        moves: Vec<String>,
        white_clock_ms: Option<u64>,
        black_clock_ms: Option<u64>,
    ) {
        if let Some(game) = self
            .live_games
            .iter_mut()
            .find(|game| game.game_key == game_key)
        {
            game.moves = moves;
            game.last_uci = uci;
            game.score_cp = score_cp;
            game.depth = depth;
            game.nodes = nodes;
            if white_clock_ms.is_some() {
                game.white_clock_ms = white_clock_ms;
            }
            if black_clock_ms.is_some() {
                game.black_clock_ms = black_clock_ms;
            }
            if white_clock_ms.is_some() || black_clock_ms.is_some() {
                game.clock_synced_ms = Some(now_unix_ms());
            }
            let _ = ply;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_thinking(
        &mut self,
        game_key: &str,
        score_cp: i32,
        depth: i32,
        nodes: u64,
        pv: Vec<String>,
        multipv_lines: Vec<ThinkingLine>,
        white_clock_ms: Option<u64>,
        black_clock_ms: Option<u64>,
    ) {
        if let Some(game) = self
            .live_games
            .iter_mut()
            .find(|game| game.game_key == game_key)
        {
            game.score_cp = score_cp;
            game.depth = depth;
            game.nodes = nodes;
            game.pv = pv;
            game.multipv_lines = multipv_lines;
            if white_clock_ms.is_some() {
                game.white_clock_ms = white_clock_ms;
            }
            if black_clock_ms.is_some() {
                game.black_clock_ms = black_clock_ms;
            }
            if white_clock_ms.is_some() || black_clock_ms.is_some() {
                game.clock_synced_ms = Some(now_unix_ms());
            }
        }
    }

    pub fn finish_live_game(&mut self, game_key: &str, white_score: f64, moves: Vec<String>) {
        if let Some(index) = self
            .live_games
            .iter()
            .position(|game| game.game_key == game_key)
        {
            let live = self.live_games.remove(index);
            let id = self.played_games.len();
            self.played_games.push(PlayedGame {
                id,
                match_index: live.match_index,
                round: live.round,
                white: live.white,
                black: live.black,
                white_score,
                initial_fen: live.initial_fen,
                moves,
            });
        }
        self.drop_placeholder_games();
    }
}

#[derive(Clone, Debug)]
pub struct LiveTournamentHandle {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
    pub abort_game: Arc<AtomicBool>,
    pub snapshot: Arc<Mutex<LiveTournamentSnapshot>>,
}

impl LiveTournamentHandle {
    pub fn new(format: TournamentFormat) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            abort_game: Arc::new(AtomicBool::new(false)),
            snapshot: Arc::new(Mutex::new(LiveTournamentSnapshot {
                running: true,
                format_label: format.to_string(),
                status_line: "Starting tournament…".to_owned(),
                ..LiveTournamentSnapshot::default()
            })),
        }
    }

    pub fn request_cancel(&self) {
        self.pause
            .store(false, std::sync::atomic::Ordering::Release);
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.snapshot.lock() {
            guard.paused = false;
            guard.status_line =
                "Stop requested — interrupting the current search with UCI stop / XBoard ?."
                    .to_owned();
        }
    }

    pub fn request_pause(&self) {
        if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        self.pause.store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.snapshot.lock() {
            guard.paused = true;
            guard.status_line =
                "Paused — current search is being stopped; clocks stay frozen until Resume."
                    .to_owned();
        }
    }

    pub fn request_resume(&self) {
        self.pause
            .store(false, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.snapshot.lock() {
            guard.paused = false;
            if !self.cancel.load(std::sync::atomic::Ordering::Acquire) {
                guard.status_line = "Resumed — searching from the paused position.".to_owned();
            }
        }
    }

    pub fn request_abort_game(&self) {
        self.abort_game
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.snapshot.lock() {
            guard.status_line =
                "Stopping this game — interrupting the engine, then continuing the event."
                    .to_owned();
        }
    }

    pub fn clone_snapshot(&self) -> LiveTournamentSnapshot {
        self.snapshot
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

pub fn standings_from_played(engine_names: &[String], games: &[PlayedGame]) -> Vec<StandingRow> {
    use mujrim_study::rating::published_reference_elo;
    use mujrim_study::tournament::{Entrant, Pairing, TournamentResult, standings};
    let entrants: Vec<Entrant> = engine_names
        .iter()
        .enumerate()
        .map(|(index, name)| Entrant {
            id: index.to_string(),
            name: name.clone(),
            seed_elo: published_reference_elo(name),
        })
        .collect();
    let results: Vec<TournamentResult> = games
        .iter()
        .filter_map(|game| {
            let white = engine_names.iter().position(|name| name == &game.white)?;
            let black = engine_names.iter().position(|name| name == &game.black)?;
            Some(TournamentResult {
                pairing: Pairing {
                    round: game.round,
                    white,
                    black,
                },
                white_score: game.white_score,
            })
        })
        .collect();
    standing_rows(engine_names, &standings(&entrants, &results))
}

pub fn standing_rows(engine_names: &[String], standings: &[Standing]) -> Vec<StandingRow> {
    standings
        .iter()
        .enumerate()
        .map(|(rank, standing)| StandingRow {
            rank: rank + 1,
            name: engine_names
                .get(standing.entrant)
                .cloned()
                .unwrap_or_else(|| format!("Engine {}", standing.entrant + 1)),
            played: standing.played,
            wins: standing.wins,
            draws: standing.draws,
            losses: standing.losses,
            points: standing.points,
            performance: standing.performance.map(|estimate| estimate.elo),
        })
        .collect()
}

pub fn score_label(white_points: f64, black_points: f64) -> String {
    format!("{white_points:.1}–{black_points:.1}")
}

pub fn result_label(white_score: f64) -> &'static str {
    if white_score >= 0.75 {
        "1-0"
    } else if white_score <= 0.25 {
        "0-1"
    } else {
        "½-½"
    }
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn format_clock_ms(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "--:--".to_owned();
    };
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mujrim_benchmarker::strength::TournamentGameSnapshot;
    use mujrim_study::tournament::{Entrant, Pairing, standings};

    #[test]
    fn progress_and_standing_helpers_are_stable() {
        let mut snap = LiveTournamentSnapshot {
            total_matches: 4,
            completed_matches: 1,
            ..LiveTournamentSnapshot::default()
        };
        assert!((snap.progress_fraction() - 0.25).abs() < f32::EPSILON);
        assert_eq!(games_per_match(1), 2);
        assert_eq!(snap.planned_games(1), 8);
        assert_eq!(snap.remaining_matches(), 3);
        assert_eq!(snap.remaining_games(1), 8);
        assert_eq!(snap.remaining_games_label(1), "8 games remaining");
        snap.played_games = vec![PlayedGame {
            id: 0,
            match_index: 1,
            round: 1,
            white: "A".into(),
            black: "B".into(),
            white_score: 1.0,
            initial_fen: String::new(),
            moves: Vec::new(),
        }];
        assert!((snap.game_progress_fraction(1) - 0.125).abs() < f32::EPSILON);
        assert_eq!(snap.remaining_games(1), 7);
        snap.completed_matches = 4;
        assert!((snap.progress_fraction() - 1.0).abs() < f32::EPSILON);
        snap.finished = true;
        assert_eq!(snap.remaining_games(1), 0);
        assert_eq!(snap.remaining_games_label(1), "0 games remaining");
        assert_eq!(snap.phase_label(), "Finished");
        assert_eq!(score_label(1.5, 0.5), "1.5–0.5");
        assert_eq!(result_label(1.0), "1-0");
        assert_eq!(result_label(0.5), "½-½");
        assert_eq!(format_clock_ms(Some(185_000)), "3:05");
        assert_eq!(format_clock_ms(None), "--:--");

        let entrants = vec![
            Entrant {
                id: "a".into(),
                name: "Alpha".into(),
                seed_elo: None,
            },
            Entrant {
                id: "b".into(),
                name: "Beta".into(),
                seed_elo: None,
            },
        ];
        let results = vec![TournamentResult {
            pairing: Pairing {
                round: 1,
                white: 0,
                black: 1,
            },
            white_score: 1.0,
        }];
        let rows = standing_rows(
            &["Alpha".into(), "Beta".into()],
            &standings(&entrants, &results),
        );
        assert_eq!(rows[0].name, "Alpha");
        assert_eq!(rows[0].points, 1.0);
        assert!(
            rows[0].performance.is_some(),
            "live standings must show Elo"
        );
        assert_ne!(rows[0].elo_label(), "—");
        assert_eq!(PodiumTier::from_rank(1), Some(PodiumTier::Gold));
        assert_eq!(PodiumTier::from_rank(2), Some(PodiumTier::Silver));
        assert_eq!(PodiumTier::from_rank(3), Some(PodiumTier::Bronze));
        assert_eq!(PodiumTier::from_rank(4), None);
        assert_eq!(rows[0].podium(), Some(PodiumTier::Gold));
        assert_eq!(LiveTournamentSnapshot::default().phase_label(), "Ready");
        assert_eq!(
            LiveTournamentSnapshot {
                running: true,
                ..LiveTournamentSnapshot::default()
            }
            .phase_label(),
            "Live"
        );
        assert_eq!(
            LiveTournamentSnapshot {
                running: true,
                paused: true,
                ..LiveTournamentSnapshot::default()
            }
            .phase_label(),
            "Paused"
        );
        assert_eq!(
            LiveTournamentSnapshot {
                cancelled: true,
                finished: true,
                ..LiveTournamentSnapshot::default()
            }
            .phase_label(),
            "Stopped"
        );
        assert_eq!(
            LiveTournamentSnapshot {
                total_matches: 1,
                played_games: vec![PlayedGame {
                    id: 0,
                    match_index: 1,
                    round: 1,
                    white: "A".into(),
                    black: "B".into(),
                    white_score: 0.5,
                    initial_fen: String::new(),
                    moves: Vec::new(),
                }],
                ..LiveTournamentSnapshot::default()
            }
            .remaining_games_label(1),
            "1 game remaining"
        );
    }

    #[test]
    fn cancel_updates_status_line() {
        let handle = LiveTournamentHandle::new(TournamentFormat::Swiss);
        handle.request_cancel();
        assert!(handle.cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(
            handle
                .clone_snapshot()
                .status_line
                .contains("Stop requested")
        );
    }

    #[test]
    fn pause_and_resume_toggle_the_live_flag() {
        let handle = LiveTournamentHandle::new(TournamentFormat::RoundRobin);
        handle.request_pause();
        assert!(handle.pause.load(std::sync::atomic::Ordering::Acquire));
        assert!(handle.clone_snapshot().paused);
        handle.request_resume();
        assert!(!handle.pause.load(std::sync::atomic::Ordering::Acquire));
        assert!(!handle.clone_snapshot().paused);
        handle.request_abort_game();
        assert!(handle.abort_game.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn live_ply_updates_and_finish_move_to_played() {
        let mut snap = LiveTournamentSnapshot::default();
        snap.upsert_live_game(LiveGameBoard {
            game_key: "g1".into(),
            match_index: 1,
            round: 1,
            white: "A".into(),
            black: "B".into(),
            initial_fen: mujrim_study::opening::START_FEN.to_owned(),
            moves: Vec::new(),
            last_uci: String::new(),
            score_cp: 0,
            depth: 0,
            nodes: 0,
            white_clock_ms: Some(180_000),
            black_clock_ms: Some(180_000),
            ..LiveGameBoard::default()
        });
        snap.apply_ply(
            "g1",
            1,
            "e2e4".into(),
            12,
            8,
            1000,
            vec!["e2e4".into()],
            Some(179_000),
            Some(180_000),
        );
        assert_eq!(snap.live_games[0].moves.len(), 1);
        assert_eq!(snap.live_games[0].white_clock_ms, Some(179_000));
        snap.finish_live_game("g1", 1.0, vec!["e2e4".into(), "e7e5".into()]);
        assert!(snap.live_games.is_empty());
        assert_eq!(snap.played_games.len(), 1);
        assert_eq!(snap.played_games[0].white_score, 1.0);
    }

    #[test]
    fn real_games_replace_the_optimistic_placeholder() {
        let mut snap = LiveTournamentSnapshot::default();
        snap.upsert_live_game(LiveGameBoard {
            game_key: "pending-0".into(),
            white_clock_ms: Some(0),
            black_clock_ms: Some(180_000),
            clock_synced_ms: Some(1),
            ..LiveGameBoard::default()
        });
        snap.upsert_live_game(LiveGameBoard {
            game_key: "g-43".into(),
            white: "A".into(),
            black: "B".into(),
            white_clock_ms: Some(180_000),
            black_clock_ms: Some(180_000),
            clock_synced_ms: Some(now_unix_ms()),
            ..LiveGameBoard::default()
        });
        assert_eq!(snap.live_games.len(), 1);
        assert_eq!(snap.live_games[0].game_key, "g-43");
        assert_eq!(snap.live_games[0].white_clock_ms, Some(180_000));
        snap.upsert_live_game(LiveGameBoard {
            game_key: "pending-0".into(),
            ..LiveGameBoard::default()
        });
        snap.finish_live_game("g-43", 0.0, Vec::new());
        assert!(snap.live_games.is_empty());
    }

    #[test]
    fn parallel_game_keys_do_not_clobber_each_other() {
        let mut snap = LiveTournamentSnapshot::default();
        snap.upsert_live_game(LiveGameBoard {
            game_key: "g-a".into(),
            white: "A".into(),
            black: "B".into(),
            ..LiveGameBoard::default()
        });
        snap.upsert_live_game(LiveGameBoard {
            game_key: "g-b".into(),
            white: "C".into(),
            black: "D".into(),
            ..LiveGameBoard::default()
        });
        snap.apply_ply(
            "g-a",
            1,
            "e2e4".into(),
            20,
            6,
            100,
            vec!["e2e4".into()],
            Some(1000),
            None,
        );
        snap.apply_thinking(
            "g-b",
            -15,
            7,
            200,
            vec!["d7d5".into()],
            vec![ThinkingLine {
                multipv: 1,
                score_cp: -15,
                pv: vec!["d7d5".into()],
            }],
            None,
            Some(2000),
        );
        let a = snap
            .live_games
            .iter()
            .find(|game| game.game_key == "g-a")
            .expect("a");
        let b = snap
            .live_games
            .iter()
            .find(|game| game.game_key == "g-b")
            .expect("b");
        assert_eq!(a.last_uci, "e2e4");
        assert_eq!(a.score_cp, 20);
        assert!(b.last_uci.is_empty());
        assert_eq!(b.score_cp, -15);
        assert_eq!(b.pv, vec!["d7d5".to_owned()]);
        assert_eq!(b.black_clock_ms, Some(2000));
        assert_eq!(a.black_clock_ms, None);
    }

    #[test]
    fn append_games_assigns_stable_ids_and_titles() {
        let mut snap = LiveTournamentSnapshot::default();
        snap.append_games(vec![
            TournamentGameSnapshot {
                match_index: 1,
                round: 1,
                white: "Alpha".into(),
                black: "Beta".into(),
                white_score: 1.0,
                initial_fen: mujrim_study::opening::START_FEN.to_owned(),
                moves: vec!["e2e4".into(), "e7e5".into()],
            },
            TournamentGameSnapshot {
                match_index: 1,
                round: 1,
                white: "Beta".into(),
                black: "Alpha".into(),
                white_score: 0.5,
                initial_fen: mujrim_study::opening::START_FEN.to_owned(),
                moves: vec!["d2d4".into()],
            },
        ]);
        assert_eq!(snap.played_games.len(), 2);
        assert_eq!(snap.played_games[0].id, 0);
        assert_eq!(snap.played_games[1].id, 1);
        assert_eq!(snap.latest_game_id(), Some(1));
        assert_eq!(snap.game(0).unwrap().result_label(), "1-0");
        assert!(snap.game(0).unwrap().title().contains("Alpha"));
        assert_eq!(snap.game(0).unwrap().moves.len(), 2);
    }
}
