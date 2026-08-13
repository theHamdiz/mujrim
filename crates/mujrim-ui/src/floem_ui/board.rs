//! Chess board: SVG pieces, arrow canvas, pointer / premove / annotation hit-testing.

use floem::kurbo::{BezPath, Circle, Point, Rect, Stroke};
use floem::prelude::*;
use floem::ui_events::pointer::{PointerButton, PointerButtonEvent, PointerUpdate};
use types::Square;

use crate::app_core::arrows::{ArrowAppearance, arrow_geometry};
use crate::app_core::game;
use crate::app_core::motion::AnimPace;
use crate::app_core::palette::Rgba;
use crate::app_core::settings::CaptureAnimStyle;

use super::actions;
use super::state::{AppHandles, AppState};
use super::theme;

pub fn board_view(state: AppState, handles: AppHandles) -> impl IntoView {
    let painted = canvas(move |cx, size| {
        let settings = state.settings.get();
        let Some(game) = state.game.get() else {
            return;
        };
        let sq = (size.width.min(size.height) / 8.0) as f32;
        let colors = settings.board_theme.colors();
        let review = state.review_ply.get();
        let board = if let Some(ply) = review {
            crate::app_core::logic::board_at_ply(
                &state.initial_fen.get(),
                &state.move_log.get(),
                ply,
            )
            .unwrap_or_else(|_| game.board.clone())
        } else {
            game.board.clone()
        };
        for row in 0..8 {
            for col in 0..8 {
                let light = (row + col) % 2 == 0;
                let sq_id = game::display_to_square(row, col, game.flipped);
                let mut fill = if light { colors.light } else { colors.dark };
                if game.selected_square == Some(sq_id) {
                    fill = colors.selected;
                } else if settings.show_last_move && game.last_move_squares.contains(&sq_id) {
                    fill = if light {
                        colors.last_light
                    } else {
                        colors.last_dark
                    };
                }
                let x = col as f64 * sq as f64;
                let y = row as f64 * sq as f64;
                cx.fill(
                    &Rect::new(x, y, x + sq as f64, y + sq as f64),
                    theme::rgba(fill),
                    0.0,
                );
                if settings.show_legal_moves && game.legal_highlights.contains(&sq_id) {
                    let occupied = board.piece_on(sq_id).is_some();
                    let center = Point::new(x + sq as f64 * 0.5, y + sq as f64 * 0.5);
                    if occupied {
                        cx.stroke(
                            &Circle::new(center, sq as f64 * 0.42),
                            theme::rgba(Rgba::rgba(0.12, 0.12, 0.12, 0.35)),
                            &Stroke::new(3.0),
                        );
                    } else {
                        cx.fill(
                            &Circle::new(center, sq as f64 * 0.14),
                            theme::rgba(Rgba::rgba(0.12, 0.12, 0.12, 0.28)),
                            0.0,
                        );
                    }
                }
                if game
                    .premove_queue
                    .iter()
                    .any(|premove| premove.from == sq_id || premove.to == sq_id)
                {
                    cx.stroke(
                        &Rect::new(x + 2.0, y + 2.0, x + sq as f64 - 2.0, y + sq as f64 - 2.0),
                        Color::from_rgba8(80, 160, 255, 180),
                        &Stroke::new(2.0),
                    );
                }
            }
        }
        if settings.show_coords {
            for i in 0..8 {
                let file = if game.flipped { 7 - i } else { i };
                let rank = if game.flipped { i } else { 7 - i };
                let file_ch = (b'a' + file as u8) as char;
                let _ = (file_ch, rank);
            }
        }
        let appearance = ArrowAppearance {
            shape: settings.arrow_shape,
            color: settings.arrow_color,
            size: settings.arrow_size,
        };
        if settings.draw_arrows {
            for arrow in game.arrows.iter().chain(game.overlay_arrows.iter()) {
                for geom in arrow_geometry(arrow, sq, game.flipped, appearance) {
                    let mut path = BezPath::new();
                    path.move_to((geom.shaft.points[0].x as f64, geom.shaft.points[0].y as f64));
                    for point in &geom.shaft.points[1..] {
                        path.line_to((point.x as f64, point.y as f64));
                    }
                    path.close_path();
                    cx.fill(&path, theme::rgba(geom.fill), 0.0);
                    let mut head = BezPath::new();
                    head.move_to((geom.head.a.x as f64, geom.head.a.y as f64));
                    head.line_to((geom.head.b.x as f64, geom.head.b.y as f64));
                    head.line_to((geom.head.c.x as f64, geom.head.c.y as f64));
                    head.close_path();
                    cx.fill(&head, theme::rgba(geom.fill), 0.0);
                }
            }
        }
        let burst = state.capture_burst.get();
        if burst > 0.0
            && let Some(slide) = state.slide.get()
            && slide.captured
        {
            let (row, col) = square_display(slide.to, game.flipped);
            let cx_pos = (col as f64 + 0.5) * sq as f64;
            let cy_pos = (row as f64 + 0.5) * sq as f64;
            let radius = sq as f64 * (0.2 + (1.0 - burst as f64) * 0.5);
            let style = settings.capture_anim_style;
            let color = match style {
                CaptureAnimStyle::Fire => Color::from_rgba8(255, 120, 40, (burst * 180.0) as u8),
                CaptureAnimStyle::Explosion => {
                    Color::from_rgba8(255, 220, 80, (burst * 160.0) as u8)
                }
                CaptureAnimStyle::Instant => Color::from_rgba8(255, 255, 255, 0),
            };
            cx.stroke(
                &Circle::new(Point::new(cx_pos, cy_pos), radius),
                color,
                &Stroke::new(3.0),
            );
        }
    })
    .style(move |s| {
        let px = state.board_px.get();
        s.size(px, px)
    });

    let pieces = piece_layer(state, handles.clone());
    let coords = coord_layer(state);

    Stack::new((painted, pieces, coords))
        .style(move |s| {
            let px = state.board_px.get();
            s.size(px, px)
        })
        .on_event_stop(el::PointerDown, {
            let handles = handles.clone();
            move |_, event: &PointerButtonEvent| {
                on_pointer_down(state, &handles, event);
            }
        })
        .on_event_cont(el::PointerMove, {
            let handles = handles.clone();
            move |_, event: &PointerUpdate| {
                on_pointer_move(state, &handles, event);
            }
        })
        .on_event_stop(el::PointerUp, {
            let handles = handles.clone();
            move |_, event: &PointerButtonEvent| {
                on_pointer_up(state, &handles, event);
            }
        })
}

fn piece_layer(state: AppState, handles: AppHandles) -> impl IntoView {
    const EMPTY_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#;
    let sliding_handles = handles.clone();
    let squares = (0..64).map(move |index| {
        let handles = handles.clone();
        svg(move || {
            let settings = state.settings.get();
            let Some(game) = state.game.get() else {
                return EMPTY_SVG.to_owned();
            };
            let square = Square::from_index(index);
            let review = state.review_ply.get();
            let board = if let Some(ply) = review {
                crate::app_core::logic::board_at_ply(
                    &state.initial_fen.get(),
                    &state.move_log.get(),
                    ply,
                )
                .unwrap_or_else(|_| game.board.clone())
            } else {
                game.board.clone()
            };
            let Some((piece, color)) = board.piece_on(square) else {
                return EMPTY_SVG.to_owned();
            };
            let slide = state.slide.get();
            let t = AnimPace::ease(state.slide_t.get());
            if let Some(anim) = slide {
                if anim.from == square {
                    return EMPTY_SVG.to_owned();
                }
                if anim.to == square && t < 1.0 {
                    return EMPTY_SVG.to_owned();
                }
            }
            handles
                .assets
                .get_str(settings.piece_set, piece, color)
                .to_owned()
        })
        .style(move |s| {
            let Some(game) = state.game.get() else {
                return s.absolute().size(0, 0);
            };
            let sq_size = state.board_px.get() / 8.0;
            let square = Square::from_index(index);
            let (row, col) = square_display(square, game.flipped);
            s.absolute()
                .inset_left(col as f64 * sq_size)
                .inset_top(row as f64 * sq_size)
                .size(sq_size, sq_size)
        })
    });
    let sliding = {
        let handles = sliding_handles;
        svg(move || {
            let settings = state.settings.get();
            let Some(game) = state.game.get() else {
                return EMPTY_SVG.to_owned();
            };
            let Some(anim) = state.slide.get() else {
                return EMPTY_SVG.to_owned();
            };
            if AnimPace::ease(state.slide_t.get()) >= 1.0 {
                return EMPTY_SVG.to_owned();
            }
            let Some((piece, color)) = game.board.piece_on(anim.to) else {
                return EMPTY_SVG.to_owned();
            };
            handles
                .assets
                .get_str(settings.piece_set, piece, color)
                .to_owned()
        })
        .style(move |s| {
            let Some(game) = state.game.get() else {
                return s.absolute().size(0, 0);
            };
            let Some(anim) = state.slide.get() else {
                return s.absolute().size(0, 0);
            };
            let t = AnimPace::ease(state.slide_t.get()) as f64;
            if t >= 1.0 {
                return s.absolute().size(0, 0);
            }
            let sq_size = state.board_px.get() / 8.0;
            let (from_row, from_col) = square_display(anim.from, game.flipped);
            let (to_row, to_col) = square_display(anim.to, game.flipped);
            let row = from_row as f64 * (1.0 - t) + to_row as f64 * t;
            let col = from_col as f64 * (1.0 - t) + to_col as f64 * t;
            s.absolute()
                .inset_left(col * sq_size)
                .inset_top(row * sq_size)
                .size(sq_size, sq_size)
        })
    };
    squares
        .chain(std::iter::once(sliding))
        .collect::<Vec<_>>()
        .into_view()
        .style(|s| s.size_full().absolute())
}

fn coord_layer(state: AppState) -> impl IntoView {
    Label::derived(move || {
        let settings = state.settings.get();
        if !settings.show_coords {
            return String::new();
        }
        let Some(game) = state.game.get() else {
            return String::new();
        };
        let files: String = (0..8)
            .map(|i| {
                let file = if game.flipped { 7 - i } else { i };
                (b'a' + file as u8) as char
            })
            .collect();
        files
    })
    .style(|s| s.absolute().inset_bottom(4.).inset_left(8.).font_size(11.0))
}

fn square_display(square: Square, flipped: bool) -> (usize, usize) {
    let file = square.file();
    let rank = square.rank();
    if flipped {
        (rank as usize, 7 - file as usize)
    } else {
        (7 - rank as usize, file as usize)
    }
}

fn event_square(state: AppState, x: f64, y: f64) -> Option<Square> {
    let game = state.game.get_untracked()?;
    let sq = (state.board_px.get_untracked() / 8.0) as f32;
    let (row, col) = game::point_to_display(x as f32, y as f32, sq)?;
    Some(game::display_to_square(row, col, game.flipped))
}

fn on_pointer_down(state: AppState, handles: &AppHandles, event: &PointerButtonEvent) {
    let point = event.state.logical_point();
    let Some(square) = event_square(state, point.x, point.y) else {
        return;
    };
    if event.button == Some(PointerButton::Secondary) {
        state.game.update(|game| {
            if let Some(game) = game.as_mut() {
                game.begin_arrow(square);
            }
        });
        return;
    }
    if event.button != Some(PointerButton::Primary) {
        return;
    }
    actions::on_board_press(state, handles, square);
}

fn on_pointer_move(state: AppState, _handles: &AppHandles, event: &PointerUpdate) {
    let point = event.current.logical_point();
    let Some(square) = event_square(state, point.x, point.y) else {
        return;
    };
    state.game.update(|game| {
        if let Some(game) = game.as_mut()
            && game.drag_from.is_some()
        {
            game.update_drag(square);
        }
    });
}

fn on_pointer_up(state: AppState, handles: &AppHandles, event: &PointerButtonEvent) {
    let point = event.state.logical_point();
    let Some(square) = event_square(state, point.x, point.y) else {
        return;
    };
    if event.button == Some(PointerButton::Secondary) {
        let color = state.settings.get_untracked().arrow_color;
        state.game.update(|game| {
            if let Some(game) = game.as_mut() {
                game.finish_arrow(square, color);
            }
        });
        return;
    }
    actions::on_board_release(state, handles, square);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mapping_roundtrips_unflipped_corners() {
        types::init();
        let a1 = Square::from_index(0);
        let (row, col) = square_display(a1, false);
        assert_eq!(game::display_to_square(row, col, false), a1);
        let h8 = Square::from_index(63);
        let (row, col) = square_display(h8, false);
        assert_eq!(game::display_to_square(row, col, false), h8);
    }
}
