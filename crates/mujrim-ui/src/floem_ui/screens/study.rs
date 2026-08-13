//! Study library hub.

use floem::prelude::*;
use floem::taffy::style::FlexWrap;

use crate::app_core::logic;

use super::super::actions;
use super::super::state::{AppHandles, AppState};
use super::super::theme;
use super::super::widgets;

pub fn study(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::curious_title("Study", 36.0),
        Label::new("Search the library, import PGN, and train tactics.")
            .style(move |s| s.font_size(13.0).color(theme::rgba(pal().text_secondary))),
        widgets::card(
            state,
            Stack::horizontal((
                TextInput::new(state.study_query).style(|s| {
                    s.flex_grow(1.0f32)
                        .min_width(160.0)
                        .height(36.0)
                        .border_radius(10.0)
                }),
                widgets::primary_button(state, "Search", {
                    let handles = handles.clone();
                    move || actions::refresh_study(state, &handles)
                }),
                widgets::ghost_button(state, "Import PGN", {
                    let handles = handles.clone();
                    move || actions::import_pgn(state, &handles)
                }),
                widgets::ghost_button(state, "Index openings", {
                    let handles = handles.clone();
                    move || actions::index_openings(state, &handles)
                }),
                widgets::ghost_button(state, "Train", {
                    let handles = handles.clone();
                    move || actions::start_puzzle(state, &handles)
                }),
            ))
            .style(|s| {
                s.width_full()
                    .col_gap(8.0)
                    .row_gap(8.0)
                    .items_center()
                    .flex_wrap(FlexWrap::Wrap)
            }),
        ),
        Label::derived(move || {
            format!(
                "{} games · {} openings indexed · {} puzzles due",
                state.study_results.get().len(),
                state.opening_indexed.get(),
                state.training_due.get().len()
            )
        })
        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        dyn_view(move || {
            let results = state.study_results.get();
            let handles = handles.clone();
            results
                .into_iter()
                .take(24)
                .map(|summary| {
                    let id = summary.id.clone();
                    let (title, detail) = logic::game_summary_label(&summary);
                    Stack::vertical((
                        Label::new(title).style(|s| s.font_size(13.0).font_bold()),
                        Label::new(detail).style(move |s| {
                            s.font_size(11.0).color(theme::rgba(pal().text_secondary))
                        }),
                        widgets::ghost_button(state, "Load game", {
                            let handles = handles.clone();
                            move || actions::load_library_game(state, &handles, id.clone())
                        }),
                    ))
                    .style(move |s| {
                        let pal = pal();
                        s.width(260.0)
                            .padding(12.0)
                            .row_gap(6.0)
                            .border_radius(12.0)
                            .background(theme::rgba(pal.panel))
                            .border(1.0)
                            .border_color(theme::rgba(pal.border))
                    })
                })
                .collect::<Vec<_>>()
                .into_view()
                .style(|s| {
                    s.width_full()
                        .flex_row()
                        .flex_wrap(FlexWrap::Wrap)
                        .col_gap(10.0)
                        .row_gap(10.0)
                })
        }),
        Label::derived(move || state.status.get()),
    ))
    .style(move |s| {
        s.size_full()
            .padding(24.0)
            .row_gap(14.0)
            .min_width(0.0)
            .min_height(0.0)
            .color(theme::rgba(pal().text_primary))
    })
    .scroll()
}
