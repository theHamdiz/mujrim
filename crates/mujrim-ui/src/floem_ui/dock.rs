//! Collapsible bottom dock: Results | Histogram.

use std::time::Duration;

use floem::prelude::*;
use floem::style::Transition;
use floem::taffy::style::{Display, Overflow};

use crate::app_core::layout::{self, DockTab};
use crate::app_core::logic;

use super::eval_graph;
use super::state::{AppHandles, AppState};
use super::theme;
use super::widgets;

pub fn bottom_dock(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let dragging = RwSignal::new(false);
    let drag_origin_y = RwSignal::new(0.0);
    let drag_origin_height = RwSignal::new(layout::DOCK_OPEN_PX);
    let pane_height = RwSignal::new(800.0);
    Stack::vertical((
        dock_split_handle(
            state,
            pal,
            dragging,
            drag_origin_y,
            drag_origin_height,
            pane_height,
        ),
        tab_bar(state),
        dock_body(state, handles),
    ))
    .style(move |s| {
        let pal = pal();
        let height = layout::dock_height(
            state.dock_open.get(),
            layout::clamp_dock_height(state.settings.get().dock_height_px, pane_height.get()),
        );
        let s = s
            .width_full()
            .height(height)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
            .background(theme::rgba(pal.sidebar))
            .border_top(1.0)
            .border_color(theme::rgba(pal.border));
        if dragging.get() {
            s
        } else {
            s.transition(
                floem::style::Height,
                Transition::ease_in_out(Duration::from_millis(220)),
            )
        }
    })
}

fn dock_split_handle(
    state: AppState,
    pal: impl Fn() -> crate::app_core::palette::GuiPalette + Copy + 'static,
    dragging: RwSignal<bool>,
    drag_origin_y: RwSignal<f64>,
    drag_origin_height: RwSignal<f64>,
    pane_height: RwSignal<f64>,
) -> impl IntoView {
    Empty::new()
        .style(move |s| {
            let active = dragging.get();
            s.width_full()
                .height(layout::SPLIT_HANDLE_PX)
                .flex_shrink(0.0f32)
                .cursor(floem::style::CursorStyle::RowResize)
                .background(if active {
                    theme::rgba(pal().accent)
                } else {
                    theme::rgba(pal().border)
                })
                .hover(|s| {
                    s.background(theme::rgba(pal().accent))
                        .cursor(floem::style::CursorStyle::RowResize)
                })
        })
        .on_event_stop(
            el::PointerDown,
            move |cx, event: &floem::ui_events::pointer::PointerButtonEvent| {
                if let Some(pointer_id) = event.pointer.pointer_id {
                    cx.request_pointer_capture(pointer_id);
                }
                pane_height.set(dock_window_height(
                    cx,
                    state.settings.get_untracked().dock_height_px,
                ));
                dragging.set(true);
                drag_origin_y.set(event.state.logical_point().y);
                drag_origin_height.set(state.settings.get_untracked().dock_height_px);
            },
        )
        .on_event_cont(
            el::PointerMove,
            move |cx, event: &floem::ui_events::pointer::PointerUpdate| {
                if !dragging.get_untracked() {
                    return;
                }
                let window = dock_window_height(cx, drag_origin_height.get_untracked())
                    .max(pane_height.get_untracked());
                pane_height.set(window);
                let dy = event.current.logical_point().y - drag_origin_y.get_untracked();
                let next = layout::apply_dock_drag(drag_origin_height.get_untracked(), dy, window);
                state
                    .settings
                    .update(|settings| settings.dock_height_px = next);
            },
        )
        .on_event_stop(
            el::PointerUp,
            move |_, _: &floem::ui_events::pointer::PointerButtonEvent| {
                finish_dock_drag(state, dragging);
            },
        )
        .on_event_stop(el::LostPointerCapture, move |_, _| {
            finish_dock_drag(state, dragging);
        })
}

fn dock_window_height(cx: &floem::event::EventCx, current_dock: f64) -> f64 {
    let mut id = cx.target.owning_id();
    let mut observed = 0.0;
    for _ in 0..12 {
        if let Some(size) = id.parent_size()
            && size.height > observed
        {
            observed = size.height;
        }
        match id.parent() {
            Some(parent) => id = parent,
            None => break,
        }
    }
    layout::dock_resize_window_height(observed, current_dock)
}

fn finish_dock_drag(state: AppState, dragging: RwSignal<bool>) {
    if !dragging.get_untracked() {
        return;
    }
    dragging.set(false);
    state.persist_settings();
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
    Stack::new((
        results_pane(state, handles.clone())
            .style(move |s| dock_tab_host(s, state, DockTab::Results)),
        eval_graph::eval_histogram(state, 148.0)
            .style(move |s| dock_tab_host(s, state, DockTab::Histogram)),
        engine_log_pane(state, handles).style(move |s| dock_tab_host(s, state, DockTab::EngineLog)),
    ))
    .style(|s| {
        s.width_full()
            .flex_grow(1.0f32)
            .min_height(0.0)
            .padding_horiz(12.0)
            .padding_bottom(10.0)
    })
}

fn dock_tab_host(style: floem::style::Style, state: AppState, tab: DockTab) -> floem::style::Style {
    if state.dock_tab.get() == tab {
        style.size_full().min_height(0.0)
    } else {
        style.display(Display::None)
    }
}

fn results_pane(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::results_export_bar(state, handles),
        Stack::horizontal((
            widgets::standing_rows_list(state, "Standings appear as matches finish.")
                .style(|s| s.width_pct(42.0).min_width(0.0).min_height(0.0)),
            widgets::filling_scroll(
                (0..12)
                    .map(|index| played_game_slot(state, index, pal))
                    .collect::<Vec<_>>()
                    .into_view()
                    .style(|s| s.width_full().flex_col().row_gap(2.0).min_width(0.0)),
            )
            .style(|s| s.width_pct(58.0).min_width(0.0).min_height(0.0)),
        ))
        .style(|s| s.size_full().col_gap(16.0).min_height(0.0)),
    ))
    .style(|s| s.size_full().row_gap(8.0).min_height(0.0))
}

fn played_game_slot(
    state: AppState,
    index: usize,
    pal: impl Fn() -> crate::app_core::palette::GuiPalette + Copy + 'static,
) -> impl IntoView {
    Button::new(Label::derived(move || {
        state
            .tournament_snapshot
            .get()
            .played_games
            .iter()
            .rev()
            .nth(index)
            .map(|game| game.title())
            .unwrap_or_default()
    }))
    .action(move || {
        let Some(id) = state
            .tournament_snapshot
            .get_untracked()
            .played_games
            .iter()
            .rev()
            .nth(index)
            .map(|game| game.id)
        else {
            return;
        };
        state.selected_tournament_game_id.set(Some(id));
        let snap = state.tournament_snapshot.get_untracked();
        if let Some(played) = snap.game(id).cloned()
            && let Ok(board) = logic::replay_study_game(&played.initial_fen, &played.moves)
        {
            state.game.set(Some(board));
            state.move_log.set(played.moves);
            state.initial_fen.set(played.initial_fen);
        }
    })
    .style(move |s| {
        let snap = state.tournament_snapshot.get();
        let selected = state.selected_tournament_game_id.get();
        let game = snap.played_games.iter().rev().nth(index);
        let pal = pal();
        if game.is_none() {
            return s.display(Display::None);
        }
        s.width_full()
            .padding_horiz(8.0)
            .padding_vert(4.0)
            .border_radius(6.0)
            .border(0.0)
            .font_size(12.0)
            .background(if selected == game.map(|game| game.id) {
                theme::rgba(pal.accent)
            } else {
                Color::TRANSPARENT
            })
            .color(theme::rgba(pal.text_primary))
    })
}

fn engine_log_pane(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let telemetry = handles.telemetry.clone();
    let telemetry_lines = telemetry.clone();
    widgets::filling_scroll(
        Stack::vertical((
            Label::derived(move || {
                let tel = telemetry.get();
                if tel.label.is_empty() {
                    state.status.get()
                } else {
                    format!(
                        "depth {}/{} · {:+} cp · {} nodes",
                        tel.depth, tel.seldepth, tel.score_cp, tel.nodes
                    )
                }
            })
            .style(move |s| {
                s.font_size(12.0)
                    .width_full()
                    .min_width(0.0)
                    .text_wrap()
                    .color(theme::rgba(pal().text_primary))
            }),
            Label::derived(move || {
                let tel = telemetry_lines.get();
                let snap = state.tournament_snapshot.get();
                let live = layout::select_live_game(
                    &snap.live_games,
                    state.focused_live_key.get().as_deref(),
                );
                let lines = if !tel.multipv_lines.is_empty() {
                    tel.multipv_lines
                        .iter()
                        .map(|(rank, score, pv)| format!("#{rank} {score:+}  {}", pv.join(" ")))
                        .collect::<Vec<_>>()
                } else if let Some(game) = live.filter(|game| !game.multipv_lines.is_empty()) {
                    game.multipv_lines
                        .iter()
                        .map(|line| {
                            format!(
                                "#{} {:+}  {}",
                                line.multipv,
                                line.score_cp,
                                line.pv.join(" ")
                            )
                        })
                        .collect()
                } else if let Some(game) = live.filter(|game| !game.pv.is_empty()) {
                    vec![format!("pv {}", game.pv.join(" "))]
                } else if !tel.pv.is_empty() {
                    vec![format!("pv {}", tel.pv.join(" "))]
                } else {
                    Vec::new()
                };
                if lines.is_empty() {
                    "No engine lines yet.".to_owned()
                } else {
                    lines.join("\n")
                }
            })
            .style(move |s| {
                s.font_size(12.0)
                    .width_full()
                    .min_width(0.0)
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            }),
        ))
        .style(|s| s.width_full().row_gap(6.0).min_width(0.0)),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn engine_dock_renders_numbered_multipv_lines() {
        let src = include_str!("dock.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("multipv_lines"));
        assert!(production.contains("RowResize"));
        assert!(production.contains("apply_dock_drag"));
        assert!(production.contains("dock_height_px"));
        assert!(production.contains("dock_resize_window_height"));
        assert!(production.contains("LostPointerCapture"));
        assert!(production.contains("#{}"));
    }
}
