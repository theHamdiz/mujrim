//! Helpers for the Tournament Studio Results panel.

use super::tournament_live::{
    LiveTournamentSnapshot, PlayedGame, StandingRow, losses_to_label, standings_from_played,
};

/// Whether the Results panel should render standings/game tables.
pub fn panel_open(snapshot: &LiveTournamentSnapshot, forced: bool) -> bool {
    forced || snapshot.show_results_panel
}

pub fn standings_ready(standings: &[StandingRow]) -> bool {
    !standings.is_empty()
}

/// Rankings plus who each engine lost to, always from every finished game.
pub fn detailed_results(
    engine_names: &[String],
    games: &[PlayedGame],
) -> Vec<(StandingRow, String)> {
    standings_from_played(engine_names, games)
        .into_iter()
        .map(|row| {
            let losses = losses_to_label(&row.name, games);
            (row, losses)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::tournament_live::PlayedGame;

    #[test]
    fn panel_open_respects_forced_flag() {
        let snap = LiveTournamentSnapshot::default();
        assert!(!panel_open(&snap, false));
        assert!(panel_open(&snap, true));
        let mut open = snap;
        open.show_results_panel = true;
        assert!(panel_open(&open, false));
    }

    #[test]
    fn standings_ready_is_false_when_empty() {
        assert!(!standings_ready(&[]));
    }

    #[test]
    fn detailed_results_use_all_games_and_name_losses() {
        let games = vec![
            PlayedGame {
                id: 1,
                match_index: 1,
                round: 1,
                white: "Alpha".into(),
                black: "Beta".into(),
                white_score: 1.0,
                initial_fen: String::new(),
                moves: Vec::new(),
            },
            PlayedGame {
                id: 2,
                match_index: 2,
                round: 1,
                white: "Gamma".into(),
                black: "Alpha".into(),
                white_score: 1.0,
                initial_fen: String::new(),
                moves: Vec::new(),
            },
        ];
        let names = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        let rows = detailed_results(&names, &games);
        assert_eq!(rows.len(), 3);
        let alpha = rows
            .iter()
            .find(|(row, _)| row.name == "Alpha")
            .expect("alpha");
        assert!(alpha.1.contains("Gamma"));
        let gamma = rows
            .iter()
            .find(|(row, _)| row.name == "Gamma")
            .expect("gamma");
        assert_eq!(gamma.1, "Undefeated");
        assert!(alpha.0.score_line().starts_with("Elo "));
    }
}
