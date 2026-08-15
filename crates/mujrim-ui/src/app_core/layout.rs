//! Shared desktop layout metrics for the Floem (and future) workspaces.

use super::settings::Screen;
use super::tournament_live::{LiveGameBoard, PlayedGame, format_clock_ms};

pub const BOARD_PANE_PCT: f64 = 76.0;
pub const SIDE_PANE_PCT: f64 = 24.0;
pub const SIDEBAR_MIN_PX: f64 = 300.0;
pub const SIDEBAR_IDEAL_PX: f64 = 360.0;
pub const SIDEBAR_MAX_PX: f64 = 720.0;
pub const SIDEBAR_MAX_FRACTION: f64 = 0.62;
pub const SPLIT_HANDLE_PX: f64 = 10.0;
pub const BOARD_MIN_PX: f64 = 120.0;
pub const TITLE_BAR_PX: f64 = 44.0;
pub const OVERLAY_MAX_WIDTH: f64 = 760.0;
pub const TOURNAMENT_OVERLAY_MAX_WIDTH: f64 = 920.0;
pub const OVERLAY_PAD: f64 = 24.0;
pub const DOCK_TAB_BAR_PX: f64 = 36.0;
pub const DOCK_OPEN_PX: f64 = 248.0;
pub const DOCK_MIN_PX: f64 = 120.0;
pub const DOCK_MAX_PX: f64 = 560.0;
pub const LIST_SCROLL_PX: f64 = 260.0;
pub const PICKER_SCROLL_PX: f64 = 220.0;
pub const MODAL_LIST_SCROLL_PX: f64 = 280.0;
pub const LOW_TIME_MS: u64 = 10_000;
pub const STANDING_SLOTS: usize = 24;
pub const LIVE_BOARD_SLOTS: usize = 16;
pub const COORD_GUTTER_PX: f64 = 18.0;
pub const EVAL_BAR_PX: f64 = 18.0;
pub const MOVE_CHIP_HEIGHT: f64 = 32.0;
pub const MOVE_NUM_WIDTH: f64 = 28.0;
pub const MOVE_CHIP_GAP: f64 = 6.0;

/// White and black move chips share the leftover row width equally.
pub fn move_chip_width(row_inner_width: f64) -> f64 {
    let leftover = row_inner_width - MOVE_NUM_WIDTH - MOVE_CHIP_GAP * 2.0;
    (leftover * 0.5).max(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardGeom {
    pub origin_x: f64,
    pub origin_y: f64,
    pub side: f64,
}

impl Default for BoardGeom {
    fn default() -> Self {
        Self {
            origin_x: 0.0,
            origin_y: 0.0,
            side: 560.0,
        }
    }
}

impl BoardGeom {
    pub fn square(self) -> f64 {
        self.side / 8.0
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.origin_x
            && y >= self.origin_y
            && x < self.origin_x + self.side
            && y < self.origin_y + self.side
    }
}

pub fn board_side(avail_width: f64, avail_height: f64) -> f64 {
    avail_width.min(avail_height).max(BOARD_MIN_PX)
}

pub fn board_geom(avail_width: f64, avail_height: f64) -> BoardGeom {
    let side = board_side(avail_width, avail_height);
    BoardGeom {
        origin_x: ((avail_width - side) * 0.5).max(0.0),
        origin_y: ((avail_height - side) * 0.5).max(0.0),
        side,
    }
}

pub fn sidebar_width(window_width: f64) -> f64 {
    let from_pct = window_width * SIDE_PANE_PCT / 100.0;
    from_pct.clamp(
        SIDEBAR_MIN_PX,
        SIDEBAR_MAX_PX.min(window_width * SIDEBAR_MAX_FRACTION),
    )
}

pub fn clamp_sidebar_width(width: f64, pane_width: f64) -> f64 {
    let max = SIDEBAR_MAX_PX
        .min((pane_width - BOARD_MIN_PX - SPLIT_HANDLE_PX).max(SIDEBAR_MIN_PX))
        .min(pane_width * SIDEBAR_MAX_FRACTION);
    width.clamp(SIDEBAR_MIN_PX, max.max(SIDEBAR_MIN_PX))
}

pub fn board_remainder_px(pane_width: f64, sidebar_width: f64) -> f64 {
    (pane_width - SPLIT_HANDLE_PX - sidebar_width).max(BOARD_MIN_PX)
}

/// Sidebar sits on the right: dragging the split left (negative dx) grows it.
pub fn apply_sidebar_drag(width: f64, delta_x: f64, window_width: f64) -> f64 {
    clamp_sidebar_width(width - delta_x, window_width)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    Results,
    Histogram,
    EngineLog,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockFace {
    pub name: String,
    pub display: String,
    pub low_time: bool,
    pub to_move: bool,
}

pub fn dock_height(open: bool, height_px: f64) -> f64 {
    if open {
        height_px.max(DOCK_TAB_BAR_PX)
    } else {
        DOCK_TAB_BAR_PX
    }
}

pub fn clamp_dock_height(height: f64, window_height: f64) -> f64 {
    let max = DOCK_MAX_PX
        .min((window_height - TITLE_BAR_PX - BOARD_MIN_PX).max(DOCK_MIN_PX))
        .max(DOCK_MIN_PX);
    height.clamp(DOCK_MIN_PX, max)
}

/// Parent of the dock split handle is the dock itself. Using that height as the
/// window size pins the clamp at `DOCK_MIN_PX` after the first shrink.
pub fn dock_resize_window_height(observed: f64, current_dock: f64) -> f64 {
    observed
        .max(current_dock + TITLE_BAR_PX + BOARD_MIN_PX)
        .max(DOCK_MAX_PX + TITLE_BAR_PX + BOARD_MIN_PX)
}

/// Dock sits at the bottom: dragging the split up (negative dy) grows it.
pub fn apply_dock_drag(height: f64, delta_y: f64, window_height: f64) -> f64 {
    clamp_dock_height(height - delta_y, window_height)
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

/// Concurrent tournament: hide the single-board sidebar and show every live game.
pub fn is_arena_mode(concurrency: u32, live: &[LiveGameBoard]) -> bool {
    concurrency > 1 || live.iter().filter(|game| !game.is_placeholder()).count() > 1
}

pub fn tournament_arena_layout(screen: Screen, concurrency: u32, live: &[LiveGameBoard]) -> bool {
    matches!(screen, Screen::Tournaments) && is_arena_mode(concurrency, live)
}

pub fn tournament_shows_move_list(
    screen: Screen,
    concurrency: u32,
    live: &[LiveGameBoard],
) -> bool {
    !tournament_arena_layout(screen, concurrency, live)
}

pub fn live_white_to_move(game: &LiveGameBoard) -> bool {
    let starts_white = game
        .initial_fen
        .split_whitespace()
        .nth(1)
        .is_none_or(|side| side != "b");
    if game.moves.len().is_multiple_of(2) {
        starts_white
    } else {
        !starts_white
    }
}

pub fn select_live_game<'a>(
    live: &'a [LiveGameBoard],
    preferred: Option<&str>,
) -> Option<&'a LiveGameBoard> {
    preferred
        .and_then(|key| {
            live.iter()
                .find(|game| game.game_key == key && !game.is_placeholder())
        })
        .or_else(|| live.iter().rev().find(|game| !game.is_placeholder()))
        .or_else(|| focused_live_game(live))
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

pub fn clock_player_label(name: &str, white: bool) -> String {
    let side = if white { "White" } else { "Black" };
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(side) {
        side.to_owned()
    } else {
        format!("{trimmed} playing as {side}")
    }
}

pub fn remaining_clock_ms(
    stored: Option<u64>,
    synced_ms: Option<u64>,
    now_ms: u64,
    active: bool,
    paused: bool,
) -> Option<u64> {
    let stored = stored?;
    if !active || paused {
        return Some(stored);
    }
    let synced = synced_ms.unwrap_or(now_ms);
    Some(stored.saturating_sub(now_ms.saturating_sub(synced)))
}

pub fn live_clock_faces(
    live: Option<&LiveGameBoard>,
    played: Option<&PlayedGame>,
    fallback_ms: Option<u64>,
    white_to_move: bool,
) -> (ClockFace, ClockFace) {
    live_clock_faces_at(live, played, fallback_ms, white_to_move, None, false)
}

pub fn live_clock_faces_at(
    live: Option<&LiveGameBoard>,
    played: Option<&PlayedGame>,
    fallback_ms: Option<u64>,
    white_to_move: bool,
    now_ms: Option<u64>,
    paused: bool,
) -> (ClockFace, ClockFace) {
    let white_name = live
        .map(|game| game.white.as_str())
        .or_else(|| played.map(|game| game.white.as_str()))
        .unwrap_or("White");
    let black_name = live
        .map(|game| game.black.as_str())
        .or_else(|| played.map(|game| game.black.as_str()))
        .unwrap_or("Black");
    let white_stored = live.and_then(|game| game.white_clock_ms).or(fallback_ms);
    let black_stored = live.and_then(|game| game.black_clock_ms).or(white_stored);
    let synced = live.and_then(|game| game.clock_synced_ms);
    let now = now_ms.unwrap_or(synced.unwrap_or(0));
    let countdown = paused || live.is_none_or(|game| !game.clocks_should_tick());
    let white_ms = remaining_clock_ms(white_stored, synced, now, white_to_move, countdown);
    let black_ms = remaining_clock_ms(black_stored, synced, now, !white_to_move, countdown);
    (
        ClockFace {
            name: clock_player_label(white_name, true),
            display: format_clock_live(white_ms),
            low_time: clock_is_low(white_ms),
            to_move: white_to_move,
        },
        ClockFace {
            name: clock_player_label(black_name, false),
            display: format_clock_live(black_ms),
            low_time: clock_is_low(black_ms),
            to_move: !white_to_move,
        },
    )
}

pub fn eval_bar_fill(score_cp: i32) -> f32 {
    let normalized = (score_cp as f32 / 400.0).tanh();
    (0.5 + normalized * 0.5).clamp(0.02, 0.98)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordLabel {
    pub text: char,
    pub x: f64,
    pub y: f64,
    pub outside: bool,
    pub on_light: bool,
}

pub fn coord_labels(
    origin_x: f64,
    origin_y: f64,
    side: f64,
    flipped: bool,
    outside: bool,
) -> Vec<CoordLabel> {
    let sq = side / 8.0;
    let gutter = COORD_GUTTER_PX;
    let mut labels = Vec::with_capacity(16);
    for i in 0..8 {
        let file = if flipped { 7 - i } else { i };
        let rank = if flipped { i } else { 7 - i };
        let file_x = if outside {
            origin_x + i as f64 * sq + sq * 0.5 - 4.0
        } else {
            origin_x + i as f64 * sq + 4.0
        };
        let file_y = if outside {
            origin_y + side + 2.0
        } else {
            origin_y + side - 16.0
        };
        let rank_x = if outside {
            (origin_x - gutter + 4.0).max(0.0)
        } else {
            origin_x + 4.0
        };
        let rank_y = origin_y + i as f64 * sq + if outside { sq * 0.5 - 6.0 } else { 4.0 };
        labels.push(CoordLabel {
            text: (b'a' + file as u8) as char,
            x: file_x,
            y: file_y,
            outside,
            on_light: (7 + i) % 2 == 0,
        });
        labels.push(CoordLabel {
            text: (b'1' + rank as u8) as char,
            x: rank_x,
            y: rank_y,
            outside,
            on_light: i % 2 == 0,
        });
    }
    labels
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
            position_fen: String::new(),
            moves: Vec::new(),
            last_uci: String::new(),
            score_cp: 12,
            depth: 18,
            nodes: 1000,
            white_clock_ms: white_ms,
            black_clock_ms: black_ms,
            clock_synced_ms: None,
            pv: Vec::new(),
            multipv_lines: Vec::new(),
        }
    }

    #[test]
    fn panes_sum_to_full_width() {
        const {
            assert!((BOARD_PANE_PCT + SIDE_PANE_PCT - 100.0).abs() < f64::EPSILON);
            assert!(BOARD_PANE_PCT > SIDE_PANE_PCT);
            assert!(TOURNAMENT_OVERLAY_MAX_WIDTH > OVERLAY_MAX_WIDTH);
        }
    }

    #[test]
    fn board_geom_letterboxes_to_a_square_inside_the_pane() {
        let geom = board_geom(800.0, 500.0);
        assert!((geom.side - 500.0).abs() < f64::EPSILON);
        assert!((geom.origin_x - 150.0).abs() < f64::EPSILON);
        assert!(geom.origin_y.abs() < f64::EPSILON);
        assert!(geom.contains(150.0, 0.0));
        assert!(!geom.contains(149.0, 0.0));
        assert!((geom.square() - 62.5).abs() < f64::EPSILON);
    }

    #[test]
    fn sidebar_stays_in_viewport_on_narrow_and_wide_windows() {
        assert!(sidebar_width(800.0) >= SIDEBAR_MIN_PX);
        assert!(sidebar_width(800.0) < 800.0 * 0.5);
        assert!(sidebar_width(1920.0) <= SIDEBAR_MAX_PX);
        assert!(sidebar_width(1920.0) >= SIDEBAR_IDEAL_PX - 40.0);
        assert!(clamp_sidebar_width(900.0, 1280.0) <= SIDEBAR_MAX_PX);
        assert!(clamp_sidebar_width(100.0, 1280.0) >= SIDEBAR_MIN_PX);
        let grown = apply_sidebar_drag(SIDEBAR_IDEAL_PX, -40.0, 1280.0);
        assert!(grown > SIDEBAR_IDEAL_PX);
        let shrunk = apply_sidebar_drag(SIDEBAR_IDEAL_PX, 40.0, 1280.0);
        assert!(shrunk < SIDEBAR_IDEAL_PX);
    }

    #[test]
    fn dragging_the_split_grows_the_sidebar_and_shrinks_the_board() {
        let pane = 1100.0;
        let start = SIDEBAR_IDEAL_PX;
        let grown = apply_sidebar_drag(start, -180.0, pane);
        assert!((grown - (start + 180.0)).abs() < f64::EPSILON);
        assert!(board_remainder_px(pane, grown) < board_remainder_px(pane, start));
        assert!(board_remainder_px(pane, grown) >= BOARD_MIN_PX);

        let capped = apply_sidebar_drag(start, -900.0, pane);
        assert!(capped <= pane * SIDEBAR_MAX_FRACTION + f64::EPSILON);
        assert!(capped <= SIDEBAR_MAX_PX);
        assert!(board_remainder_px(pane, capped) >= BOARD_MIN_PX);

        let fake_wide = apply_sidebar_drag(start, -900.0, 1600.0);
        assert!(
            capped < fake_wide,
            "clamp must use the live pane width, not a 1600px stand-in"
        );
    }

    #[test]
    fn dock_collapses_to_tab_bar_and_expands_upward() {
        const {
            assert!(DOCK_OPEN_PX > DOCK_TAB_BAR_PX);
            assert!(LIST_SCROLL_PX > 180.0);
            assert!(PICKER_SCROLL_PX > 120.0);
            assert!(MODAL_LIST_SCROLL_PX >= LIST_SCROLL_PX);
        }
        assert_eq!(dock_height(false, DOCK_OPEN_PX), DOCK_TAB_BAR_PX);
        assert_eq!(dock_height(true, DOCK_OPEN_PX), DOCK_OPEN_PX);
        assert_eq!(dock_height(true, 400.0), 400.0);
        let grown = apply_dock_drag(DOCK_OPEN_PX, -80.0, 900.0);
        assert!(grown > DOCK_OPEN_PX);
        let shrunk = apply_dock_drag(DOCK_OPEN_PX, 80.0, 900.0);
        assert!(shrunk < DOCK_OPEN_PX);
        assert!(clamp_dock_height(20.0, 900.0) >= DOCK_MIN_PX);
        assert!(clamp_dock_height(900.0, 400.0) <= DOCK_MAX_PX);
        let after_shrink = apply_dock_drag(DOCK_OPEN_PX, 200.0, 900.0);
        assert!(after_shrink <= DOCK_MIN_PX + 1.0);
        let stuck = apply_dock_drag(after_shrink, -200.0, after_shrink);
        assert!(
            (stuck - after_shrink).abs() < 1.0,
            "passing the dock height as the window pins the clamp"
        );
        let window = dock_resize_window_height(after_shrink, after_shrink);
        let grown_again = apply_dock_drag(after_shrink, -200.0, window);
        assert!(
            grown_again > after_shrink + 40.0,
            "resize must be able to grow after the first shrink"
        );
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
        assert_eq!(
            next_dock_state(DockTab::Results, true, DockTab::EngineLog),
            (DockTab::EngineLog, true)
        );
    }

    #[test]
    fn focused_live_game_prefers_the_newest_board() {
        let live = vec![board("g0", None, None), board("g1", Some(1), Some(2))];
        let focused = focused_live_game(&live).expect("board");
        assert_eq!(focused.game_key, "g1");
        assert_eq!(
            select_live_game(&live, Some("g0"))
                .expect("preferred")
                .game_key,
            "g0"
        );
        assert_eq!(
            select_live_game(&live, Some("missing"))
                .expect("fallback")
                .game_key,
            "g1"
        );
        let mixed = vec![
            board("pending-0", Some(0), Some(180_000)),
            board("g2", Some(180_000), Some(180_000)),
        ];
        assert_eq!(
            select_live_game(&mixed, Some("pending-0"))
                .expect("skip placeholder")
                .game_key,
            "g2"
        );
        assert_eq!(select_live_game(&mixed, None).expect("real").game_key, "g2");
    }

    #[test]
    fn arena_mode_is_on_for_concurrent_or_multiple_live_boards() {
        assert!(!is_arena_mode(1, &[]));
        assert!(is_arena_mode(15, &[]));
        let two = vec![board("g0", None, None), board("g1", None, None)];
        assert!(is_arena_mode(1, &two));
        let pending = vec![board("pending-0", None, None), board("g0", None, None)];
        assert!(!is_arena_mode(1, &pending));
        assert!(!tournament_arena_layout(Screen::Playing, 15, &[]));
        assert!(tournament_arena_layout(Screen::Tournaments, 15, &[]));
        assert!(tournament_shows_move_list(Screen::Playing, 15, &[]));
        assert!(tournament_shows_move_list(Screen::Tournaments, 1, &[]));
        assert!(!tournament_shows_move_list(Screen::Tournaments, 15, &[]));
        let mut start = board("g0", None, None);
        start.initial_fen = mujrim_study::opening::START_FEN.to_owned();
        assert!(live_white_to_move(&start));
        start.moves.push("e2e4".into());
        assert!(!live_white_to_move(&start));
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
        assert_eq!(white.name, "Alpha playing as White");
        assert_eq!(white.display, "1:01");
        assert!(!white.to_move);
        assert_eq!(black.name, "Beta playing as Black");
        assert_eq!(black.display, "0:08.0");
        assert!(black.low_time);
        assert!(black.to_move);
        assert_eq!(clock_player_label("", true), "White");
        assert_eq!(clock_player_label("White", true), "White");
    }

    #[test]
    fn remaining_clock_ticks_only_the_active_side() {
        assert_eq!(
            remaining_clock_ms(Some(10_000), Some(1_000), 1_400, true, false),
            Some(9_600)
        );
        assert_eq!(
            remaining_clock_ms(Some(10_000), Some(1_000), 1_400, false, false),
            Some(10_000)
        );
        assert_eq!(
            remaining_clock_ms(Some(10_000), Some(1_000), 1_400, true, true),
            Some(10_000)
        );
        let mut live = board("g1", Some(8_000), Some(60_000));
        live.clock_synced_ms = Some(5_000);
        let (white, black) = live_clock_faces_at(Some(&live), None, None, true, Some(5_900), false);
        assert_eq!(white.display, "0:07.1");
        assert_eq!(black.display, "1:00");
        let mut pending = board("pending-0", Some(180_000), Some(180_000));
        pending.clock_synced_ms = Some(1_000);
        let (white, _) = live_clock_faces_at(
            Some(&pending),
            None,
            Some(180_000),
            true,
            Some(90_000),
            false,
        );
        assert_eq!(
            white.display, "3:00",
            "placeholder clocks must not drain between games"
        );
        let mut waiting = board("pair0-cw", Some(180_000), Some(180_000));
        waiting.depth = 0;
        waiting.nodes = 0;
        waiting.last_uci.clear();
        waiting.moves.clear();
        waiting.pv.clear();
        waiting.clock_synced_ms = Some(1_000);
        let (white, black) = live_clock_faces_at(
            Some(&waiting),
            None,
            Some(180_000),
            true,
            Some(90_000),
            false,
        );
        assert_eq!(
            white.display, "3:00",
            "White must not drain before the first search or ply"
        );
        assert_eq!(black.display, "3:00");
        waiting.depth = 6;
        let (white, _) = live_clock_faces_at(
            Some(&waiting),
            None,
            Some(180_000),
            true,
            Some(2_000),
            false,
        );
        assert_eq!(white.display, "2:59");
    }

    #[test]
    fn move_chips_share_equal_width() {
        let width = move_chip_width(360.0);
        assert!((width - 160.0).abs() < f64::EPSILON);
        assert_eq!(move_chip_width(360.0), move_chip_width(360.0));
        assert_eq!(MOVE_CHIP_HEIGHT, 32.0);
    }

    #[test]
    fn eval_bar_fill_is_white_from_the_bottom() {
        assert!((eval_bar_fill(0) - 0.5).abs() < 0.02);
        assert!(eval_bar_fill(100) > eval_bar_fill(0));
        assert!(eval_bar_fill(-100) < eval_bar_fill(0));
        assert!(eval_bar_fill(10_000) > 0.9);
        assert!(eval_bar_fill(-10_000) < 0.1);
    }

    #[test]
    fn outside_coords_sit_in_the_gutter() {
        let inside = coord_labels(20.0, 20.0, 160.0, false, false);
        let outside = coord_labels(20.0, 20.0, 160.0, false, true);
        assert_eq!(inside.len(), 16);
        assert_eq!(outside.len(), 16);
        let file_a = outside.iter().find(|label| label.text == 'a').expect("a");
        assert!(file_a.y > 20.0 + 160.0);
        let rank_8 = outside.iter().find(|label| label.text == '8').expect("8");
        assert!(rank_8.x < 20.0);
        let flipped = coord_labels(20.0, 20.0, 160.0, true, true);
        assert_eq!(
            flipped.iter().find(|label| label.x > 20.0).map(|l| l.text),
            Some('h')
        );
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
