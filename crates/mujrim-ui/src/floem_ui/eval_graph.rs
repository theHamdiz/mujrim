//! Eval sparkline canvas.

use floem::kurbo::{BezPath, Stroke};
use floem::prelude::*;

use crate::app_core::arrows::eval_graph_points;

use super::state::AppState;
use super::theme;

pub fn eval_graph(state: AppState) -> impl IntoView {
    canvas(move |cx, size| {
        let pal = theme::palette(state.settings.get().board_theme);
        cx.fill(
            &floem::kurbo::Rect::new(0.0, 0.0, size.width, size.height),
            theme::rgba(pal.panel),
            0.0,
        );
        let scores = state.analysis_scores.get();
        let points = eval_graph_points(&scores, size.width as f32, size.height as f32);
        if points.len() < 2 {
            return;
        }
        let mut path = BezPath::new();
        path.move_to((points[0].x as f64, points[0].y as f64));
        for point in &points[1..] {
            path.line_to((point.x as f64, point.y as f64));
        }
        cx.stroke(&path, theme::rgba(pal.accent), &Stroke::new(2.0));
    })
    .style(|s| s.width_full().height(72.0).border_radius(6.0))
}
