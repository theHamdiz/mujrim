//! Menu, playing, study, tournament, and analysis screens.

mod home;
mod study;
mod workspace;

use floem::prelude::*;
use floem::taffy::style::Overflow;

use crate::app_core::settings::Screen;

use super::state::{AppHandles, AppState};

pub fn root_content(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || match state.screen.get() {
        Screen::Menu => home::menu(state, handles.clone()).into_any(),
        Screen::Playing => workspace::playing(state, handles.clone()).into_any(),
        Screen::Study => workspace::study(state, handles.clone()).into_any(),
        Screen::Learn => workspace::learn(state, handles.clone()).into_any(),
        Screen::Tournaments => workspace::tournaments(state, handles.clone()).into_any(),
        Screen::Analysis => workspace::analysis(state, handles.clone()).into_any(),
    })
    .style(|s| {
        s.size_full()
            .min_width(0.0)
            .min_height(0.0)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}
