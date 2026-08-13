//! Live tournament digital clocks (White / Black).

use floem::prelude::*;
use types::Color as Side;

use crate::app_core::layout::{self, ClockFace};

use super::state::AppState;
use super::theme;

pub fn live_clocks(state: AppState) -> impl IntoView {
    Stack::horizontal((clock_card(state, true), clock_card(state, false))).style(|s| {
        s.width_full()
            .col_gap(8.0)
            .padding_bottom(8.0)
            .items_stretch()
    })
}

fn clock_card(state: AppState, white: bool) -> impl IntoView {
    Stack::vertical((
        Label::derived(move || face(state, white).name).style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.font_size(11.0)
                .color(theme::rgba(pal.text_secondary))
                .text_ellipsis()
        }),
        Label::derived(move || face(state, white).display).style(move |s| {
            let face = face(state, white);
            let pal = theme::palette(state.settings.get().board_theme);
            let color = if face.low_time {
                Color::from_rgb8(255, 138, 76)
            } else if face.to_move {
                theme::rgba(pal.text_primary)
            } else {
                theme::rgba(pal.text_secondary)
            };
            s.font_size(26.0).font_bold().color(color)
        }),
        Empty::new().style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.width_full().height(2.0).border_radius(1.0).background(
                if face(state, white).to_move {
                    theme::rgba(pal.accent)
                } else {
                    Color::TRANSPARENT
                },
            )
        }),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let active = face(state, white).to_move;
        s.flex_grow(1.0f32)
            .min_width(0.0)
            .padding_horiz(12.0)
            .padding_vert(8.0)
            .row_gap(2.0)
            .border_radius(10.0)
            .background(theme::rgba(pal.panel))
            .border(1.0)
            .border_color(if active {
                theme::rgba(pal.accent)
            } else {
                theme::rgba(pal.border)
            })
    })
}

fn face(state: AppState, white: bool) -> ClockFace {
    let snap = state.tournament_snapshot.get();
    let selected = state.selected_tournament_game_id.get();
    let played = selected.and_then(|id| snap.game(id).cloned());
    let live = selected
        .is_none()
        .then(|| layout::focused_live_game(&snap.live_games).cloned())
        .flatten();
    let fallback = Some(
        state
            .tournament_setup
            .get()
            .time_control
            .match_clock()
            .initial
            .as_millis() as u64,
    );
    let white_to_move = state
        .game
        .get()
        .is_none_or(|game| game.board.side_to_move == Side::White);
    let (white_face, black_face) =
        layout::live_clock_faces(live.as_ref(), played.as_ref(), fallback, white_to_move);
    if white { white_face } else { black_face }
}

#[cfg(test)]
mod tests {
    use crate::app_core::layout;
    use crate::app_core::tournament_live::LiveGameBoard;

    #[test]
    fn widget_faces_track_live_clocks() {
        let live = LiveGameBoard {
            game_key: "g".into(),
            match_index: 0,
            round: 1,
            white: "W".into(),
            black: "B".into(),
            initial_fen: String::new(),
            moves: Vec::new(),
            last_uci: String::new(),
            score_cp: 0,
            depth: 0,
            nodes: 0,
            white_clock_ms: Some(4_100),
            black_clock_ms: Some(180_000),
        };
        let (white, black) = layout::live_clock_faces(Some(&live), None, None, true);
        assert_eq!(white.display, "0:04.1");
        assert!(white.low_time && white.to_move);
        assert_eq!(black.display, "3:00");
        assert!(!black.to_move);
    }
}
