//! Menu, playing, study, tournament, and analysis screens.

use floem::prelude::*;
use mujrim_protocols::catalog::DiscoveredEngine;

use crate::app_core::engine::{GameMode, PlayerConfig, bundled_engine_choices};
use crate::app_core::logic;
use crate::app_core::settings::Screen;
use crate::app_core::uci_process::ExternalEngineProtocol;

use super::actions;
use super::board;
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
    let logo = img(move || logo_bytes.clone()).style(|s| s.size(96, 96));
    Stack::vertical((
        logo,
        Label::new("Mujrim Chess").style(|s| s.font_size(32.0).font_bold()),
        Label::new("Play, study, and run engine tournaments.")
            .style(move |s| s.color(theme::rgba(pal().text_secondary))),
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
        .style(|s| s.col_gap(10.0)),
        Label::derived(move || state.status.get())
            .style(move |s| s.color(theme::rgba(pal().text_secondary))),
    ))
    .style(move |s| {
        s.size_full()
            .items_center()
            .justify_center()
            .row_gap(16.0)
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
        Label::derived(move || format!("White: {}", state.white_player.get())),
        Label::derived(move || format!("Black: {}", state.black_player.get())),
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
    .style(|s| s.row_gap(8.0).items_center())
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
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::horizontal((
        Stack::vertical((
            board::board_view(state, handles.clone()),
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
                    .color(theme::rgba(pal().text_secondary))
                    .padding_top(8.0)
            }),
        ))
        .style(|s| s.padding(16.0).row_gap(8.0)),
        Stack::vertical((
            Label::new("Moves").style(|s| s.font_size(16.0).font_bold()),
            Label::derived(move || {
                state
                    .move_log
                    .get()
                    .iter()
                    .enumerate()
                    .map(|(idx, mv)| {
                        logic::annotated_move_label(
                            mv,
                            state.move_annotations.get().get(idx).copied().flatten(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
            })
            .style(|s| s.font_size(13.0)),
            eval_graph::eval_graph(state),
            Button::new("Coach review").action(move || actions::annotate_last_move(state)),
            Label::derived(move || {
                let searching = if state.searching.get() {
                    "searching"
                } else {
                    "idle"
                };
                format!("{searching} · {}", state.status.get())
            }),
        ))
        .style(move |s| {
            s.width(360.0)
                .padding(16.0)
                .row_gap(10.0)
                .background(theme::rgba(pal().panel))
                .height_full()
        })
        .scroll(),
    ))
    .style(move |s| s.size_full().color(theme::rgba(pal().text_primary)))
}

fn study(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::new("Study").style(|s| s.font_size(22.0).font_bold()),
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
        }),
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

fn tournaments(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::new("Tournaments").style(|s| s.font_size(22.0).font_bold()),
        Stack::horizontal((
            Button::new("Setup").action(move || state.show_tournament_setup.set(true)),
            Button::new("Cancel event").action({
                let handles = handles.clone();
                move || actions::cancel_tournament(&handles)
            }),
        ))
        .style(|s| s.col_gap(8.0)),
        Label::derived(move || state.tournament_status.get()),
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            format!(
                "{} · {}/{} matches · {}",
                snap.format_label, snap.completed_matches, snap.total_matches, snap.status_line
            )
        }),
        Label::derived(move || {
            state
                .tournament_snapshot
                .get()
                .standings
                .iter()
                .map(|row| {
                    format!(
                        "{}. {}  {:.1} pts  ({}-{}-{})",
                        row.rank, row.name, row.points, row.wins, row.draws, row.losses
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }),
        Label::derived(move || {
            state
                .tournament_snapshot
                .get()
                .live_games
                .iter()
                .map(|game| {
                    format!(
                        "Live R{} {} vs {}  {}",
                        game.round, game.white, game.black, game.last_uci
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }),
    ))
    .style(move |s| {
        s.size_full()
            .padding(20.0)
            .row_gap(10.0)
            .color(theme::rgba(pal().text_primary))
    })
}

fn analysis(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let telemetry = handles.telemetry.clone();
    Stack::horizontal((
        board::board_view(state, handles.clone()),
        Stack::vertical((
            Label::new("Analysis").style(|s| s.font_size(18.0).font_bold()),
            eval_graph::eval_graph(state),
            Label::derived(move || {
                state
                    .analysis
                    .get()
                    .map_or_else(|| telemetry.get().label, |snap| snap.status)
            }),
            Label::derived(move || {
                state
                    .analysis
                    .get()
                    .and_then(|snap| snap.consensus.clone())
                    .unwrap_or_default()
            }),
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
            .style(|s| s.col_gap(6.0)),
        ))
        .style(move |s| {
            s.width(380.0)
                .padding(16.0)
                .row_gap(10.0)
                .background(theme::rgba(pal().panel))
                .height_full()
        }),
    ))
    .style(move |s| {
        s.size_full()
            .padding(16.0)
            .col_gap(12.0)
            .color(theme::rgba(pal().text_primary))
    })
}

fn cycle_enum(
    _state: AppState,
    label: &'static str,
    value: impl Fn() -> String + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Stack::horizontal((
        Label::new(label),
        Button::new(Label::derived(value)).action(action),
    ))
    .style(|s| s.col_gap(8.0).items_center())
}
