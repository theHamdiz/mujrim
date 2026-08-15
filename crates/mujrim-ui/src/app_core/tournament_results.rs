//! Helpers for the Tournament Studio Results panel.

use super::tournament_live::{
    LiveTournamentSnapshot, PlayedGame, StandingRow, losses_to_label, standings_from_played,
};

/// How a follow-up field is chosen from a finished event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowUpKind {
    TopHalf,
    FinalFive,
    FinalThree,
    Remaining,
}

/// One offered follow-up size on the results screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FollowUpChoice {
    pub size: usize,
    pub kind: FollowUpKind,
}

impl FollowUpChoice {
    pub fn label(self) -> String {
        match self.kind {
            FollowUpKind::TopHalf => format!("Play top {}", self.size),
            FollowUpKind::FinalFive => "Play top 5".to_owned(),
            FollowUpKind::FinalThree => "Play top 3".to_owned(),
            FollowUpKind::Remaining => format!("Play remaining {}", self.size),
        }
    }
}

/// Ranked engines when standings exist, otherwise the finished roster size.
pub fn follow_up_field(standings: usize, engine_names: usize) -> usize {
    if standings > 0 {
        standings
    } else {
        engine_names
    }
}

/// Follow-up sizes: top half when that is more than 5, otherwise 5 and/or 3
/// if the finished field can supply them, else whatever remains (at least 2).
pub fn follow_up_choices(field: usize) -> Vec<FollowUpChoice> {
    if field < 2 {
        return Vec::new();
    }
    let half = field.div_ceil(2);
    if half > 5 {
        return vec![FollowUpChoice {
            size: half,
            kind: FollowUpKind::TopHalf,
        }];
    }
    let mut choices = Vec::new();
    if field >= 5 {
        choices.push(FollowUpChoice {
            size: 5,
            kind: FollowUpKind::FinalFive,
        });
    }
    if field >= 3 {
        choices.push(FollowUpChoice {
            size: 3,
            kind: FollowUpKind::FinalThree,
        });
    }
    if choices.is_empty() {
        choices.push(FollowUpChoice {
            size: field,
            kind: FollowUpKind::Remaining,
        });
    }
    choices
}

pub fn follow_up_names(standings: &[StandingRow], size: usize) -> Vec<String> {
    standings
        .iter()
        .take(size)
        .map(|row| row.name.clone())
        .collect()
}

pub fn follow_up_event_name(current: &str, choice: FollowUpChoice) -> String {
    let base = current
        .split(" · Top ")
        .next()
        .and_then(|name| name.split(" · Final ").next())
        .and_then(|name| name.split(" · Remaining").next())
        .unwrap_or(current)
        .trim();
    match choice.kind {
        FollowUpKind::TopHalf => format!("{base} · Top {}", choice.size),
        FollowUpKind::FinalFive | FollowUpKind::FinalThree => {
            format!("{base} · Final {}", choice.size)
        }
        FollowUpKind::Remaining => format!("{base} · Remaining"),
    }
}

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

    #[test]
    fn follow_up_uses_top_half_when_that_is_more_than_five() {
        let choices = follow_up_choices(19);
        assert_eq!(
            choices,
            [FollowUpChoice {
                size: 10,
                kind: FollowUpKind::TopHalf
            }]
        );
        assert_eq!(choices[0].label(), "Play top 10");
        assert_eq!(
            follow_up_event_name("Mujrim Tournament V2", choices[0]),
            "Mujrim Tournament V2 · Top 10"
        );
    }

    #[test]
    fn follow_up_offers_five_and_three_when_half_is_small() {
        assert_eq!(
            follow_up_choices(10),
            [
                FollowUpChoice {
                    size: 5,
                    kind: FollowUpKind::FinalFive
                },
                FollowUpChoice {
                    size: 3,
                    kind: FollowUpKind::FinalThree
                },
            ]
        );
        assert_eq!(
            follow_up_choices(8),
            [
                FollowUpChoice {
                    size: 5,
                    kind: FollowUpKind::FinalFive
                },
                FollowUpChoice {
                    size: 3,
                    kind: FollowUpKind::FinalThree
                },
            ]
        );
        assert_eq!(
            follow_up_choices(4),
            [FollowUpChoice {
                size: 3,
                kind: FollowUpKind::FinalThree
            }]
        );
        assert_eq!(
            follow_up_choices(2),
            [FollowUpChoice {
                size: 2,
                kind: FollowUpKind::Remaining
            }]
        );
        assert!(follow_up_choices(1).is_empty());
        assert_eq!(follow_up_field(10, 19), 10);
        assert_eq!(follow_up_field(0, 8), 8);
        assert_eq!(
            follow_up_choices(11),
            [FollowUpChoice {
                size: 6,
                kind: FollowUpKind::TopHalf
            }]
        );
        assert_eq!(
            follow_up_choices(6),
            [
                FollowUpChoice {
                    size: 5,
                    kind: FollowUpKind::FinalFive
                },
                FollowUpChoice {
                    size: 3,
                    kind: FollowUpKind::FinalThree
                },
            ]
        );
    }

    #[test]
    fn follow_up_names_take_the_leading_standings() {
        let standings = (1..=6)
            .map(|rank| StandingRow {
                rank,
                name: format!("E{rank}"),
                played: 4,
                wins: 2,
                draws: 1,
                losses: 1,
                points: 2.5,
                performance: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            follow_up_names(&standings, 3),
            ["E1".to_owned(), "E2".to_owned(), "E3".to_owned()]
        );
        assert_eq!(
            follow_up_event_name("V2 · Top 10 · leftover", follow_up_choices(10)[0]),
            "V2 · Final 5"
        );
    }
}
