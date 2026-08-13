//! Annotation arrow types and geometry (framework-free).

use mujrim_study::board_marks::{ArrowRole, BoardArrow, MarkColor};

use super::palette::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArrowShape {
    Smart,
    Straight,
}

impl ArrowShape {
    pub const ALL: [Self; 2] = [Self::Smart, Self::Straight];
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
    pub const ALL: [Self; 4] = [Self::Orange, Self::Green, Self::Blue, Self::Red];

    pub const fn to_mark(self) -> MarkColor {
        match self {
            Self::Orange => MarkColor::Orange,
            Self::Green => MarkColor::Green,
            Self::Blue => MarkColor::Blue,
            Self::Red => MarkColor::Red,
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
    pub const ALL: [Self; 3] = [Self::Slim, Self::Normal, Self::Bold];

    pub const fn scale(self) -> f32 {
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

impl ArrowAppearance {
    pub const fn user_color(self) -> ArrowColor {
        self.color
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

pub fn mark_colors(color: MarkColor, opacity: f32) -> (Rgba, Rgba) {
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
    let fill = Rgba::rgba(r, g, b, opacity);
    let outline = Rgba::rgba(
        (r * 0.72).clamp(0.0, 1.0),
        (g * 0.72).clamp(0.0, 1.0),
        (b * 0.72).clamp(0.0, 1.0),
        (opacity + 0.10).min(1.0),
    );
    (fill, outline)
}

pub fn is_knight_move(from_file: i32, from_rank: i32, to_file: i32, to_rank: i32) -> bool {
    let df = (from_file - to_file).abs();
    let dr = (from_rank - to_rank).abs();
    (df == 1 && dr == 2) || (df == 2 && dr == 1)
}

pub fn sq_center(file: u8, rank: u8, sq_size: f32, flipped: bool) -> Point {
    let display_col = if flipped { 7 - file } else { file } as f32;
    let display_row = if flipped { rank } else { 7 - rank } as f32;
    Point::new(
        display_col * sq_size + sq_size * 0.5,
        display_row * sq_size + sq_size * 0.5,
    )
}

pub fn display_square_at(x: f32, y: f32, sq_size: f32) -> Option<(usize, usize)> {
    super::game::point_to_display(x, y, sq_size)
}

pub fn user_arrow(from: types::Square, to: types::Square, color: ArrowColor) -> BoardArrow {
    BoardArrow::new(from, to, color.to_mark(), ArrowRole::User)
}

#[derive(Debug, Clone, Copy)]
pub struct Poly {
    pub points: [Point; 4],
}

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub a: Point,
    pub b: Point,
    pub c: Point,
}

#[derive(Debug, Clone)]
pub struct ArrowGeometry {
    pub shaft: Poly,
    pub head: Triangle,
    pub fill: Rgba,
    pub outline: Rgba,
    pub step: Option<(Point, u8, Rgba)>,
}

pub fn arrow_geometry(
    arrow: &BoardArrow,
    sq_size: f32,
    flipped: bool,
    appearance: ArrowAppearance,
) -> Vec<ArrowGeometry> {
    let from_file = arrow.from.file() as i32;
    let from_rank = arrow.from.rank() as i32;
    let to_file = arrow.to.file() as i32;
    let to_rank = arrow.to.rank() as i32;
    let color = if arrow.role == ArrowRole::User {
        appearance.user_color().to_mark()
    } else {
        arrow.color
    };
    let (fill, outline) = mark_colors(color, arrow.resolved_opacity());
    let scale = appearance.size.scale();
    let mut out = Vec::new();
    if appearance.shape == ArrowShape::Smart
        && is_knight_move(from_file, from_rank, to_file, to_rank)
    {
        out.extend(knight_geometry(
            arrow.from, arrow.to, sq_size, flipped, scale, fill, outline,
        ));
    } else {
        let p_from = sq_center(arrow.from.file(), arrow.from.rank(), sq_size, flipped);
        let p_to = sq_center(arrow.to.file(), arrow.to.rank(), sq_size, flipped);
        if let Some(geom) = straight_geometry(p_from, p_to, sq_size, scale, fill, outline) {
            out.push(geom);
        }
    }
    if let Some(step) = arrow.step.filter(|_| arrow.role.shows_step()) {
        let tip = sq_center(arrow.to.file(), arrow.to.rank(), sq_size, flipped);
        if let Some(last) = out.last_mut() {
            last.step = Some((
                Point::new(tip.x + sq_size * 0.18, tip.y - sq_size * 0.18),
                step,
                fill,
            ));
        }
    }
    out
}

fn straight_geometry(
    from: Point,
    to: Point,
    sq_size: f32,
    scale: f32,
    fill: Rgba,
    outline: Rgba,
) -> Option<ArrowGeometry> {
    let shaft_w = sq_size * 0.25 * scale;
    let head_len = sq_size * 0.40;
    let head_half_w = shaft_w * 1.2;
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return None;
    }
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;
    let shaft_end_x = to.x - ux * head_len;
    let shaft_end_y = to.y - uy * head_len;
    Some(ArrowGeometry {
        shaft: Poly {
            points: [
                Point::new(from.x + px * shaft_w * 0.5, from.y + py * shaft_w * 0.5),
                Point::new(
                    shaft_end_x + px * shaft_w * 0.5,
                    shaft_end_y + py * shaft_w * 0.5,
                ),
                Point::new(
                    shaft_end_x - px * shaft_w * 0.5,
                    shaft_end_y - py * shaft_w * 0.5,
                ),
                Point::new(from.x - px * shaft_w * 0.5, from.y - py * shaft_w * 0.5),
            ],
        },
        head: Triangle {
            a: to,
            b: Point::new(
                shaft_end_x + px * head_half_w,
                shaft_end_y + py * head_half_w,
            ),
            c: Point::new(
                shaft_end_x - px * head_half_w,
                shaft_end_y - py * head_half_w,
            ),
        },
        fill,
        outline,
        step: None,
    })
}

fn knight_geometry(
    from_square: types::Square,
    to_square: types::Square,
    sq_size: f32,
    flipped: bool,
    scale: f32,
    fill: Rgba,
    outline: Rgba,
) -> Vec<ArrowGeometry> {
    let from_file = from_square.file() as i32;
    let from_rank = from_square.rank() as i32;
    let to_file = to_square.file() as i32;
    let to_rank = to_square.rank() as i32;
    let from = sq_center(from_square.file(), from_square.rank(), sq_size, flipped);
    let to = sq_center(to_square.file(), to_square.rank(), sq_size, flipped);
    let df = (to_file - from_file).abs();
    let dr = (to_rank - from_rank).abs();
    let (corner_file, corner_rank) = if df > dr {
        (to_file, from_rank)
    } else {
        (from_file, to_rank)
    };
    let corner = sq_center(corner_file as u8, corner_rank as u8, sq_size, flipped);
    let mut out = Vec::new();
    if let Some(leg) = leg_geometry(from, corner, sq_size * 0.25 * scale, fill, outline) {
        out.push(leg);
    }
    if let Some(head) = straight_geometry(corner, to, sq_size, scale, fill, outline) {
        out.push(head);
    }
    out
}

fn leg_geometry(
    from: Point,
    to: Point,
    shaft_w: f32,
    fill: Rgba,
    outline: Rgba,
) -> Option<ArrowGeometry> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return None;
    }
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py = ux;
    let ext = shaft_w * 0.5;
    let to_ext = Point::new(to.x + ux * ext, to.y + uy * ext);
    Some(ArrowGeometry {
        shaft: Poly {
            points: [
                Point::new(from.x + px * shaft_w * 0.5, from.y + py * shaft_w * 0.5),
                Point::new(to_ext.x + px * shaft_w * 0.5, to_ext.y + py * shaft_w * 0.5),
                Point::new(to_ext.x - px * shaft_w * 0.5, to_ext.y - py * shaft_w * 0.5),
                Point::new(from.x - px * shaft_w * 0.5, from.y - py * shaft_w * 0.5),
            ],
        },
        head: Triangle {
            a: to,
            b: to,
            c: to,
        },
        fill,
        outline,
        step: None,
    })
}

/// Eval-graph polyline in unit space (x in 0..width, y in 0..height).
pub fn eval_graph_points(scores: &[Option<i32>], width: f32, height: f32) -> Vec<Point> {
    scores
        .iter()
        .enumerate()
        .filter_map(|(index, score)| {
            let score = (*score)?;
            let x = if scores.len() <= 1 {
                width * 0.5
            } else {
                index as f32 * width / (scores.len() - 1) as f32
            };
            let normalized = (score as f32 / 450.0).tanh();
            let y = height * (0.5 - normalized * 0.44);
            Some(Point::new(x, y))
        })
        .collect()
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
        assert_eq!(display_square_at(sq * 0.5, sq * 0.5, sq), Some((0, 0)));
        assert_eq!(display_square_at(-1.0, 10.0, sq), None);
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

    #[test]
    fn eval_graph_accepts_sparse_and_decisive_scores() {
        let scores = [None, Some(0), Some(42), Some(30_000), Some(-30_000)];
        let points = eval_graph_points(&scores, 100.0, 40.0);
        assert_eq!(points.len(), 4);
        assert!((30_000_f32 / 450.0).tanh() <= 1.0);
    }

    #[test]
    fn smart_knight_arrow_emits_geometry() {
        let arrow = user_arrow(
            types::Square::from_index(6),
            types::Square::from_index(21),
            ArrowColor::Orange,
        );
        let geom = arrow_geometry(
            &arrow,
            64.0,
            false,
            ArrowAppearance {
                shape: ArrowShape::Smart,
                color: ArrowColor::Orange,
                size: ArrowSize::Normal,
            },
        );
        assert!(!geom.is_empty());
    }
}
