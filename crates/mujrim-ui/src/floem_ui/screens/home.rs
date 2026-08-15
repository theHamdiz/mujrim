//! Home hub: setup + engine settings + start.

use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap, Overflow};

use crate::app_core::engine::{BundledEngineChoice, GameMode, MAX_GUI_HASH_MB, PlayerConfig};
use crate::app_core::hub::{self, CoinFlipState};
use crate::app_core::logic;
use crate::app_core::settings::Screen;
use crate::app_core::uci_process::ExternalEngineProtocol;

use super::super::actions;
use super::super::state::{AppHandles, AppState, refresh_ateed_cli};
use super::super::theme;
use super::super::widgets;

pub fn menu(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let logo_bytes = handles.logo.clone();
    let bg_bytes = handles.chess_bg.clone();
    let entrance = move || 0.55 + 0.45 * state.hub_progress.get();

    let hero = Stack::vertical((
        img(move || logo_bytes.clone()).style(|s| s.size(128, 128).border_radius(28.0)),
        widgets::curious_title("MUJRIM", 64.0)
            .style(move |s| s.color(theme::rgba(pal().text_primary)).opacity(entrance())),
        Label::new("Play · Analyze · Prepare · Compete").style(move |s| {
            s.font_size(18.0)
                .color(theme::rgba(pal().accent_alt))
                .opacity(entrance())
        }),
        Label::new("A full desktop chess studio for every UCI engine on your machine.")
            .style(move |s| s.font_size(14.0).color(theme::rgba(pal().text_secondary))),
        Stack::horizontal((
            widgets::ghost_button(state, "Analyze Position", {
                let handles = handles.clone();
                move || {
                    if state.game.get_untracked().is_none() {
                        actions::new_game(state, &handles);
                    }
                    actions::analyze_game(state, &handles);
                }
            }),
            widgets::ghost_button(state, "Open Study", {
                let handles = handles.clone();
                move || {
                    actions::ensure_study_board(state, &handles);
                    state.screen.set(Screen::Study);
                }
            }),
            widgets::ghost_button(state, "Open Learn", {
                let handles = handles.clone();
                move || {
                    actions::open_learn(state, &handles);
                }
            }),
            widgets::ghost_button(state, "Engine Tournament", {
                let handles = handles.clone();
                move || {
                    actions::open_tournaments_screen(state, &handles);
                }
            }),
            widgets::ghost_button(state, "Ateed Studio", move || {
                refresh_ateed_cli(state);
                state.screen.set(Screen::Ateed);
            }),
        ))
        .style(|s| s.col_gap(10.0).flex_wrap(FlexWrap::Wrap).justify_center()),
    ))
    .style(|s| s.items_center().row_gap(8.0).width_full());

    let setup = widgets::glass_card(state, game_setup(state, handles.clone())).style(|s| {
        s.width(360.0)
            .min_width(280.0)
            .flex_grow(1.0f32)
            .max_width(420.0)
    });
    let settings = widgets::glass_card(state, engine_settings(state, handles.clone())).style(|s| {
        s.width(360.0)
            .min_width(280.0)
            .flex_grow(1.0f32)
            .max_width(420.0)
    });

    let start = widgets::primary_button(state, "Start Game", {
        let handles = handles.clone();
        move || actions::new_game(state, &handles)
    })
    .style(|s| s.min_width(220.0).height(48.0).font_size(16.0));

    let foreground = Stack::vertical((
        hero,
        Stack::horizontal((setup, settings)).style(|s| {
            s.width_full()
                .max_width(780.0)
                .col_gap(24.0)
                .row_gap(16.0)
                .flex_wrap(FlexWrap::Wrap)
                .items_stretch()
                .justify_center()
        }),
        game_resume_banner(state, handles.clone()),
        start,
        Label::new("Studio · multi-engine ready")
            .style(move |s| s.font_size(11.0).color(theme::rgba(pal().text_secondary))),
        Label::derived(move || state.status.get())
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
    ))
    .style(|s| {
        s.width_full()
            .items_center()
            .padding(24.0)
            .row_gap(16.0)
            .min_width(0.0)
    })
    .scroll();

    Stack::new((
        img(move || bg_bytes.clone()).style(|s| s.size_full().absolute().pointer_events_none()),
        foreground.style(|s| s.size_full().min_width(0.0).min_height(0.0)),
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

fn game_resume_banner(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Label::derived(move || {
            state
                .game_resume_prompt
                .get()
                .map_or_else(String::new, |checkpoint| {
                    format!(
                        "Interrupted game · {} vs {} · {} ply",
                        checkpoint.parsed_white(),
                        checkpoint.parsed_black(),
                        checkpoint.moves.len()
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
        Label::new("Restore the saved position. The side to move will think again.").style(
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
            widgets::primary_button(state, "Resume game", {
                let handles = handles.clone();
                move || actions::resume_paused_game(state, &handles)
            }),
            widgets::ghost_button(state, "Discard", move || {
                actions::discard_paused_game(state)
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let s = s
            .width_full()
            .max_width(520.0)
            .row_gap(8.0)
            .padding(12.0)
            .border_radius(12.0)
            .border(1.0)
            .border_color(theme::rgba(pal.accent))
            .background(theme::rgba(pal.panel));
        if state.game_resume_prompt.get().is_some() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn game_setup(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Game Setup", pal),
        Label::new("Choose players and load engines")
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        Label::new("Mode")
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        widgets::picker(state, move || state.selected_mode.get(), GameMode::ALL, {
            let handles = handles.clone();
            move |mode| actions::select_mode(state, &handles, mode)
        }),
        player_column(state, &handles, true),
        player_column(state, &handles, false),
        coin_flip_row(state),
    ))
    .style(|s| s.width_full().row_gap(10.0).min_width(0.0))
}

fn engine_settings(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Engine Settings", pal),
        Label::new("Tune search, book, and network")
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        widgets::stepper_row(
            state,
            "Time / Move",
            "s",
            move || state.engine_cfg.get().time_per_move,
            move |value| {
                state
                    .engine_cfg
                    .update(|cfg| cfg.time_per_move = hub::clamp_cfg_time(value));
            },
            1,
            30,
        ),
        widgets::stepper_row(
            state,
            "Max Depth",
            "",
            move || state.engine_cfg.get().max_depth,
            move |value| {
                state
                    .engine_cfg
                    .update(|cfg| cfg.max_depth = hub::clamp_cfg_depth(value));
            },
            1,
            64,
        ),
        widgets::stepper_row(
            state,
            "Hash",
            "MB",
            move || state.engine_cfg.get().hash_mb,
            move |value| {
                state
                    .engine_cfg
                    .update(|cfg| cfg.hash_mb = value.clamp(1, MAX_GUI_HASH_MB));
            },
            1,
            MAX_GUI_HASH_MB,
        ),
        widgets::stepper_row(
            state,
            "Threads",
            "",
            move || state.engine_cfg.get().threads,
            move |value| {
                state
                    .engine_cfg
                    .update(|cfg| cfg.threads = hub::clamp_cfg_threads(value));
            },
            1,
            32,
        ),
        widgets::toggle_row(
            state,
            "Ponder",
            move || state.engine_cfg.get().ponder,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.ponder = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Opening Book",
            move || state.engine_cfg.get().use_book,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.use_book = value);
            },
        ),
        widgets::toggle_row(
            state,
            "NNUE Eval",
            move || state.engine_cfg.get().use_nnue,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.use_nnue = value);
            },
        ),
        Label::derived(move || {
            state
                .engine_cfg
                .get()
                .eval_file
                .as_ref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Embedded".to_owned())
        })
        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
        Stack::horizontal((
            widgets::ghost_button(state, "Load NNUE File", {
                let handles = handles.clone();
                move || actions::pick_eval_file(state, &handles)
            }),
            widgets::ghost_button(state, "Use Embedded", move || {
                actions::clear_eval_file(state)
            }),
        ))
        .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
    ))
    .style(|s| s.width_full().row_gap(8.0).min_width(0.0))
}

fn coin_flip_row(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    dyn_view(move || {
        if !matches!(state.selected_mode.get(), GameMode::HumanVsEngine) {
            return Empty::new().into_any();
        }
        match state.coin_flip.get() {
            CoinFlipState::Idle => widgets::primary_button(state, "Flip for side", move || {
                actions::start_coin_flip(state)
            })
            .into_any(),
            CoinFlipState::Flipping => Label::new("Flipping…")
                .style(move |s| s.font_size(13.0).color(theme::rgba(pal().accent)))
                .into_any(),
            CoinFlipState::Done { heads } => {
                let text = if heads {
                    "You play White!"
                } else {
                    "You play Black!"
                };
                Label::new(text)
                    .style(move |s| s.font_size(14.0).color(theme::rgba(pal().accent_alt)))
                    .into_any()
            }
        }
    })
}

fn player_column(state: AppState, handles: &AppHandles, white: bool) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
    let choices: Vec<BundledEngineChoice> = roster
        .iter()
        .enumerate()
        .map(|(index, engine)| BundledEngineChoice {
            index,
            label: engine.name.clone(),
        })
        .collect();
    let paths: Vec<std::path::PathBuf> = roster.iter().map(|engine| engine.path.clone()).collect();
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
            let choices = choices.clone();
            let paths = paths.clone();
            let handles = handles.clone();
            move || {
                if !hub::engine_picker_visible(state.selected_mode.get(), white) {
                    return Empty::new().into_any();
                }
                if choices.is_empty() {
                    return Label::new(
                        "No local engines found — using Mujrim built-in. Run scripts/vendor-linux-engines.sh.",
                    )
                    .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary)))
                    .into_any();
                }
                let list = choices.clone();
                let paths = paths.clone();
                let handles = handles.clone();
                Stack::vertical((
                    widgets::picker(
                        state,
                        {
                            let paths = paths.clone();
                            let choices = choices.clone();
                            move || {
                                let player = if white {
                                    state.white_player.get()
                                } else {
                                    state.black_player.get()
                                };
                                let PlayerConfig::External { path, .. } = player else {
                                    return choices.first().cloned().unwrap_or(BundledEngineChoice {
                                        index: 0,
                                        label: "Engine".into(),
                                    });
                                };
                                paths
                                    .iter()
                                    .position(|item| item == std::path::Path::new(&path))
                                    .and_then(|index| choices.get(index).cloned())
                                    .or_else(|| choices.first().cloned())
                                    .unwrap_or(BundledEngineChoice {
                                        index: 0,
                                        label: "Engine".into(),
                                    })
                            }
                        },
                        list,
                        {
                            let paths = paths.clone();
                            move |choice: BundledEngineChoice| {
                                if let Some(path) = paths.get(choice.index) {
                                    let player = PlayerConfig::External {
                                        path: path.to_string_lossy().into_owned(),
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
                            let handles = handles.clone();
                            move || {
                                actions::pick_external_engine(
                                    state,
                                    &handles,
                                    white,
                                    ExternalEngineProtocol::Uci,
                                );
                            }
                        }),
                        widgets::ghost_button(state, "Load XBoard", {
                            let handles = handles.clone();
                            move || {
                                actions::pick_external_engine(
                                    state,
                                    &handles,
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
