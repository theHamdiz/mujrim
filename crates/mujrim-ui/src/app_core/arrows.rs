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

pub fn step_badge_center(arrow: &BoardArrow, sq_size: f32, flipped: bool) -> Option<(Point, u8)> {
    let step = arrow.step.filter(|_| arrow.role.shows_step())?;
    let tip = sq_center(arrow.to.file(), arrow.to.rank(), sq_size, flipped);
    Some((
        Point::new(tip.x + sq_size * 0.18, tip.y - sq_size * 0.18),
        step,
    ))
}

pub fn hit_step_badge(
    arrows: &[BoardArrow],
    local_x: f32,
    local_y: f32,
    sq_size: f32,
    flipped: bool,
) -> Option<u8> {
    let radius = sq_size * 0.20;
    for arrow in arrows.iter().rev() {
        let Some((center, step)) = step_badge_center(arrow, sq_size, flipped) else {
            continue;
        };
        let dx = local_x - center.x;
        let dy = local_y - center.y;
        if dx * dx + dy * dy <= radius * radius {
            return Some(step);
        }
    }
    None
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
    /// Closed outline for a single-unit arrow. When present, paint this instead of
    /// `shaft` + `head` (used for knight L-arrows with a rounded joint).
    pub body: Option<Vec<Point>>,
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
        body: None,
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
    let shaft_w = sq_size * 0.25 * scale;
    let Some(body) = rounded_knight_body(from, corner, to, shaft_w, sq_size * 0.40) else {
        return Vec::new();
    };
    vec![ArrowGeometry {
        shaft: Poly {
            points: [body[0], body[0], body[0], body[0]],
        },
        head: Triangle {
            a: to,
            b: to,
            c: to,
        },
        body: Some(body),
        fill,
        outline,
        step: None,
    }]
}

fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

fn scale_pt(a: Point, s: f32) -> Point {
    Point::new(a.x * s, a.y * s)
}

fn length(a: Point) -> f32 {
    (a.x * a.x + a.y * a.y).sqrt()
}

fn normalize(a: Point) -> Option<Point> {
    let len = length(a);
    (len > 1.0).then(|| scale_pt(a, 1.0 / len))
}

fn sample_arc(center: Point, start: Point, end: Point, steps: usize) -> Vec<Point> {
    let a0 = (start.y - center.y).atan2(start.x - center.x);
    let a1 = (end.y - center.y).atan2(end.x - center.x);
    let mut delta = a1 - a0;
    const PI: f32 = std::f32::consts::PI;
    while delta > PI {
        delta -= 2.0 * PI;
    }
    while delta < -PI {
        delta += 2.0 * PI;
    }
    let radius = length(sub(start, center)).max(1.0);
    let steps = steps.max(2);
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let angle = a0 + delta * t;
            Point::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}

/// Single closed L-arrow: round start cap, quarter-circle elbow, chevron head.
fn rounded_knight_body(
    from: Point,
    corner: Point,
    to: Point,
    shaft_w: f32,
    head_len: f32,
) -> Option<Vec<Point>> {
    let u = normalize(sub(corner, from))?;
    let v = normalize(sub(to, corner))?;
    let n_u = Point::new(-u.y, u.x);
    let n_v = Point::new(-v.y, v.x);
    let turn = u.x * v.y - u.y * v.x;
    if turn.abs() < 0.2 {
        return None;
    }
    let sign = turn.signum();
    let outer_u = scale_pt(n_u, -sign);
    let outer_v = scale_pt(n_v, -sign);
    let inner_u = scale_pt(n_u, sign);
    let inner_v = scale_pt(n_v, sign);
    let w = shaft_w * 0.5;
    let head_half = w * 2.4;
    let head_base = sub(to, scale_pt(v, head_len));
    let start_outer = add(from, scale_pt(outer_u, w));
    let start_inner = add(from, scale_pt(inner_u, w));
    let cap_back = sub(from, scale_pt(u, w));
    let outer_in = add(corner, scale_pt(outer_u, w));
    let outer_out = add(corner, scale_pt(outer_v, w));
    let inner_out = add(corner, scale_pt(inner_v, w));
    let inner_in = add(corner, scale_pt(inner_u, w));
    let mut body = Vec::with_capacity(40);
    body.push(start_inner);
    body.extend(sample_arc(from, start_inner, cap_back, 6));
    body.extend(sample_arc(from, cap_back, start_outer, 6));
    body.push(outer_in);
    body.extend(sample_arc(corner, outer_in, outer_out, 8));
    body.push(add(head_base, scale_pt(outer_v, w)));
    body.push(add(head_base, scale_pt(outer_v, head_half)));
    body.push(to);
    body.push(add(head_base, scale_pt(inner_v, head_half)));
    body.push(add(head_base, scale_pt(inner_v, w)));
    body.push(inner_out);
    body.extend(sample_arc(corner, inner_out, inner_in, 8));
    body.push(start_inner);
    Some(body)
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
    fn step_badge_hit_uses_numbered_tip() {
        types::init();
        let arrow = BoardArrow::new(
            types::Square::from_index(12),
            types::Square::from_index(28),
            MarkColor::Orange,
            ArrowRole::Gambit,
        )
        .with_step(3);
        let (center, step) = step_badge_center(&arrow, 64.0, false).expect("badge");
        assert_eq!(step, 3);
        assert_eq!(
            hit_step_badge(
                std::slice::from_ref(&arrow),
                center.x,
                center.y,
                64.0,
                false
            ),
            Some(3)
        );
        assert_eq!(
            hit_step_badge(std::slice::from_ref(&arrow), 0.0, 0.0, 64.0, false),
            None
        );
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
        assert_eq!(geom.len(), 1);
        let body = geom[0].body.as_ref().expect("knight arrow is one path");
        assert!(
            body.len() >= 16,
            "rounded elbow must be sampled, not two rectangles"
        );
        let df = (arrow.to.file() as i32 - arrow.from.file() as i32).abs();
        let dr = (arrow.to.rank() as i32 - arrow.from.rank() as i32).abs();
        let elbow = if df > dr {
            sq_center(arrow.to.file(), arrow.from.rank(), 64.0, false)
        } else {
            sq_center(arrow.from.file(), arrow.to.rank(), 64.0, false)
        };
        let min_elbow = body
            .iter()
            .map(|point| {
                let dx = point.x - elbow.x;
                let dy = point.y - elbow.y;
                (dx * dx + dy * dy).sqrt()
            })
            .fold(f32::MAX, f32::min);
        assert!(
            min_elbow < 64.0 * 0.20,
            "joint stays on a rounded fillet around the elbow, got {min_elbow}"
        );
    }

    #[test]
    fn rounded_knight_body_is_a_closed_loop() {
        let from = Point::new(40.0, 520.0);
        let corner = Point::new(40.0, 360.0);
        let to = Point::new(200.0, 360.0);
        let body = rounded_knight_body(from, corner, to, 16.0, 24.0).unwrap();
        let first = body.first().unwrap();
        let last = body.last().unwrap();
        assert!((first.x - last.x).abs() < 0.5);
        assert!((first.y - last.y).abs() < 0.5);
        assert_eq!(body.iter().filter(|p| **p == to).count(), 1);
    }
}
