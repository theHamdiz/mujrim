//! Menu, playing, study, tournament, and analysis screens.

mod home;
mod study;
mod workspace;

use floem::prelude::*;
use floem::taffy::style::{Display, Overflow};

use crate::app_core::settings::Screen;

use super::state::{AppHandles, AppState};

pub fn root_content(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::new((
        home::menu(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Menu))),
        workspace::playing(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Playing))),
        workspace::study(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Study))),
        workspace::learn(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Learn))),
        workspace::library(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Library))),
        workspace::tournaments(state, handles.clone())
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Tournaments))),
        workspace::analysis(state, handles)
            .style(move |s| screen_host(s, matches!(state.screen.get(), Screen::Analysis))),
    ))
    .style(|s| {
        s.size_full()
            .min_width(0.0)
            .min_height(0.0)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn screen_host(style: floem::style::Style, visible: bool) -> floem::style::Style {
    let style = style
        .size_full()
        .min_width(0.0)
        .min_height(0.0)
        .overflow_x(Overflow::Clip)
        .overflow_y(Overflow::Clip);
    if visible {
        style
    } else {
        style.display(Display::None)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn screens_stay_mounted_on_the_window_root() {
        let src = include_str!("mod.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(
            production.contains("Display::None"),
            "inactive screens must stay mounted and hidden"
        );
        assert!(
            !production.contains("dyn_view(move || match state.screen.get()"),
            "swapping screens inside dyn_view creates widgets without a window root"
        );
    }
}
