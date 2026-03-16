//! Board view — renders the chess board with image-based pieces.

use iced::widget::{button, column, container, row, text, Image};
use iced::{Alignment, Color, Element};

use crate::game::GameState;
use crate::pieces::PieceAssets;
use crate::Msg;

const SQ_SIZE: f32 = 72.0;
const DOT_SIZE: f32 = SQ_SIZE * 0.25;
const DOT_RADIUS: f32 = SQ_SIZE * 0.125;
const PIECE_SIZE: f32 = SQ_SIZE * 0.85;

/// Renders the chess board as an 8×8 grid with image pieces.
pub fn view_board<'a>(gs: &'a GameState, assets: &'a PieceAssets) -> Element<'a, Msg> {

    // File labels
    let files = if gs.flipped {
        ['h', 'g', 'f', 'e', 'd', 'c', 'b', 'a']
    } else {
        ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']
    };

    let mut board_col = column![].spacing(0).align_x(Alignment::Center);

    for display_row in 0..8 {
        let rank = if gs.flipped { display_row } else { 7 - display_row };
        let rank_label = text(format!(" {} ", rank + 1))
            .size(13)
            .color(Color::from_rgb(0.6, 0.6, 0.6));

        let mut rank_row = row![
            container(rank_label).width(22).center_y(SQ_SIZE)
        ]
        .spacing(0)
        .align_y(Alignment::Center);

        for display_col in 0..8 {
            let file = if gs.flipped { 7 - display_col } else { display_col };
            let sq_index = rank * 8 + file;
            let sq = types::Square::from_index(sq_index);

            let is_light = (rank + file) % 2 != 0;

            // Determine square background color
            let is_selected = gs.selected_square == Some(sq);
            let is_highlight = gs.legal_highlights.contains(&sq);
            let is_last_move = gs.last_move_squares.contains(&sq);

            let bg_color = if is_selected {
                Color::from_rgb(0.35, 0.58, 0.38) // selection green
            } else if is_highlight {
                if is_light {
                    Color::from_rgb(0.72, 0.84, 0.55) // light legal highlight
                } else {
                    Color::from_rgb(0.55, 0.72, 0.38) // dark legal highlight
                }
            } else if is_last_move {
                if is_light {
                    Color::from_rgb(0.96, 0.96, 0.60) // light last move
                } else {
                    Color::from_rgb(0.73, 0.79, 0.36) // dark last move
                }
            } else if is_light {
                Color::from_rgb(0.94, 0.90, 0.83) // light square — warm cream
            } else {
                Color::from_rgb(0.47, 0.60, 0.36) // dark square — green
            };

            // Build cell content: either piece image or empty
            let cell_content: Element<'a, Msg> = if let Some((piece, color)) = gs.board.piece_on(sq) {
                let handle = assets.get(piece, color);
                Image::new(handle.clone())
                    .width(PIECE_SIZE)
                    .height(PIECE_SIZE)
                    .into()
            } else if is_highlight {
                // Show a dot for legal move targets on empty squares
                container(
                    container(text(""))
                        .width(DOT_SIZE)
                        .height(DOT_SIZE)
                        .style(|_theme| container::Style {
                            background: Some(iced::Background::Color(
                                Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                            )),
                            border: iced::Border {
                                radius: DOT_RADIUS.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                .center_x(SQ_SIZE)
                .center_y(SQ_SIZE)
                .into()
            } else {
                text("").into()
            };

            let cell = button(
                container(cell_content)
                    .center_x(SQ_SIZE)
                    .center_y(SQ_SIZE)
                    .width(SQ_SIZE)
                    .height(SQ_SIZE),
            )
            .on_press(Msg::BoardClick(display_row, display_col))
            .width(SQ_SIZE)
            .height(SQ_SIZE)
            .style(move |_theme, status| {
                let hover_overlay = if matches!(status, button::Status::Hovered) {
                    0.06
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

    // File labels at bottom
    let mut file_labels_row = row![
        container(text("")).width(22)
    ]
    .spacing(0);

    for f in &files {
        file_labels_row = file_labels_row.push(
            container(
                text(format!("{f}"))
                    .size(12)
                    .color(Color::from_rgb(0.6, 0.6, 0.6)),
            )
            .center_x(SQ_SIZE)
            .width(SQ_SIZE),
        );
    }
    board_col = board_col.push(file_labels_row);

    // Wrap in a container with rounded corners and subtle shadow
    container(board_col)
        .padding(2)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.25, 0.25, 0.25))),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: Color::from_rgb(0.15, 0.15, 0.15),
            },
            ..Default::default()
        })
        .into()
}
