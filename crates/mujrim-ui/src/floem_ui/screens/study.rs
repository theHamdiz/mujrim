//! Study and Learn sidebars: explorer, library, preparations, training, gambits.

use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap};
use mujrim_study::gambit;
use mujrim_study::opening::PrepSide;

use crate::app_core::layout;
use crate::app_core::logic;
use crate::app_core::settings::StudyTab;

use super::super::actions;
use super::super::state::{AppHandles, AppState};
use super::super::theme;
use super::super::widgets;
use super::workspace;

pub fn study_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        progress_strip(state, handles.clone()),
        study_tab_bar(state, handles.clone()),
        dyn_view({
            let handles = handles.clone();
            move || match state.settings.get().study_tab {
                StudyTab::Studies => studies_panel(state, handles.clone()).into_any(),
                StudyTab::Explore => opening_card(state, handles.clone()).into_any(),
                StudyTab::Prepare => preparations_card(state, handles.clone()).into_any(),
                StudyTab::Learn => learn_panel(state, handles.clone()).into_any(),
                StudyTab::Library => library_card(state, handles.clone()).into_any(),
            }
        }),
        variation_tree_card(state, handles.clone()),
        notes_card(state, handles.clone()),
        Stack::vertical((
            pane_title("Moves"),
            workspace::move_list(state, handles.clone()),
            workspace::ply_nav(state, handles),
        ))
        .style(|s| s.row_gap(6.0).width_full().min_width(0.0)),
    ))
    .style(|s| {
        s.flex_col()
            .row_gap(10.0)
            .width_full()
            .min_width(0.0)
            .min_height(0.0)
    })
}

#[allow(dead_code)]
pub fn library_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    study_sidebar(state, handles)
}

#[allow(dead_code)]
pub fn learn_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    study_sidebar(state, handles)
}

fn study_tab_bar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal(
        StudyTab::ALL
            .into_iter()
            .map(|tab| {
                let handles = handles.clone();
                widgets::ghost_button(state, tab.label(), move || {
                    actions::set_study_tab(state, tab);
                    if tab == StudyTab::Learn {
                        actions::refresh_learn_catalog(state, &handles);
                    }
                    if tab == StudyTab::Library {
                        actions::refresh_study(state, &handles);
                    }
                })
            })
            .collect::<Vec<_>>(),
    )
    .style(|s| s.col_gap(4.0).flex_wrap(FlexWrap::Wrap).width_full())
}

fn progress_strip(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Label::derived({
            let handles = handles.clone();
            move || {
                let due = state.training_due.get().len();
                let lines = state.saved_lines.get().len();
                let studies = state.studies.get().len();
                let games = handles
                    .study
                    .borrow()
                    .as_ref()
                    .map(mujrim_study::database::StudyDatabase::len)
                    .unwrap_or(0);
                format!("{studies} studies · {lines} prep lines · {due} due · {games} games")
            }
        })
        .style(move |s| {
            s.font_size(12.0)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(pal().text_secondary))
        }),
    )
}

fn studies_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Studies", pal),
            widgets::body_copy(
                "Create a study, add chapters, and fork sidelines by playing a different move.",
                pal,
            ),
            TextInput::new(state.study_title).style(|s| {
                s.width_full()
                    .min_width(0.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            TextInput::new(state.chapter_title).style(|s| {
                s.width_full()
                    .min_width(0.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            Stack::horizontal((
                widgets::primary_button(state, "New study", {
                    let handles = handles.clone();
                    move || actions::create_study(state, &handles)
                }),
                widgets::ghost_button(state, "Add chapter", {
                    let handles = handles.clone();
                    move || actions::add_study_chapter(state, &handles)
                }),
                widgets::ghost_button(state, "Save chapter line", {
                    let handles = handles.clone();
                    move || actions::save_chapter_from_board(state, &handles)
                }),
            ))
            .style(|s| s.col_gap(6.0).flex_wrap(FlexWrap::Wrap)),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let studies = state.studies.get();
                    if studies.is_empty() {
                        return Label::new("No studies yet.")
                            .style(move |s| {
                                s.font_size(12.0)
                                    .min_width(0.0)
                                    .width_full()
                                    .text_wrap()
                                    .color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    let list = studies
                        .into_iter()
                        .map(|study| {
                            let id = study.id.clone();
                            let delete_id = id.clone();
                            Stack::vertical((
                                Label::new(format!(
                                    "{} · {} chapters",
                                    study.title,
                                    study.chapters.len()
                                ))
                                .style(|s| {
                                    s.font_size(13.0)
                                        .font_bold()
                                        .min_width(0.0)
                                        .width_full()
                                        .text_wrap()
                                }),
                                Stack::horizontal((
                                    widgets::ghost_button(state, "Open", {
                                        let handles = handles.clone();
                                        move || actions::load_study(state, &handles, id.clone())
                                    }),
                                    widgets::ghost_button(state, "Delete", {
                                        let handles = handles.clone();
                                        move || {
                                            actions::delete_study(
                                                state,
                                                &handles,
                                                delete_id.clone(),
                                            )
                                        }
                                    }),
                                ))
                                .style(|s| s.col_gap(6.0).flex_wrap(FlexWrap::Wrap)),
                            ))
                            .style(move |s| {
                                let pal = pal();
                                s.width_full()
                                    .row_gap(4.0)
                                    .padding(8.0)
                                    .border_radius(10.0)
                                    .background(theme::rgba(pal.bg))
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col());
                    widgets::capped_scroll(list, layout::LIST_SCROLL_PX).into_any()
                }
            }),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
    )
}

fn learn_panel(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        training_card(state, handles.clone()),
        gambit_card(state, handles.clone()),
        book_replies_card(state, handles.clone()),
        coaching_card(state, handles.clone()),
        widgets::explanation_card(state, move || workspace::explanation_lines(state)),
        widgets::ghost_button(state, "Coach review", {
            let handles = handles.clone();
            move || actions::review_played_game(state, &handles)
        }),
        widgets::ghost_button(state, "Analyze game", {
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
    ))
    .style(|s| s.row_gap(8.0).width_full().min_width(0.0))
}

fn variation_tree_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Lines", pal),
            widgets::body_copy(
                "Click a move to jump. Playing off-book forks a new variation in the active chapter.",
                pal,
            ),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let labels = actions::study_tree_labels(state);
                    if labels.is_empty() {
                        return Label::new("Open or create a study to see the tree.")
                            .style(move |s| {
                                s.font_size(12.0)
                                    .min_width(0.0)
                                    .width_full()
                                    .text_wrap()
                                    .color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    let list = labels
                        .into_iter()
                        .map(|(path, label)| {
                            let handles = handles.clone();
                            Button::new(label)
                                .action(move || {
                                    actions::jump_study_path(state, &handles, path.clone())
                                })
                                .style(move |s| {
                                    let pal = pal();
                                    s.width_full()
                                        .min_width(0.0)
                                        .padding(8.0)
                                        .border(0.0)
                                        .border_radius(8.0)
                                        .font_size(12.0)
                                        .background(theme::rgba(pal.bg))
                                })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(4.0).flex_col());
                    widgets::capped_scroll(list, layout::LIST_SCROLL_PX).into_any()
                }
            }),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
    )
}

fn pane_title(label: &'static str) -> impl IntoView {
    Label::new(label).style(|s| s.font_size(12.0).font_bold().min_width(0.0).width_full())
}

fn notes_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Move notes", pal),
            TextInput::new(state.move_note).style(|s| {
                s.width_full()
                    .min_width(120.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            widgets::primary_button(state, "Save note", {
                let handles = handles.clone();
                move || actions::save_move_note(state, &handles)
            }),
            widgets::toggle_row(
                state,
                "Threat highlights",
                move || state.settings.get().show_threats,
                move |value| {
                    actions::update_settings(state, |settings| settings.show_threats = value);
                },
            ),
            pane_title("Import / Export"),
            widgets::game_io_bar(state, handles),
        ))
        .style(|s| s.row_gap(8.0).width_full()),
    )
}

fn library_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Game Library", pal),
            Label::derived({
                let handles = handles.clone();
                move || {
                    let count = handles
                        .study
                        .borrow()
                        .as_ref()
                        .map(mujrim_study::database::StudyDatabase::len)
                        .unwrap_or(0);
                    format!("{count} locally indexed games")
                }
            })
            .style(|s| {
                s.font_size(16.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::body_copy(
                "Search by player, event, ECO, or Elo. Click a game to replay it on the board.",
                pal,
            ),
            Stack::horizontal((
                TextInput::new(state.study_query).style(|s| {
                    s.flex_grow(1.0f32)
                        .min_width(120.0)
                        .height(34.0)
                        .border_radius(10.0)
                }),
                widgets::primary_button(state, "Search", {
                    let handles = handles.clone();
                    move || actions::refresh_study(state, &handles)
                }),
            ))
            .style(|s| {
                s.width_full()
                    .col_gap(8.0)
                    .row_gap(8.0)
                    .items_center()
                    .flex_wrap(FlexWrap::Wrap)
            }),
            Stack::horizontal((
                widgets::ghost_button(state, "Import PGN", {
                    let handles = handles.clone();
                    move || actions::import_pgn(state, &handles)
                }),
                widgets::ghost_button(state, "Save current", {
                    let handles = handles.clone();
                    move || actions::save_to_library(state, &handles)
                }),
            ))
            .style(|s| s.col_gap(8.0).flex_wrap(FlexWrap::Wrap)),
            dyn_view(move || {
                let results = state.study_results.get();
                let handles = handles.clone();
                if results.is_empty() {
                    return Label::new(if state.study_query.get().trim().is_empty() {
                        "Import a PGN collection to begin your library."
                    } else {
                        "No games match this search."
                    })
                    .style(move |s| {
                        s.font_size(12.0)
                            .min_width(0.0)
                            .width_full()
                            .text_wrap()
                            .color(theme::rgba(pal().text_secondary))
                    })
                    .into_any();
                }
                let list = results
                    .into_iter()
                    .take(100)
                    .map(|summary| {
                        let id = summary.id.clone();
                        let (title, detail) = logic::game_summary_label(&summary);
                        Button::new(
                            Stack::vertical((
                                Label::new(title).style(|s| {
                                    s.font_size(13.0)
                                        .font_bold()
                                        .min_width(0.0)
                                        .width_full()
                                        .text_ellipsis()
                                }),
                                Label::new(detail).style(move |s| {
                                    s.font_size(11.0)
                                        .min_width(0.0)
                                        .width_full()
                                        .text_wrap()
                                        .color(theme::rgba(pal().text_secondary))
                                }),
                            ))
                            .style(|s| s.row_gap(2.0).width_full().min_width(0.0)),
                        )
                        .action({
                            let handles = handles.clone();
                            move || actions::load_library_game(state, &handles, id.clone())
                        })
                        .style(move |s| {
                            let pal = pal();
                            s.width_full()
                                .min_width(0.0)
                                .padding(8.0)
                                .border_radius(10.0)
                                .border(0.0)
                                .background(theme::rgba(pal.bg))
                                .color(theme::rgba(pal.text_primary))
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_view()
                    .style(|s| s.width_full().row_gap(6.0).flex_col());
                widgets::capped_scroll(list, layout::LIST_SCROLL_PX).into_any()
            }),
        )),
    )
}

fn coaching_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Coach & Review", pal),
            Label::derived(move || {
                let id = state.active_gambit_id.get();
                let catalog = state.learn_catalog.get();
                id.and_then(|id| {
                    mujrim_study::gambit::find_owned(&id, &catalog).map(|lesson| {
                        format!("{} · click a numbered disc or use ← → ↑ ↓", lesson.name)
                    })
                })
                .unwrap_or_else(|| {
                    "Click a gambit, then step the line. ← → previous/next, ↑ ↓ first/last."
                        .to_owned()
                })
            })
            .style(|s| {
                s.font_size(13.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::body_copy(
                "Numbered discs on the board jump to that ply. Blunder and mistake badges sit on the destination square after Coach review.",
                pal,
            ),
            Stack::horizontal((
                widgets::ghost_button(state, "◀", {
                    let handles = handles.clone();
                    move || actions::gambit_step(state, &handles, -1)
                }),
                widgets::ghost_button(state, "▶", {
                    let handles = handles.clone();
                    move || actions::gambit_step(state, &handles, 1)
                }),
                widgets::ghost_button(state, "Start", {
                    let handles = handles.clone();
                    move || actions::navigate_board_ply(state, &handles, logic::BoardPlyNav::First)
                }),
                widgets::ghost_button(state, "End", {
                    let handles = handles.clone();
                    move || actions::navigate_board_ply(state, &handles, logic::BoardPlyNav::Last)
                }),
            ))
            .style(|s| s.col_gap(6.0).flex_wrap(FlexWrap::Wrap).min_width(0.0)),
        )),
    )
}

fn training_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Training Queue", pal),
            Label::derived({
                let handles = handles.clone();
                move || {
                    let count = handles
                        .training
                        .borrow()
                        .as_ref()
                        .map(mujrim_study::training_store::TrainingStore::len)
                        .unwrap_or(0);
                    format!(
                        "{count} positions · {} due today",
                        state.training_due.get().len()
                    )
                }
            })
            .style(|s| {
                s.font_size(16.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::body_copy(
                "Legal puzzle replay with persisted spaced-repetition scheduling.",
                pal,
            ),
            widgets::primary_button(state, "Install starter set", {
                let handles = handles.clone();
                move || actions::seed_training(state, &handles)
            }),
            widgets::ghost_button(state, "Train now", {
                let handles = handles.clone();
                move || actions::start_puzzle(state, &handles)
            }),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let due = state.training_due.get();
                    let training_count = handles
                        .training
                        .borrow()
                        .as_ref()
                        .map(mujrim_study::training_store::TrainingStore::len)
                        .unwrap_or(0);
                    if due.is_empty() {
                        return Label::new(if training_count == 0 {
                            "Add the starter set to begin spaced repetition."
                        } else {
                            "Nothing is due today."
                        })
                        .style(move |s| {
                            s.font_size(12.0)
                                .min_width(0.0)
                                .width_full()
                                .text_wrap()
                                .color(theme::rgba(pal().text_secondary))
                        })
                        .into_any();
                    }
                    let list = due
                        .into_iter()
                        .take(20)
                        .map(|item| {
                            let id = item.puzzle.id.clone();
                            let themes = item.puzzle.themes.join(", ");
                            let rating = format!("{} Elo", item.puzzle.rating);
                            Button::new(
                                Stack::horizontal((
                                    Label::new(themes).style(|s| {
                                        s.font_size(12.0)
                                            .flex_grow(1.0f32)
                                            .min_width(0.0)
                                            .text_ellipsis()
                                    }),
                                    Label::new(rating).style(move |s| {
                                        s.font_size(11.0).color(theme::rgba(pal().text_secondary))
                                    }),
                                ))
                                .style(|s| {
                                    s.width_full().col_gap(8.0).items_center().min_width(0.0)
                                }),
                            )
                            .action({
                                let handles = handles.clone();
                                move || actions::start_training(state, &handles, id.clone())
                            })
                            .style(move |s| {
                                let pal = pal();
                                s.width_full()
                                    .min_width(0.0)
                                    .padding(8.0)
                                    .border(0.0)
                                    .border_radius(10.0)
                                    .background(theme::rgba(pal.bg))
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col());
                    widgets::capped_scroll(list, layout::LIST_SCROLL_PX).into_any()
                }
            }),
        )),
    )
}

fn opening_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Opening Explorer", pal),
            Label::derived(move || {
                format!(
                    "{} games indexed · first 12 moves",
                    state.opening_indexed.get()
                )
            })
            .style(|s| {
                s.font_size(16.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::body_copy(
                "Play a book move to stay on this board. White / draw / Black bars match the library sample.",
                pal,
            ),
            widgets::ghost_button(state, "Index openings", {
                let handles = handles.clone();
                move || actions::index_openings(state, &handles)
            }),
            Label::derived({
                let handles = handles.clone();
                move || {
                    if explorer_entry(state, &handles, 0).is_some() {
                        String::new()
                    } else {
                        "No library games reach this position.".to_owned()
                    }
                }
            })
            .style({
                let handles = handles.clone();
                move |s| {
                    let pal = pal();
                    let s = s
                        .font_size(12.0)
                        .min_width(0.0)
                        .width_full()
                        .text_wrap()
                        .color(theme::rgba(pal.text_secondary));
                    if explorer_entry(state, &handles, 0).is_some() {
                        s.display(Display::None)
                    } else {
                        s
                    }
                }
            }),
            widgets::capped_scroll(
                (0..16)
                    .map(|index| explorer_slot(state, handles.clone(), index, pal))
                    .collect::<Vec<_>>()
                    .into_view()
                    .style(|s| s.width_full().row_gap(6.0).flex_col()),
                layout::LIST_SCROLL_PX,
            ),
        )),
    )
}

fn explorer_entry(
    state: AppState,
    handles: &AppHandles,
    index: usize,
) -> Option<(String, String, u64, u64, u64, u64)> {
    types::init();
    let fen = logic::displayed_study_fen(
        &state.initial_fen.get(),
        &state.move_log.get(),
        state.review_ply.get(),
        state.game.get().map(|game| game.board.to_fen()),
    );
    let board = types::Board::from_fen(&fen).ok();
    let explorer = handles.explorer.borrow();
    explorer
        .moves(&fen)
        .into_iter()
        .nth(index)
        .map(|(uci, stats)| {
            let san = board
                .as_ref()
                .map(|board| logic::uci_to_san(board, uci))
                .unwrap_or_else(|| uci.to_owned());
            (
                uci.to_owned(),
                san,
                stats.games,
                stats.white_wins,
                stats.draws,
                stats.black_wins,
            )
        })
}

fn explorer_slot(
    state: AppState,
    handles: AppHandles,
    index: usize,
    pal: impl Fn() -> crate::app_core::palette::GuiPalette + Copy + 'static,
) -> impl IntoView {
    Button::new(
        Stack::vertical((
            Stack::horizontal((
                Label::derived({
                    let handles = handles.clone();
                    move || {
                        explorer_entry(state, &handles, index)
                            .map(|(_, san, _, _, _, _)| san)
                            .unwrap_or_default()
                    }
                })
                .style(|s| s.font_size(14.0).font_bold().min_width(0.0).text_ellipsis()),
                Label::derived({
                    let handles = handles.clone();
                    move || {
                        explorer_entry(state, &handles, index)
                            .map(|(_, _, games, white, draws, _)| {
                                let score = white
                                    .saturating_mul(100)
                                    .saturating_add(draws.saturating_mul(50))
                                    .checked_div(games)
                                    .unwrap_or(0);
                                format!("{games} · {score}% White")
                            })
                            .unwrap_or_default()
                    }
                })
                .style(move |s| {
                    s.font_size(11.0)
                        .flex_grow(1.0f32)
                        .min_width(0.0)
                        .text_ellipsis()
                        .color(theme::rgba(pal().text_secondary))
                }),
            ))
            .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0)),
            explorer_result_bar(state, handles.clone(), index),
        ))
        .style(|s| s.width_full().row_gap(4.0).min_width(0.0)),
    )
    .action({
        let handles = handles.clone();
        move || {
            if let Some((uci, _, _, _, _, _)) = explorer_entry(state, &handles, index) {
                actions::study_opening_move(state, uci);
            }
        }
    })
    .style(move |s| {
        let pal = pal();
        let s = s
            .width_full()
            .padding(8.0)
            .border(0.0)
            .border_radius(10.0)
            .background(theme::rgba(pal.bg));
        if explorer_entry(state, &handles, index).is_some() {
            s
        } else {
            s.display(Display::None)
        }
    })
}

fn explorer_result_bar(state: AppState, handles: AppHandles, index: usize) -> impl IntoView {
    Stack::horizontal((
        Empty::new().style({
            let handles = handles.clone();
            move |s| {
                let (white, draws, black) = explorer_entry(state, &handles, index)
                    .map(|(_, _, _, white, draws, black)| (white, draws, black))
                    .unwrap_or((0, 0, 0));
                let total = white.saturating_add(draws).saturating_add(black).max(1);
                s.height(7.0)
                    .flex_grow((white as f32 / total as f32).max(0.001))
                    .border_radius(4.0)
                    .background(Color::from_rgb8(236, 236, 236))
            }
        }),
        Empty::new().style({
            let handles = handles.clone();
            move |s| {
                let (white, draws, black) = explorer_entry(state, &handles, index)
                    .map(|(_, _, _, white, draws, black)| (white, draws, black))
                    .unwrap_or((0, 0, 0));
                let total = white.saturating_add(draws).saturating_add(black).max(1);
                s.height(7.0)
                    .flex_grow((draws as f32 / total as f32).max(0.001))
                    .background(Color::from_rgb8(128, 128, 128))
            }
        }),
        Empty::new().style({
            let handles = handles.clone();
            move |s| {
                let (white, draws, black) = explorer_entry(state, &handles, index)
                    .map(|(_, _, _, white, draws, black)| (white, draws, black))
                    .unwrap_or((0, 0, 0));
                let total = white.saturating_add(draws).saturating_add(black).max(1);
                s.height(7.0)
                    .flex_grow((black as f32 / total as f32).max(0.001))
                    .border_radius(4.0)
                    .background(Color::from_rgb8(48, 48, 48))
            }
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(7.0)
            .border_radius(4.0)
            .overflow_x(floem::taffy::style::Overflow::Clip)
            .overflow_y(floem::taffy::style::Overflow::Clip)
    })
}

fn preparations_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Preparations", pal),
            widgets::body_copy(
                "Name the current line, pick a side, and save it to your repertoire.",
                pal,
            ),
            TextInput::new(state.line_name).style(|s| {
                s.width_full()
                    .min_width(0.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            widgets::picker(
                state,
                move || state.prep_side.get(),
                PrepSide::ALL,
                move |side| state.prep_side.set(side),
            ),
            TextInput::new(state.prep_notes).style(|s| {
                s.width_full()
                    .min_width(0.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            widgets::ghost_button(state, "Train this line", {
                let handles = handles.clone();
                move || actions::train_current_preparation(state, &handles)
            }),
            widgets::primary_button(state, "Save line", {
                let handles = handles.clone();
                move || actions::save_preparation(state, &handles)
            }),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let lines = state.saved_lines.get();
                    if lines.is_empty() {
                        return Label::new("No saved lines yet.")
                            .style(move |s| {
                                s.font_size(12.0)
                                    .min_width(0.0)
                                    .width_full()
                                    .text_wrap()
                                    .color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    let list = lines
                        .into_iter()
                        .map(|line| {
                            let id = line.id.clone();
                            let load_id = id.clone();
                            let title =
                                format!("{} · {} · {} ply", line.name, line.side, line.moves.len());
                            Stack::vertical((
                                Label::new(title).style(|s| {
                                    s.font_size(13.0)
                                        .font_bold()
                                        .min_width(0.0)
                                        .width_full()
                                        .text_wrap()
                                }),
                                Label::new(if line.notes.is_empty() {
                                    "No notes".to_owned()
                                } else {
                                    line.notes
                                })
                                .style(move |s| {
                                    s.font_size(11.0)
                                        .min_width(0.0)
                                        .width_full()
                                        .text_wrap()
                                        .color(theme::rgba(pal().text_secondary))
                                }),
                                Stack::horizontal((
                                    widgets::ghost_button(state, "Load", {
                                        let handles = handles.clone();
                                        move || {
                                            actions::load_preparation(
                                                state,
                                                &handles,
                                                load_id.clone(),
                                            )
                                        }
                                    }),
                                    widgets::ghost_button(state, "Delete", {
                                        let handles = handles.clone();
                                        move || {
                                            actions::delete_preparation(state, &handles, id.clone())
                                        }
                                    }),
                                ))
                                .style(|s| s.col_gap(6.0).flex_wrap(FlexWrap::Wrap).min_width(0.0)),
                            ))
                            .style(move |s| {
                                let pal = pal();
                                s.width_full()
                                    .row_gap(4.0)
                                    .padding(8.0)
                                    .border_radius(10.0)
                                    .background(theme::rgba(pal.bg))
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col());
                    widgets::capped_scroll(list, layout::LIST_SCROLL_PX).into_any()
                }
            }),
        )),
    )
}

fn book_replies_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Book replies", pal),
            widgets::body_copy(
                "Polyglot replies from this position. Click one to play it on the board.",
                pal,
            ),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let fen = logic::displayed_study_fen(
                        &state.initial_fen.get(),
                        &state.move_log.get(),
                        state.review_ply.get(),
                        state.game.get().map(|game| game.board.to_fen()),
                    );
                    let replies = logic::book_replies(handles.book.as_ref().as_ref(), &fen);
                    if replies.is_empty() {
                        return Label::new("No book move from here.")
                            .style(move |s| {
                                s.font_size(12.0)
                                    .min_width(0.0)
                                    .width_full()
                                    .text_wrap()
                                    .color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    replies
                        .into_iter()
                        .map(|reply| {
                            let uci = reply.uci.clone();
                            Stack::horizontal((
                                Label::new(format!("{} · wt {}", reply.san, reply.weight)).style(
                                    |s| {
                                        s.font_size(12.0)
                                            .flex_grow(1.0f32)
                                            .min_width(0.0)
                                            .text_wrap()
                                    },
                                ),
                                widgets::ghost_button(state, "Play", {
                                    let handles = handles.clone();
                                    move || actions::play_learn_reply(state, &handles, uci.clone())
                                }),
                            ))
                            .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0))
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col().min_width(0.0))
                        .into_any()
                }
            }),
        )),
    )
}

fn gambit_card(state: AppState, handles: AppHandles) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Gambit Laboratory", pal),
            Label::derived(move || {
                let total = state.learn_catalog.get().len().max(gambit::catalog().len());
                let book = state
                    .learn_catalog
                    .get()
                    .iter()
                    .filter(|lesson| lesson.in_book || lesson.eco == "Book")
                    .count();
                format!("{total} lines · {book} from the opening book")
            })
            .style(|s| {
                s.font_size(13.0)
                    .font_bold()
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
            }),
            widgets::body_copy(
                "Search ECO or name. Learn loads the full line; numbered discs and arrow keys step it.",
                pal,
            ),
            TextInput::new(state.gambit_query).style(|s| {
                s.width_full()
                    .min_width(120.0)
                    .height(34.0)
                    .border_radius(10.0)
            }),
            widgets::ghost_button(state, "Refresh from book", {
                let handles = handles.clone();
                move || actions::refresh_learn_catalog(state, &handles)
            }),
            dyn_view({
                let handles = handles.clone();
                move || {
                    let query = state.gambit_query.get();
                    let query = query.trim().to_ascii_lowercase();
                    let catalog = {
                        let live = state.learn_catalog.get();
                        if live.is_empty() {
                            gambit::catalog()
                                .iter()
                                .map(gambit::OwnedGambit::from)
                                .collect()
                        } else {
                            live
                        }
                    };
                    let rows = catalog
                        .into_iter()
                        .filter(|lesson| {
                            query.is_empty()
                                || lesson.name.to_ascii_lowercase().contains(&query)
                                || lesson.eco.to_ascii_lowercase().contains(&query)
                                || lesson.summary.to_ascii_lowercase().contains(&query)
                        })
                        .map(|lesson| {
                            let id = lesson.id.clone();
                            let badge = if lesson.in_book || lesson.eco == "Book" {
                                "Book"
                            } else {
                                lesson.eco.as_str()
                            };
                            Stack::horizontal((
                                Stack::vertical((
                                    Label::new(format!("{} ({badge})", lesson.name)).style(|s| {
                                        s.font_size(13.0)
                                            .font_bold()
                                            .min_width(0.0)
                                            .width_full()
                                            .text_wrap()
                                    }),
                                    Label::new(format!(
                                        "{} · {} plies",
                                        lesson.summary,
                                        lesson.moves.len()
                                    ))
                                    .style(move |s| {
                                        s.font_size(11.0)
                                            .min_width(0.0)
                                            .width_full()
                                            .text_wrap()
                                            .color(theme::rgba(pal().text_secondary))
                                    }),
                                ))
                                .style(|s| s.flex_grow(1.0f32).row_gap(2.0).min_width(0.0)),
                                widgets::ghost_button(state, "Learn", {
                                    let handles = handles.clone();
                                    move || {
                                        actions::start_gambit_lesson(state, &handles, id.clone())
                                    }
                                }),
                            ))
                            .style(|s| s.width_full().col_gap(8.0).items_start().min_width(0.0))
                        })
                        .collect::<Vec<_>>();
                    widgets::capped_scroll(
                        rows.into_view()
                            .style(|s| s.width_full().row_gap(6.0).flex_col()),
                        layout::LIST_SCROLL_PX,
                    )
                    .into_any()
                }
            }),
        )),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn study_hub_covers_workspace_panels() {
        let src = include_str!("study.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "Game Library",
            "Coach & Review",
            "Training Queue",
            "Opening Explorer",
            "Gambit Laboratory",
            "Book replies",
            "gambit_query",
            "learn_catalog",
            "start_gambit_lesson",
            "refresh_learn_catalog",
            "book_replies",
            "BoardPlyNav",
            "Install starter set",
            "Save current",
            "Preparations",
            "Save line",
            "explorer_slot",
            "uci_to_san",
            "study_opening_move",
            "Save note",
            "Threat highlights",
            "move_note",
            "game_io_bar",
            "library_sidebar",
            "New study",
            "Train this line",
            "Studies",
            "study_tab_bar",
            "explanation_card",
            "body_copy",
            "text_wrap()",
            "text_ellipsis()",
            "capped_scroll",
            "LIST_SCROLL_PX",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
        assert!(
            production.contains("min_width(0.0)"),
            "study panels must shrink with the sidebar instead of overflowing"
        );
        let gambit = production
            .split("fn gambit_card")
            .nth(1)
            .expect("gambit_card");
        assert!(
            gambit.contains("capped_scroll"),
            "gambit repertoire must scroll inside a capped viewport"
        );
        assert!(
            !gambit.contains("max_height(200.0)"),
            "gambit list height must live on the scroll view"
        );
    }
}
