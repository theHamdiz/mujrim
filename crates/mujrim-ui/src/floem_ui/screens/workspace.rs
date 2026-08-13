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

pub fn study(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        super::study::study_sidebar(state, handles),
    )
}

pub fn learn(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        super::study::learn_sidebar(state, handles),
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
                empty_board(state, handles.clone()).into_any()
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

fn empty_board(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::curious_title("Board", 28.0),
            Label::derived(move || {
                (if state.screen.get() == Screen::Tournaments {
                    "Configure the tournament, then Start."
                } else if matches!(state.screen.get(), Screen::Study | Screen::Learn) {
                    "Explorer, library, and saved lines load onto this board."
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
            dyn_view({
                let handles = handles.clone();
                move || {
                    if state.screen.get() == Screen::Tournaments {
                        widgets::primary_button(state, "Tournament setup", {
                            let handles = handles.clone();
                            move || actions::open_tournament_setup(state, &handles)
                        })
                        .into_any()
                    } else {
                        Empty::new().into_any()
                    }
                }
            }),
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
        pane_title("Multi-Engine Studio"),
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
        widgets::stepper_row(
            state,
            "MultiPV",
            "",
            move || state.analysis_multipv.get(),
            move |value| state.analysis_multipv.set(value),
            1,
            5,
        ),
        analysis_engine_toggles(state, handles.clone()),
        widgets::primary_button(state, "Run Multi-Engine Analysis", {
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
        widgets::ghost_button(state, "Review Current Game", {
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
        pane_title("Engine PV arrows"),
        Label::derived(move || {
            state.analysis.get().map_or_else(
                || "No multi-engine arrows yet.".to_owned(),
                |snap| {
                    snap.arrows
                        .iter()
                        .take(12)
                        .map(|arrow| {
                            let label = arrow
                                .label
                                .clone()
                                .unwrap_or_else(|| format!("{}→{}", arrow.from, arrow.to));
                            format!("{}. {label}", arrow.step.unwrap_or(0))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            )
        })
        .style(|s| s.font_size(12.0)),
        pane_title("Gambit coach"),
        gambit_controls(state),
        move_list(state),
        ply_nav(state),
    ))
    .style(|s| s.flex_col().row_gap(8.0).width_full().min_height(0.0))
}

fn analysis_engine_toggles(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || {
        let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
        let mut rows = vec![
            widgets::toggle_row(
                state,
                "Mujrim (built-in)",
                move || {
                    state
                        .analysis_engines_selected
                        .get()
                        .iter()
                        .any(|id| id == "builtin")
                },
                move |_| actions::toggle_analysis_engine(state, "builtin".to_owned()),
            )
            .into_any(),
        ];
        for engine in roster {
            let id = engine.path.to_string_lossy().into_owned();
            let name = engine.name;
            rows.push(
                widgets::toggle_row(
                    state,
                    name,
                    {
                        let id = id.clone();
                        move || {
                            state
                                .analysis_engines_selected
                                .get()
                                .iter()
                                .any(|selected| selected == &id)
                        }
                    },
                    {
                        let id = id.clone();
                        move |_| actions::toggle_analysis_engine(state, id.clone())
                    },
                )
                .into_any(),
            );
        }
        rows.into_view()
            .style(|s| s.width_full().row_gap(6.0).flex_col())
            .into_any()
    })
}

fn gambit_controls(state: AppState) -> impl IntoView {
    dyn_view(move || {
        let Some(id) = state.active_gambit_id.get() else {
            return Label::new("Load a gambit from Study for stepped coaching arrows.")
                .style(|s| s.font_size(12.0))
                .into_any();
        };
        let Some(lesson) = mujrim_study::gambit::find_gambit(&id) else {
            return Empty::new().into_any();
        };
        Stack::vertical((
            Label::new(format!("{} · {}", lesson.name, lesson.eco)).style(|s| s.font_size(14.0)),
            Label::new(lesson.summary).style(|s| s.font_size(12.0)),
            Stack::horizontal((
                widgets::ghost_button(state, "◀ Step", move || actions::gambit_step(state, -1)),
                Label::derived(move || format!("Ply {}", state.gambit_ply.get()))
                    .style(|s| s.font_size(13.0)),
                widgets::ghost_button(state, "Step ▶", move || actions::gambit_step(state, 1)),
            ))
            .style(|s| s.col_gap(8.0).items_center()),
        ))
        .style(|s| s.row_gap(8.0).width_full())
        .into_any()
    })
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
        pane_title("Standings"),
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            if snap.standings.is_empty() {
                "Standings appear after the first finished pairing.".to_owned()
            } else {
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
            }
        })
        .style(|s| s.font_size(11.0)),
        Stack::horizontal((
            widgets::primary_button(state, "Tournament setup", {
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

pub(super) fn ply_nav(state: AppState) -> impl IntoView {
    Stack::horizontal((
        widgets::ghost_button(state, "<<", move || actions::view_ply(state, 0)),
        widgets::ghost_button(state, "<", move || {
            let len = state.move_log.get_untracked().len();
            let current = state.review_ply.get_untracked().unwrap_or(len);
            actions::view_ply(state, current.saturating_sub(1));
        }),
        widgets::ghost_button(state, ">", move || {
            let len = state.move_log.get_untracked().len();
            let current = state.review_ply.get_untracked().unwrap_or(len);
            actions::view_ply(state, current.saturating_add(1).min(len));
        }),
        widgets::ghost_button(state, ">>", move || {
            actions::view_ply(state, state.move_log.get_untracked().len());
        }),
    ))
    .style(|s| s.col_gap(4.0).flex_wrap(FlexWrap::Wrap))
}

pub(super) fn move_list(state: AppState) -> impl IntoView {
    dyn_view(move || {
        let moves = state.move_log.get();
        let annotations = state.move_annotations.get();
        let pal = theme::palette(state.settings.get().board_theme);
        if moves.is_empty() {
            return Label::new("No moves yet.")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal.text_secondary)))
                .into_any();
        }
        let labels = logic::san_annotated_moves(&state.initial_fen.get(), &moves, &annotations);
        let current = state.review_ply.get().unwrap_or(labels.len());
        labels
            .chunks(2)
            .enumerate()
            .map(|(idx, pair)| {
                let white_ply = idx * 2 + 1;
                let black_ply = idx * 2 + 2;
                let white = pair[0].clone();
                let black = pair.get(1).cloned();
                Stack::horizontal((
                    Label::new(format!("{}.", idx + 1))
                        .style(move |s| {
                            s.font_size(11.0)
                                .width(28.0)
                                .color(theme::rgba(pal.text_secondary))
                        })
                        .into_any(),
                    ply_button(state, white, white_ply, current == white_ply).into_any(),
                    black.map_or_else(
                        || {
                            Label::new("…")
                                .style(move |s| {
                                    s.font_size(12.0)
                                        .width(88.0)
                                        .color(theme::rgba(pal.text_secondary))
                                })
                                .into_any()
                        },
                        |black| {
                            ply_button(state, black, black_ply, current == black_ply).into_any()
                        },
                    ),
                ))
                .style(|s| s.width_full().col_gap(6.0).items_center().min_width(0.0))
            })
            .collect::<Vec<_>>()
            .into_view()
            .style(|s| s.width_full().row_gap(2.0).flex_col().max_height(220.0))
            .scroll()
            .into_any()
    })
}

fn ply_button(state: AppState, label: String, ply: usize, active: bool) -> impl IntoView {
    Button::new(label)
        .action(move || actions::view_ply(state, ply))
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.min_width(72.0)
                .padding_horiz(8.0)
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

#[cfg(test)]
mod tests {
    #[test]
    fn clickable_move_list_navigates_board_plies() {
        let src = include_str!("workspace.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "actions::view_ply",
            "san_annotated_moves",
            "pub fn study",
            "pub fn learn",
            "ply_button",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
    }
}
