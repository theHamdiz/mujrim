//! Canvas-based annotation arrows with numbered multi-color steps.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Event, Length, Point, mouse};

use mujrim_study::board_marks::{ArrowRole, BoardArrow, MarkColor};

use super::app::Msg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArrowShape {
    Smart,
    Straight,
}

impl std::fmt::Display for ArrowShape {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Smart => "Smart / Knight L",
            Self::Straight => "Straight",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArrowColor {
    Orange,
    Green,
    Blue,
    Red,
}

impl ArrowColor {
    pub const fn to_mark(self) -> MarkColor {
        match self {
            Self::Orange => MarkColor::Orange,
            Self::Green => MarkColor::Green,
            Self::Blue => MarkColor::Blue,
            Self::Red => MarkColor::Red,
        }
    }
}

impl From<ArrowColor> for crate::app_core::arrows::ArrowColor {
    fn from(color: ArrowColor) -> Self {
        match color {
            ArrowColor::Orange => Self::Orange,
            ArrowColor::Green => Self::Green,
            ArrowColor::Blue => Self::Blue,
            ArrowColor::Red => Self::Red,
        }
    }
}

impl std::fmt::Display for ArrowColor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Orange => "Orange",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Red => "Red",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArrowSize {
    Slim,
    Normal,
    Bold,
}

impl ArrowSize {
    const fn scale(self) -> f32 {
        match self {
            Self::Slim => 0.68,
            Self::Normal => 1.0,
            Self::Bold => 1.35,
        }
    }
}

impl std::fmt::Display for ArrowSize {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Slim => "Slim",
            Self::Normal => "Normal",
            Self::Bold => "Bold",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArrowAppearance {
    pub shape: ArrowShape,
    /// Default color for newly drawn user arrows (applied at draw-time via settings).
    pub color: ArrowColor,
    pub size: ArrowSize,
}

impl ArrowAppearance {
    pub const fn user_color(self) -> ArrowColor {
        self.color
    }
}

fn mark_colors(color: MarkColor, opacity: f32) -> (Color, Color) {
    let (r, g, b) = match color {
        MarkColor::Orange => (0.922, 0.478, 0.118),
        MarkColor::Green => (0.20, 0.78, 0.42),
        MarkColor::Blue => (0.20, 0.55, 0.96),
        MarkColor::Red => (0.92, 0.22, 0.25),
        MarkColor::Purple => (0.70, 0.40, 0.90),
        MarkColor::Cyan => (0.20, 0.78, 0.86),
        MarkColor::Gold => (0.92, 0.78, 0.22),
        MarkColor::Gray => (0.62, 0.64, 0.70),
    };
    let fill = Color::from_rgba(r, g, b, opacity);
    let outline = Color::from_rgba(
        (r * 0.72).clamp(0.0, 1.0),
        (g * 0.72).clamp(0.0, 1.0),
        (b * 0.72).clamp(0.0, 1.0),
        (opacity + 0.10).min(1.0),
    );
    (fill, outline)
}

/// Returns true if the move between `from` and `to` is a knight jump.
#[allow(dead_code)]
pub fn is_knight_move(from_file: i32, from_rank: i32, to_file: i32, to_rank: i32) -> bool {
    let df = (from_file - to_file).abs();
    let dr = (from_rank - to_rank).abs();
    (df == 1 && dr == 2) || (df == 2 && dr == 1)
}

/// Pixel center of a square on the board.
pub fn sq_center(file: u8, rank: u8, sq_size: f32, flipped: bool) -> Point {
    let display_col = if flipped { 7 - file } else { file } as f32;
    let display_row = if flipped { rank } else { 7 - rank } as f32;
    Point::new(
        display_col * sq_size + sq_size * 0.5,
        display_row * sq_size + sq_size * 0.5,
    )
}

/// Maps a point inside the board canvas to a display-row/col used by board messages.
pub fn display_square_at(point: Point, sq_size: f32) -> Option<(usize, usize)> {
    crate::app_core::game::point_to_display(point.x, point.y, sq_size)
}

#[allow(dead_code)]
pub fn user_arrow(from: types::Square, to: types::Square, color: ArrowColor) -> BoardArrow {
    BoardArrow::new(from, to, color.to_mark(), ArrowRole::User)
}

/// Renders annotation arrows as a transparent canvas overlay.
pub fn arrow_canvas<'a>(
    overlay_arrows: &'a [BoardArrow],
    user_arrows: &'a [BoardArrow],
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
) -> Element<'a, Msg> {
    let board_px = sq_size * 8.0;
    let mut arrows = overlay_arrows.to_vec();
    arrows.extend(user_arrows.iter().cloned());
    let overlay = ArrowOverlay {
        arrows,
        sq_size,
        flipped,
        appearance,
        cache: Cache::new(),
    };
    Canvas::new(overlay)
        .width(Length::Fixed(board_px))
        .height(Length::Fixed(board_px))
        .into()
}

struct ArrowOverlay {
    arrows: Vec<BoardArrow>,
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
    cache: Cache,
}

impl canvas::Program<Msg> for ArrowOverlay {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Msg>> {
        let position = cursor.position_in(bounds)?;
        let (row, col) = display_square_at(position, self.sq_size)?;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                Some(canvas::Action::publish(Msg::BoardRightDown(row, col)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                Some(canvas::Action::publish(Msg::BoardRightUp(row, col)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                Some(canvas::Action::publish(Msg::BoardPointerDown(row, col)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(canvas::Action::publish(Msg::BoardPointerUp(row, col)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                Some(canvas::Action::publish(Msg::BoardPointerMove(row, col)))
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geom = self.cache.draw(renderer, bounds.size(), |frame| {
            for arrow in &self.arrows {
                draw_board_arrow(frame, arrow, self.sq_size, self.flipped, self.appearance);
            }
        });
        vec![geom]
    }
}

fn draw_board_arrow(
    frame: &mut Frame,
    arrow: &BoardArrow,
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
) {
    let p_from = sq_center(arrow.from.file(), arrow.from.rank(), sq_size, flipped);
    let p_to = sq_center(arrow.to.file(), arrow.to.rank(), sq_size, flipped);
    let color = if arrow.role == ArrowRole::User {
        appearance.user_color().to_mark()
    } else {
        arrow.color
    };
    let (fill, outline) = mark_colors(color, arrow.resolved_opacity());
    let scale = appearance.size.scale();
    let appearance_core = crate::app_core::arrows::ArrowAppearance {
        shape: match appearance.shape {
            ArrowShape::Smart => crate::app_core::arrows::ArrowShape::Smart,
            ArrowShape::Straight => crate::app_core::arrows::ArrowShape::Straight,
        },
        color: appearance.color.into(),
        size: match appearance.size {
            ArrowSize::Slim => crate::app_core::arrows::ArrowSize::Slim,
            ArrowSize::Normal => crate::app_core::arrows::ArrowSize::Normal,
            ArrowSize::Bold => crate::app_core::arrows::ArrowSize::Bold,
        },
    };
    for geom in crate::app_core::arrows::arrow_geometry(arrow, sq_size, flipped, appearance_core) {
        if let Some(body) = geom.body.filter(|body| body.len() >= 3) {
            let path = Path::new(|builder| {
                builder.move_to(Point::new(body[0].x, body[0].y));
                for point in &body[1..] {
                    builder.line_to(Point::new(point.x, point.y));
                }
                builder.close();
            });
            frame.fill(&path, fill);
            frame.stroke(&path, Stroke::default().with_color(outline).with_width(1.4));
        } else {
            draw_straight_arrow(frame, p_from, p_to, sq_size, scale, fill, outline);
        }
        if let Some(step) = arrow.step.filter(|_| arrow.role.shows_step()) {
            draw_step_badge(frame, p_to, sq_size, step, fill);
        }
    }
}

fn draw_step_badge(frame: &mut Frame, tip: Point, sq_size: f32, step: u8, fill: Color) {
    let radius = sq_size * 0.16;
    let center = Point::new(tip.x + sq_size * 0.18, tip.y - sq_size * 0.18);
    let badge = Path::circle(center, radius);
    frame.fill(&badge, Color::from_rgba(0.08, 0.08, 0.10, 0.82));
    frame.stroke(&badge, Stroke::default().with_color(fill).with_width(1.5));
    frame.fill_text(Text {
        content: step.to_string(),
        position: center,
        color: Color::WHITE,
        size: (sq_size * 0.18).into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

fn draw_straight_arrow(
    frame: &mut Frame,
    from: Point,
    to: Point,
    sq_size: f32,
    scale: f32,
    fill: Color,
    outline: Color,
) {
    let shaft_w = sq_size * 0.25 * scale;
    let head_len = sq_size * 0.40;
    let head_half_w = shaft_w * 1.2;

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;

    let shaft_end_x = to.x - ux * head_len;
    let shaft_end_y = to.y - uy * head_len;

    let s1 = Point::new(from.x + px * shaft_w * 0.5, from.y + py * shaft_w * 0.5);
    let s2 = Point::new(from.x - px * shaft_w * 0.5, from.y - py * shaft_w * 0.5);
    let s3 = Point::new(
        shaft_end_x - px * shaft_w * 0.5,
        shaft_end_y - py * shaft_w * 0.5,
    );
    let s4 = Point::new(
        shaft_end_x + px * shaft_w * 0.5,
        shaft_end_y + py * shaft_w * 0.5,
    );

    let h_tip = to;
    let h_left = Point::new(
        shaft_end_x + px * head_half_w,
        shaft_end_y + py * head_half_w,
    );
    let h_right = Point::new(
        shaft_end_x - px * head_half_w,
        shaft_end_y - py * head_half_w,
    );

    let shaft_path = Path::new(|b| {
        b.move_to(s1);
        b.line_to(s4);
        b.line_to(s3);
        b.line_to(s2);
        b.close();
    });
    frame.fill(&shaft_path, fill);
    frame.stroke(
        &shaft_path,
        Stroke::default().with_color(outline).with_width(1.2),
    );

    let head_path = Path::new(|b| {
        b.move_to(h_tip);
        b.line_to(h_left);
        b.line_to(h_right);
        b.close();
    });
    frame.fill(&head_path, fill);
    frame.stroke(
        &head_path,
        Stroke::default().with_color(outline).with_width(1.2),
    );
}

#[allow(dead_code, clippy::too_many_arguments)]
fn draw_knight_arrow(
    frame: &mut Frame,
    from_square: types::Square,
    to_square: types::Square,
    sq_size: f32,
    flipped: bool,
    scale: f32,
    fill: Color,
    outline: Color,
) {
    let from_file = from_square.file() as i32;
    let from_rank = from_square.rank() as i32;
    let to_file = to_square.file() as i32;
    let to_rank = to_square.rank() as i32;
    let from = sq_center(from_square.file(), from_square.rank(), sq_size, flipped);
    let to = sq_center(to_square.file(), to_square.rank(), sq_size, flipped);
    let shaft_w = sq_size * 0.25 * scale;
    let head_len = sq_size * 0.40;
    let head_half_w = shaft_w * 1.2;

    let df = (to_file - from_file).abs();
    let dr = (to_rank - from_rank).abs();
    let (corner_file, corner_rank) = if df > dr {
        (to_file, from_rank)
    } else {
        (from_file, to_rank)
    };
    let corner = sq_center(corner_file as u8, corner_rank as u8, sq_size, flipped);
    draw_leg_segment(frame, from, corner, shaft_w, fill, outline);

    let dx = to.x - corner.x;
    let dy = to.y - corner.y;
    let seg_len = (dx * dx + dy * dy).sqrt();
    if seg_len < 1.0 {
        return;
    }
    let ux = dx / seg_len;
    let uy = dy / seg_len;
    let px = -uy;
    let py = ux;
    let shaft_end_x = to.x - ux * head_len;
    let shaft_end_y = to.y - uy * head_len;

    let s1 = Point::new(corner.x + px * shaft_w * 0.5, corner.y + py * shaft_w * 0.5);
    let s2 = Point::new(corner.x - px * shaft_w * 0.5, corner.y - py * shaft_w * 0.5);
    let s3 = Point::new(
        shaft_end_x - px * shaft_w * 0.5,
        shaft_end_y - py * shaft_w * 0.5,
    );
    let s4 = Point::new(
        shaft_end_x + px * shaft_w * 0.5,
        shaft_end_y + py * shaft_w * 0.5,
    );

    let shaft_path = Path::new(|b| {
        b.move_to(s1);
        b.line_to(s4);
        b.line_to(s3);
        b.line_to(s2);
        b.close();
    });
    frame.fill(&shaft_path, fill);
    frame.stroke(
        &shaft_path,
        Stroke::default().with_color(outline).with_width(1.2),
    );

    let h_tip = to;
    let h_left = Point::new(
        shaft_end_x + px * head_half_w,
        shaft_end_y + py * head_half_w,
    );
    let h_right = Point::new(
        shaft_end_x - px * head_half_w,
        shaft_end_y - py * head_half_w,
    );
    let head_path = Path::new(|b| {
        b.move_to(h_tip);
        b.line_to(h_left);
        b.line_to(h_right);
        b.close();
    });
    frame.fill(&head_path, fill);
    frame.stroke(
        &head_path,
        Stroke::default().with_color(outline).with_width(1.2),
    );
}

#[allow(dead_code)]
fn draw_leg_segment(
    frame: &mut Frame,
    from: Point,
    to: Point,
    shaft_w: f32,
    fill: Color,
    outline: Color,
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;
    let ext = shaft_w * 0.5;
    let to_ext = Point::new(to.x + ux * ext, to.y + uy * ext);
    let s1 = Point::new(from.x + px * shaft_w * 0.5, from.y + py * shaft_w * 0.5);
    let s2 = Point::new(from.x - px * shaft_w * 0.5, from.y - py * shaft_w * 0.5);
    let s3 = Point::new(to_ext.x - px * shaft_w * 0.5, to_ext.y - py * shaft_w * 0.5);
    let s4 = Point::new(to_ext.x + px * shaft_w * 0.5, to_ext.y + py * shaft_w * 0.5);
    let path = Path::new(|b| {
        b.move_to(s1);
        b.line_to(s4);
        b.line_to(s3);
        b.line_to(s2);
        b.close();
    });
    frame.fill(&path, fill);
    frame.stroke(&path, Stroke::default().with_color(outline).with_width(1.2));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sq_center_not_flipped() {
        let sq_size = 80.0;
        let p = sq_center(0, 0, sq_size, false);
        assert!((p.x - 40.0).abs() < 0.01);
        assert!((p.y - (7.0 * 80.0 + 40.0)).abs() < 0.01);
        let p = sq_center(7, 7, sq_size, false);
        assert!((p.x - (7.0 * 80.0 + 40.0)).abs() < 0.01);
        assert!((p.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_sq_center_flipped() {
        let sq_size = 80.0;
        let p = sq_center(0, 0, sq_size, true);
        assert!((p.x - (7.0 * 80.0 + 40.0)).abs() < 0.01);
        assert!((p.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_is_knight_move() {
        assert!(is_knight_move(1, 0, 2, 2));
        assert!(is_knight_move(6, 0, 5, 2));
        assert!(!is_knight_move(4, 1, 4, 3));
    }

    #[test]
    fn display_square_at_maps_centers_to_display_grid() {
        let sq = 64.0;
        assert_eq!(
            display_square_at(Point::new(sq * 0.5, sq * 0.5), sq),
            Some((0, 0))
        );
        assert_eq!(display_square_at(Point::new(-1.0, 10.0), sq), None);
    }

    #[test]
    fn user_arrow_maps_settings_color() {
        let arrow = user_arrow(
            types::Square::from_index(12),
            types::Square::from_index(28),
            ArrowColor::Blue,
        );
        assert_eq!(arrow.color, MarkColor::Blue);
        assert_eq!(arrow.role, ArrowRole::User);
    }
}
