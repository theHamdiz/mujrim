//! Canvas-based arrow overlay for chess.com-style annotation arrows.
//!
//! Draws semi-transparent orange arrows between square centers.
//! Supports straight arrows and L-shaped knight-move arrows.

use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Path, Stroke};
use iced::{Color, Element, Length, Point, mouse};

use crate::Msg;

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
    const fn colors(self) -> (Color, Color) {
        match self {
            Self::Orange => (
                Color::from_rgba(0.922, 0.478, 0.118, 0.80),
                Color::from_rgba(0.700, 0.350, 0.060, 0.90),
            ),
            Self::Green => (
                Color::from_rgba(0.20, 0.78, 0.42, 0.80),
                Color::from_rgba(0.08, 0.50, 0.24, 0.92),
            ),
            Self::Blue => (
                Color::from_rgba(0.20, 0.55, 0.96, 0.80),
                Color::from_rgba(0.08, 0.32, 0.72, 0.92),
            ),
            Self::Red => (
                Color::from_rgba(0.92, 0.22, 0.25, 0.80),
                Color::from_rgba(0.66, 0.08, 0.10, 0.92),
            ),
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
    pub color: ArrowColor,
    pub size: ArrowSize,
}

/// Returns true if the move between `from` and `to` is a knight jump.
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

/// Renders all annotation arrows as a transparent canvas overlay.
pub fn arrow_canvas<'a>(
    arrows: &[(types::Square, types::Square)],
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
) -> Element<'a, Msg> {
    let board_px = sq_size * 8.0;
    let overlay = ArrowOverlay {
        arrows: arrows.to_vec(),
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
    arrows: Vec<(types::Square, types::Square)>,
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
    cache: Cache,
}

impl canvas::Program<Msg> for ArrowOverlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geom = self.cache.draw(renderer, bounds.size(), |frame| {
            for &(from, to) in &self.arrows {
                let from_file = from.file() as i32;
                let from_rank = from.rank() as i32;
                let to_file = to.file() as i32;
                let to_rank = to.rank() as i32;

                let p_from = sq_center(from.file(), from.rank(), self.sq_size, self.flipped);
                let p_to = sq_center(to.file(), to.rank(), self.sq_size, self.flipped);

                let (fill, outline) = self.appearance.color.colors();
                let scale = self.appearance.size.scale();
                if self.appearance.shape == ArrowShape::Smart
                    && is_knight_move(from_file, from_rank, to_file, to_rank)
                {
                    draw_knight_arrow(
                        frame,
                        from,
                        to,
                        self.sq_size,
                        self.flipped,
                        scale,
                        fill,
                        outline,
                    );
                } else {
                    draw_straight_arrow(frame, p_from, p_to, self.sq_size, scale, fill, outline);
                }
            }
        });
        vec![geom]
    }
}

/// Draws a straight arrow from `from` to `to` with a fat shaft and triangular head.
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
    // Unit direction
    let ux = dx / len;
    let uy = dy / len;
    // Perpendicular
    let px = -uy;
    let py = ux;

    // The shaft ends where the arrowhead base begins
    let shaft_end_x = to.x - ux * head_len;
    let shaft_end_y = to.y - uy * head_len;

    // Shaft rectangle (4 corners)
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

    // Arrowhead triangle
    let h_tip = to;
    let h_left = Point::new(
        shaft_end_x + px * head_half_w,
        shaft_end_y + py * head_half_w,
    );
    let h_right = Point::new(
        shaft_end_x - px * head_half_w,
        shaft_end_y - py * head_half_w,
    );

    // Draw filled shaft
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

    // Draw filled arrowhead
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

/// Draws an L-shaped arrow for knight moves: horizontal leg then vertical leg.
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

    // Determine which leg is longer to figure out the correct L-shape.
    // Convention: go in the direction of larger displacement first.
    let df = (to_file - from_file).abs();
    let dr = (to_rank - from_rank).abs();

    // The corner square of the L: first go along the longer axis
    let (corner_file, corner_rank) = if df > dr {
        // horizontal first (2 squares), then vertical (1 square)
        (to_file, from_rank)
    } else {
        // vertical first (2 squares), then horizontal (1 square)
        (from_file, to_rank)
    };

    let corner = sq_center(corner_file as u8, corner_rank as u8, sq_size, flipped);

    // --- First leg: from → corner (no arrowhead, just a fat line) ---
    draw_leg_segment(frame, from, corner, shaft_w, fill, outline);

    // --- Second leg: corner → to (with arrowhead) ---
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

    // Shaft of second leg
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

    // Arrowhead
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

/// Draws a simple fat line segment (no arrowhead) for the first leg of an L-arrow.
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

    // Extend past the corner by half the shaft width so the join looks clean
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
        // A1 = file 0, rank 0 → display_col 0, display_row 7 → center (40, 600 - 40)
        let p = sq_center(0, 0, sq_size, false);
        assert!((p.x - 40.0).abs() < 0.01);
        assert!((p.y - (7.0 * 80.0 + 40.0)).abs() < 0.01);

        // H8 = file 7, rank 7 → display_col 7, display_row 0 → center (600 - 40, 40)
        let p = sq_center(7, 7, sq_size, false);
        assert!((p.x - (7.0 * 80.0 + 40.0)).abs() < 0.01);
        assert!((p.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_sq_center_flipped() {
        let sq_size = 80.0;
        // A1 flipped → display_col 7, display_row 0
        let p = sq_center(0, 0, sq_size, true);
        assert!((p.x - (7.0 * 80.0 + 40.0)).abs() < 0.01);
        assert!((p.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_is_knight_move() {
        // b1→c3: df=1, dr=2
        assert!(is_knight_move(1, 0, 2, 2));
        // g1→f3: df=1, dr=2
        assert!(is_knight_move(6, 0, 5, 2));
        // b1→a3: df=1, dr=2
        assert!(is_knight_move(1, 0, 0, 2));
        // Wide knight: e4→c5 df=2, dr=1
        assert!(is_knight_move(4, 3, 2, 4));
        // Not a knight: e2→e4 df=0, dr=2
        assert!(!is_knight_move(4, 1, 4, 3));
        // Not a knight: diagonal df=1, dr=1
        assert!(!is_knight_move(3, 3, 4, 4));
    }

    #[test]
    fn test_arrowhead_geometry() {
        // A straight arrow from center of board going right
        let from = Point::new(200.0, 320.0);
        let to = Point::new(360.0, 320.0);
        let dx: f32 = to.x - from.x;
        let dy: f32 = to.y - from.y;
        let len: f32 = (dx * dx + dy * dy).sqrt();
        assert!(len > 0.0);
        let ux: f32 = dx / len;
        let uy: f32 = dy / len;
        // Direction should be purely horizontal
        assert!((ux - 1.0).abs() < 0.01);
        assert!(uy.abs() < 0.01);
    }
}
