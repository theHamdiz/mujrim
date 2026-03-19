//! Board view — renders a premium chess board with configurable themes.
//!
//! Features:
//! - Multiple board color themes (Classic, Emerald, Ocean, Royal, Walnut)
//! - Configurable coordinate labels (inside or outside the board)
//! - Auto-sized piece images with smaller pawns
//! - Selection glow, last-move highlights, legal move dots
//! - Premove square highlights
//! - Drawing arrows overlay
//! - Move animation overlay with capture effects (instant or explosion)

use iced::widget::{Image, button, column, container, mouse_area, row, text};
use iced::{Alignment, Color, Element};

use crate::game::GameState;
use crate::pieces::PieceAssets;
use crate::{CaptureAnimStyle, CoordPosition, Msg};

/// Board color theme — also controls the entire GUI palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BoardTheme {
    Classic,
    Emerald,
    Ocean,
    Royal,
    Walnut,
    Midnight,
    Forest,
    Sakura,
}

impl std::fmt::Display for BoardTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Classic => write!(f, "Classic"),
            Self::Emerald => write!(f, "Emerald"),
            Self::Ocean => write!(f, "Ocean"),
            Self::Royal => write!(f, "Royal"),
            Self::Walnut => write!(f, "Walnut"),
            Self::Midnight => write!(f, "Midnight"),
            Self::Forest => write!(f, "Forest"),
            Self::Sakura => write!(f, "Sakura"),
        }
    }
}

/// Full GUI color palette so themes control the entire app.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct GuiPalette {
    pub bg: Color,
    pub panel: Color,
    pub sidebar: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub accent: Color,
    pub accent_alt: Color,
    pub border: Color,
}

impl BoardTheme {
    pub const ALL: [BoardTheme; 8] = [
        BoardTheme::Classic,
        BoardTheme::Emerald,
        BoardTheme::Ocean,
        BoardTheme::Royal,
        BoardTheme::Walnut,
        BoardTheme::Midnight,
        BoardTheme::Forest,
        BoardTheme::Sakura,
    ];

    /// GUI palette for the entire application.
    pub fn gui_palette(self) -> GuiPalette {
        match self {
            BoardTheme::Classic | BoardTheme::Walnut => GuiPalette {
                bg: Color::from_rgb(0.102, 0.102, 0.180),
                panel: Color::from_rgb(0.086, 0.129, 0.243),
                sidebar: Color::from_rgb(0.059, 0.204, 0.376),
                text_primary: Color::from_rgb(0.96, 0.96, 0.96),
                text_secondary: Color::from_rgb(0.627, 0.627, 0.690),
                accent: Color::from_rgb(0.914, 0.271, 0.376),
                accent_alt: Color::from_rgb(0.325, 0.749, 0.616),
                border: Color::from_rgb(0.16, 0.18, 0.28),
            },
            BoardTheme::Emerald | BoardTheme::Forest => GuiPalette {
                bg: Color::from_rgb(0.067, 0.118, 0.086),
                panel: Color::from_rgb(0.090, 0.157, 0.114),
                sidebar: Color::from_rgb(0.118, 0.216, 0.153),
                text_primary: Color::from_rgb(0.92, 0.96, 0.93),
                text_secondary: Color::from_rgb(0.55, 0.65, 0.58),
                accent: Color::from_rgb(0.325, 0.749, 0.416),
                accent_alt: Color::from_rgb(0.706, 0.569, 0.235),
                border: Color::from_rgb(0.14, 0.22, 0.16),
            },
            BoardTheme::Ocean => GuiPalette {
                bg: Color::from_rgb(0.059, 0.094, 0.149),
                panel: Color::from_rgb(0.078, 0.129, 0.204),
                sidebar: Color::from_rgb(0.098, 0.176, 0.290),
                text_primary: Color::from_rgb(0.92, 0.95, 0.98),
                text_secondary: Color::from_rgb(0.55, 0.63, 0.72),
                accent: Color::from_rgb(0.357, 0.645, 0.949),
                accent_alt: Color::from_rgb(0.325, 0.749, 0.616),
                border: Color::from_rgb(0.12, 0.18, 0.27),
            },
            BoardTheme::Royal => GuiPalette {
                bg: Color::from_rgb(0.110, 0.075, 0.165),
                panel: Color::from_rgb(0.145, 0.102, 0.220),
                sidebar: Color::from_rgb(0.188, 0.133, 0.290),
                text_primary: Color::from_rgb(0.95, 0.93, 0.97),
                text_secondary: Color::from_rgb(0.62, 0.58, 0.68),
                accent: Color::from_rgb(0.700, 0.400, 0.900),
                accent_alt: Color::from_rgb(0.914, 0.271, 0.576),
                border: Color::from_rgb(0.18, 0.14, 0.26),
            },
            BoardTheme::Midnight => GuiPalette {
                bg: Color::from_rgb(0.047, 0.047, 0.082),
                panel: Color::from_rgb(0.063, 0.067, 0.118),
                sidebar: Color::from_rgb(0.082, 0.090, 0.161),
                text_primary: Color::from_rgb(0.90, 0.92, 0.96),
                text_secondary: Color::from_rgb(0.50, 0.53, 0.62),
                accent: Color::from_rgb(0.400, 0.600, 1.000),
                accent_alt: Color::from_rgb(0.914, 0.271, 0.376),
                border: Color::from_rgb(0.10, 0.11, 0.18),
            },
            BoardTheme::Sakura => GuiPalette {
                bg: Color::from_rgb(0.141, 0.082, 0.106),
                panel: Color::from_rgb(0.188, 0.110, 0.145),
                sidebar: Color::from_rgb(0.243, 0.149, 0.192),
                text_primary: Color::from_rgb(0.98, 0.93, 0.95),
                text_secondary: Color::from_rgb(0.68, 0.56, 0.62),
                accent: Color::from_rgb(0.957, 0.400, 0.600),
                accent_alt: Color::from_rgb(0.706, 0.569, 0.235),
                border: Color::from_rgb(0.22, 0.14, 0.18),
            },
        }
    }

    /// (light_square, dark_square, selected, last_light, last_dark,
    ///  legal_light, legal_dark, coord_on_light, coord_on_dark)
    fn colors(self) -> ThemeColors {
        match self {
            BoardTheme::Classic => ThemeColors {
                light: Color::from_rgb(0.941, 0.851, 0.710),
                dark: Color::from_rgb(0.710, 0.533, 0.388),
                selected: Color::from_rgb(0.510, 0.592, 0.412),
                last_light: Color::from_rgb(0.969, 0.969, 0.514),
                last_dark: Color::from_rgb(0.855, 0.824, 0.459),
                legal_light: Color::from_rgb(0.820, 0.878, 0.600),
                legal_dark: Color::from_rgb(0.680, 0.753, 0.490),
            },
            BoardTheme::Emerald => ThemeColors {
                light: Color::from_rgb(0.933, 0.933, 0.824),
                dark: Color::from_rgb(0.463, 0.588, 0.337),
                selected: Color::from_rgb(0.725, 0.769, 0.286),
                last_light: Color::from_rgb(0.957, 0.969, 0.580),
                last_dark: Color::from_rgb(0.690, 0.780, 0.412),
                legal_light: Color::from_rgb(0.820, 0.878, 0.600),
                legal_dark: Color::from_rgb(0.580, 0.700, 0.420),
            },
            BoardTheme::Ocean => ThemeColors {
                light: Color::from_rgb(0.871, 0.890, 0.902),
                dark: Color::from_rgb(0.357, 0.545, 0.749),
                selected: Color::from_rgb(0.400, 0.580, 0.800),
                last_light: Color::from_rgb(0.690, 0.830, 0.957),
                last_dark: Color::from_rgb(0.380, 0.580, 0.780),
                legal_light: Color::from_rgb(0.750, 0.870, 0.920),
                legal_dark: Color::from_rgb(0.450, 0.620, 0.780),
            },
            BoardTheme::Royal => ThemeColors {
                light: Color::from_rgb(0.910, 0.855, 0.965),
                dark: Color::from_rgb(0.608, 0.447, 0.812),
                selected: Color::from_rgb(0.700, 0.500, 0.850),
                last_light: Color::from_rgb(0.870, 0.780, 0.960),
                last_dark: Color::from_rgb(0.650, 0.480, 0.830),
                legal_light: Color::from_rgb(0.860, 0.820, 0.940),
                legal_dark: Color::from_rgb(0.630, 0.520, 0.800),
            },
            BoardTheme::Walnut => ThemeColors {
                light: Color::from_rgb(0.941, 0.824, 0.706),
                dark: Color::from_rgb(0.627, 0.408, 0.251),
                selected: Color::from_rgb(0.510, 0.443, 0.322),
                last_light: Color::from_rgb(0.957, 0.890, 0.710),
                last_dark: Color::from_rgb(0.690, 0.530, 0.350),
                legal_light: Color::from_rgb(0.880, 0.830, 0.650),
                legal_dark: Color::from_rgb(0.600, 0.500, 0.350),
            },
            BoardTheme::Midnight => ThemeColors {
                light: Color::from_rgb(0.780, 0.800, 0.840),
                dark: Color::from_rgb(0.290, 0.330, 0.440),
                selected: Color::from_rgb(0.400, 0.500, 0.700),
                last_light: Color::from_rgb(0.650, 0.720, 0.880),
                last_dark: Color::from_rgb(0.350, 0.420, 0.580),
                legal_light: Color::from_rgb(0.700, 0.760, 0.860),
                legal_dark: Color::from_rgb(0.380, 0.440, 0.580),
            },
            BoardTheme::Forest => ThemeColors {
                light: Color::from_rgb(0.878, 0.910, 0.859),
                dark: Color::from_rgb(0.337, 0.463, 0.325),
                selected: Color::from_rgb(0.500, 0.650, 0.350),
                last_light: Color::from_rgb(0.820, 0.900, 0.700),
                last_dark: Color::from_rgb(0.420, 0.560, 0.380),
                legal_light: Color::from_rgb(0.800, 0.870, 0.700),
                legal_dark: Color::from_rgb(0.400, 0.530, 0.380),
            },
            BoardTheme::Sakura => ThemeColors {
                light: Color::from_rgb(0.965, 0.882, 0.910),
                dark: Color::from_rgb(0.750, 0.450, 0.550),
                selected: Color::from_rgb(0.850, 0.400, 0.550),
                last_light: Color::from_rgb(0.950, 0.800, 0.860),
                last_dark: Color::from_rgb(0.780, 0.500, 0.600),
                legal_light: Color::from_rgb(0.930, 0.820, 0.870),
                legal_dark: Color::from_rgb(0.720, 0.480, 0.570),
            },
        }
    }
}

struct ThemeColors {
    light: Color,
    dark: Color,
    selected: Color,
    last_light: Color,
    last_dark: Color,
    legal_light: Color,
    legal_dark: Color,
}

impl ThemeColors {
    /// Coordinate color: contrasts with the square.
    fn coord_color(&self, is_light: bool) -> Color {
        if is_light {
            // Dark text on light square
            Color::from_rgba(self.dark.r, self.dark.g, self.dark.b, 0.8)
        } else {
            Color::from_rgba(self.light.r, self.light.g, self.light.b, 0.8)
        }
    }
}

/// Compact animation data (Copy for easy passing).
#[derive(Clone, Copy)]
pub struct AnimInfo {
    pub from_sq: types::Square,
    pub to_sq: types::Square,
    pub _piece: types::Piece,
    pub _color: types::Color,
    pub progress: f32,
    pub captured: Option<(types::Piece, types::Color)>,
    pub is_capture: bool,
}

/// Renders the chess board as an 8x8 grid with dynamic square size.
pub fn view_board<'a>(
    gs: &'a GameState,
    assets: &'a PieceAssets,
    sq_size: f32,
    anim: Option<AnimInfo>,
    theme: BoardTheme,
    show_coords: bool,
    coord_position: CoordPosition,
    capture_anim_style: CaptureAnimStyle,
) -> Element<'a, Msg> {
    let colors = theme.colors();
    let dot_size = sq_size * 0.28;
    let dot_radius = sq_size * 0.14;
    let piece_sz_normal = sq_size * 0.88;
    let piece_sz_pawn = sq_size * 0.70;

    // Premoved squares for highlighting
    let premove_sqs: Vec<types::Square> = gs
        .premove_queue
        .iter()
        .flat_map(|(from, to)| vec![*from, *to])
        .collect();

    // Only show coords inside the board if position is Inside
    let show_inside_coords = show_coords && coord_position == CoordPosition::Inside;

    let files = if gs.flipped {
        ['h', 'g', 'f', 'e', 'd', 'c', 'b', 'a']
    } else {
        ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']
    };

    let mut board_col = column![].spacing(0);

    for display_row in 0..8 {
        let rank = if gs.flipped {
            display_row
        } else {
            7 - display_row
        };
        let mut rank_row = row![].spacing(0);

        for display_col in 0..8 {
            let file = if gs.flipped {
                7 - display_col
            } else {
                display_col
            };
            let sq_index = rank * 8 + file;
            let sq = types::Square::from_index(sq_index);

            let is_light = (rank + file) % 2 != 0;
            let is_selected = gs.selected_square == Some(sq);
            let is_highlight = gs.legal_highlights.contains(&sq);
            let is_last_move = gs.last_move_squares.contains(&sq);
            let is_premove = premove_sqs.contains(&sq);

            // Capture flash effect
            let capture_flash = if capture_anim_style == CaptureAnimStyle::Explosion {
                anim.and_then(|a| {
                    if a.is_capture && a.to_sq == sq && a.progress < 0.3 {
                        Some(1.0 - (a.progress / 0.3))
                    } else {
                        None
                    }
                })
            } else {
                None
            };

            let bg_color = if let Some(flash) = capture_flash {
                let base = if is_light { colors.light } else { colors.dark };
                Color::from_rgb(
                    (base.r + flash * 0.6).min(1.0),
                    (base.g + flash * 0.6).min(1.0),
                    (base.b + flash * 0.4).min(1.0),
                )
            } else if is_premove {
                // Blue-ish tint for premoved squares
                let base = if is_light { colors.light } else { colors.dark };
                Color::from_rgb(
                    (base.r * 0.6 + 0.15).min(1.0),
                    (base.g * 0.6 + 0.20).min(1.0),
                    (base.b * 0.6 + 0.45).min(1.0),
                )
            } else if is_selected {
                colors.selected
            } else if is_highlight {
                if is_light {
                    colors.legal_light
                } else {
                    colors.legal_dark
                }
            } else if is_last_move {
                if is_light {
                    colors.last_light
                } else {
                    colors.last_dark
                }
            } else if is_light {
                colors.light
            } else {
                colors.dark
            };

            let coord_color = colors.coord_color(is_light);

            let hide_piece = anim.is_some_and(|a| a.from_sq == sq);
            let fading_capture = anim.and_then(|a| {
                if a.to_sq == sq {
                    a.captured.map(|(p, c)| (p, c, a.progress))
                } else {
                    None
                }
            });

            let cell_content: Element<'a, Msg> =
                if let Some((fade_piece, fade_color, progress)) = fading_capture {
                    match capture_anim_style {
                        CaptureAnimStyle::Instant => {
                            // Don't render the captured piece at all — it disappears instantly
                            text("").into()
                        }
                        CaptureAnimStyle::Explosion => {
                            let handle = assets.get(fade_piece, fade_color);
                            let base_sz = if fade_piece == types::Piece::Pawn {
                                piece_sz_pawn
                            } else {
                                piece_sz_normal
                            };
                            // Explosion: burst outward then fade away
                            let scale = if progress < 0.15 {
                                1.0 + (progress / 0.15) * 0.5
                            } else {
                                (1.5 * (1.0 - (progress - 0.15) / 0.85)).max(0.0)
                            };
                            let piece_sz = base_sz * scale;
                            let opacity = (1.0 - progress * 1.5).max(0.0);

                            container(
                                Image::new(handle.clone())
                                    .width(piece_sz)
                                    .height(piece_sz)
                                    .opacity(opacity),
                            )
                            .center_x(sq_size)
                            .center_y(sq_size)
                            .into()
                        }
                    }
                } else if !hide_piece {
                    if let Some((piece, color)) = gs.board.piece_on(sq) {
                        let handle = assets.get(piece, color);
                        let piece_sz = if piece == types::Piece::Pawn {
                            piece_sz_pawn
                        } else {
                            piece_sz_normal
                        };
                        container(Image::new(handle.clone()).width(piece_sz).height(piece_sz))
                            .center_x(sq_size)
                            .center_y(sq_size)
                            .into()
                    } else if is_highlight {
                        let dot_br: iced::border::Radius = dot_radius.into();
                        container(container(text("")).width(dot_size).height(dot_size).style(
                            move |_theme| container::Style {
                                background: Some(iced::Background::Color(Color::from_rgba(
                                    0.0, 0.0, 0.0, 0.18,
                                ))),
                                border: iced::Border {
                                    radius: dot_br,
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                        ))
                        .center_x(sq_size)
                        .center_y(sq_size)
                        .into()
                    } else {
                        text("").into()
                    }
                } else {
                    text("").into()
                };

            // Coordinate labels inside the board (only if show_inside_coords)
            let has_rank_label = show_inside_coords && display_col == 0;
            let has_file_label = show_inside_coords && display_row == 7;

            let cell_with_coords: Element<'a, Msg> = if has_rank_label || has_file_label {
                let mut overlay_col = column![].width(sq_size).height(sq_size);

                if has_rank_label {
                    overlay_col = overlay_col
                        .push(text(format!(" {}", rank + 1)).size(10).color(coord_color));
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

                container(overlay_col).width(sq_size).height(sq_size).into()
            } else {
                container(cell_content)
                    .center_x(sq_size)
                    .center_y(sq_size)
                    .width(sq_size)
                    .height(sq_size)
                    .into()
            };

            let cell = button(container(cell_with_coords).width(sq_size).height(sq_size))
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

            // Wrap with mouse_area for right-click arrow drawing
            let cell_with_right_click = mouse_area(cell)
                .on_right_press(Msg::BoardRightDown(display_row, display_col))
                .on_right_release(Msg::BoardRightUp(display_row, display_col));

            rank_row = rank_row.push(cell_with_right_click);
        }

        board_col = board_col.push(rank_row);
    }

    // Board frame color matches theme
    let frame_color = Color::from_rgb(
        colors.dark.r * 0.5,
        colors.dark.g * 0.5,
        colors.dark.b * 0.5,
    );

    // Build the board with optional outside coordinates
    let board_elem: Element<'a, Msg> = container(board_col)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(frame_color)),
            border: iced::Border {
                radius: 4.0.into(),
                width: 3.0,
                color: frame_color,
            },
            ..Default::default()
        })
        .into();

    if show_coords && coord_position == CoordPosition::Outside {
        let coord_label_color = colors.coord_color(true);

        // Left rank labels
        let mut rank_labels = column![].spacing(0);
        for display_row in 0..8 {
            let rank = if gs.flipped {
                display_row
            } else {
                7 - display_row
            };
            rank_labels = rank_labels.push(
                container(
                    text(format!("{}", rank + 1))
                        .size(11)
                        .color(coord_label_color),
                )
                .height(sq_size)
                .center_y(sq_size)
                .width(20)
                .align_x(Alignment::End),
            );
        }

        // Bottom file labels
        let mut file_labels = row![].spacing(0);
        file_labels = file_labels.push(iced::widget::Space::new().width(20)); // offset for rank column
        for display_col in 0..8 {
            file_labels = file_labels.push(
                container(
                    text(format!("{}", files[display_col]))
                        .size(11)
                        .color(coord_label_color),
                )
                .width(sq_size)
                .center_x(sq_size)
                .height(20),
            );
        }

        // Assemble: rank labels | board, file labels below
        column![
            row![rank_labels, board_elem]
                .spacing(0)
                .align_y(Alignment::Start),
            file_labels,
        ]
        .spacing(0)
        .into()
    } else {
        // Arrow overlay hint (arrows drawn as colored text markers on endpoints)
        // Note: Full arrow rendering would need canvas; we mark endpoints for now
        if !gs.arrows.is_empty() {
            // Arrows are stored but iced's widget system can't easily overlay SVG arrows
            // They're tracked in state for future canvas rendering
        }
        board_elem
    }
}
