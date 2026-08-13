//! Chess board: squares, SVG pieces, arrows, and pointer hit-testing.
//!
//! Pieces are painted in the same canvas as the squares (no inherited text-color
//! tint). The widget fills its pane and letterboxes a square inside it.

use std::cell::RefCell;
use std::collections::HashMap;

use floem::kurbo::{BezPath, Circle, Point, Rect, Stroke};
use floem::prelude::*;
use floem::ui_events::pointer::{PointerButton, PointerButtonEvent, PointerUpdate};
use types::Square;

use crate::app_core::arrows::{ArrowAppearance, arrow_geometry};
use crate::app_core::game;
use crate::app_core::layout;
use crate::app_core::motion::AnimPace;
use crate::app_core::palette::Rgba;
use crate::app_core::settings::{CaptureAnimStyle, CoordPosition};

use super::actions;
use super::state::{AppHandles, AppState};
use super::theme;

thread_local! {
    static PIECE_TREES: RefCell<HashMap<usize, usvg::Tree>> = RefCell::new(HashMap::new());
}

pub fn board_view(state: AppState, handles: AppHandles) -> impl IntoView {
    let painted = canvas({
        let handles = handles.clone();
        move |cx, size| paint_board(cx, size, state, &handles)
    })
    .style(|s| s.size_full().min_width(0.0).min_height(0.0));

    Stack::new((painted, coord_layer(state)))
        .style(|s| s.size_full().min_width(0.0).min_height(0.0))
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

fn paint_board(
    cx: &mut floem::context::PaintCx<'_>,
    size: floem::kurbo::Size,
    state: AppState,
    handles: &AppHandles,
) {
    let settings = state.settings.get();
    let mut geom = layout::board_geom(size.width, size.height);
    if settings.coord_position == CoordPosition::Outside && settings.show_coords {
        let pad = 18.0;
        geom.origin_x += pad;
        geom.origin_y += pad;
        geom.side = (geom.side - pad * 2.0).max(layout::BOARD_MIN_PX);
    }
    let current = state.board_geom.get_untracked();
    if (current.side - geom.side).abs() > 0.5
        || (current.origin_x - geom.origin_x).abs() > 0.5
        || (current.origin_y - geom.origin_y).abs() > 0.5
    {
        state.board_geom.set(geom);
    }
    let Some(game) = state.game.get() else {
        return;
    };
    let sq = geom.square();
    let colors = settings.board_theme.colors();
    let review = state.review_ply.get();
    let board = if let Some(ply) = review {
        crate::app_core::logic::board_at_ply(&state.initial_fen.get(), &state.move_log.get(), ply)
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
            let x = geom.origin_x + col as f64 * sq;
            let y = geom.origin_y + row as f64 * sq;
            cx.fill(&Rect::new(x, y, x + sq, y + sq), theme::rgba(fill), 0.0);
            if settings.show_legal_moves && game.legal_highlights.contains(&sq_id) {
                let occupied = board.piece_on(sq_id).is_some();
                let center = Point::new(x + sq * 0.5, y + sq * 0.5);
                if occupied {
                    cx.stroke(
                        &Circle::new(center, sq * 0.42),
                        theme::rgba(Rgba::rgba(0.12, 0.12, 0.12, 0.35)),
                        &Stroke::new(3.0),
                    );
                } else {
                    cx.fill(
                        &Circle::new(center, sq * 0.14),
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
                    &Rect::new(x + 2.0, y + 2.0, x + sq - 2.0, y + sq - 2.0),
                    Color::from_rgba8(80, 160, 255, 180),
                    &Stroke::new(2.0),
                );
            }
        }
    }

    let slide = state.slide.get();
    let t = AnimPace::ease(state.slide_t.get());
    for square in types::Square::ALL {
        let Some((piece, color)) = board.piece_on(square) else {
            continue;
        };
        if let Some(anim) = slide {
            if anim.from == square {
                continue;
            }
            if anim.to == square && t < 1.0 {
                continue;
            }
        }
        let (row, col) = square_display(square, game.flipped);
        let svg = handles.assets.get_str(settings.piece_set, piece, color);
        draw_piece(
            cx,
            svg,
            geom.origin_x + col as f64 * sq,
            geom.origin_y + row as f64 * sq,
            sq,
        );
    }
    if let Some(anim) = slide
        && t < 1.0
        && let Some((piece, color)) = game.board.piece_on(anim.to)
    {
        let (from_row, from_col) = square_display(anim.from, game.flipped);
        let (to_row, to_col) = square_display(anim.to, game.flipped);
        let row = from_row as f64 * (1.0 - t as f64) + to_row as f64 * t as f64;
        let col = from_col as f64 * (1.0 - t as f64) + to_col as f64 * t as f64;
        let svg = handles.assets.get_str(settings.piece_set, piece, color);
        draw_piece(
            cx,
            svg,
            geom.origin_x + col * sq,
            geom.origin_y + row * sq,
            sq,
        );
    }

    let appearance = ArrowAppearance {
        shape: settings.arrow_shape,
        color: settings.arrow_color,
        size: settings.arrow_size,
    };
    if settings.draw_arrows {
        for arrow in game.arrows.iter().chain(game.overlay_arrows.iter()) {
            for geom_arrow in arrow_geometry(arrow, sq as f32, game.flipped, appearance) {
                let mut path = BezPath::new();
                path.move_to((
                    geom.origin_x + geom_arrow.shaft.points[0].x as f64,
                    geom.origin_y + geom_arrow.shaft.points[0].y as f64,
                ));
                for point in &geom_arrow.shaft.points[1..] {
                    path.line_to((
                        geom.origin_x + point.x as f64,
                        geom.origin_y + point.y as f64,
                    ));
                }
                path.close_path();
                cx.fill(&path, theme::rgba(geom_arrow.fill), 0.0);
                let mut head = BezPath::new();
                head.move_to((
                    geom.origin_x + geom_arrow.head.a.x as f64,
                    geom.origin_y + geom_arrow.head.a.y as f64,
                ));
                head.line_to((
                    geom.origin_x + geom_arrow.head.b.x as f64,
                    geom.origin_y + geom_arrow.head.b.y as f64,
                ));
                head.line_to((
                    geom.origin_x + geom_arrow.head.c.x as f64,
                    geom.origin_y + geom_arrow.head.c.y as f64,
                ));
                head.close_path();
                cx.fill(&head, theme::rgba(geom_arrow.fill), 0.0);
            }
        }
    }
    let burst = state.capture_burst.get();
    if burst > 0.0
        && let Some(slide) = slide
        && slide.captured
    {
        let (row, col) = square_display(slide.to, game.flipped);
        let cx_pos = geom.origin_x + (col as f64 + 0.5) * sq;
        let cy_pos = geom.origin_y + (row as f64 + 0.5) * sq;
        let radius = sq * (0.2 + (1.0 - burst as f64) * 0.5);
        let color = match settings.capture_anim_style {
            CaptureAnimStyle::Fire => Color::from_rgba8(255, 120, 40, (burst * 180.0) as u8),
            CaptureAnimStyle::Explosion => Color::from_rgba8(255, 220, 80, (burst * 160.0) as u8),
            CaptureAnimStyle::Instant => Color::from_rgba8(255, 255, 255, 0),
        };
        cx.stroke(
            &Circle::new(Point::new(cx_pos, cy_pos), radius),
            color,
            &Stroke::new(3.0),
        );
    }
}

fn draw_piece(cx: &mut floem::context::PaintCx<'_>, svg: &str, x: f64, y: f64, size: f64) {
    if svg.is_empty() {
        return;
    }
    let key = svg.as_ptr() as usize;
    PIECE_TREES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&key)
            && let Ok(tree) = usvg::Tree::from_str(svg, &usvg::Options::default())
        {
            cache.insert(key, tree);
        }
        if let Some(tree) = cache.get(&key) {
            let inset = size * 0.06;
            cx.draw_svg(
                floem::RendererSvg {
                    tree,
                    hash: svg.as_bytes(),
                },
                Rect::new(x + inset, y + inset, x + size - inset, y + size - inset),
                None::<&floem::peniko::Brush>,
            );
        }
    });
}

fn coord_layer(state: AppState) -> impl IntoView {
    let files = (0..8).map(move |i| {
        Label::derived(move || {
            let settings = state.settings.get();
            if !settings.show_coords {
                return String::new();
            }
            let Some(game) = state.game.get() else {
                return String::new();
            };
            let file = if game.flipped { 7 - i } else { i };
            ((b'a' + file as u8) as char).to_string()
        })
        .style(move |s| {
            let geom = state.board_geom.get();
            let sq = geom.square();
            let pal = theme::palette(state.settings.get().board_theme);
            s.absolute()
                .inset_left(geom.origin_x + i as f64 * sq + 4.0)
                .inset_top(geom.origin_y + geom.side - 16.0)
                .font_size(11.0)
                .font_bold()
                .color(theme::rgba(pal.text_primary))
                .pointer_events_none()
        })
    });
    let ranks = (0..8).map(move |i| {
        Label::derived(move || {
            let settings = state.settings.get();
            if !settings.show_coords {
                return String::new();
            }
            let Some(game) = state.game.get() else {
                return String::new();
            };
            let rank = if game.flipped { i } else { 7 - i };
            (rank + 1).to_string()
        })
        .style(move |s| {
            let geom = state.board_geom.get();
            let sq = geom.square();
            let pal = theme::palette(state.settings.get().board_theme);
            s.absolute()
                .inset_left(geom.origin_x + 4.0)
                .inset_top(geom.origin_y + i as f64 * sq + 4.0)
                .font_size(11.0)
                .font_bold()
                .color(theme::rgba(pal.text_primary))
                .pointer_events_none()
        })
    });
    files
        .chain(ranks)
        .collect::<Vec<_>>()
        .into_view()
        .style(|s| s.size_full().absolute().pointer_events_none())
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
    let geom = state.board_geom.get_untracked();
    let local_x = (x - geom.origin_x) as f32;
    let local_y = (y - geom.origin_y) as f32;
    let (row, col) = game::point_to_display(local_x, local_y, geom.square() as f32)?;
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
    use crate::app_core::layout::BoardGeom;
    use crate::app_core::pieces::{PieceAssets, PieceSet};

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

    #[test]
    fn letterboxed_hits_ignore_margins() {
        types::init();
        let geom = BoardGeom {
            origin_x: 40.0,
            origin_y: 10.0,
            side: 400.0,
        };
        assert!(geom.contains(40.0, 10.0));
        assert!(!geom.contains(39.0, 10.0));
        let local_x = 40.0 + 25.0;
        let (row, col) =
            game::point_to_display((local_x - geom.origin_x) as f32, 0.0, geom.square() as f32)
                .expect("square");
        assert_eq!(col, 0);
        assert_eq!(row, 0);
    }

    #[test]
    fn embedded_piece_svgs_parse_without_a_color_override() {
        types::init();
        let assets = PieceAssets::load();
        for set in PieceSet::ALL {
            for piece in types::Piece::ALL {
                for color in [types::Color::White, types::Color::Black] {
                    let svg = assets.get_str(set, piece, color);
                    usvg::Tree::from_str(svg, &usvg::Options::default())
                        .unwrap_or_else(|error| panic!("{set} {piece:?} {color:?}: {error}"));
                }
            }
        }
    }
}
