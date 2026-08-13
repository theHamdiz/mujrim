//! Single-window board workspace: play, analysis, tournaments.

use floem::prelude::*;
use floem::taffy::style::{FlexWrap, Overflow};

use crate::app_core::layout;
use crate::app_core::logic;
use crate::app_core::settings::Screen;
use crate::app_core::tournament_arena;

use super::super::actions;
use super::super::board;
use super::super::clock;
use super::super::dock;
use super::super::eval_graph;
use super::super::state::{AppHandles, AppState};
use super::super::theme;
use super::super::widgets;

pub fn playing(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        playing_sidebar(state, handles),
    )
}

pub fn analysis(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        analysis_sidebar(state, handles),
    )
}

pub fn tournaments(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        true,
        tournament_sidebar(state, handles),
    )
}

fn workspace(
    state: AppState,
    handles: AppHandles,
    show_clocks: bool,
    sidebar: impl IntoView + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Stack::horizontal((
            board_pane(state, handles.clone(), show_clocks),
            sidebar.style(move |s| {
                s.width(layout::SIDEBAR_IDEAL_PX)
                    .min_width(layout::SIDEBAR_MIN_PX)
                    .max_width(layout::SIDEBAR_MAX_PX)
                    .height_full()
                    .min_height(0.0)
                    .padding(14.0)
                    .row_gap(10.0)
                    .background(theme::rgba(pal().sidebar))
                    .border_left(1.0)
                    .border_color(theme::rgba(pal().border))
                    .overflow_x(Overflow::Clip)
                    .overflow_y(Overflow::Scroll)
            }),
        ))
        .style(|s| {
            s.width_full()
                .flex_grow(1.0f32)
                .min_width(0.0)
                .min_height(0.0)
                .items_stretch()
                .overflow_x(Overflow::Clip)
        }),
        dock::bottom_dock(state, handles),
    ))
    .style(move |s| {
        s.size_full()
            .min_width(0.0)
            .min_height(0.0)
            .color(theme::rgba(pal().text_primary))
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn board_pane(state: AppState, handles: AppHandles, show_clocks: bool) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        dyn_view(move || {
            if show_clocks {
                clock::live_clocks(state).into_any()
            } else {
                Empty::new().into_any()
            }
        }),
        dyn_view(move || {
            if state.game.get().is_some() {
                board::board_view(state, handles.clone()).into_any()
            } else {
                empty_board(state).into_any()
            }
        })
        .style(|s| {
            s.size_full()
                .flex_grow(1.0f32)
                .min_width(0.0)
                .min_height(0.0)
        }),
        Label::derived(move || state.status.get()).style(move |s| {
            s.font_size(11.0)
                .padding_top(4.0)
                .color(theme::rgba(pal().text_secondary))
                .text_ellipsis()
        }),
    ))
    .style(|s| {
        s.flex_grow(1.0f32)
            .height_full()
            .min_width(0.0)
            .min_height(0.0)
            .padding(12.0)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn empty_board(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::curious_title("Board", 28.0),
            Label::derived(move || {
                (if state.screen.get() == Screen::Tournaments {
                    "Configure the tournament, then Start."
                } else {
                    "Start a game from Home."
                })
                .to_owned()
            })
            .style(|s| s.font_size(15.0).font_bold()),
            Label::derived(move || {
                (if state.screen.get() == Screen::Tournaments {
                    "Games play with real clocks on one full board."
                } else {
                    "The board fills this pane once a position is loaded."
                })
                .to_owned()
            })
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        ))
        .style(|s| s.row_gap(8.0).items_center()),
    )
    .style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .color(theme::rgba(pal().text_primary))
    })
}

fn playing_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        pane_title("Moves"),
        move_list(state),
        pane_title("Engine"),
        engine_lines(state, handles),
        eval_graph::eval_graph(state),
        widgets::ghost_button(state, "Coach review", move || {
            actions::annotate_last_move(state)
        }),
        Label::derived(move || {
            let searching = if state.searching.get() {
                "searching"
            } else {
                "idle"
            };
            format!("{searching} · {}", state.status.get())
        })
        .style(|s| s.font_size(11.0)),
    ))
    .style(|s| s.flex_col().row_gap(8.0).width_full().min_height(0.0))
}

fn analysis_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    let telemetry = handles.telemetry.clone();
    Stack::vertical((
        pane_title("Analysis"),
        eval_graph::eval_graph(state),
        Label::derived(move || {
            state
                .analysis
                .get()
                .map_or_else(|| telemetry.get().label, |snap| snap.status)
        })
        .style(|s| s.font_size(12.0)),
        Label::derived(move || {
            state
                .analysis
                .get()
                .and_then(|snap| snap.consensus.clone())
                .unwrap_or_default()
        })
        .style(|s| s.font_size(12.0)),
        move_list(state),
        widgets::primary_button(state, "Re-run", {
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
        Stack::horizontal((
            widgets::ghost_button(state, "<<", move || {
                state.review_ply.set(Some(0));
            }),
            widgets::ghost_button(state, "<", move || {
                state.review_ply.update(|ply| {
                    *ply = Some(
                        ply.unwrap_or(state.move_log.get_untracked().len())
                            .saturating_sub(1),
                    );
                });
            }),
            widgets::ghost_button(state, ">", move || {
                let len = state.move_log.get_untracked().len();
                state.review_ply.update(|ply| {
                    let next = ply.unwrap_or(0).saturating_add(1).min(len);
                    *ply = Some(next);
                });
            }),
            widgets::ghost_button(state, ">>", move || {
                state
                    .review_ply
                    .set(Some(state.move_log.get_untracked().len()));
            }),
        ))
        .style(|s| s.col_gap(4.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.flex_col().row_gap(8.0).width_full().min_height(0.0))
}

fn tournament_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    let telemetry = handles.telemetry.clone();
    Stack::vertical((
        pane_title("Live"),
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            layout::focused_live_game(&snap.live_games)
                .map(|game| format!("R{} · {} vs {}", game.round, game.white, game.black))
                .unwrap_or_else(|| {
                    format!(
                        "{} · {}/{}",
                        snap.format_label, snap.completed_matches, snap.total_matches
                    )
                })
        })
        .style(|s| s.font_size(13.0).font_bold()),
        Label::derived(move || state.tournament_status.get()).style(move |s| {
            s.font_size(11.0).color(theme::rgba(
                theme::palette(state.settings.get().board_theme).text_secondary,
            ))
        }),
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            layout::focused_live_game(&snap.live_games)
                .map(|game| {
                    format!(
                        "{}  d{}  {} nodes\n{}",
                        tournament_arena::score_text(game.score_cp),
                        game.depth,
                        game.nodes,
                        if game.last_uci.is_empty() {
                            "—"
                        } else {
                            game.last_uci.as_str()
                        }
                    )
                })
                .unwrap_or_else(|| telemetry.get().label)
        })
        .style(|s| s.font_size(12.0)),
        pane_title("Moves"),
        move_list(state),
        eval_graph::eval_graph(state),
        Stack::horizontal((
            widgets::ghost_button(state, "Setup", {
                let handles = handles.clone();
                move || actions::open_tournament_setup(state, &handles)
            }),
            widgets::ghost_button(state, "Cancel", {
                let handles = handles.clone();
                move || actions::cancel_tournament(&handles)
            }),
        ))
        .style(|s| s.col_gap(6.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.flex_col().row_gap(8.0).width_full().min_height(0.0))
}

fn pane_title(label: &'static str) -> impl IntoView {
    Label::new(label).style(|s| s.font_size(12.0).font_bold())
}

fn move_list(state: AppState) -> impl IntoView {
    Label::derived(move || {
        let moves = state.move_log.get();
        if moves.is_empty() {
            return "No moves yet.".to_owned();
        }
        moves
            .chunks(2)
            .enumerate()
            .map(|(idx, pair)| {
                let white = logic::annotated_move_label(
                    &pair[0],
                    state.move_annotations.get().get(idx * 2).copied().flatten(),
                );
                let black = pair.get(1).map(|mv| {
                    logic::annotated_move_label(
                        mv,
                        state
                            .move_annotations
                            .get()
                            .get(idx * 2 + 1)
                            .copied()
                            .flatten(),
                    )
                });
                match black {
                    Some(black) => format!("{}. {white} {black}", idx + 1),
                    None => format!("{}. {white}", idx + 1),
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    })
    .style(move |s| {
        s.font_size(12.0)
            .width_full()
            .min_width(0.0)
            .color(theme::rgba(
                theme::palette(state.settings.get().board_theme).text_primary,
            ))
    })
}

fn engine_lines(state: AppState, handles: AppHandles) -> impl IntoView {
    Label::derived(move || {
        let tel = handles.telemetry.get();
        if tel.pv.is_empty() {
            if tel.label.is_empty() {
                state.status.get()
            } else {
                tel.label
            }
        } else {
            format!(
                "d{}  {:+.2}  {}",
                tel.depth,
                tel.score_cp as f32 / 100.0,
                tel.pv.join(" ")
            )
        }
    })
    .style(move |s| {
        s.font_size(12.0).color(theme::rgba(
            theme::palette(state.settings.get().board_theme).text_secondary,
        ))
    })
}
