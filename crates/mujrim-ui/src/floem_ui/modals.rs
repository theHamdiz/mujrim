//! Options and tournament-setup overlays.

use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap, Overflow};
use updater::syzygy::SyzygyPieceSet;

use crate::app_core::arrows::{ArrowColor, ArrowShape, ArrowSize};
use crate::app_core::audio::{GameMood, SoundTheme};
use crate::app_core::layout;
use crate::app_core::motion::AnimPace;
use crate::app_core::palette::BoardTheme;
use crate::app_core::pieces::PieceSet;
use crate::app_core::settings::{CaptureAnimStyle, CoordPosition, OptionsTab};
use crate::app_core::tournament_setup::TimeControlPreset;
use mujrim_study::tournament::TournamentFormat;

use super::actions;
use super::state::{AppHandles, AppState};
use super::theme;
use super::widgets;

pub fn options_modal(state: AppState, handles: AppHandles) -> impl IntoView {
    widgets::overlay_frame(
        state,
        move || {
            state.show_options.set(false);
            state.persist_settings();
        },
        Stack::vertical((
            Stack::horizontal((
                widgets::curious_title("Options", 28.0),
                Stack::horizontal((
                    tab_btn(state, OptionsTab::Display, "Display"),
                    tab_btn(state, OptionsTab::Motion, "Motion"),
                    tab_btn(state, OptionsTab::Arrows, "Arrows"),
                    tab_btn(state, OptionsTab::Audio, "Audio"),
                    tab_btn(state, OptionsTab::Analysis, "Analysis"),
                    tab_btn(state, OptionsTab::Tools, "Tools"),
                    widgets::ghost_button(state, "Close", move || {
                        state.show_options.set(false);
                        state.persist_settings();
                    }),
                ))
                .style(|s| s.col_gap(8.0).items_center().flex_wrap(FlexWrap::Wrap)),
            ))
            .style(|s| {
                s.width_full()
                    .items_center()
                    .justify_between()
                    .col_gap(12.0)
                    .row_gap(8.0)
                    .flex_wrap(FlexWrap::Wrap)
            }),
            dyn_view(move || match state.options_tab.get() {
                OptionsTab::Display => display_tab(state).into_any(),
                OptionsTab::Motion => motion_tab(state).into_any(),
                OptionsTab::Arrows => arrows_tab(state).into_any(),
                OptionsTab::Audio => audio_tab(state, handles.clone()).into_any(),
                OptionsTab::Analysis => analysis_tab(state, handles.clone()).into_any(),
                OptionsTab::Tools => tools_tab(state, handles.clone()).into_any(),
            }),
        ))
        .style(|s| s.width_full().row_gap(14.0).min_width(0.0)),
    )
}

fn tab_btn(state: AppState, tab: OptionsTab, label: &'static str) -> impl IntoView {
    Button::new(label)
        .action(move || {
            state.options_tab.set(tab);
            if tab == OptionsTab::Tools {
                actions::refresh_updater_status(state);
            }
        })
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            let active = state.options_tab.get() == tab;
            s.padding_horiz(12.0)
                .padding_vert(6.0)
                .border_radius(10.0)
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

fn display_tab(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Display", pal),
        widgets::picker_row(
            state,
            "Theme",
            move || state.settings.get().board_theme,
            BoardTheme::ALL,
            move |theme| {
                actions::update_settings(state, |settings| settings.board_theme = theme);
            },
        ),
        widgets::picker_row(
            state,
            "Pieces",
            move || state.settings.get().piece_set,
            PieceSet::ALL,
            move |set| {
                actions::update_settings(state, |settings| settings.piece_set = set);
            },
        ),
        widgets::picker_row(
            state,
            "UI font",
            move || crate::app_core::fonts::FontChoice {
                family: state.settings.get().ui_font,
            },
            crate::app_core::fonts::bundled_ui_fonts()
                .into_iter()
                .chain(state.settings.get().custom_font_paths.iter().map(|path| {
                    crate::app_core::fonts::FontChoice {
                        family: std::path::Path::new(path)
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().replace('-', " "))
                            .unwrap_or_else(|| path.clone()),
                    }
                }))
                .collect::<Vec<_>>(),
            move |choice| {
                actions::update_settings(state, |settings| settings.ui_font = choice.family);
            },
        ),
        widgets::picker_row(
            state,
            "Mono font",
            move || crate::app_core::fonts::FontChoice {
                family: state.settings.get().mono_font,
            },
            crate::app_core::fonts::bundled_mono_fonts(),
            move |choice| {
                actions::update_settings(state, |settings| settings.mono_font = choice.family);
            },
        ),
        widgets::toggle_row(
            state,
            "Ligatures",
            move || state.settings.get().font_ligatures,
            move |value| {
                actions::update_settings(state, |settings| settings.font_ligatures = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Speak explanations",
            move || state.settings.get().explain_speak,
            move |value| {
                actions::update_settings(state, |settings| settings.explain_speak = value);
            },
        ),
        widgets::ghost_button(state, "Add font file", move || {
            actions::import_ui_font(state)
        }),
        widgets::toggle_row(
            state,
            "Coordinates",
            move || state.settings.get().show_coords,
            move |value| {
                actions::update_settings(state, |settings| settings.show_coords = value);
            },
        ),
        widgets::picker_row(
            state,
            "Coord position",
            move || state.settings.get().coord_position,
            CoordPosition::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.coord_position = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Auto-flip Black",
            move || state.settings.get().auto_flip_black,
            move |value| {
                actions::update_settings(state, |settings| settings.auto_flip_black = value);
            },
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full().min_width(0.0))
}

fn audio_tab(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Audio", pal),
        widgets::toggle_row(state, "Background music", move || state.bgm_on.get(), {
            let handles = handles.clone();
            move |value| {
                state.bgm_on.set(value);
                actions::update_settings(state, |settings| settings.bgm_on = value);
                if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                    if value {
                        sound.play_bgm(crate::app_core::audio::BgmTrack::Menu);
                    } else {
                        sound.stop_bgm();
                    }
                }
            }
        }),
        widgets::toggle_row(state, "SFX", move || state.settings.get().sfx_on, {
            let handles = handles.clone();
            move |value| {
                actions::update_settings(state, |settings| settings.sfx_on = value);
                if let Some(sound) = handles.sound.borrow_mut().as_mut()
                    && !value
                    && !state.bgm_on.get_untracked()
                {
                    sound.stop_bgm();
                }
            }
        }),
        widgets::stepper_row(
            state,
            "BGM volume",
            "%",
            move || state.settings.get().bgm_volume,
            {
                let handles = handles.clone();
                move |value| {
                    actions::update_settings(state, |settings| settings.bgm_volume = value);
                    if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                        sound.set_volume(value as f32 / 100.0);
                    }
                }
            },
            0,
            100,
        ),
        widgets::picker_row(
            state,
            "Mood",
            move || state.settings.get().game_mood,
            GameMood::ALL,
            {
                let handles = handles.clone();
                move |mood| {
                    actions::update_settings(state, |settings| settings.game_mood = mood);
                    if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                        sound.set_mood(mood);
                    }
                }
            },
        ),
        widgets::picker_row(
            state,
            "Board SFX",
            move || state.settings.get().sound_theme,
            SoundTheme::ALL,
            {
                let handles = handles.clone();
                move |theme| {
                    actions::update_settings(state, |settings| settings.sound_theme = theme);
                    if let Some(sound) = handles.sound.borrow_mut().as_mut() {
                        sound.set_sound_theme(theme);
                    }
                }
            },
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full().min_width(0.0))
}

fn analysis_tab(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Analysis", pal),
        widgets::toggle_row(
            state,
            "Legal dots",
            move || state.settings.get().show_legal_moves,
            move |value| {
                actions::update_settings(state, |settings| settings.show_legal_moves = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Last move",
            move || state.settings.get().show_last_move,
            move |value| {
                actions::update_settings(state, |settings| settings.show_last_move = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Threat highlights",
            move || state.settings.get().show_threats,
            move |value| {
                actions::update_settings(state, |settings| settings.show_threats = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Premoves",
            move || state.settings.get().premoves_enabled,
            move |value| {
                actions::update_settings(state, |settings| settings.premoves_enabled = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Multi-premoves",
            move || state.settings.get().multi_premoves,
            move |value| {
                actions::update_settings(state, |settings| settings.multi_premoves = value);
            },
        ),
        eval_bar_engine_picker(state, handles),
        engine_rows(state),
    ))
    .style(|s| s.row_gap(10.0).width_full().min_width(0.0))
}

fn arrows_tab(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Arrows", pal),
        widgets::toggle_row(
            state,
            "Draw arrows",
            move || state.settings.get().draw_arrows,
            move |value| {
                actions::update_settings(state, |settings| settings.draw_arrows = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Last-move arrow",
            move || state.settings.get().last_move_arrow,
            move |value| {
                actions::update_settings(state, |settings| settings.last_move_arrow = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Ponder arrow",
            move || state.settings.get().ponder_arrow,
            move |value| {
                actions::update_settings(state, |settings| settings.ponder_arrow = value);
            },
        ),
        widgets::picker_row(
            state,
            "Shape",
            move || state.settings.get().arrow_shape,
            ArrowShape::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.arrow_shape = value);
            },
        ),
        widgets::picker_row(
            state,
            "Color",
            move || state.settings.get().arrow_color,
            ArrowColor::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.arrow_color = value);
            },
        ),
        widgets::picker_row(
            state,
            "Size",
            move || state.settings.get().arrow_size,
            ArrowSize::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.arrow_size = value);
            },
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full().min_width(0.0))
}

fn motion_tab(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Motion", pal),
        widgets::toggle_row(
            state,
            "Piece slide",
            move || state.settings.get().piece_slide,
            move |value| {
                actions::update_settings(state, |settings| settings.piece_slide = value);
            },
        ),
        widgets::toggle_row(
            state,
            "System motion",
            move || state.settings.get().system_motion,
            move |value| {
                actions::update_settings(state, |settings| settings.system_motion = value);
            },
        ),
        widgets::picker_row(
            state,
            "Anim speed",
            move || AnimPace::from_setting(state.settings.get().anim_speed),
            AnimPace::ALL,
            move |pace| {
                actions::update_settings(state, |settings| settings.anim_speed = pace.to_setting());
            },
        ),
        widgets::picker_row(
            state,
            "Capture FX",
            move || state.settings.get().capture_anim_style,
            CaptureAnimStyle::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.capture_anim_style = value);
            },
        ),
        widgets::picker_row(
            state,
            "Piece motion",
            move || state.settings.get().piece_anim_style,
            crate::app_core::settings::PieceAnimStyle::ALL,
            move |value| {
                actions::update_settings(state, |settings| settings.piece_anim_style = value);
            },
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full().min_width(0.0))
}

fn eval_bar_engine_picker(state: AppState, handles: AppHandles) -> impl IntoView {
    dyn_view(move || {
        let items = crate::app_core::logic::eval_bar_engine_choices(
            &handles.bundled,
            &handles.catalog.borrow(),
        )
        .into_iter()
        .map(|choice| choice.id)
        .collect::<Vec<_>>();
        widgets::picker_row(
            state,
            "Eval-bar engine",
            move || state.settings.get().eval_bar_engine,
            items,
            move |id| {
                actions::update_settings(state, |settings| {
                    settings.eval_bar_engine = id;
                });
            },
        )
        .into_any()
    })
}

fn engine_rows(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Engine", pal),
        widgets::picker_row(
            state,
            "Hash MB",
            move || state.engine_cfg.get().hash_mb,
            [16, 32, 64, 128, 256],
            move |value| {
                state.engine_cfg.update(|cfg| cfg.hash_mb = value);
            },
        ),
        widgets::picker_row(
            state,
            "Threads",
            move || state.engine_cfg.get().threads,
            1..=8,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.threads = value);
            },
        ),
        widgets::picker_row(
            state,
            "Move time (s)",
            move || state.engine_cfg.get().time_per_move,
            [1, 2, 3, 5, 8, 10, 15],
            move |value| {
                state.engine_cfg.update(|cfg| cfg.time_per_move = value);
            },
        ),
        widgets::toggle_row(
            state,
            "NNUE",
            move || state.engine_cfg.get().use_nnue,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.use_nnue = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Book",
            move || state.engine_cfg.get().use_book,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.use_book = value);
            },
        ),
        widgets::toggle_row(
            state,
            "Ponder",
            move || state.engine_cfg.get().ponder,
            move |value| {
                state.engine_cfg.update(|cfg| cfg.ponder = value);
            },
        ),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

fn tools_tab(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::vertical((
        widgets::section_label("Syzygy", pal),
        Label::derived(move || state.syzygy_status.get()),
        widgets::picker_row(
            state,
            "Piece set",
            move || state.syzygy_piece_set.get(),
            [
                SyzygyPieceSet::Standard,
                SyzygyPieceSet::Extended,
                SyzygyPieceSet::Full,
            ],
            move |set| state.syzygy_piece_set.set(set),
        ),
        widgets::primary_button(state, "Download Syzygy", {
            let handles = handles.clone();
            move || actions::download_syzygy(state, &handles)
        }),
        widgets::section_label("NNUE", pal),
        Label::derived(move || state.nnue_status.get()),
        widgets::primary_button(state, "Download NNUE nets", {
            let handles = handles.clone();
            move || actions::download_nnue(state, &handles)
        }),
        widgets::section_label("Tuning", pal),
        Label::derived(move || state.tuning_status.get()),
        widgets::ghost_button(state, "Refresh status", move || {
            actions::refresh_updater_status(state)
        }),
    ))
    .style(|s| s.row_gap(10.0).width_full())
}

pub fn tournament_setup_modal(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::overlay_frame_sized(
        state,
        move || state.show_tournament_setup.set(false),
        Stack::vertical((
            Stack::vertical((
                widgets::curious_title("Tournament Setup", 26.0),
                widgets::body_copy(
                    "Local engines/ binaries only. Hash, threads, and nets are routed to each engine's advertised UCI options. Native builds preferred.",
                    pal,
                ),
            ))
            .style(|s| s.width_full().row_gap(6.0).min_width(0.0)),
            Stack::new((
                Stack::vertical((
                    widgets::section_label("Event", pal),
                    Label::new("Event name")
                        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
                    TextInput::new(state.tournament_event).style(|s| {
                        s.width_full()
                            .height(36.0)
                            .border_radius(10.0)
                            .min_width(0.0)
                    }),
                    Label::new("Site (optional)")
                        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
                    TextInput::new(state.tournament_site).style(|s| {
                        s.width_full()
                            .height(36.0)
                            .border_radius(10.0)
                            .min_width(0.0)
                    }),
                    widgets::picker_row(
                        state,
                        "Format",
                        move || state.tournament_setup.get().format,
                        TournamentFormat::ALL,
                        move |format| {
                            state.tournament_setup.update(|setup| setup.format = format);
                        },
                    ),
                    widgets::stepper_row(
                        state,
                        "Swiss rounds",
                        "",
                        move || state.tournament_setup.get().swiss_rounds as i32,
                        move |value| {
                            state
                                .tournament_setup
                                .update(|setup| setup.swiss_rounds = value.max(1) as u32);
                        },
                        1,
                        16,
                    )
                    .style(move |s| {
                        if state.tournament_setup.get().format == TournamentFormat::Swiss {
                            s
                        } else {
                            s.display(Display::None)
                        }
                    }),
                    widgets::picker_row(
                        state,
                        "Time control",
                        move || state.tournament_setup.get().time_control,
                        TimeControlPreset::ALL,
                        move |time| {
                            state
                                .tournament_setup
                                .update(|setup| setup.time_control = time);
                        },
                    ),
                    widgets::stepper_row(
                        state,
                        "Games / pairing",
                        "",
                        move || state.tournament_setup.get().games_per_encounter as i32,
                        move |value| {
                            state.tournament_setup.update(|setup| {
                                setup.games_per_encounter = (value as u32).clamp(1, 4);
                            });
                        },
                        1,
                        4,
                    ),
                    widgets::picker_row(
                        state,
                        "Hash",
                        move || state.tournament_setup.get().hash_mb as i32,
                        [16, 32, 64, 128, 256],
                        move |value| {
                            state.tournament_setup.update(|setup| {
                                setup.hash_mb = (value as u32).clamp(
                                    16,
                                    crate::app_core::tournament_setup::GUI_TOURNAMENT_MAX_HASH_MB,
                                );
                            });
                        },
                    ),
                    widgets::stepper_row(
                        state,
                        "Threads",
                        "",
                        move || state.tournament_setup.get().engine_threads as i32,
                        move |value| {
                            state.tournament_setup.update(|setup| {
                                setup.engine_threads = value.max(1) as u32;
                                setup.sanitize_for_gui();
                            });
                        },
                        1,
                        crate::app_core::tournament_setup::GUI_TOURNAMENT_MAX_THREADS as i32,
                    ),
                    widgets::stepper_row(
                        state,
                        "Simultaneous games",
                        "",
                        move || state.tournament_setup.get().concurrency as i32,
                        move |value| {
                            state.tournament_setup.update(|setup| {
                                setup.concurrency = value.max(1) as u32;
                                setup.sanitize_for_gui();
                            });
                        },
                        1,
                        crate::app_core::tournament_setup::detected_safe_games() as i32,
                    ),
                    widgets::wrapping_label(
                        move || {
                            let cores = std::thread::available_parallelism()
                                .map(|n| n.get())
                                .unwrap_or(1);
                            let setup = state.tournament_setup.get();
                            let safe = crate::app_core::tournament_setup::max_simultaneous_games(
                                &setup,
                            );
                            format!(
                                "{cores} CPU cores · event allows {safe} simultaneous game(s). Hash/threads are sent only when the engine advertises them. Crashes and missing nets forfeit that game."
                            )
                        },
                        pal,
                    ),
                ))
                .style(|s| s.min_width(300.0).flex_grow(1.0f32).row_gap(8.0)),
                Stack::vertical((
                    widgets::section_label("Players", pal),
                    widgets::ghost_button(state, "Toggle all local engines", {
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
                    dyn_view({
                        let handles = handles.clone();
                        move || {
                            let roster = crate::app_core::logic::tournament_engine_roster(
                                &handles.bundled,
                                &handles.catalog.borrow(),
                            );
                            if roster.is_empty() {
                                return Label::new(
                                    "No UCI engines were found under the local engines/ folder.",
                                )
                                .style(move |s| {
                                    s.font_size(13.0)
                                        .min_width(0.0)
                                        .width_full()
                                        .text_wrap()
                                        .color(theme::rgba(pal().text_secondary))
                                })
                                .into_any();
                            }
                            let list = roster
                                .into_iter()
                                .map(|engine| {
                                    let path = engine.path.clone();
                                    let selected_path = path.clone();
                                    widgets::toggle_row(
                                        state,
                                        engine.name,
                                        move || {
                                            crate::app_core::logic::engine_is_selected(
                                                &state.tournament_setup.get().selected_engine_paths,
                                                &selected_path,
                                            )
                                        },
                                        move |enable| {
                                            state.tournament_setup.update(|setup| {
                                                if enable {
                                                    if !crate::app_core::logic::engine_is_selected(
                                                        &setup.selected_engine_paths,
                                                        &path,
                                                    ) {
                                                        setup.selected_engine_paths.push(path.clone());
                                                    }
                                                } else {
                                                    let key =
                                                        crate::app_core::logic::engine_identity_key(
                                                            &path,
                                                        );
                                                    setup.selected_engine_paths.retain(|item| {
                                                        crate::app_core::logic::engine_identity_key(
                                                            item,
                                                        ) != key
                                                    });
                                                }
                                            });
                                        },
                                    )
                                })
                                .collect::<Vec<_>>()
                                .into_view()
                                .style(|s| s.width_full().row_gap(6.0).flex_col());
                            widgets::capped_scroll(
                                list,
                                crate::app_core::layout::MODAL_LIST_SCROLL_PX,
                            )
                            .into_any()
                        }
                    }),
                    Label::derived(move || {
                        format!(
                            "{} engines selected",
                            state.tournament_setup.get().selected_engine_paths.len()
                        )
                    }),
                    Label::derived(move || state.tournament_status.get()).style(move |s| {
                        s.font_size(12.0)
                            .min_width(0.0)
                            .width_full()
                            .text_wrap()
                            .color(theme::rgba(pal().accent_alt))
                    }),
                    Stack::horizontal((
                        widgets::primary_button(state, "Start", {
                            let handles = handles.clone();
                            move || actions::start_tournament(state, &handles)
                        }),
                        widgets::ghost_button(state, "Close", move || {
                            state.show_tournament_setup.set(false)
                        }),
                    ))
                    .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
                ))
                .style(|s| s.min_width(300.0).flex_grow(1.0f32).row_gap(8.0)),
            ))
            .style(|s| {
                s.width_full()
                    .col_gap(20.0)
                    .row_gap(16.0)
                    .flex_row()
                    .flex_wrap(FlexWrap::Wrap)
                    .min_width(0.0)
            }),
        ))
        .style(|s| s.width_full().row_gap(12.0).min_width(0.0)),
        crate::app_core::layout::TOURNAMENT_OVERLAY_MAX_WIDTH,
    )
}

pub fn tournament_results_modal(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::overlay_frame_sized(
        state,
        move || state.show_tournament_results.set(false),
        Stack::vertical((
            widgets::curious_title("Tournament results", 30.0),
            Label::derived(move || {
                let snap = state.tournament_snapshot.get();
                let games = snap.unique_played_count();
                let phase = snap.phase_label();
                format!(
                    "{phase} · {games} games · {}",
                    mujrim_study::rating::EVENT_ELO_CAPTION
                )
            })
            .style(move |s| {
                s.font_size(13.0)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            }),
            (0..layout::STANDING_SLOTS)
                .map(|index| results_rank_card(state, index))
                .collect::<Vec<_>>()
                .into_view()
                .style(|s| s.width_full().flex_col().row_gap(10.0).min_width(0.0)),
            follow_up_actions(state, handles.clone()),
            Stack::horizontal((
                widgets::primary_button(state, "New Tournament", {
                    let handles = handles.clone();
                    move || actions::open_new_tournament_setup(state, &handles)
                }),
                widgets::primary_button(state, "Close", move || {
                    state.show_tournament_results.set(false);
                }),
            ))
            .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap).min_width(0.0)),
        ))
        .style(|s| s.width_full().row_gap(16.0).min_width(0.0)),
        crate::app_core::layout::TOURNAMENT_OVERLAY_MAX_WIDTH,
    )
}

fn follow_up_actions(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        follow_up_choice_button(state, handles.clone(), 0),
        follow_up_choice_button(state, handles.clone(), 1),
        follow_up_choice_button(state, handles, 2),
    ))
    .style(move |s| {
        let snap = state.tournament_snapshot.get();
        let field = crate::app_core::tournament_results::follow_up_field(
            snap.standings.len(),
            snap.engine_names.len(),
        );
        let s = s
            .width_full()
            .col_gap(8.0)
            .row_gap(8.0)
            .flex_wrap(FlexWrap::Wrap)
            .min_width(0.0);
        if crate::app_core::tournament_results::follow_up_choices(field).is_empty() {
            s.display(Display::None)
        } else {
            s
        }
    })
}

fn follow_up_choice_button(state: AppState, handles: AppHandles, index: usize) -> impl IntoView {
    Button::new(Label::derived(move || {
        let snap = state.tournament_snapshot.get();
        let field = crate::app_core::tournament_results::follow_up_field(
            snap.standings.len(),
            snap.engine_names.len(),
        );
        crate::app_core::tournament_results::follow_up_choices(field)
            .get(index)
            .map(|choice| choice.label())
            .unwrap_or_default()
    }))
    .action(move || {
        let snap = state.tournament_snapshot.get_untracked();
        let field = crate::app_core::tournament_results::follow_up_field(
            snap.standings.len(),
            snap.engine_names.len(),
        );
        if let Some(choice) =
            crate::app_core::tournament_results::follow_up_choices(field).get(index)
        {
            actions::start_follow_up_tournament(state, &handles, choice.size);
        }
    })
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        let snap = state.tournament_snapshot.get();
        let field = crate::app_core::tournament_results::follow_up_field(
            snap.standings.len(),
            snap.engine_names.len(),
        );
        let choices = crate::app_core::tournament_results::follow_up_choices(field);
        let s = s
            .min_width(0.0)
            .padding_horiz(12.0)
            .padding_vert(8.0)
            .border_radius(10.0)
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .font_size(12.0)
            .background(Color::TRANSPARENT)
            .color(theme::rgba(pal.text_primary))
            .hover(|s| s.background(theme::rgba(pal.panel)));
        if index < choices.len() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn results_losses_line(state: AppState, index: usize) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    const LOSS_CHIPS: usize = 24;
    Stack::horizontal((
        Label::derived(move || {
            let snap = state.tournament_snapshot.get();
            let Some(row) = snap.standings.get(index) else {
                return String::new();
            };
            let items =
                crate::app_core::tournament_live::losses_to_items(&row.name, &snap.played_games);
            if items.first().is_some_and(|item| item == "Undefeated") {
                String::new()
            } else {
                "Lost to".to_owned()
            }
        })
        .style(move |s| s.font_size(13.0).color(theme::rgba(pal().accent_alt))),
        (0..LOSS_CHIPS)
            .map(|chip| {
                Label::derived(move || {
                    let snap = state.tournament_snapshot.get();
                    snap.standings
                        .get(index)
                        .and_then(|row| {
                            crate::app_core::tournament_live::losses_to_items(
                                &row.name,
                                &snap.played_games,
                            )
                            .get(chip)
                            .cloned()
                        })
                        .unwrap_or_default()
                })
                .style(move |s| {
                    let snap = state.tournament_snapshot.get();
                    let text = snap
                        .standings
                        .get(index)
                        .and_then(|row| {
                            crate::app_core::tournament_live::losses_to_items(
                                &row.name,
                                &snap.played_games,
                            )
                            .get(chip)
                            .cloned()
                        })
                        .unwrap_or_default();
                    let s = s
                        .font_size(12.0)
                        .padding_horiz(8.0)
                        .padding_vert(3.0)
                        .border_radius(999.0)
                        .background(theme::rgba(pal().bg))
                        .color(theme::rgba(pal().accent_alt));
                    if text.is_empty() {
                        s.display(Display::None)
                    } else {
                        s
                    }
                })
            })
            .collect::<Vec<_>>()
            .into_view(),
    ))
    .style(|s| {
        s.width_full()
            .min_width(0.0)
            .col_gap(6.0)
            .row_gap(6.0)
            .flex_wrap(FlexWrap::Wrap)
            .overflow_x(Overflow::Clip)
    })
}

fn results_rank_card(state: AppState, index: usize) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let row = move || {
        state
            .tournament_snapshot
            .get()
            .standings
            .get(index)
            .cloned()
    };
    Stack::vertical((
        Stack::horizontal((
            Label::derived(move || {
                row()
                    .map(|row| format!("#{}", row.rank))
                    .unwrap_or_default()
            })
            .style(move |s| {
                let (tr, tg, tb) = row()
                    .and_then(|row| row.podium())
                    .map(crate::app_core::tournament_live::PodiumTier::rgb)
                    .unwrap_or((160, 160, 168));
                s.font_size(18.0)
                    .font_bold()
                    .width(44.0)
                    .color(Color::from_rgb8(tr, tg, tb))
            }),
            Stack::vertical((
                Label::derived(move || row().map(|row| row.name).unwrap_or_default()).style(
                    move |s| {
                        s.font_size(16.0)
                            .font_bold()
                            .min_width(0.0)
                            .width_full()
                            .text_ellipsis()
                            .color(theme::rgba(pal().text_primary))
                    },
                ),
                Label::derived(move || row().map(|row| row.score_line()).unwrap_or_default())
                    .style(move |s| {
                        s.font_size(12.0)
                            .min_width(0.0)
                            .width_full()
                            .text_wrap()
                            .color(theme::rgba(pal().text_secondary))
                    }),
            ))
            .style(|s| s.flex_grow(1.0f32).min_width(0.0).row_gap(2.0)),
        ))
        .style(|s| s.width_full().col_gap(10.0).items_center().min_width(0.0)),
        results_losses_line(state, index),
    ))
    .style(move |s| {
        let Some(row) = row() else {
            return s.display(Display::None);
        };
        let s = s
            .width_full()
            .row_gap(6.0)
            .padding(14.0)
            .border_radius(14.0)
            .border(1.0)
            .border_color(theme::rgba(pal().border))
            .min_width(0.0)
            .overflow_x(Overflow::Clip);
        if let Some(podium) = row.podium() {
            let (tr, tg, tb) = podium.rgb();
            s.background(Color::from_rgba8(tr, tg, tb, 32))
        } else {
            s.background(theme::rgba(pal().bg))
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn tournament_setup_exposes_event_fields() {
        let src = include_str!("modals.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "Event name",
            "Site (optional)",
            "Games / pairing",
            "Hash",
            "Threads",
            "Time control",
            "Background music",
            "Last-move arrow",
            "Ponder arrow",
            "BGM volume",
            "Threat highlights",
            "Eval-bar engine",
            "Simultaneous games",
            "tournament_results_modal",
            "follow_up_actions",
            "follow_up_choice_button",
            "follow_up_field",
            "start_follow_up_tournament",
            "New Tournament",
            "open_new_tournament_setup",
            "results_rank_card",
            "results_losses_line",
            "losses_to_items",
            "overflow_x(Overflow::Clip)",
            "EVENT_ELO_CAPTION",
            "OptionsTab::Display",
            "OptionsTab::Motion",
            "OptionsTab::Arrows",
            "OptionsTab::Audio",
            "OptionsTab::Analysis",
            "capped_scroll",
            "MODAL_LIST_SCROLL_PX",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
        assert!(
            production.contains("display(Display::None)"),
            "Swiss rounds must stay mounted instead of swapping Empty views"
        );
    }
}
