//! Live tournament progress snapshot shared between the worker and UI.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use mujrim_study::tournament::{Standing, TournamentFormat, TournamentResult};

#[derive(Clone, Debug)]
pub struct FinishedMatchRow {
    #[allow(dead_code)]
    pub index: usize,
    pub round: usize,
    pub white: String,
    pub black: String,
    pub white_points: f64,
    pub black_points: f64,
    pub error: Option<String>,
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
    pub cancelled: bool,
    pub finished: bool,
    pub status_line: String,
    pub error: Option<String>,
}

impl LiveTournamentSnapshot {
    pub fn progress_fraction(&self) -> f32 {
        if self.total_matches == 0 {
            return if self.finished { 1.0 } else { 0.0 };
        }
        (self.completed_matches as f32 / self.total_matches as f32).clamp(0.0, 1.0)
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
}

#[derive(Clone, Debug)]
pub struct LiveTournamentHandle {
    pub cancel: Arc<AtomicBool>,
    pub snapshot: Arc<Mutex<LiveTournamentSnapshot>>,
}

impl LiveTournamentHandle {
    pub fn new(format: TournamentFormat) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            snapshot: Arc::new(Mutex::new(LiveTournamentSnapshot {
                running: true,
                format_label: format.to_string(),
                status_line: "Starting tournament…".to_owned(),
                ..LiveTournamentSnapshot::default()
            })),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.snapshot.lock() {
            guard.status_line =
                "Cancel requested — finishing current engine move safely…".to_owned();
        }
    }

    pub fn clone_snapshot(&self) -> LiveTournamentSnapshot {
        self.snapshot
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mujrim_study::tournament::{Entrant, Pairing, standings};

    #[test]
    fn progress_and_standing_helpers_are_stable() {
        let mut snap = LiveTournamentSnapshot {
            total_matches: 4,
            completed_matches: 1,
            ..LiveTournamentSnapshot::default()
        };
        assert!((snap.progress_fraction() - 0.25).abs() < f32::EPSILON);
        snap.completed_matches = 4;
        assert!((snap.progress_fraction() - 1.0).abs() < f32::EPSILON);
        assert_eq!(score_label(1.5, 0.5), "1.5–0.5");
        assert_eq!(result_label(1.0), "1-0");
        assert_eq!(result_label(0.5), "½-½");

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
                .contains("Cancel requested")
        );
    }
}
