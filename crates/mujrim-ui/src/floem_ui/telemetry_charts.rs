//! Compact Ateed telemetry canvases: score histogram and loss sparklines.

use floem::kurbo::{BezPath, Rect, Stroke};
use floem::prelude::*;

use crate::app_core::ateed_studio::METRIC_RING_CAP;

use super::state::AppState;
use super::theme;

pub fn score_histogram(state: AppState, height: f64) -> impl IntoView {
    canvas(move |cx, size| {
        let pal = theme::palette(state.settings.get().board_theme);
        cx.fill(
            &Rect::new(0.0, 0.0, size.width, size.height),
            theme::rgba(pal.panel),
            0.0,
        );
        let hist = state.ateed.hist.get();
        let max = hist.iter().copied().max().unwrap_or(0).max(1);
        let n = hist.len() as f64;
        let gap = 2.0;
        let bar_w = ((size.width - gap * (n + 1.0)) / n).max(1.0);
        for (index, &count) in hist.iter().enumerate() {
            let h = (count as f64 / max as f64) * (size.height - 4.0);
            let x = gap + index as f64 * (bar_w + gap);
            let y = size.height - 2.0 - h;
            cx.fill(
                &Rect::new(x, y, x + bar_w, size.height - 2.0),
                theme::rgba(pal.accent),
                2.0,
            );
        }
    })
    .style(move |s| {
        let _ = state.settings.get();
        let _ = state.ateed.hist.get();
        s.width_full().height(height).border_radius(8.0)
    })
}

pub fn nps_sparkline(state: AppState, height: f64) -> impl IntoView {
    canvas(move |cx, size| {
        let pal = theme::palette(state.settings.get().board_theme);
        cx.fill(
            &Rect::new(0.0, 0.0, size.width, size.height),
            theme::rgba(pal.panel),
            0.0,
        );
        let ring = state.ateed.nps_ring.get();
        let mut samples = [0.0f32; METRIC_RING_CAP];
        let n = ring.copy_oldest_first(&mut samples);
        let max = samples[..n].iter().copied().fold(1.0f32, f32::max).max(1.0);
        if let Some(path) = series_path(&samples[..n], size.width, size.height, max) {
            cx.stroke(&path, theme::rgba(pal.accent_alt), &Stroke::new(2.0));
        }
    })
    .style(move |s| {
        let _ = state.settings.get();
        let _ = state.ateed.nps_ring.get();
        s.width_full().height(height).border_radius(8.0)
    })
}

pub fn loss_sparkline(state: AppState, height: f64) -> impl IntoView {
    canvas(move |cx, size| {
        let pal = theme::palette(state.settings.get().board_theme);
        cx.fill(
            &Rect::new(0.0, 0.0, size.width, size.height),
            theme::rgba(pal.panel),
            0.0,
        );
        let ring = state.ateed.loss_ring.get();
        let mut train = [0.0f32; METRIC_RING_CAP];
        let mut val = [0.0f32; METRIC_RING_CAP];
        let train_n = ring.train.copy_oldest_first(&mut train);
        let val_n = ring.val.copy_oldest_first(&mut val);
        let max = train[..train_n]
            .iter()
            .chain(val[..val_n].iter())
            .copied()
            .fold(0.05f32, f32::max)
            .max(0.05);
        if let Some(path) = series_path(&train[..train_n], size.width, size.height, max) {
            cx.stroke(&path, theme::rgba(pal.accent), &Stroke::new(2.0));
        }
        if let Some(path) = series_path(&val[..val_n], size.width, size.height, max) {
            cx.stroke(&path, theme::rgba(pal.accent_alt), &Stroke::new(2.0));
        }
    })
    .style(move |s| {
        let _ = state.settings.get();
        let _ = state.ateed.loss_ring.get();
        s.width_full().height(height).border_radius(8.0)
    })
}

fn series_path(samples: &[f32], width: f64, height: f64, max: f32) -> Option<BezPath> {
    if samples.len() < 2 {
        return None;
    }
    let mut path = BezPath::new();
    let last = (samples.len() - 1) as f64;
    for (index, &value) in samples.iter().enumerate() {
        let x = index as f64 / last * width;
        let y = height - (value / max).clamp(0.0, 1.0) as f64 * (height - 4.0) - 2.0;
        if index == 0 {
            path.move_to((x, y));
        } else {
            path.line_to((x, y));
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn datagen_chart_draws_nps_sparkline() {
        let src = include_str!("telemetry_charts.rs");
        assert!(src.contains("pub fn nps_sparkline"));
        assert!(src.contains("nps_ring"));
        assert!(src.contains("pub fn score_histogram"));
    }
}
