//! Shared desktop layout metrics for the Floem (and future) workspaces.

use super::tournament_live::{LiveGameBoard, PlayedGame, format_clock_ms};

pub const BOARD_PANE_PCT: f64 = 80.0;
pub const SIDE_PANE_PCT: f64 = 20.0;
pub const DOCK_TAB_BAR_PX: f64 = 36.0;
pub const DOCK_OPEN_PX: f64 = 220.0;
pub const LOW_TIME_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    Results,
    Histogram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockFace {
    pub name: String,
    pub display: String,
    pub low_time: bool,
    pub to_move: bool,
}

pub fn dock_height(open: bool) -> f64 {
    if open { DOCK_OPEN_PX } else { DOCK_TAB_BAR_PX }
}

pub fn next_dock_state(current: DockTab, open: bool, clicked: DockTab) -> (DockTab, bool) {
    if current == clicked {
        (current, !open)
    } else {
        (clicked, true)
    }
}

pub fn focused_live_game(live: &[LiveGameBoard]) -> Option<&LiveGameBoard> {
    live.last()
}

pub fn clock_is_low(ms: Option<u64>) -> bool {
    ms.is_some_and(|value| value <= LOW_TIME_MS)
}

pub fn format_clock_live(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return format_clock_ms(None);
    };
    if ms < LOW_TIME_MS {
        let secs = ms / 1000;
        let tenths = (ms % 1000) / 100;
        format!("0:{secs:02}.{tenths}")
    } else {
        format_clock_ms(Some(ms))
    }
}

pub fn live_clock_faces(
    live: Option<&LiveGameBoard>,
    played: Option<&PlayedGame>,
    fallback_ms: Option<u64>,
    white_to_move: bool,
) -> (ClockFace, ClockFace) {
    let white_name = live
        .map(|game| game.white.as_str())
        .or_else(|| played.map(|game| game.white.as_str()))
        .unwrap_or("White");
    let black_name = live
        .map(|game| game.black.as_str())
        .or_else(|| played.map(|game| game.black.as_str()))
        .unwrap_or("Black");
    let white_ms = live.and_then(|game| game.white_clock_ms).or(fallback_ms);
    let black_ms = live.and_then(|game| game.black_clock_ms).or(white_ms);
    (
        ClockFace {
            name: white_name.to_owned(),
            display: format_clock_live(white_ms),
            low_time: clock_is_low(white_ms),
            to_move: white_to_move,
        },
        ClockFace {
            name: black_name.to_owned(),
            display: format_clock_live(black_ms),
            low_time: clock_is_low(black_ms),
            to_move: !white_to_move,
        },
    )
}

pub fn extend_histogram(scores: &mut Vec<Option<i32>>, ply_count: usize, score_cp: i32) {
    if ply_count < scores.len() {
        scores.truncate(ply_count);
    }
    while scores.len() < ply_count {
        scores.push(Some(score_cp));
    }
    if let Some(last) = scores.last_mut() {
        *last = Some(score_cp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::tournament_live::LiveGameBoard;

    fn board(key: &str, white_ms: Option<u64>, black_ms: Option<u64>) -> LiveGameBoard {
        LiveGameBoard {
            game_key: key.into(),
            match_index: 1,
            round: 2,
            white: "Alpha".into(),
            black: "Beta".into(),
            initial_fen: String::new(),
            moves: Vec::new(),
            last_uci: String::new(),
            score_cp: 12,
            depth: 18,
            nodes: 1000,
            white_clock_ms: white_ms,
            black_clock_ms: black_ms,
        }
    }

    #[test]
    fn panes_sum_to_full_width() {
        const {
            assert!((BOARD_PANE_PCT + SIDE_PANE_PCT - 100.0).abs() < f64::EPSILON);
            assert!(BOARD_PANE_PCT > SIDE_PANE_PCT);
        }
    }

    #[test]
    fn dock_collapses_to_tab_bar_and_expands_upward() {
        const {
            assert!(DOCK_OPEN_PX > DOCK_TAB_BAR_PX);
        }
        assert_eq!(dock_height(false), DOCK_TAB_BAR_PX);
        assert_eq!(dock_height(true), DOCK_OPEN_PX);
    }

    #[test]
    fn dock_tab_click_toggles_or_opens() {
        assert_eq!(
            next_dock_state(DockTab::Histogram, true, DockTab::Histogram),
            (DockTab::Histogram, false)
        );
        assert_eq!(
            next_dock_state(DockTab::Histogram, false, DockTab::Results),
            (DockTab::Results, true)
        );
    }

    #[test]
    fn focused_live_game_prefers_the_newest_board() {
        let live = vec![board("g0", None, None), board("g1", Some(1), Some(2))];
        let focused = focused_live_game(&live).expect("board");
        assert_eq!(focused.game_key, "g1");
    }

    #[test]
    fn live_clocks_use_tenths_under_ten_seconds() {
        assert_eq!(format_clock_live(None), "--:--");
        assert_eq!(format_clock_live(Some(185_000)), "3:05");
        assert_eq!(format_clock_live(Some(9_250)), "0:09.2");
        assert!(clock_is_low(Some(9_250)));
        assert!(!clock_is_low(Some(11_000)));
    }

    #[test]
    fn live_clock_faces_label_players_and_active_side() {
        let live = board("g1", Some(61_000), Some(8_000));
        let (white, black) = live_clock_faces(Some(&live), None, Some(180_000), false);
        assert_eq!(white.name, "Alpha");
        assert_eq!(white.display, "1:01");
        assert!(!white.to_move);
        assert_eq!(black.name, "Beta");
        assert_eq!(black.display, "0:08.0");
        assert!(black.low_time);
        assert!(black.to_move);
    }

    #[test]
    fn histogram_extends_and_rewinds_with_plies() {
        let mut scores = Vec::new();
        extend_histogram(&mut scores, 2, 30);
        assert_eq!(scores, vec![Some(30), Some(30)]);
        extend_histogram(&mut scores, 2, 44);
        assert_eq!(scores, vec![Some(30), Some(44)]);
        extend_histogram(&mut scores, 1, 10);
        assert_eq!(scores, vec![Some(10)]);
        extend_histogram(&mut scores, 0, 0);
        assert!(scores.is_empty());
    }
}
