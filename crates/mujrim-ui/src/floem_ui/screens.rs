//! Menu, playing, study, tournament, and analysis screens.

use floem::prelude::*;
use mujrim_protocols::catalog::DiscoveredEngine;

use crate::app_core::engine::{GameMode, PlayerConfig, bundled_engine_choices};
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

pub fn root_content(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || match state.screen.get() {
        Screen::Menu => menu(state, handles.clone()).into_any(),
        Screen::Playing => playing(state, handles.clone()).into_any(),
        Screen::Study => study(state, handles.clone()).into_any(),
        Screen::Tournaments => tournaments(state, handles.clone()).into_any(),
        Screen::Analysis => analysis(state, handles.clone()).into_any(),
    })
    .style(|s| s.size_full().min_height(0.0))
}

fn menu(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let logo_bytes = handles.logo.clone();
    let logo = img(move || logo_bytes.clone()).style(|s| s.size(80, 80).border_radius(16.0));
    Stack::vertical((
        logo,
        Label::new("Mujrim Chess").style(|s| s.font_size(28.0).font_bold()),
        Label::new("Play, study, and run engine tournaments.")
            .style(move |s| s.font_size(13.0).color(theme::rgba(pal().text_secondary))),
        player_pickers(state, &handles),
        Stack::horizontal((
            Button::new("Start game").action({
                let handles = handles.clone();
                move || actions::new_game(state, &handles)
            }),
            Button::new("Study").action(move || state.screen.set(Screen::Study)),
            Button::new("Tournaments").action(move || {
                state.show_tournament_setup.set(true);
                state.screen.set(Screen::Tournaments);
            }),
            Button::new("Analyze").action({
                let handles = handles.clone();
                move || {
                    if state.game.get_untracked().is_none() {
                        actions::new_game(state, &handles);
                    }
                    actions::analyze_game(state, &handles);
                }
            }),
        ))
        .style(|s| s.col_gap(8.0)),
        Label::derived(move || state.status.get())
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
    ))
    .style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .row_gap(14.0)
            .padding(24.0)
            .color(theme::rgba(pal().text_primary))
    })
}

fn player_pickers(state: AppState, handles: &AppHandles) -> impl IntoView {
    let engines = handles.bundled.clone();
    let engines_mode = engines.clone();
    Stack::vertical((
        cycle_enum(
            state,
            "Mode",
            move || state.selected_mode.get().to_string(),
            move || {
                let next = match state.selected_mode.get_untracked() {
                    GameMode::HumanVsHuman => GameMode::HumanVsEngine,
                    GameMode::HumanVsEngine => GameMode::EngineVsEngine,
                    GameMode::EngineVsEngine => GameMode::HumanVsHuman,
                };
                state.selected_mode.set(next);
                match next {
                    GameMode::HumanVsHuman => {
                        state.white_player.set(PlayerConfig::Human);
                        state.black_player.set(PlayerConfig::Human);
                    }
                    GameMode::HumanVsEngine => {
                        state.white_player.set(PlayerConfig::Human);
                        state.black_player.set(default_engine(&engines_mode));
                    }
                    GameMode::EngineVsEngine => {
                        state.white_player.set(default_engine(&engines_mode));
                        state.black_player.set(PlayerConfig::BuiltIn { depth: 12 });
                    }
                }
            },
        ),
        Label::derived(move || format!("White: {}", state.white_player.get()))
            .style(|s| s.font_size(12.0)),
        Label::derived(move || format!("Black: {}", state.black_player.get()))
            .style(|s| s.font_size(12.0)),
        Button::new("Cycle Black engine").action({
            let bundled = engines.clone();
            move || {
                let choices = bundled_engine_choices(&bundled);
                if choices.is_empty() {
                    state.black_player.update(|player| {
                        *player = match player {
                            PlayerConfig::BuiltIn { depth } => PlayerConfig::BuiltIn {
                                depth: (*depth % 20) + 4,
                            },
                            _ => PlayerConfig::BuiltIn { depth: 16 },
                        };
                    });
                    return;
                }
                let current = match state.black_player.get_untracked() {
                    PlayerConfig::External { path, .. } => bundled
                        .iter()
                        .position(|engine| engine.path.to_string_lossy() == path)
                        .unwrap_or(0),
                    _ => 0,
                };
                let next = (current + 1) % bundled.len();
                state.black_player.set(PlayerConfig::External {
                    path: bundled[next].path.to_string_lossy().into_owned(),
                    protocol: ExternalEngineProtocol::Uci,
                });
            }
        }),
    ))
    .style(|s| s.row_gap(6.0).items_center())
}

fn default_engine(bundled: &[DiscoveredEngine]) -> PlayerConfig {
    bundled
        .first()
        .map_or(PlayerConfig::BuiltIn { depth: 16 }, |engine| {
            PlayerConfig::External {
                path: engine.path.to_string_lossy().into_owned(),
                protocol: ExternalEngineProtocol::Uci,
            }
        })
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
                s.width_pct(layout::SIDE_PANE_PCT)
                    .height_full()
                    .min_width(0.0)
                    .padding(12.0)
                    .row_gap(8.0)
                    .background(theme::rgba(pal().sidebar))
                    .border_left(1.0)
                    .border_color(theme::rgba(pal().border))
            }),
        ))
        .style(|s| {
            s.width_full()
                .flex_grow(1.0f32)
                .min_height(0.0)
                .items_stretch()
        }),
        dock::bottom_dock(state),
    ))
    .style(move |s| s.size_full().color(theme::rgba(pal().text_primary)))
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
        .style(|s| s.size_full().flex_grow(1.0f32).min_height(0.0)),
        Label::derived(move || state.status.get()).style(move |s| {
            s.font_size(11.0)
                .padding_top(6.0)
                .color(theme::rgba(pal().text_secondary))
        }),
    ))
    .style(|s| {
        s.width_pct(layout::BOARD_PANE_PCT)
            .height_full()
            .min_width(0.0)
            .padding(12.0)
    })
}

fn empty_board(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::derived(move || {
            (if state.screen.get() == Screen::Tournaments {
                "Configure the tournament, then Start."
            } else {
                "Start a game from Home."
            })
            .to_owned()
        })
        .style(|s| s.font_size(16.0).font_bold()),
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
    .style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .row_gap(8.0)
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
        Button::new("Coach review").action(move || actions::annotate_last_move(state)),
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
    .style(|s| s.flex_col().row_gap(8.0).height_full().min_height(0.0))
    .scroll()
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
        Button::new("Re-run").action({
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
        Stack::horizontal((
            Button::new("<<").action(move || {
                state.review_ply.set(Some(0));
            }),
            Button::new("<").action(move || {
                state.review_ply.update(|ply| {
                    *ply = Some(
                        ply.unwrap_or(state.move_log.get_untracked().len())
                            .saturating_sub(1),
                    );
                });
            }),
            Button::new(">").action(move || {
                let len = state.move_log.get_untracked().len();
                state.review_ply.update(|ply| {
                    let next = ply.unwrap_or(0).saturating_add(1).min(len);
                    *ply = Some(next);
                });
            }),
            Button::new(">>").action(move || {
                state
                    .review_ply
                    .set(Some(state.move_log.get_untracked().len()));
            }),
        ))
        .style(|s| s.col_gap(4.0)),
    ))
    .style(|s| s.flex_col().row_gap(8.0).height_full().min_height(0.0))
    .scroll()
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
            Button::new("Setup").action(move || state.show_tournament_setup.set(true)),
            Button::new("Cancel").action({
                let handles = handles.clone();
                move || actions::cancel_tournament(&handles)
            }),
        ))
        .style(|s| s.col_gap(6.0)),
    ))
    .style(|s| s.flex_col().row_gap(8.0).height_full().min_height(0.0))
    .scroll()
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
        s.font_size(12.0).color(theme::rgba(
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
        Label::new("Study").style(|s| s.font_size(20.0).font_bold()),
        Stack::horizontal((
            TextInput::new(state.study_query).style(|s| s.width(280.0)),
            Button::new("Search").action({
                let handles = handles.clone();
                move || actions::refresh_study(state, &handles)
            }),
            Button::new("Import PGN").action({
                let handles = handles.clone();
                move || actions::import_pgn(state, &handles)
            }),
            Button::new("Index openings").action({
                let handles = handles.clone();
                move || actions::index_openings(state, &handles)
            }),
            Button::new("Train").action({
                let handles = handles.clone();
                move || actions::start_puzzle(state, &handles)
            }),
        ))
        .style(|s| s.col_gap(8.0).items_center()),
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
                    Button::new(format!("{title}\n{detail}")).action({
                        let handles = handles.clone();
                        move || actions::load_library_game(state, &handles, id.clone())
                    })
                })
                .collect::<Vec<_>>()
                .into_view()
        }),
        Label::derived(move || state.status.get()),
    ))
    .style(move |s| {
        s.size_full()
            .padding(20.0)
            .row_gap(12.0)
            .color(theme::rgba(pal().text_primary))
    })
    .scroll()
}

fn cycle_enum(
    _state: AppState,
    label: &'static str,
    value: impl Fn() -> String + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Stack::horizontal((
        Label::new(label).style(|s| s.font_size(12.0)),
        Button::new(Label::derived(value)).action(action),
    ))
    .style(|s| s.col_gap(8.0).items_center())
}
