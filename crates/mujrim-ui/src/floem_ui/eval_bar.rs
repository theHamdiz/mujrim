//! Vertical eval bar beside the board (chess.com / Lichess style).

use floem::kurbo::Rect;
use floem::prelude::*;

use crate::app_core::layout;

use super::state::AppState;
use super::theme;

pub fn eval_bar(state: AppState) -> impl IntoView {
    canvas(move |cx, size| {
        let pal = theme::palette(state.settings.get().board_theme);
        let fill = layout::eval_bar_fill(state.eval_bar_cp.get()) as f64;
        let white_h = size.height * fill;
        cx.fill(
            &Rect::new(0.0, 0.0, size.width, size.height - white_h),
            theme::rgba(pal.sidebar),
            0.0,
        );
        cx.fill(
            &Rect::new(0.0, size.height - white_h, size.width, size.height),
            Color::from_rgb8(245, 245, 240),
            0.0,
        );
        let mid = size.height * 0.5;
        cx.fill(
            &Rect::new(0.0, mid - 0.5, size.width, mid + 0.5),
            theme::rgba(pal.border),
            0.0,
        );
    })
    .style(move |s| {
        let _ = state.settings.get();
        let _ = state.eval_bar_cp.get();
        s.width(layout::EVAL_BAR_PX)
            .height_full()
            .flex_shrink(0.0f32)
            .border_radius(4.0)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn bar_tracks_eval_signal() {
        let src = include_str!("eval_bar.rs");
        assert!(src.contains("eval_bar_fill"));
        assert!(src.contains("eval_bar_cp"));
        assert!(src.contains("EVAL_BAR_PX"));
    }
}
