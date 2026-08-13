//! Options and tournament-setup modals.

use floem::prelude::*;
use updater::syzygy::SyzygyPieceSet;

use crate::app_core::arrows::{ArrowColor, ArrowShape, ArrowSize};
use crate::app_core::audio::{GameMood, SoundTheme};
use crate::app_core::engine::EngineConfig;
use crate::app_core::palette::BoardTheme;
use crate::app_core::pieces::PieceSet;
use crate::app_core::settings::{CaptureAnimStyle, CoordPosition, OptionsTab};
use crate::app_core::tournament_setup::TimeControlPreset;
use mujrim_study::tournament::TournamentFormat;

use super::actions;
use super::state::{AppHandles, AppState};
use super::theme;

pub fn options_modal(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Stack::horizontal((
            tab_btn(state, OptionsTab::Settings, "Display"),
            tab_btn(state, OptionsTab::Tools, "Tools"),
            Button::new("Close").action(move || {
                state.show_options.set(false);
                state.persist_settings();
            }),
        ))
        .style(|s| s.col_gap(8.0).items_center()),
        dyn_view(move || {
            if state.options_tab.get() == OptionsTab::Tools {
                tools_tab(state, handles.clone()).into_any()
            } else {
                settings_tab(state, handles.clone()).into_any()
            }
        }),
    ))
    .style(move |s| {
        let pal = pal();
        s.absolute()
            .inset_top(52.0)
            .inset_left(80.0)
            .width(680.0)
            .max_height(600.0)
            .padding(14.0)
            .border_radius(10.0)
            .background(theme::rgba(pal.panel))
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .z_index(30)
            .flex_col()
            .row_gap(10.0)
    })
    .scroll()
}

fn tab_btn(state: AppState, tab: OptionsTab, label: &'static str) -> impl IntoView {
    Button::new(label).action(move || {
        state.options_tab.set(tab);
        if tab == OptionsTab::Tools {
            actions::refresh_updater_status(state);
        }
    })
}

fn settings_tab(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Stack::vertical((
            Label::new("Display").style(|s| s.font_size(16.0).font_bold()),
            cycle_row(
                state,
                "Theme",
                move || state.settings.get().board_theme.to_string(),
                move || {
                    state.settings.update(|settings| {
                        let idx = BoardTheme::ALL
                            .iter()
                            .position(|theme| *theme == settings.board_theme)
                            .unwrap_or(0);
                        settings.board_theme = BoardTheme::ALL[(idx + 1) % BoardTheme::ALL.len()];
                    });
                },
            ),
            cycle_row(
                state,
                "Pieces",
                move || state.settings.get().piece_set.to_string(),
                move || {
                    state.settings.update(|settings| {
                        let idx = PieceSet::ALL
                            .iter()
                            .position(|set| *set == settings.piece_set)
                            .unwrap_or(0);
                        settings.piece_set = PieceSet::ALL[(idx + 1) % PieceSet::ALL.len()];
                    });
                },
            ),
            toggle_row(
                state,
                "Coordinates",
                move || state.settings.get().show_coords,
                move || {
                    state.settings.update(|s| s.show_coords = !s.show_coords);
                },
            ),
            cycle_row(
                state,
                "Coord position",
                move || state.settings.get().coord_position.to_string(),
                move || {
                    state.settings.update(|s| {
                        s.coord_position = match s.coord_position {
                            CoordPosition::Inside => CoordPosition::Outside,
                            CoordPosition::Outside => CoordPosition::Inside,
                        };
                    });
                },
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        Stack::vertical((
            Label::new("Audio").style(|s| s.font_size(16.0).font_bold()),
            toggle_row(state, "SFX", move || state.settings.get().sfx_on, {
                let handles = handles.clone();
                move || {
                    state.settings.update(|s| s.sfx_on = !s.sfx_on);
                    if let Some(sound) = handles.sound.borrow_mut().as_mut()
                        && !state.settings.get_untracked().sfx_on
                    {
                        sound.stop_bgm();
                    }
                }
            }),
            cycle_row(
                state,
                "Mood",
                move || state.settings.get().game_mood.to_string(),
                {
                    let handles = handles.clone();
                    move || {
                        state.settings.update(|s| {
                            s.game_mood = match s.game_mood {
                                GameMood::Playful => GameMood::Joyful,
                                GameMood::Joyful => GameMood::Mystique,
                                GameMood::Mystique => GameMood::Playful,
                            };
                        });
                        if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                            sound.set_mood(state.settings.get_untracked().game_mood);
                        }
                    }
                },
            ),
            cycle_row(
                state,
                "Board SFX",
                move || state.settings.get().sound_theme.to_string(),
                {
                    let handles = handles.clone();
                    move || {
                        state.settings.update(|s| {
                            s.sound_theme = match s.sound_theme {
                                SoundTheme::Wood => SoundTheme::Crystal,
                                SoundTheme::Crystal => SoundTheme::Soft,
                                SoundTheme::Soft => SoundTheme::Wood,
                            };
                        });
                        if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                            sound.set_sound_theme(state.settings.get_untracked().sound_theme);
                        }
                    }
                },
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        Stack::vertical((
            Label::new("Gameplay").style(|s| s.font_size(16.0).font_bold()),
            toggle_row(
                state,
                "Legal dots",
                move || state.settings.get().show_legal_moves,
                move || {
                    state
                        .settings
                        .update(|s| s.show_legal_moves = !s.show_legal_moves);
                },
            ),
            toggle_row(
                state,
                "Last move",
                move || state.settings.get().show_last_move,
                move || {
                    state
                        .settings
                        .update(|s| s.show_last_move = !s.show_last_move);
                },
            ),
            toggle_row(
                state,
                "Premoves",
                move || state.settings.get().premoves_enabled,
                move || {
                    state
                        .settings
                        .update(|s| s.premoves_enabled = !s.premoves_enabled);
                },
            ),
            toggle_row(
                state,
                "Multi-premoves",
                move || state.settings.get().multi_premoves,
                move || {
                    state
                        .settings
                        .update(|s| s.multi_premoves = !s.multi_premoves);
                },
            ),
            toggle_row(
                state,
                "Auto-flip Black",
                move || state.settings.get().auto_flip_black,
                move || {
                    state
                        .settings
                        .update(|s| s.auto_flip_black = !s.auto_flip_black);
                },
            ),
            cycle_row(
                state,
                "Capture FX",
                move || state.settings.get().capture_anim_style.to_string(),
                move || {
                    state.settings.update(|s| {
                        s.capture_anim_style = match s.capture_anim_style {
                            CaptureAnimStyle::Instant => CaptureAnimStyle::Explosion,
                            CaptureAnimStyle::Explosion => CaptureAnimStyle::Fire,
                            CaptureAnimStyle::Fire => CaptureAnimStyle::Instant,
                        };
                    });
                },
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        Stack::vertical((
            Label::new("Arrows").style(|s| s.font_size(16.0).font_bold()),
            toggle_row(
                state,
                "Draw arrows",
                move || state.settings.get().draw_arrows,
                move || {
                    state.settings.update(|s| s.draw_arrows = !s.draw_arrows);
                },
            ),
            cycle_row(
                state,
                "Shape",
                move || state.settings.get().arrow_shape.to_string(),
                move || {
                    state.settings.update(|s| {
                        s.arrow_shape = match s.arrow_shape {
                            ArrowShape::Smart => ArrowShape::Straight,
                            ArrowShape::Straight => ArrowShape::Smart,
                        };
                    });
                },
            ),
            cycle_row(
                state,
                "Color",
                move || state.settings.get().arrow_color.to_string(),
                move || {
                    state.settings.update(|s| {
                        let idx = ArrowColor::ALL
                            .iter()
                            .position(|c| *c == s.arrow_color)
                            .unwrap_or(0);
                        s.arrow_color = ArrowColor::ALL[(idx + 1) % ArrowColor::ALL.len()];
                    });
                },
            ),
            cycle_row(
                state,
                "Size",
                move || state.settings.get().arrow_size.to_string(),
                move || {
                    state.settings.update(|s| {
                        let idx = ArrowSize::ALL
                            .iter()
                            .position(|c| *c == s.arrow_size)
                            .unwrap_or(0);
                        s.arrow_size = ArrowSize::ALL[(idx + 1) % ArrowSize::ALL.len()];
                    });
                },
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        Stack::vertical((
            Label::new("Motion").style(|s| s.font_size(16.0).font_bold()),
            toggle_row(
                state,
                "Piece slide",
                move || state.settings.get().piece_slide,
                move || {
                    state.settings.update(|s| s.piece_slide = !s.piece_slide);
                },
            ),
            toggle_row(
                state,
                "System motion",
                move || state.settings.get().system_motion,
                move || {
                    state
                        .settings
                        .update(|s| s.system_motion = !s.system_motion);
                },
            ),
            cycle_row(
                state,
                "Anim speed",
                move || {
                    crate::app_core::motion::AnimPace::from_setting(state.settings.get().anim_speed)
                        .label()
                        .to_owned()
                },
                move || {
                    state
                        .settings
                        .update(|s| s.anim_speed = (s.anim_speed + 1) % 3);
                },
            ),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
        engine_rows(state),
    ))
    .style(|s| s.row_gap(8.0).width_full())
}

fn engine_rows(state: AppState) -> impl IntoView {
    Stack::vertical((
        Label::new("Engine").style(|s| s.font_size(16.0).font_bold()),
        cycle_row(
            state,
            "Hash MB",
            move || state.engine_cfg.get().hash_mb.to_string(),
            move || {
                state.engine_cfg.update(|cfg| {
                    cfg.hash_mb = match cfg.hash_mb {
                        16 => 32,
                        32 => 64,
                        64 => 128,
                        128 => 256,
                        _ => 16,
                    };
                });
            },
        ),
        cycle_row(
            state,
            "Threads",
            move || state.engine_cfg.get().threads.to_string(),
            move || {
                state
                    .engine_cfg
                    .update(|cfg| cfg.threads = cfg.threads % 8 + 1);
            },
        ),
        cycle_row(
            state,
            "Move time (s)",
            move || state.engine_cfg.get().time_per_move.to_string(),
            move || {
                state.engine_cfg.update(|cfg| {
                    cfg.time_per_move = (cfg.time_per_move % 15) + 1;
                });
            },
        ),
        toggle_row(
            state,
            "NNUE",
            move || state.engine_cfg.get().use_nnue,
            move || {
                state.engine_cfg.update(|cfg| cfg.use_nnue = !cfg.use_nnue);
            },
        ),
        toggle_row(
            state,
            "Book",
            move || state.engine_cfg.get().use_book,
            move || {
                state.engine_cfg.update(|cfg| cfg.use_book = !cfg.use_book);
            },
        ),
        toggle_row(
            state,
            "Ponder",
            move || state.engine_cfg.get().ponder,
            move || {
                state.engine_cfg.update(|cfg| cfg.ponder = !cfg.ponder);
            },
        ),
        Button::new("Apply engine limits").action(move || {
            actions::persist_engine(state, state.engine_cfg.get_untracked());
        }),
    ))
    .style(|s| s.row_gap(8.0))
}

fn tools_tab(state: AppState, _handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        Label::new("Syzygy").style(|s| s.font_size(16.0).font_bold()),
        Label::derived(move || state.syzygy_status.get()),
        cycle_row(
            state,
            "Piece set",
            move || format!("{:?}", state.syzygy_piece_set.get()),
            move || {
                state.syzygy_piece_set.update(|set| {
                    *set = match *set {
                        SyzygyPieceSet::Standard => SyzygyPieceSet::Extended,
                        SyzygyPieceSet::Extended => SyzygyPieceSet::Full,
                        SyzygyPieceSet::Full => SyzygyPieceSet::Standard,
                    };
                });
            },
        ),
        Button::new("Download Syzygy").action(move || actions::download_syzygy(state)),
        Label::new("NNUE").style(|s| s.font_size(16.0).font_bold()),
        Label::derived(move || state.nnue_status.get()),
        Button::new("Download NNUE nets").action(move || actions::download_nnue(state)),
        Label::new("Tuning").style(|s| s.font_size(16.0).font_bold()),
        Label::derived(move || state.tuning_status.get()),
        Button::new("Refresh status").action(move || actions::refresh_updater_status(state)),
    ))
    .style(|s| s.row_gap(8.0))
}

pub fn tournament_setup_modal(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        Label::new("Tournament setup").style(|s| s.font_size(16.0).font_bold()),
        cycle_row(
            state,
            "Format",
            move || state.tournament_setup.get().format.to_string(),
            move || {
                state.tournament_setup.update(|setup| {
                    setup.format = match setup.format {
                        TournamentFormat::RoundRobin => TournamentFormat::DoubleRoundRobin,
                        TournamentFormat::DoubleRoundRobin => TournamentFormat::Swiss,
                        TournamentFormat::Swiss => TournamentFormat::Knockout,
                        TournamentFormat::Knockout => TournamentFormat::RoundRobin,
                    };
                });
            },
        ),
        cycle_row(
            state,
            "Time",
            move || format!("{:?}", state.tournament_setup.get().time_control),
            move || {
                state.tournament_setup.update(|setup| {
                    setup.time_control = match setup.time_control {
                        TimeControlPreset::ThreePlusTwo => TimeControlPreset::FivePlusThree,
                        TimeControlPreset::FivePlusThree => TimeControlPreset::ThreePlusTwo,
                    };
                });
            },
        ),
        Button::new("Toggle all local engines").action({
            let handles = handles.clone();
            move || {
                let roster = crate::app_core::logic::tournament_engine_roster(
                    &handles.bundled,
                    &handles.catalog.borrow(),
                );
                state.tournament_setup.update(|setup| {
                    if setup.selected_engine_paths.len() == roster.len() {
                        setup.selected_engine_paths.clear();
                    } else {
                        setup.selected_engine_paths =
                            roster.iter().map(|engine| engine.path.clone()).collect();
                    }
                });
            }
        }),
        Label::derived(move || {
            format!(
                "{} engines selected",
                state.tournament_setup.get().selected_engine_paths.len()
            )
        }),
        Stack::horizontal((
            Button::new("Start").action({
                let handles = handles.clone();
                move || actions::start_tournament(state, &handles)
            }),
            Button::new("Cancel").action(move || state.show_tournament_setup.set(false)),
        ))
        .style(|s| s.col_gap(8.0)),
    ))
    .style(move |s| {
        let pal = pal();
        s.absolute()
            .inset_top(72.0)
            .inset_left(140.0)
            .width(460.0)
            .padding(14.0)
            .border_radius(10.0)
            .background(theme::rgba(pal.panel))
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .z_index(30)
            .row_gap(8.0)
    })
}

fn cycle_row(
    state: AppState,
    label: &'static str,
    value: impl Fn() -> String + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Stack::horizontal((
        Label::new(label).style(|s| s.width(160.0)),
        Button::new(Label::derived(value)).action(action),
    ))
    .style(move |s| {
        s.col_gap(12.0).items_center().color(theme::rgba(
            theme::palette(state.settings.get().board_theme).text_primary,
        ))
    })
}

fn toggle_row(
    state: AppState,
    label: &'static str,
    value: impl Fn() -> bool + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    cycle_row(
        state,
        label,
        move || if value() { "On".into() } else { "Off".into() },
        action,
    )
}

#[allow(dead_code)]
fn _engine_cfg_copy(cfg: EngineConfig) -> EngineConfig {
    cfg
}
