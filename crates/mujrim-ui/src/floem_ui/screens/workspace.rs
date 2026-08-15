//! Single-window board workspace: play, analysis, tournaments.

use floem::prelude::*;
use floem::style::CursorStyle;
use floem::taffy::style::{Display, FlexWrap, Overflow};

use crate::app_core::layout;
use crate::app_core::logic;
use crate::app_core::settings::Screen;
use crate::app_core::tournament_arena;

use super::super::actions;
use super::super::board;
use super::super::chrome;
use super::super::clock;
use super::super::dock;
use super::super::engine;
use super::super::eval_bar;
use super::super::eval_graph;
use super::super::icons;
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

pub fn library(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        super::study::library_sidebar(state, handles),
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
    let dragging = RwSignal::new(false);
    let drag_origin_x = RwSignal::new(0.0);
    let drag_origin_width = RwSignal::new(layout::SIDEBAR_IDEAL_PX);
    let pane_width = RwSignal::new(1280.0);
    Stack::vertical((
        Stack::horizontal((
            board_pane(state, handles.clone(), show_clocks).style(|s| {
                s.flex_grow(1.0f32)
                    .flex_shrink(1.0f32)
                    .min_width(layout::BOARD_MIN_PX)
                    .min_height(0.0)
                    .height_full()
            }),
            split_handle(
                state,
                pal,
                dragging,
                drag_origin_x,
                drag_origin_width,
                pane_width,
            ),
            sidebar.style(move |s| {
                if arena_layout(state) {
                    return s
                        .display(Display::None)
                        .width(0.0)
                        .min_width(0.0)
                        .max_width(0.0);
                }
                let width = layout::clamp_sidebar_width(
                    state.settings.get().sidebar_width_px,
                    pane_width.get(),
                );
                s.width(width)
                    .min_width(layout::SIDEBAR_MIN_PX)
                    .max_width(layout::SIDEBAR_MAX_PX)
                    .flex_grow(0.0f32)
                    .flex_shrink(0.0f32)
                    .height_full()
                    .min_height(0.0)
                    .padding(16.0)
                    .row_gap(12.0)
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
                .flex_wrap(FlexWrap::NoWrap)
                .overflow_x(Overflow::Clip)
        })
        .on_event_cont(el::WindowResized, move |_, size: &floem::kurbo::Size| {
            if size.width > 1.0 {
                pane_width.set(size.width);
            }
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

fn arena_layout(state: AppState) -> bool {
    layout::tournament_arena_layout(
        state.screen.get(),
        state.tournament_setup.get().concurrency,
        &state.tournament_snapshot.get().live_games,
    )
}

fn split_handle(
    state: AppState,
    pal: impl Fn() -> crate::app_core::palette::GuiPalette + Copy + 'static,
    dragging: RwSignal<bool>,
    drag_origin_x: RwSignal<f64>,
    drag_origin_width: RwSignal<f64>,
    pane_width: RwSignal<f64>,
) -> impl IntoView {
    Empty::new()
        .style(move |s| {
            let active = dragging.get();
            s.width(layout::SPLIT_HANDLE_PX)
                .flex_shrink(0.0f32)
                .height_full()
                .cursor(CursorStyle::ColResize)
                .background(if active {
                    theme::rgba(pal().accent)
                } else {
                    theme::rgba(pal().border)
                })
                .hover(|s| {
                    s.background(theme::rgba(pal().accent))
                        .cursor(CursorStyle::ColResize)
                })
        })
        .on_event_stop(
            el::PointerDown,
            move |cx, event: &floem::ui_events::pointer::PointerButtonEvent| {
                if let Some(pointer_id) = event.pointer.pointer_id {
                    cx.request_pointer_capture(pointer_id);
                }
                if let Some(size) = cx.target.owning_id().parent_size()
                    && size.width > 1.0
                {
                    pane_width.set(size.width);
                }
                dragging.set(true);
                drag_origin_x.set(window_pointer_x(cx, event.state.logical_point()));
                drag_origin_width.set(state.settings.get_untracked().sidebar_width_px);
            },
        )
        .on_event_cont(
            el::PointerMove,
            move |cx, event: &floem::ui_events::pointer::PointerUpdate| {
                if !dragging.get_untracked() {
                    return;
                }
                let dx = window_pointer_x(cx, event.current.logical_point())
                    - drag_origin_x.get_untracked();
                let next = layout::apply_sidebar_drag(
                    drag_origin_width.get_untracked(),
                    dx,
                    pane_width.get_untracked(),
                );
                state
                    .settings
                    .update(|settings| settings.sidebar_width_px = next);
            },
        )
        .on_event_stop(el::PointerUp, move |_, _| {
            if dragging.get_untracked() {
                dragging.set(false);
                state.persist_settings();
            }
        })
        .on_event_stop(el::LostPointerCapture, move |_, _| {
            if dragging.get_untracked() {
                dragging.set(false);
                state.persist_settings();
            }
        })
        .style(move |s| {
            if arena_layout(state) {
                s.display(Display::None).width(0.0)
            } else {
                s
            }
        })
}

fn window_pointer_x(cx: &floem::event::EventCx, local: floem::kurbo::Point) -> f64 {
    (cx.world_transform.inverse() * local).x
}

fn board_pane(state: AppState, handles: AppHandles, show_clocks: bool) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let clocks = if show_clocks {
        clock::live_clocks(state).into_any()
    } else {
        Empty::new()
            .style(|s| s.width_full().height(0.0))
            .into_any()
    };
    Stack::vertical((
        chrome::screen_tools(state, handles.clone())
            .style(|s| s.width_full().padding_bottom(6.0).min_width(0.0)),
        clocks.style(move |s| {
            if arena_layout(state) {
                s.display(Display::None).height(0.0)
            } else {
                s
            }
        }),
        Stack::horizontal((
            eval_bar::eval_bar(state).style(move |s| {
                if arena_layout(state) || state.game.get().is_none() {
                    s.display(Display::None)
                } else {
                    s
                }
            }),
            Stack::new((
                board::board_view(state, handles.clone())
                    .style(|s| s.size_full().min_width(0.0).min_height(0.0)),
                empty_board(state, handles.clone(), show_clocks).style(move |s| {
                    if arena_layout(state) || state.game.get().is_some() {
                        s.display(Display::None)
                    } else {
                        s.size_full()
                    }
                }),
            ))
            .style(|s| {
                s.size_full()
                    .flex_grow(1.0f32)
                    .min_width(0.0)
                    .min_height(0.0)
            }),
        ))
        .style(move |s| {
            let s = s
                .size_full()
                .flex_grow(1.0f32)
                .min_width(0.0)
                .min_height(0.0)
                .col_gap(8.0);
            if arena_layout(state) {
                s.display(Display::None).flex_grow(0.0f32).height(0.0)
            } else {
                s
            }
        }),
        live_board_grid(state, handles),
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

fn empty_board(state: AppState, handles: AppHandles, tournament_board: bool) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let setup = if tournament_board {
        widgets::primary_button(state, "Tournament setup", {
            let handles = handles.clone();
            move || actions::open_tournament_setup(state, &handles)
        })
        .into_any()
    } else {
        Empty::new().into_any()
    };
    widgets::card(
        state,
        Stack::vertical((
            widgets::curious_title("Board", 28.0),
            Label::derived(move || {
                (if state.screen.get() == Screen::Tournaments {
                    "Configure the tournament, then Start."
                } else if matches!(
                    state.screen.get(),
                    Screen::Study | Screen::Learn | Screen::Library
                ) {
                    "Explorer, library, and saved lines load onto this board."
                } else {
                    "Start a game from Home."
                })
                .to_owned()
            })
            .style(|s| {
                s.font_size(15.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            Label::derived(move || {
                (if state.screen.get() == Screen::Tournaments {
                    "Games play with real clocks. Concurrent pairings appear as a live board grid."
                } else {
                    "The board fills this pane once a position is loaded."
                })
                .to_owned()
            })
            .style(move |s| {
                s.font_size(12.0)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            }),
            setup,
        ))
        .style(|s| s.row_gap(8.0).items_center().min_width(0.0).width_full()),
    )
    .style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .color(theme::rgba(pal().text_primary))
    })
}

fn live_board_grid(state: AppState, handles: AppHandles) -> impl IntoView {
    let slots = (0..layout::LIVE_BOARD_SLOTS)
        .map(|index| live_board_slot(state, handles.clone(), index))
        .collect::<Vec<_>>();
    Stack::vertical((
        Label::new("Live boards").style(move |s| {
            s.font_size(12.0).font_bold().color(theme::rgba(
                theme::palette(state.settings.get().board_theme).accent_alt,
            ))
        }),
        slots.into_view().style(|s| {
            s.width_full()
                .flex_row()
                .flex_wrap(FlexWrap::Wrap)
                .col_gap(10.0)
                .row_gap(10.0)
                .flex_grow(1.0f32)
                .min_height(0.0)
        }),
    ))
    .style(move |s| {
        let s = s
            .width_full()
            .flex_grow(1.0f32)
            .min_width(0.0)
            .min_height(0.0)
            .row_gap(8.0)
            .overflow_y(Overflow::Scroll);
        if arena_layout(state) {
            s
        } else {
            s.display(Display::None).flex_grow(0.0f32).height(0.0)
        }
    })
}

fn live_board_slot(state: AppState, handles: AppHandles, index: usize) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let board = move || {
        let snap = state.tournament_snapshot.get();
        let concurrency = state.tournament_setup.get().concurrency.max(1) as usize;
        tournament_arena::visible_live_boards(&snap.live_games, concurrency)
            .get(index)
            .cloned()
    };
    Stack::vertical((
        Label::derived(move || {
            board()
                .map(|game| format!("{} vs {}", game.white, game.black))
                .unwrap_or_default()
        })
        .style(|s| s.font_size(12.0).font_bold().min_width(0.0).text_ellipsis()),
        Label::derived(move || {
            let Some(game) = board() else {
                return String::new();
            };
            let snap = state.tournament_snapshot.get();
            let (white, black) = layout::live_clock_faces_at(
                Some(&game),
                None,
                None,
                layout::live_white_to_move(&game),
                Some(state.clock_now_ms.get()),
                snap.paused,
            );
            format!("{}  ·  {}", white.display, black.display)
        })
        .style(move |s| {
            s.font_size(11.0)
                .min_width(0.0)
                .color(theme::rgba(pal().text_secondary))
        }),
        board::live_mini_board(state, handles, index),
        Label::derived(move || {
            board()
                .map(|game| {
                    if game.last_uci.is_empty() {
                        format!("{:+} cp", game.score_cp)
                    } else {
                        format!("{}  {:+} cp", game.last_uci, game.score_cp)
                    }
                })
                .unwrap_or_default()
        })
        .style(move |s| {
            s.font_size(10.0)
                .min_width(0.0)
                .color(theme::rgba(pal().text_secondary))
        }),
    ))
    .style(move |s| {
        let Some(_) = board() else {
            return s.display(Display::None);
        };
        s.min_width(220.0)
            .flex_grow(1.0f32)
            .flex_basis(220.0)
            .padding(10.0)
            .row_gap(6.0)
            .border_radius(12.0)
            .border(1.0)
            .border_color(theme::rgba(pal().border))
            .background(theme::rgba(pal().panel))
    })
}

fn playing_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        pane_title("Moves"),
        move_list(state, handles.clone()),
        ply_nav(state, handles.clone()),
        pane_title("Import / Export"),
        widgets::game_io_bar(state, handles.clone()),
        pane_title("Engine"),
        engine_lines(state, handles.clone()),
        widgets::ghost_button(state, "Stop search", {
            let handles = handles.clone();
            move || actions::stop_engine_search(state, &handles)
        })
        .style(move |s| {
            if state.searching.get() {
                s
            } else {
                s.display(Display::None)
            }
        }),
        widgets::ghost_button(state, "Resume search", {
            let handles = handles.clone();
            move || engine::maybe_start_engine_turn(state, &handles)
        })
        .style(move |s| {
            let searching = state.searching.get();
            let Some(game) = state.game.get() else {
                return s.display(Display::None);
            };
            let player = match game.board.side_to_move {
                types::Color::White => state.white_player.get(),
                types::Color::Black => state.black_player.get(),
            };
            if searching
                || game.game_over
                || matches!(player, crate::app_core::engine::PlayerConfig::Human)
            {
                s.display(Display::None)
            } else {
                s
            }
        }),
        eval_graph::eval_graph(state),
        widgets::ghost_button(state, "Coach review", {
            let handles = handles.clone();
            move || actions::review_played_game(state, &handles)
        }),
        Label::derived(move || {
            let searching = if state.searching.get() {
                "searching"
            } else {
                "idle"
            };
            format!("{searching} · {}", state.status.get())
        })
        .style(|s| s.font_size(11.0).min_width(0.0).width_full().text_wrap()),
    ))
    .style(|s| {
        s.flex_col()
            .row_gap(8.0)
            .width_full()
            .min_width(0.0)
            .height_full()
            .min_height(0.0)
    })
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
        .style(|s| s.font_size(12.0).min_width(0.0).width_full().text_wrap()),
        Label::derived(move || {
            state
                .analysis
                .get()
                .and_then(|snap| snap.consensus.clone())
                .unwrap_or_default()
        })
        .style(|s| s.font_size(12.0).min_width(0.0).width_full().text_wrap()),
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
            move || actions::review_played_game(state, &handles)
        }),
        widgets::explanation_card(state, move || explanation_text(state)),
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
        .style(|s| s.font_size(12.0).min_width(0.0).width_full().text_wrap()),
        pane_title("Gambit coach"),
        gambit_controls(state),
        Stack::vertical((
            move_list(state, handles.clone()),
            ply_nav(state, handles.clone()),
            pane_title("Import / Export"),
            widgets::game_io_bar(state, handles),
        ))
        .style(|s| {
            s.row_gap(8.0)
                .width_full()
                .flex_grow(1.0f32)
                .min_height(0.0)
        }),
    ))
    .style(|s| {
        s.flex_col()
            .row_gap(8.0)
            .width_full()
            .min_width(0.0)
            .height_full()
            .min_height(0.0)
    })
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
                move |enabled| actions::set_analysis_engine(state, "builtin".to_owned(), enabled),
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
                        move |enabled| actions::set_analysis_engine(state, id.clone(), enabled)
                    },
                )
                .into_any(),
            );
        }
        widgets::capped_scroll(
            rows.into_view()
                .style(|s| s.width_full().row_gap(6.0).flex_col()),
            layout::LIST_SCROLL_PX,
        )
        .into_any()
    })
}

fn gambit_controls(state: AppState) -> impl IntoView {
    dyn_view(move || {
        let Some(id) = state.active_gambit_id.get() else {
            return Label::new("Load a gambit from Study for stepped coaching arrows.")
                .style(|s| s.font_size(12.0).min_width(0.0).width_full().text_wrap())
                .into_any();
        };
        let Some(lesson) = mujrim_study::gambit::find_gambit(&id) else {
            return Empty::new().into_any();
        };
        Stack::vertical((
            Label::new(format!("{} · {}", lesson.name, lesson.eco))
                .style(|s| s.font_size(14.0).min_width(0.0).width_full().text_wrap()),
            Label::new(lesson.summary)
                .style(|s| s.font_size(12.0).min_width(0.0).width_full().text_wrap()),
            Stack::horizontal((
                widgets::ghost_button(state, "◀ Step", move || actions::gambit_step(state, -1)),
                Label::derived(move || format!("Ply {}", state.gambit_ply.get()))
                    .style(|s| s.font_size(13.0)),
                widgets::ghost_button(state, "Step ▶", move || actions::gambit_step(state, 1)),
            ))
            .style(|s| {
                s.col_gap(8.0)
                    .items_center()
                    .flex_wrap(FlexWrap::Wrap)
                    .min_width(0.0)
            }),
        ))
        .style(|s| s.row_gap(8.0).width_full().min_width(0.0))
        .into_any()
    })
}

fn tournament_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        resume_banner(state, handles.clone()),
        tournament_live_card(state, handles.clone()),
        Stack::vertical((
            pane_title("Moves"),
            move_list(state, handles.clone()),
            ply_nav(state, handles.clone()),
            eval_graph::eval_graph(state),
        ))
        .style(|s| {
            s.row_gap(8.0)
                .width_full()
                .flex_grow(1.0f32)
                .min_height(0.0)
        }),
        Stack::vertical((
            pane_title("Standings"),
            widgets::standing_rows_list(
                state,
                "Standings appear after the first finished pairing.",
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        Stack::vertical((
            pane_title("Previous events"),
            tournament_history(state, handles.clone()),
            pane_title("Export games"),
            widgets::results_export_bar(state, handles),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
    ))
    .style(|s| {
        s.flex_col()
            .row_gap(8.0)
            .width_full()
            .min_width(0.0)
            .height_full()
            .min_height(0.0)
    })
}

fn tournament_live_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let telemetry = handles.telemetry.clone();
    widgets::card(
        state,
        Stack::vertical((
            Stack::horizontal((
                Label::derived(move || state.tournament_snapshot.get().phase_label().to_owned())
                    .style(move |s| {
                        let snap = state.tournament_snapshot.get();
                        let pal = pal();
                        let (bg, fg) = match snap.phase_label() {
                            "Live" => (theme::rgba(pal.accent), theme::rgba(pal.text_primary)),
                            "Paused" => {
                                (theme::rgba(pal.accent_alt), theme::rgba(pal.text_primary))
                            }
                            "Finished" => (theme::rgba(pal.panel), theme::rgba(pal.accent_alt)),
                            "Stopped" => (theme::rgba(pal.bg), theme::rgba(pal.text_secondary)),
                            _ => (theme::rgba(pal.bg), theme::rgba(pal.text_secondary)),
                        };
                        s.padding_horiz(8.0)
                            .padding_vert(3.0)
                            .border_radius(999.0)
                            .font_size(10.0)
                            .font_bold()
                            .background(bg)
                            .color(fg)
                    }),
                Label::derived(move || {
                    let snap = state.tournament_snapshot.get();
                    if snap.format_label.is_empty() {
                        "Tournament".to_owned()
                    } else {
                        snap.format_label
                    }
                })
                .style(move |s| {
                    s.flex_grow(1.0f32)
                        .min_width(0.0)
                        .font_size(theme::TYPE_CAPTION)
                        .font_bold()
                        .text_ellipsis()
                        .color(theme::rgba(pal().text_secondary))
                }),
                Label::derived(move || {
                    state.tournament_setup.get().time_control.label().to_owned()
                })
                .style(move |s| {
                    s.font_size(10.0)
                        .text_ellipsis()
                        .color(theme::rgba(pal().text_secondary))
                }),
            ))
            .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0)),
            Label::derived(move || {
                let snap = state.tournament_snapshot.get();
                layout::select_live_game(&snap.live_games, state.focused_live_key.get().as_deref())
                    .map(|game| format!("R{} · {} vs {}", game.round, game.white, game.black))
                    .unwrap_or_else(|| snap.current_match_label())
            })
            .style(move |s| {
                s.font_size(theme::TYPE_BODY)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_primary))
            }),
            tournament_progress_bar(state),
            Stack::horizontal((
                svg(icons::TROPHY).style(move |s| s.size(14, 14).color(theme::rgba(pal().accent))),
                Label::derived(move || {
                    let snap = state.tournament_snapshot.get();
                    let games = state.tournament_setup.get().games_per_encounter;
                    snap.remaining_games_label(games)
                })
                .style(move |s| {
                    s.flex_grow(1.0f32)
                        .min_width(0.0)
                        .font_size(theme::TYPE_BODY)
                        .font_bold()
                        .text_wrap()
                        .color(theme::rgba(pal().accent_alt))
                }),
                Label::derived(move || {
                    let snap = state.tournament_snapshot.get();
                    let games = state.tournament_setup.get().games_per_encounter;
                    let planned = snap.planned_games(games);
                    if planned == 0 {
                        return String::new();
                    }
                    format!("{} / {planned}", snap.played_games.len())
                })
                .style(move |s| {
                    s.font_size(theme::TYPE_CAPTION)
                        .color(theme::rgba(pal().text_secondary))
                }),
            ))
            .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0)),
            Stack::horizontal((
                tournament_stat_chip(state, "Left", move || {
                    let snap = state.tournament_snapshot.get();
                    let games = state.tournament_setup.get().games_per_encounter;
                    if snap.planned_games(games) == 0 {
                        "—".to_owned()
                    } else {
                        snap.remaining_games(games).to_string()
                    }
                }),
                tournament_stat_chip(state, "Played", move || {
                    state
                        .tournament_snapshot
                        .get()
                        .played_games
                        .len()
                        .to_string()
                }),
                tournament_stat_chip(state, "Live", move || {
                    state.tournament_snapshot.get().live_games.len().to_string()
                }),
            ))
            .style(|s| s.width_full().col_gap(6.0).min_width(0.0)),
            Label::derived(move || {
                let snap = state.tournament_snapshot.get();
                layout::select_live_game(&snap.live_games, state.focused_live_key.get().as_deref())
                    .map(|game| {
                        format!(
                            "{}  d{}  {} nodes · {}",
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
            .style(move |s| {
                s.font_size(theme::TYPE_CAPTION)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            }),
            Label::derived(move || state.tournament_status.get()).style(move |s| {
                s.font_size(theme::TYPE_CAPTION)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            }),
            tournament_controls(state, handles),
        ))
        .style(|s| s.row_gap(10.0).width_full().min_width(0.0)),
    )
}

fn tournament_progress_bar(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let fraction = move || {
        let snap = state.tournament_snapshot.get();
        let games = state.tournament_setup.get().games_per_encounter;
        snap.game_progress_fraction(games)
    };
    Stack::horizontal((
        Empty::new().style(move |s| {
            let done = fraction().clamp(0.0, 1.0);
            let s = s
                .height(6.0)
                .flex_grow(done.max(0.001))
                .border_radius(99.0)
                .background(theme::rgba(pal().accent));
            if done <= 0.0 {
                s.display(Display::None)
            } else {
                s
            }
        }),
        Empty::new().style(move |s| {
            let rest = (1.0 - fraction().clamp(0.0, 1.0)).max(0.0);
            s.height(6.0).flex_grow(rest.max(0.001))
        }),
    ))
    .style(move |s| {
        s.width_full()
            .height(6.0)
            .border_radius(99.0)
            .background(theme::rgba(pal().bg))
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn tournament_stat_chip(
    state: AppState,
    label: &'static str,
    value: impl Fn() -> String + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::derived(value).style(move |s| {
            s.font_size(18.0)
                .font_bold()
                .min_width(0.0)
                .width_full()
                .color(theme::rgba(pal().text_primary))
        }),
        Label::new(label).style(move |s| {
            s.font_size(10.0)
                .min_width(0.0)
                .width_full()
                .color(theme::rgba(pal().text_secondary))
        }),
    ))
    .style(move |s| {
        s.flex_grow(1.0f32)
            .min_width(0.0)
            .padding_horiz(8.0)
            .padding_vert(8.0)
            .row_gap(2.0)
            .border_radius(10.0)
            .background(theme::rgba(pal().bg))
    })
}

fn resume_banner(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Label::derived(move || {
            state
                .resume_prompt
                .get()
                .map_or_else(String::new, |checkpoint| {
                    format!(
                        "Paused: {} · {} vs {}",
                        checkpoint.event, checkpoint.white, checkpoint.black
                    )
                })
        })
        .style(move |s| {
            s.font_size(13.0)
                .font_bold()
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(
                    theme::palette(state.settings.get().board_theme).accent_alt,
                ))
        }),
        Label::new("Pick up the last event, or discard it and start a new tournament.").style(
            move |s| {
                s.font_size(11.0)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(
                        theme::palette(state.settings.get().board_theme).text_secondary,
                    ))
            },
        ),
        Stack::horizontal((
            widgets::primary_button(state, "Resume event", {
                let handles = handles.clone();
                move || actions::resume_paused_tournament(state, &handles)
            }),
            widgets::ghost_button(state, "Start fresh", {
                let handles = handles.clone();
                move || actions::discard_paused_tournament(state, &handles)
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let s = s
            .width_full()
            .row_gap(8.0)
            .padding(12.0)
            .border_radius(12.0)
            .border(1.0)
            .border_color(theme::rgba(pal.accent))
            .background(theme::rgba(pal.panel));
        if state.resume_prompt.get().is_some() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn tournament_controls(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        widgets::primary_button(state, "Tournament setup", {
            let handles = handles.clone();
            move || actions::open_tournament_setup(state, &handles)
        }),
        widgets::ghost_button(state, "Pause", {
            let handles = handles.clone();
            move || actions::pause_tournament(state, &handles)
        })
        .style(move |s| {
            let snap = state.tournament_snapshot.get();
            if snap.running && !snap.paused {
                s
            } else {
                s.display(Display::None)
            }
        }),
        widgets::ghost_button(state, "Resume", {
            let handles = handles.clone();
            move || actions::resume_tournament(state, &handles)
        })
        .style(move |s| {
            let snap = state.tournament_snapshot.get();
            if snap.running && snap.paused {
                s
            } else {
                s.display(Display::None)
            }
        }),
        widgets::ghost_button(state, "Stop game", {
            let handles = handles.clone();
            move || actions::abort_tournament_game(state, &handles)
        })
        .style(move |s| {
            if state.tournament_snapshot.get().running {
                s
            } else {
                s.display(Display::None)
            }
        }),
        widgets::ghost_button(state, "Stop tournament", {
            let handles = handles.clone();
            move || actions::cancel_tournament(state, &handles)
        })
        .style(move |s| {
            if state.tournament_snapshot.get().running {
                s
            } else {
                s.display(Display::None)
            }
        }),
    ))
    .style(|s| {
        s.width_full()
            .min_width(0.0)
            .col_gap(6.0)
            .row_gap(6.0)
            .flex_wrap(FlexWrap::Wrap)
    })
}

fn tournament_history(state: AppState, handles: AppHandles) -> impl IntoView {
    const HISTORY_SLOTS: usize = 16;
    let rows = (0..HISTORY_SLOTS)
        .map(|idx| {
            let pal = move || theme::palette(state.settings.get().board_theme);
            Stack::horizontal((
                Button::new(Label::derived(move || {
                    state
                        .tournament_history
                        .get()
                        .get(idx)
                        .map(|tournament| {
                            if tournament.games.is_empty() {
                                tournament.name.clone()
                            } else {
                                format!("{} · {} games", tournament.name, tournament.games.len())
                            }
                        })
                        .unwrap_or_default()
                }))
                .action({
                    let handles = handles.clone();
                    move || {
                        let Some(id) = state
                            .tournament_history
                            .get_untracked()
                            .get(idx)
                            .map(|tournament| tournament.id.clone())
                        else {
                            return;
                        };
                        actions::load_historical_tournament(state, &handles, id);
                    }
                })
                .style(move |s| {
                    let pal = pal();
                    s.min_width(0.0)
                        .flex_grow(1.0f32)
                        .padding_horiz(10.0)
                        .padding_vert(5.0)
                        .border_radius(999.0)
                        .border(1.0)
                        .border_color(theme::rgba(pal.border))
                        .font_size(12.0)
                        .text_ellipsis()
                        .background(theme::rgba(pal.panel))
                        .color(theme::rgba(pal.text_primary))
                        .hover(|s| s.background(theme::rgba(pal.accent)))
                }),
                Button::new(
                    svg(icons::TRASH)
                        .style(move |s| s.size(13, 13).color(theme::rgba(pal().text_secondary))),
                )
                .action({
                    let handles = handles.clone();
                    move || {
                        let Some(id) = state
                            .tournament_history
                            .get_untracked()
                            .get(idx)
                            .map(|tournament| tournament.id.clone())
                        else {
                            return;
                        };
                        actions::delete_historical_tournament(state, &handles, id);
                    }
                })
                .style(move |s| {
                    let pal = pal();
                    s.size(28, 28)
                        .items_center()
                        .justify_center()
                        .border_radius(999.0)
                        .border(0.0)
                        .background(Color::TRANSPARENT)
                        .color(theme::rgba(pal.text_secondary))
                        .hover(|s| {
                            s.background(theme::rgba(pal.panel))
                                .color(Color::from_rgb8(220, 72, 72))
                        })
                }),
            ))
            .style(move |s| {
                let s = s.width_full().col_gap(6.0).items_center().min_width(0.0);
                if state.tournament_history.get().get(idx).is_some() {
                    s
                } else {
                    s.display(Display::None)
                }
            })
        })
        .collect::<Vec<_>>();
    Stack::vertical((
        Label::derived(move || {
            if state.tournament_history.get().is_empty() {
                "Finished events appear here after the first tournament.".to_owned()
            } else {
                String::new()
            }
        })
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            let s = s
                .font_size(12.0)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(pal.text_secondary));
            if state.tournament_history.get().is_empty() {
                s
            } else {
                s.display(Display::None)
            }
        }),
        widgets::capped_scroll(
            rows.into_view()
                .style(|s| s.width_full().row_gap(2.0).flex_col()),
            layout::LIST_SCROLL_PX,
        ),
    ))
    .style(|s| s.width_full().row_gap(2.0).min_width(0.0))
}

fn pane_title(label: &'static str) -> impl IntoView {
    Label::new(label).style(|s| {
        s.font_size(theme::TYPE_TITLE)
            .font_bold()
            .min_width(0.0)
            .width_full()
    })
}

pub(super) fn ply_nav(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        widgets::ghost_button(state, "<<", {
            let handles = handles.clone();
            move || actions::view_ply(state, &handles, 0)
        }),
        widgets::ghost_button(state, "<", {
            let handles = handles.clone();
            move || {
                let len = state.move_log.get_untracked().len();
                let current = state.review_ply.get_untracked().unwrap_or(len);
                actions::view_ply(state, &handles, current.saturating_sub(1));
            }
        }),
        widgets::ghost_button(state, ">", {
            let handles = handles.clone();
            move || {
                let len = state.move_log.get_untracked().len();
                let current = state.review_ply.get_untracked().unwrap_or(len);
                actions::view_ply(state, &handles, current.saturating_add(1).min(len));
            }
        }),
        widgets::ghost_button(state, ">>", {
            let handles = handles.clone();
            move || actions::view_ply(state, &handles, state.move_log.get_untracked().len())
        }),
    ))
    .style(|s| s.width_full().col_gap(4.0).items_center().justify_center())
}

pub(super) fn move_list(state: AppState, handles: AppHandles) -> impl IntoView {
    const MOVE_LIST_PAIRS: usize = 64;
    let rows = (0..MOVE_LIST_PAIRS)
        .map(|idx| {
            Stack::horizontal((
                Label::derived(move || {
                    if state.move_log.get().len() > idx * 2 {
                        format!("{}.", idx + 1)
                    } else {
                        String::new()
                    }
                })
                .style(move |s| {
                    s.font_size(11.0)
                        .width(layout::MOVE_NUM_WIDTH)
                        .height(layout::MOVE_CHIP_HEIGHT)
                        .color(theme::rgba(
                            theme::palette(state.settings.get().board_theme).text_secondary,
                        ))
                }),
                ply_slot(state, handles.clone(), idx * 2),
                ply_slot(state, handles.clone(), idx * 2 + 1),
            ))
            .style(move |s| {
                let s = s
                    .width_full()
                    .col_gap(layout::MOVE_CHIP_GAP)
                    .items_stretch()
                    .min_width(0.0);
                if state.move_log.get().len() > idx * 2 {
                    s
                } else {
                    s.display(Display::None)
                }
            })
        })
        .collect::<Vec<_>>();
    Stack::vertical((
        Label::derived(move || {
            if state.move_log.get().is_empty() {
                "No moves yet.".to_owned()
            } else {
                String::new()
            }
        })
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            let s = s
                .font_size(12.0)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(pal.text_secondary));
            if state.move_log.get().is_empty() {
                s
            } else {
                s.display(Display::None)
            }
        }),
        rows.into_view()
            .style(|s| s.width_full().row_gap(2.0).flex_col())
            .scroll()
            .style(|s| s.width_full().flex_grow(1.0f32).min_height(0.0)),
    ))
    .style(|s| {
        s.width_full()
            .row_gap(2.0)
            .min_width(0.0)
            .flex_grow(1.0f32)
            .min_height(0.0)
    })
}

fn ply_slot(state: AppState, handles: AppHandles, ply_index: usize) -> impl IntoView {
    let ply = ply_index + 1;
    Button::new(Label::derived(move || {
        let moves = state.move_log.get();
        if ply_index >= moves.len() {
            return String::new();
        }
        logic::move_list_chip_labels(
            &state.initial_fen.get(),
            &moves,
            &state.move_annotations.get(),
            &state.analysis_scores.get(),
        )
        .get(ply_index)
        .cloned()
        .unwrap_or_default()
    }))
    .action(move || {
        if ply_index < state.move_log.get_untracked().len() {
            actions::view_ply(state, &handles, ply);
        }
    })
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let len = state.move_log.get().len();
        let occupied = ply_index < len;
        let current = state.review_ply.get().unwrap_or(len);
        let annotation = state
            .move_annotations
            .get()
            .get(ply_index)
            .copied()
            .flatten();
        let score = state
            .analysis_scores
            .get()
            .get(ply_index)
            .copied()
            .flatten();
        let tint = logic::annotation_tint(annotation).or_else(|| score.map(logic::eval_tint));
        let (bg, fg) = if !occupied {
            (Color::TRANSPARENT, theme::rgba(pal.text_secondary))
        } else if current == ply {
            (theme::rgba(pal.accent), theme::rgba(pal.text_primary))
        } else if let Some((r, g, b)) = tint {
            (Color::from_rgba8(r, g, b, 48), Color::from_rgb8(r, g, b))
        } else {
            (theme::rgba(pal.bg), theme::rgba(pal.text_primary))
        };
        s.min_width(0.0)
            .flex_grow(1.0f32)
            .flex_basis(0.0)
            .flex_shrink(1.0f32)
            .height(layout::MOVE_CHIP_HEIGHT)
            .min_height(layout::MOVE_CHIP_HEIGHT)
            .max_height(layout::MOVE_CHIP_HEIGHT)
            .padding_horiz(8.0)
            .items_center()
            .border_radius(8.0)
            .border(0.0)
            .font_size(12.0)
            .font_bold()
            .text_ellipsis()
            .background(bg)
            .color(fg)
            .hover(|s| {
                if occupied {
                    s.background(theme::rgba(pal.panel))
                } else {
                    s
                }
            })
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
        s.font_size(12.0)
            .min_width(0.0)
            .width_full()
            .text_wrap()
            .color(theme::rgba(
                theme::palette(state.settings.get().board_theme).text_secondary,
            ))
    })
}

pub(super) fn explanation_text(state: AppState) -> String {
    let fen = state.initial_fen.get();
    let moves = state.move_log.get();
    let ply = state.review_ply.get().unwrap_or(moves.len());
    let Ok(board) = logic::board_at_ply(&fen, &moves, ply) else {
        return "Load a position to hear the threats and plans.".to_owned();
    };
    let last = if ply > 0 {
        let annotation = state.move_annotations.get().get(ply - 1).copied().flatten();
        let score = state.analysis_scores.get().get(ply - 1).copied().flatten();
        let mv = logic::board_at_ply(&fen, &moves, ply - 1)
            .ok()
            .and_then(|mut previous| logic::find_logged_move(&mut previous, &moves[ply - 1]));
        mujrim_study::explain::MoveContext {
            annotation,
            score_cp: score,
            mv,
            san: None,
        }
    } else {
        mujrim_study::explain::MoveContext::default()
    };
    mujrim_study::explain::explain_position(&board, ply, last).panel_text()
}

#[cfg(test)]
mod tests {
    #[test]
    fn clickable_move_list_navigates_board_plies() {
        let src = include_str!("workspace.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "actions::view_ply",
            "move_list_chip_labels",
            "pub fn study",
            "pub fn learn",
            "ply_slot",
            "MOVE_LIST_PAIRS",
            "MOVE_CHIP_HEIGHT",
            "flex_basis(0.0)",
            "items_stretch()",
            "board::board_view",
            "display(Display::None)",
            "annotation_tint",
            "resume_banner",
            "Resume event",
            "Start fresh",
            "flex_grow(1.0f32)",
            "Pause",
            "Stop game",
            "Stop tournament",
            "tournament_history",
            "delete_historical_tournament",
            "icons::TRASH",
            "border_radius(999.0)",
            "justify_center()",
            "results_export_bar",
            "stop_engine_search",
            "Resume search",
            "pub fn library",
            "screen_tools",
            "apply_sidebar_drag",
            "standing_rows_list",
            "explanation_card",
            "set_analysis_engine",
            "split_handle",
            "request_pointer_capture",
            "CursorStyle::ColResize",
            "window_pointer_x",
            "FlexWrap::NoWrap",
            "Overflow::Scroll",
            "capped_scroll",
            "LIST_SCROLL_PX",
            "eval_bar::eval_bar",
            "live_board_grid",
            "live_mini_board",
            "arena_layout",
            "tournament_arena_layout",
            "LIVE_BOARD_SLOTS",
            "tournament_live_card",
            "remaining_games_label",
            "tournament_progress_bar",
            "tournament_stat_chip",
            "phase_label",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
        let split = production
            .split("fn workspace(")
            .nth(1)
            .expect("workspace")
            .split("fn split_handle(")
            .next()
            .expect("split_handle");
        assert!(
            !split.contains("FlexWrap::Wrap"),
            "board/sidebar split must stay on one row so the handle can be dragged"
        );
        assert!(
            !split.contains("1600.0"),
            "sidebar drag must clamp against the live pane width"
        );
        assert!(
            !split.contains("sidebar_scroll"),
            "every right panel including Tournament must scroll"
        );
        assert!(
            !production.contains("board::board_view(state, handles.clone()).into_any()"),
            "creating the board canvas inside dyn_view panics when a tournament position loads"
        );
        assert!(
            !production.contains("pub(super) fn move_list")
                || !production
                    .split("pub(super) fn move_list")
                    .nth(1)
                    .unwrap()
                    .split("fn ply_slot")
                    .next()
                    .unwrap()
                    .contains("dyn_view"),
            "move_list must not create ply buttons inside dyn_view"
        );
        assert!(
            !production
                .split("pub(super) fn move_list")
                .nth(1)
                .unwrap()
                .split("fn ply_slot")
                .next()
                .unwrap()
                .contains("max_height(220.0)"),
            "move list must grow with the board instead of a 220px cap"
        );
    }
}
