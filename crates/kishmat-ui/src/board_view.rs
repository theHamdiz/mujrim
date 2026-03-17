//! Board view — renders a premium chess board with chess.com-style warm colors.
//!
//! Features:
//! - Dynamic square sizing based on window dimensions
//! - Warm tan/walnut brown square colors
//! - Embedded coordinate labels in corner squares (lichess-style)
//! - Auto-sized piece images with smaller pawns
//! - Selection glow, last-move highlights, legal move dots
//! - Move animation overlay (floating piece lerp)

use iced::widget::{button, column, container, row, text, Image};
use iced::{Alignment, Color, Element};

use crate::game::GameState;
use crate::pieces::PieceAssets;
use crate::Msg;

/// Derived sizing from a single square size parameter.
pub struct BoardMetrics {
    pub sq: f32,
    pub dot: f32,
    pub dot_radius: f32,
    pub piece: f32,
    pub pawn: f32,
    pub board_total: f32,
}

impl BoardMetrics {
    pub fn from_sq(sq: f32) -> Self {
        Self {
            sq,
            dot: sq * 0.28,
            dot_radius: sq * 0.14,
            piece: sq * 0.88,
            pawn: sq * 0.70,
            board_total: sq * 8.0,
        }
    }
}

// ── Board color palette (chess.com warm style) ──────────────────
const SQ_LIGHT: Color = Color::from_rgb(0.941, 0.851, 0.710);     // #F0D9B5 warm tan
const SQ_DARK: Color = Color::from_rgb(0.710, 0.533, 0.388);      // #B58863 walnut brown
const SQ_SELECTED: Color = Color::from_rgb(0.510, 0.592, 0.412);  // #829769 forest green
const SQ_LAST_LIGHT: Color = Color::from_rgb(0.969, 0.969, 0.514);// #F7F783 light yellow
const SQ_LAST_DARK: Color = Color::from_rgb(0.855, 0.824, 0.459); // #DAD275 dark yellow
const SQ_LEGAL_LIGHT: Color = Color::from_rgb(0.820, 0.878, 0.600);
const SQ_LEGAL_DARK: Color = Color::from_rgb(0.680, 0.753, 0.490);
const COORD_LIGHT: Color = Color::from_rgb(0.710, 0.533, 0.388);  // matches dark square
const COORD_DARK: Color = Color::from_rgb(0.941, 0.851, 0.710);   // matches light square

/// Compact animation data (Copy for easy passing).
#[derive(Clone, Copy)]
pub struct AnimInfo {
    pub from_sq: types::Square,
    pub to_sq: types::Square,
    pub piece: types::Piece,
    pub color: types::Color,
    pub progress: f32,
    pub captured: Option<(types::Piece, types::Color)>,
}

/// Renders the chess board as an 8x8 grid with dynamic square size.
pub fn view_board<'a>(
    gs: &'a GameState,
    assets: &'a PieceAssets,
    sq_size: f32,
    anim: Option<AnimInfo>,
) -> Element<'a, Msg> {
    let dot_size = sq_size * 0.28;
    let dot_radius = sq_size * 0.14;
    let piece_sz_normal = sq_size * 0.88;
    let piece_sz_pawn = sq_size * 0.70;

    let files = if gs.flipped {
        ['h', 'g', 'f', 'e', 'd', 'c', 'b', 'a']
    } else {
        ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']
    };

    let mut board_col = column![].spacing(0);

    for display_row in 0..8 {
        let rank = if gs.flipped { display_row } else { 7 - display_row };
        let mut rank_row = row![].spacing(0);

        for display_col in 0..8 {
            let file = if gs.flipped { 7 - display_col } else { display_col };
            let sq_index = rank * 8 + file;
            let sq = types::Square::from_index(sq_index);

            let is_light = (rank + file) % 2 != 0;
            let is_selected = gs.selected_square == Some(sq);
            let is_highlight = gs.legal_highlights.contains(&sq);
            let is_last_move = gs.last_move_squares.contains(&sq);

            // Square background color
            let bg_color = if is_selected {
                SQ_SELECTED
            } else if is_highlight {
                if is_light { SQ_LEGAL_LIGHT } else { SQ_LEGAL_DARK }
            } else if is_last_move {
                if is_light { SQ_LAST_LIGHT } else { SQ_LAST_DARK }
            } else if is_light {
                SQ_LIGHT
            } else {
                SQ_DARK
            };

            // Coordinate label color (contrast with square)
            let coord_color = if is_light { COORD_LIGHT } else { COORD_DARK };

            // Check if this square's piece is being animated (hide it from the static render)
            let hide_piece = anim.is_some_and(|a| a.from_sq == sq);
            // Check if there's a captured piece fading out at this square
            let fading_capture = anim.and_then(|a| {
                if a.to_sq == sq {
                    a.captured.map(|(p, c)| (p, c, a.progress))
                } else {
                    None
                }
            });

            // Build cell content
            let cell_content: Element<'a, Msg> = if let Some((fade_piece, fade_color, progress)) = fading_capture {
                // Captured piece fading out
                let handle = assets.get(fade_piece, fade_color);
                let piece_sz = if fade_piece == types::Piece::Pawn { piece_sz_pawn } else { piece_sz_normal };
                let opacity = (1.0 - progress).max(0.0);
                container(
                    Image::new(handle.clone())
                        .width(piece_sz)
                        .height(piece_sz)
                        .opacity(opacity),
                )
                .center_x(sq_size)
                .center_y(sq_size)
                .into()
            } else if !hide_piece {
                if let Some((piece, color)) = gs.board.piece_on(sq) {
                    let handle = assets.get(piece, color);
                    let piece_sz = if piece == types::Piece::Pawn { piece_sz_pawn } else { piece_sz_normal };
                    container(
                        Image::new(handle.clone())
                            .width(piece_sz)
                            .height(piece_sz),
                    )
                    .center_x(sq_size)
                    .center_y(sq_size)
                    .into()
                } else if is_highlight {
                    let dot_br: iced::border::Radius = dot_radius.into();
                    container(
                        container(text(""))
                            .width(dot_size)
                            .height(dot_size)
                            .style(move |_theme| container::Style {
                                background: Some(iced::Background::Color(
                                    Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                                )),
                                border: iced::Border {
                                    radius: dot_br,
                                    ..Default::default()
                                },
                                ..Default::default()
                            }),
                    )
                    .center_x(sq_size)
                    .center_y(sq_size)
                    .into()
                } else {
                    text("").into()
                }
            } else {
                // Piece is being animated away — show empty square
                text("").into()
            };

            // Overlay coordinate labels on edge squares
            let has_rank_label = display_col == 0;
            let has_file_label = display_row == 7;

            let cell_with_coords: Element<'a, Msg> = if has_rank_label || has_file_label {
                let mut overlay_col = column![].width(sq_size).height(sq_size);

                if has_rank_label {
                    overlay_col = overlay_col.push(
                        text(format!(" {}", rank + 1))
                            .size(10)
                            .color(coord_color),
                    );
                }

                let remaining = if has_rank_label && has_file_label {
                    sq_size - 26.0
                } else if has_rank_label {
                    sq_size - 14.0
                } else {
                    sq_size - 14.0
                };

                overlay_col = overlay_col.push(
                    container(cell_content)
                        .center_x(sq_size)
                        .center_y(remaining)
                        .width(sq_size)
                        .height(remaining),
                );

                if has_file_label {
                    overlay_col = overlay_col.push(
                        container(
                            text(format!("{} ", files[display_col]))
                                .size(10)
                                .color(coord_color),
                        )
                        .width(sq_size)
                        .align_x(Alignment::End),
                    );
                }

                container(overlay_col)
                    .width(sq_size)
                    .height(sq_size)
                    .into()
            } else {
                container(cell_content)
                    .center_x(sq_size)
                    .center_y(sq_size)
                    .width(sq_size)
                    .height(sq_size)
                    .into()
            };

            let cell = button(
                container(cell_with_coords)
                    .width(sq_size)
                    .height(sq_size),
            )
            .on_press(Msg::BoardClick(display_row, display_col))
            .width(sq_size)
            .height(sq_size)
            .style(move |_theme, status| {
                let hover_overlay = if matches!(status, button::Status::Hovered) {
                    0.04
                } else {
                    0.0
                };
                button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(
                        (bg_color.r + hover_overlay).min(1.0),
                        (bg_color.g + hover_overlay).min(1.0),
                        (bg_color.b + hover_overlay).min(1.0),
                    ))),
                    border: iced::Border {
                        radius: 0.0.into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    text_color: Color::BLACK,
                    ..button::Style::default()
                }
            });

            rank_row = rank_row.push(cell);
        }

        board_col = board_col.push(rank_row);
    }

    // If animation is active, overlay the floating piece on top
    // (rendered as a positioned image; since iced doesn't support absolute
    // positioning easily, we use a layered approach in main.rs instead)

    // Board wrapper — clean dark frame
    container(board_col)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.12, 0.10, 0.08))),
            border: iced::Border {
                radius: 4.0.into(),
                width: 3.0,
                color: Color::from_rgb(0.15, 0.12, 0.10),
            },
            ..Default::default()
        })
        .into()
}

