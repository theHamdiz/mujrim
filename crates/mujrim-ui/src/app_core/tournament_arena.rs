//! Arena helpers for hybrid live tournament boards.

use super::tournament_live::{LiveGameBoard, PlayedGame};

/// Cap concurrent live boards shown in the arena grid.
pub fn visible_live_boards(live: &[LiveGameBoard], concurrency: usize) -> Vec<LiveGameBoard> {
    let limit = concurrency.clamp(1, 16);
    let boards: Vec<LiveGameBoard> = live
        .iter()
        .filter(|game| !game.is_placeholder())
        .cloned()
        .collect();
    let start = boards.len().saturating_sub(limit);
    boards[start..].to_vec()
}

pub fn finished_strip(games: &[PlayedGame], limit: usize) -> Vec<PlayedGame> {
    games.iter().rev().take(limit).cloned().collect()
}

pub fn score_text(score_cp: i32) -> String {
    format!("{:+.2}", score_cp as f32 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_live_boards_respects_concurrency_cap() {
        let boards = (0..6)
            .map(|index| LiveGameBoard {
                game_key: format!("g{index}"),
                match_index: 1,
                round: 1,
                white: "A".into(),
                black: "B".into(),
                initial_fen: String::new(),
                moves: Vec::new(),
                last_uci: String::new(),
                score_cp: 0,
                depth: 0,
                nodes: 0,
                white_clock_ms: None,
                black_clock_ms: None,
                ..LiveGameBoard::default()
            })
            .collect::<Vec<_>>();
        let visible = visible_live_boards(&boards, 2);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].game_key, "g4");
        assert_eq!(visible[1].game_key, "g5");
    }

    #[test]
    fn visible_live_boards_skips_placeholders() {
        let boards = vec![
            LiveGameBoard {
                game_key: "pending-0".into(),
                ..LiveGameBoard::default()
            },
            LiveGameBoard {
                game_key: "g1".into(),
                ..LiveGameBoard::default()
            },
        ];
        let visible = visible_live_boards(&boards, 8);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].game_key, "g1");
    }
}
