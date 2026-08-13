//! Chess board: squares, SVG pieces, arrows, and pointer hit-testing.
//!
//! Pieces are painted in the same canvas as the squares (no inherited text-color
//! tint). The widget fills its pane and letterboxes a square inside it.

use floem::kurbo::{BezPath, Circle, Point, Rect, Stroke};
use floem::prelude::*;
use floem::ui_events::pointer::{PointerButton, PointerButtonEvent, PointerUpdate};
use types::Square;

use crate::app_core::arrows::{ArrowAppearance, ArrowColor, ArrowShape, ArrowSize, arrow_geometry};
use crate::app_core::game;
use crate::app_core::layout;
use crate::app_core::motion;
use crate::app_core::palette::Rgba;
use crate::app_core::pieces::PieceSet;
use crate::app_core::settings::{CoordPosition, PieceAnimStyle, Screen};

use super::actions;
use super::state::{AppHandles, AppState};
use super::svg_cache;
use super::theme;

pub fn board_view(state: AppState, handles: AppHandles) -> impl IntoView {
    let painted = canvas({
        let handles = handles.clone();
        move |cx, size| paint_board(cx, size, state, &handles)
    })
    .style(move |s| {
        let _ = state.settings.get();
        let _ = state.game.get();
        let _ = state.slide.get();
        let _ = state.slide_t.get();
        let _ = state.capture_burst.get();
        let _ = state.review_ply.get();
        let _ = state.move_log.get();
        let _ = state.move_annotations.get();
        let _ = state.analysis.get();
        let _ = state.screen.get();
        s.size_full().min_width(0.0).min_height(0.0)
    });

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
    let reviewed = review.and_then(|ply| {
        crate::app_core::logic::board_at_ply(&state.initial_fen.get(), &state.move_log.get(), ply)
            .ok()
    });
    let board = reviewed.as_ref().unwrap_or(&game.board);
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

    let threat_marks = threat_marks_for(board, state.screen.get(), settings.show_threats);
    for mark in &threat_marks {
        let (row, col) = square_display(mark.square, game.flipped);
        let x = geom.origin_x + col as f64 * sq;
        let y = geom.origin_y + row as f64 * sq;
        let fill = if mark.hanging {
            Color::from_rgba8(220, 48, 48, 92)
        } else {
            Color::from_rgba8(255, 140, 40, 72)
        };
        cx.fill(&Rect::new(x, y, x + sq, y + sq), fill, 0.0);
    }

    let slide = state.slide.get();
    let t = state.slide_t.get();
    let piece_style = if settings.piece_slide {
        settings.piece_anim_style
    } else {
        PieceAnimStyle::Instant
    };
    for square in types::Square::ALL {
        let Some((piece, color)) = board.piece_on(square) else {
            continue;
        };
        if let Some(anim) = slide {
            if anim.from == square || anim.rook_from == Some(square) {
                continue;
            }
            if t < 1.0 && (anim.to == square || anim.rook_to == Some(square)) {
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
    {
        paint_flying_piece(
            cx,
            PieceFlightPaint {
                handles,
                piece_set: settings.piece_set,
                style: piece_style,
                t,
                from: anim.from,
                to: anim.to,
                piece: anim.piece,
                color: anim.color,
                flipped: game.flipped,
                origin_x: geom.origin_x,
                origin_y: geom.origin_y,
                sq,
            },
        );
        if let (Some(rook_from), Some(rook_to)) = (anim.rook_from, anim.rook_to) {
            paint_flying_piece(
                cx,
                PieceFlightPaint {
                    handles,
                    piece_set: settings.piece_set,
                    style: piece_style,
                    t,
                    from: rook_from,
                    to: rook_to,
                    piece: types::Piece::Rook,
                    color: anim.color,
                    flipped: game.flipped,
                    origin_x: geom.origin_x,
                    origin_y: geom.origin_y,
                    sq,
                },
            );
        }
    }

    let appearance = ArrowAppearance {
        shape: settings.arrow_shape,
        color: settings.arrow_color,
        size: settings.arrow_size,
    };
    let mut arrows = Vec::new();
    if settings.draw_arrows {
        arrows.extend(game.arrows.iter().cloned());
        if let (Some(from), Some(to)) = (game.arrow_start, game.drag_over)
            && from != to
        {
            arrows.push(crate::app_core::arrows::user_arrow(
                from,
                to,
                settings.arrow_color,
            ));
        }
    }
    for arrow in &game.overlay_arrows {
        let show = match arrow.role {
            mujrim_study::board_marks::ArrowRole::LastMove => settings.last_move_arrow,
            mujrim_study::board_marks::ArrowRole::Ponder => settings.ponder_arrow,
            mujrim_study::board_marks::ArrowRole::User => settings.draw_arrows,
            _ => true,
        };
        if show {
            arrows.push(arrow.clone());
        }
    }
    for arrow in &arrows {
        paint_arrow(
            cx,
            arrow,
            geom.origin_x,
            geom.origin_y,
            sq,
            game.flipped,
            appearance,
        );
    }
    paint_threat_arrows(
        cx,
        &threat_marks,
        geom.origin_x,
        geom.origin_y,
        sq,
        game.flipped,
    );
    if let Some((square, annotation)) = crate::app_core::logic::review_annotation_badge(
        &state.initial_fen.get(),
        &state.move_log.get(),
        state.review_ply.get(),
        &state.move_annotations.get(),
    ) {
        let (row, col) = square_display(square, game.flipped);
        let size = sq * 0.38;
        let x = geom.origin_x + (col as f64 + 1.0) * sq - size - sq * 0.04;
        let y = geom.origin_y + row as f64 * sq + sq * 0.04;
        draw_annotation_badge(cx, annotation, x, y, size);
    }
    let burst = state.capture_burst.get();
    if burst > 0.0
        && let Some(slide) = slide
        && slide.captured
    {
        let (row, col) = square_display(slide.burst_square, game.flipped);
        let cx_pos = geom.origin_x + (col as f64 + 0.5) * sq;
        let cy_pos = geom.origin_y + (row as f64 + 0.5) * sq;
        for mark in motion::capture_marks(settings.capture_anim_style, burst) {
            let x = cx_pos + mark.x * sq;
            let y = cy_pos + mark.y * sq;
            let color = Color::from_rgba8(mark.r, mark.g, mark.b, mark.a);
            if mark.ring {
                cx.stroke(
                    &Circle::new(Point::new(x, y), mark.radius * sq),
                    color,
                    &Stroke::new(3.0),
                );
            } else {
                cx.fill(&Circle::new(Point::new(x, y), mark.radius * sq), color, 0.0);
            }
        }
    }
}

fn paint_arrow(
    cx: &mut floem::context::PaintCx<'_>,
    arrow: &mujrim_study::board_marks::BoardArrow,
    origin_x: f64,
    origin_y: f64,
    sq: f64,
    flipped: bool,
    appearance: ArrowAppearance,
) {
    for geom_arrow in arrow_geometry(arrow, sq as f32, flipped, appearance) {
        if let Some(body) = geom_arrow.body.as_ref().filter(|body| body.len() >= 3) {
            let mut path = BezPath::new();
            path.move_to((origin_x + body[0].x as f64, origin_y + body[0].y as f64));
            for point in &body[1..] {
                path.line_to((origin_x + point.x as f64, origin_y + point.y as f64));
            }
            path.close_path();
            cx.fill(&path, theme::rgba(geom_arrow.fill), 0.0);
            cx.stroke(&path, theme::rgba(geom_arrow.outline), &Stroke::new(1.4));
        } else {
            let mut path = BezPath::new();
            path.move_to((
                origin_x + geom_arrow.shaft.points[0].x as f64,
                origin_y + geom_arrow.shaft.points[0].y as f64,
            ));
            for point in &geom_arrow.shaft.points[1..] {
                path.line_to((origin_x + point.x as f64, origin_y + point.y as f64));
            }
            path.close_path();
            cx.fill(&path, theme::rgba(geom_arrow.fill), 0.0);
            let mut head = BezPath::new();
            head.move_to((
                origin_x + geom_arrow.head.a.x as f64,
                origin_y + geom_arrow.head.a.y as f64,
            ));
            head.line_to((
                origin_x + geom_arrow.head.b.x as f64,
                origin_y + geom_arrow.head.b.y as f64,
            ));
            head.line_to((
                origin_x + geom_arrow.head.c.x as f64,
                origin_y + geom_arrow.head.c.y as f64,
            ));
            head.close_path();
            cx.fill(&head, theme::rgba(geom_arrow.fill), 0.0);
        }
        if let Some((tip, _step, fill)) = geom_arrow.step {
            let center = Point::new(origin_x + tip.x as f64, origin_y + tip.y as f64);
            cx.fill(
                &Circle::new(center, sq * 0.16),
                Color::from_rgba8(20, 20, 24, 210),
                0.0,
            );
            cx.stroke(
                &Circle::new(center, sq * 0.16),
                theme::rgba(fill),
                &Stroke::new(1.5),
            );
        }
    }
}

fn draw_annotation_badge(
    cx: &mut floem::context::PaintCx<'_>,
    annotation: mujrim_study::annotation::MoveAnnotation,
    x: f64,
    y: f64,
    size: f64,
) {
    let key = annotation.label();
    let svg = annotation.board_badge_svg();
    svg_cache::draw(
        cx,
        &svg,
        Rect::new(x, y, x + size, y + size),
        key.as_bytes(),
    );
}

struct PieceFlightPaint<'a> {
    handles: &'a AppHandles,
    piece_set: PieceSet,
    style: PieceAnimStyle,
    t: f32,
    from: Square,
    to: Square,
    piece: types::Piece,
    color: types::Color,
    flipped: bool,
    origin_x: f64,
    origin_y: f64,
    sq: f64,
}

fn paint_flying_piece(cx: &mut floem::context::PaintCx<'_>, flight: PieceFlightPaint<'_>) {
    let (from_row, from_col) = square_display(flight.from, flight.flipped);
    let (to_row, to_col) = square_display(flight.to, flight.flipped);
    let path = motion::piece_flight(
        flight.style,
        flight.t,
        from_row as f64,
        from_col as f64,
        to_row as f64,
        to_col as f64,
    );
    let svg = flight
        .handles
        .assets
        .get_str(flight.piece_set, flight.piece, flight.color);
    let size = flight.sq * path.scale.max(0.08);
    let x = flight.origin_x + path.col * flight.sq + (flight.sq - size) * 0.5;
    let y = flight.origin_y + path.row * flight.sq + (flight.sq - size) * 0.5;
    draw_piece(cx, svg, x, y, size);
}

fn draw_piece(cx: &mut floem::context::PaintCx<'_>, svg: &str, x: f64, y: f64, size: f64) {
    let inset = size * 0.06;
    svg_cache::draw(
        cx,
        svg,
        Rect::new(x + inset, y + inset, x + size - inset, y + size - inset),
        svg.as_bytes(),
    );
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

fn threat_marks_for(
    board: &types::Board,
    screen: Screen,
    show_threats: bool,
) -> Vec<mujrim_study::threats::ThreatMark> {
    if !show_threats
        || !matches!(
            screen,
            Screen::Study | Screen::Learn | Screen::Analysis | Screen::Library
        )
    {
        return Vec::new();
    }
    mujrim_study::threats::threatened_pieces(board)
}

fn paint_threat_arrows(
    cx: &mut floem::context::PaintCx<'_>,
    marks: &[mujrim_study::threats::ThreatMark],
    origin_x: f64,
    origin_y: f64,
    sq: f64,
    flipped: bool,
) {
    use mujrim_study::board_marks::{ArrowRole, BoardArrow};
    for mark in marks {
        let appearance = ArrowAppearance {
            shape: ArrowShape::Straight,
            color: if mark.hanging {
                ArrowColor::Red
            } else {
                ArrowColor::Orange
            },
            size: ArrowSize::Slim,
        };
        let arrow = BoardArrow::new(
            mark.attacker,
            mark.square,
            appearance.color.to_mark(),
            ArrowRole::Coach,
        );
        for geom_arrow in arrow_geometry(&arrow, sq as f32, flipped, appearance) {
            let mut path = BezPath::new();
            path.move_to((
                origin_x + geom_arrow.shaft.points[0].x as f64,
                origin_y + geom_arrow.shaft.points[0].y as f64,
            ));
            for point in &geom_arrow.shaft.points[1..] {
                path.line_to((origin_x + point.x as f64, origin_y + point.y as f64));
            }
            path.close_path();
            cx.fill(&path, theme::rgba(geom_arrow.fill), 0.0);
            let mut head = BezPath::new();
            head.move_to((
                origin_x + geom_arrow.head.a.x as f64,
                origin_y + geom_arrow.head.a.y as f64,
            ));
            head.line_to((
                origin_x + geom_arrow.head.b.x as f64,
                origin_y + geom_arrow.head.b.y as f64,
            ));
            head.line_to((
                origin_x + geom_arrow.head.c.x as f64,
                origin_y + geom_arrow.head.c.y as f64,
            ));
            head.close_path();
            cx.fill(&head, theme::rgba(geom_arrow.fill), 0.0);
        }
    }
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
        if !state.settings.get_untracked().draw_arrows {
            return;
        }
        state.game.update(|game| {
            if let Some(game) = game.as_mut() {
                game.begin_arrow(square);
                game.drag_over = Some(square);
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
        if let Some(game) = game.as_mut() {
            if game.drag_from.is_some() {
                game.update_drag(square);
            }
            if game.arrow_start.is_some() {
                game.drag_over = Some(square);
            }
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

    #[test]
    fn threat_overlay_is_limited_to_study_and_learn() {
        types::init();
        let board = types::Board::new();
        assert!(threat_marks_for(&board, Screen::Playing, true).is_empty());
        assert!(threat_marks_for(&board, Screen::Study, false).is_empty());
    }

    #[test]
    fn canvas_style_tracks_settings_and_arrows() {
        let src = include_str!("board.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "state.settings.get()",
            "state.slide_t.get()",
            "state.capture_burst.get()",
            "paint_arrow",
            "piece_flight",
            "capture_marks",
            "last_move_arrow",
            "arrow_start",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
    }
}
