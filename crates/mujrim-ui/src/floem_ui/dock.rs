//! Collapsible bottom dock: Results | Histogram.

use std::time::Duration;

use floem::prelude::*;
use floem::style::Transition;
use floem::taffy::style::Overflow;

use crate::app_core::layout::{self, DockTab};
use crate::app_core::logic;
use crate::app_core::tournament_results;

use super::eval_graph;
use super::state::{AppHandles, AppState};
use super::theme;

pub fn bottom_dock(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((tab_bar(state), dock_body(state, handles))).style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let height = layout::dock_height(state.dock_open.get());
        s.width_full()
            .height(height)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
            .background(theme::rgba(pal.sidebar))
            .border_top(1.0)
            .border_color(theme::rgba(pal.border))
            .transition(
                floem::style::Height,
                Transition::ease_in_out(Duration::from_millis(220)),
            )
    })
}

fn tab_bar(state: AppState) -> impl IntoView {
    Stack::horizontal((
        Stack::horizontal((
            dock_tab(state, DockTab::Results, "Results"),
            dock_tab(state, DockTab::Histogram, "Histogram"),
            dock_tab(state, DockTab::EngineLog, "Engine"),
        ))
        .style(|s| s.col_gap(4.0).items_center()),
        Button::new(Label::derived(move || {
            if state.dock_open.get() { "▾" } else { "▴" }
        }))
        .action(move || {
            state.dock_open.update(|open| *open = !*open);
        })
        .style(move |s| {
            s.padding_horiz(8.0)
                .padding_vert(2.0)
                .border(0.0)
                .font_size(12.0)
                .background(Color::TRANSPARENT)
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).text_secondary,
                ))
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(layout::DOCK_TAB_BAR_PX)
            .padding_horiz(12.0)
            .items_center()
            .justify_between()
    })
}

fn dock_tab(state: AppState, tab: DockTab, label: &'static str) -> impl IntoView {
    Button::new(label)
        .action(move || {
            let (next, open) = layout::next_dock_state(
                state.dock_tab.get_untracked(),
                state.dock_open.get_untracked(),
                tab,
            );
            state.dock_tab.set(next);
            state.dock_open.set(open);
        })
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            let active = state.dock_tab.get() == tab && state.dock_open.get();
            s.padding_horiz(10.0)
                .padding_vert(4.0)
                .border_radius(8.0)
                .border(0.0)
                .font_size(12.0)
                .background(if active {
                    theme::rgba(pal.accent)
                } else {
                    Color::TRANSPARENT
                })
                .color(theme::rgba(pal.text_primary))
                .hover(|s| s.background(theme::rgba(pal.panel)))
        })
}

fn dock_body(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || match state.dock_tab.get() {
        DockTab::Results => results_pane(state).into_any(),
        DockTab::Histogram => eval_graph::eval_histogram(state, 148.0).into_any(),
        DockTab::EngineLog => engine_log_pane(state, handles.clone()).into_any(),
    })
    .style(|s| {
        s.width_full()
            .flex_grow(1.0f32)
            .min_height(0.0)
            .padding_horiz(12.0)
            .padding_bottom(10.0)
    })
}

fn results_pane(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::horizontal((
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            if !tournament_results::standings_ready(&snap.standings) {
                return "Standings appear as matches finish.".to_owned();
            }
            snap.standings
                .iter()
                .map(|row| {
                    format!(
                        "{}. {}  {:.1}  ({}-{}-{})",
                        row.rank, row.name, row.points, row.wins, row.draws, row.losses
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .style(move |s| {
            s.font_size(12.0)
                .width_pct(42.0)
                .color(theme::rgba(pal().text_primary))
        }),
        dyn_view(move || {
            let snap = state.tournament_snapshot.get();
            let selected = state.selected_tournament_game_id.get();
            snap.played_games
                .iter()
                .rev()
                .take(12)
                .map(|game| {
                    let id = game.id;
                    let active = selected == Some(id);
                    Button::new(game.title())
                        .action(move || {
                            state.selected_tournament_game_id.set(Some(id));
                            let snap = state.tournament_snapshot.get_untracked();
                            if let Some(played) = snap.game(id).cloned()
                                && let Ok(board) =
                                    logic::replay_study_game(&played.initial_fen, &played.moves)
                            {
                                state.game.set(Some(board));
                                state.move_log.set(played.moves);
                                state.initial_fen.set(played.initial_fen);
                            }
                        })
                        .style(move |s| {
                            let pal = pal();
                            s.width_full()
                                .padding_horiz(8.0)
                                .padding_vert(4.0)
                                .border_radius(6.0)
                                .border(0.0)
                                .font_size(12.0)
                                .background(if active {
                                    theme::rgba(pal.accent)
                                } else {
                                    Color::TRANSPARENT
                                })
                                .color(theme::rgba(pal.text_primary))
                        })
                })
                .collect::<Vec<_>>()
                .into_view()
        })
        .style(|s| s.width_pct(58.0).flex_col().row_gap(2.0).min_width(0.0))
        .scroll(),
    ))
    .style(|s| s.size_full().col_gap(16.0).min_height(0.0))
}

fn engine_log_pane(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Label::derived(move || {
        let tel = handles.telemetry.get();
        if tel.label.is_empty() {
            state.status.get()
        } else {
            tel.label
        }
    })
    .style(move |s| {
        s.font_size(12.0)
            .width_full()
            .min_width(0.0)
            .color(theme::rgba(pal().text_primary))
    })
}
