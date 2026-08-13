//! Menu, playing, study, tournament, and analysis screens.

use floem::prelude::*;
use floem::taffy::style::{FlexWrap, Overflow};

use crate::app_core::engine::{
    BundledEngineChoice, GameMode, PlayerConfig, bundled_engine_choices, selected_bundled_engine,
};
use crate::app_core::layout;
use crate::app_core::logic;
use crate::app_core::settings::Screen;
use crate::app_core::tournament_arena;
use crate::app_core::uci_process::ExternalEngineProtocol;

use super::actions;
use super::board;
use super::clock;
use super::dock;
use super::eval_graph;
use super::state::{AppHandles, AppState};
use super::theme;
use super::widgets;

pub fn root_content(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || match state.screen.get() {
        Screen::Menu => menu(state, handles.clone()).into_any(),
        Screen::Playing => playing(state, handles.clone()).into_any(),
        Screen::Study => study(state, handles.clone()).into_any(),
        Screen::Tournaments => tournaments(state, handles.clone()).into_any(),
        Screen::Analysis => analysis(state, handles.clone()).into_any(),
    })
    .style(|s| {
        s.size_full()
            .min_width(0.0)
            .min_height(0.0)
            .overflow_x(Overflow::Clip)
            .overflow_y(Overflow::Clip)
    })
}

fn menu(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let logo_bytes = handles.logo.clone();
    let logo = img(move || logo_bytes.clone()).style(|s| s.size(96, 96).border_radius(22.0));
    let setup = widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Game Setup", pal),
            Label::new("Choose players and load engines")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            Label::new("Mode")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            widgets::picker(state, move || state.selected_mode.get(), GameMode::ALL, {
                let bundled = handles.bundled.clone();
                move |mode| actions::select_mode(state, &bundled, mode)
            }),
            player_column(state, &handles, true),
            player_column(state, &handles, false),
        ))
        .style(|s| s.width_full().row_gap(10.0).min_width(0.0)),
    )
    .style(|s| s.flex_grow(1.0f32).min_width(280.0).max_width(460.0));

    let actions_card = widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Studio", pal),
            Label::new("Open a board, a library, or a live engine event.")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            widgets::primary_button(state, "Start game", {
                let handles = handles.clone();
                move || actions::new_game(state, &handles)
            }),
            widgets::ghost_button(state, "Analyze position", {
                let handles = handles.clone();
                move || {
                    if state.game.get_untracked().is_none() {
                        actions::new_game(state, &handles);
                    }
                    actions::analyze_game(state, &handles);
                }
            }),
            widgets::ghost_button(state, "Open study", move || state.screen.set(Screen::Study)),
            widgets::ghost_button(state, "Engine tournament", move || {
                state.show_tournament_setup.set(true);
                state.screen.set(Screen::Tournaments);
            }),
            Label::derived(move || state.status.get())
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        ))
        .style(|s| s.width_full().row_gap(10.0).min_width(0.0)),
    )
    .style(|s| s.flex_grow(1.0f32).min_width(280.0).max_width(420.0));

    Stack::vertical((
        logo,
        widgets::curious_title("MUJRIM", 56.0).style(move |s| {
            s.color(theme::rgba(pal().text_primary))
                .opacity(0.55 + 0.45 * state.hub_progress.get())
        }),
        Label::new("Play · Analyze · Prepare · Compete").style(move |s| {
            s.font_size(16.0)
                .color(theme::rgba(pal().accent_alt))
                .opacity(0.55 + 0.45 * state.hub_progress.get())
        }),
        Label::new("A full desktop chess studio for every UCI engine on your machine.")
            .style(move |s| s.font_size(13.0).color(theme::rgba(pal().text_secondary))),
        Stack::horizontal((setup, actions_card)).style(|s| {
            s.width_full()
                .max_width(980.0)
                .col_gap(16.0)
                .row_gap(16.0)
                .flex_wrap(FlexWrap::Wrap)
                .items_stretch()
                .justify_center()
        }),
    ))
    .style(move |s| {
        s.size_full()
            .items_center()
            .padding(28.0)
            .row_gap(12.0)
            .min_width(0.0)
            .min_height(0.0)
            .color(theme::rgba(pal().text_primary))
    })
    .scroll()
}

fn player_column(state: AppState, handles: &AppHandles, white: bool) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let bundled = handles.bundled.clone();
    let choices = bundled_engine_choices(&bundled);
    let show_picker = move || {
        let mode = state.selected_mode.get();
        if white {
            matches!(mode, GameMode::EngineVsEngine)
        } else {
            matches!(mode, GameMode::HumanVsEngine | GameMode::EngineVsEngine)
        }
    };
    Stack::vertical((
        Stack::horizontal((
            widgets::side_badge(if white { "W" } else { "B" }, white),
            Stack::vertical((
                Label::new(if white { "White" } else { "Black" })
                    .style(move |s| s.font_size(11.0).color(theme::rgba(pal().text_secondary))),
                Label::derived(move || {
                    if white {
                        state.white_player.get().to_string()
                    } else {
                        state.black_player.get().to_string()
                    }
                })
                .style(move |s| s.font_size(13.0).color(theme::rgba(pal().text_primary))),
            ))
            .style(|s| s.row_gap(2.0).min_width(0.0)),
        ))
        .style(|s| s.col_gap(10.0).items_center().width_full()),
        dyn_view({
            let bundled = bundled.clone();
            let choices = choices.clone();
            move || {
                if !show_picker() {
                    return Empty::new().into_any();
                }
                if choices.is_empty() {
                    return Label::new("No bundled engines found — using Mujrim built-in.")
                        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary)))
                        .into_any();
                }
                let bundled = bundled.clone();
                let choices = choices.clone();
                let list = choices.clone();
                Stack::vertical((
                    widgets::picker(
                        state,
                        {
                            let bundled = bundled.clone();
                            move || {
                                let player = if white {
                                    state.white_player.get()
                                } else {
                                    state.black_player.get()
                                };
                                selected_bundled_engine(&bundled, &player)
                                    .or_else(|| choices.first().cloned())
                                    .unwrap_or(BundledEngineChoice {
                                        index: 0,
                                        label: "Engine".into(),
                                    })
                            }
                        },
                        list,
                        {
                            let bundled = bundled.clone();
                            move |choice: BundledEngineChoice| {
                                if let Some(engine) = bundled.get(choice.index) {
                                    let player = PlayerConfig::External {
                                        path: engine.path.to_string_lossy().into_owned(),
                                        protocol: ExternalEngineProtocol::Uci,
                                    };
                                    if white {
                                        state.white_player.set(player);
                                    } else {
                                        state.black_player.set(player);
                                    }
                                }
                            }
                        },
                    ),
                    Stack::horizontal((
                        widgets::ghost_button(state, "Load UCI", {
                            move || {
                                actions::pick_external_engine(
                                    state,
                                    white,
                                    ExternalEngineProtocol::Uci,
                                );
                            }
                        }),
                        widgets::ghost_button(state, "Load XBoard", {
                            move || {
                                actions::pick_external_engine(
                                    state,
                                    white,
                                    ExternalEngineProtocol::Xboard,
                                );
                            }
                        }),
                    ))
                    .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
                ))
                .style(|s| s.width_full().row_gap(8.0))
                .into_any()
            }
        }),
    ))
    .style(|s| s.width_full().row_gap(8.0).min_width(0.0))
}

fn playing(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        playing_sidebar(state, handles),
    )
}

fn analysis(state: AppState, handles: AppHandles) -> impl IntoView {
    workspace(
        state,
        handles.clone(),
        false,
        analysis_sidebar(state, handles),
    )
}

fn tournaments(state: AppState, handles: AppHandles) -> impl IntoView {
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
            board_pane(state, handles, show_clocks),
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
        dock::bottom_dock(state),
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
            widgets::ghost_button(state, "Setup", move || {
                state.show_tournament_setup.set(true)
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

fn study(state: AppState, handles: AppHandles) -> impl IntoView {
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
