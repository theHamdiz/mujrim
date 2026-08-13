//! Study and Learn sidebars: explorer, library, preparations, training, gambits.

use floem::prelude::*;
use floem::taffy::style::FlexWrap;
use mujrim_study::gambit;
use mujrim_study::opening::PrepSide;

use crate::app_core::logic;

use super::super::actions;
use super::super::state::{AppHandles, AppState};
use super::super::theme;
use super::super::widgets;
use super::workspace;

pub fn study_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        opening_card(state, handles.clone()),
        Stack::vertical((
            pane_title("Moves"),
            workspace::move_list(state),
            workspace::ply_nav(state),
        ))
        .style(|s| s.row_gap(6.0).width_full().min_width(0.0)),
        preparations_card(state, handles.clone()),
        library_card(state, handles),
    ))
    .style(|s| s.flex_col().row_gap(10.0).width_full().min_height(0.0))
}

pub fn learn_sidebar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::vertical((
        training_card(state, handles.clone()),
        gambit_card(state),
        coaching_card(state),
        widgets::ghost_button(state, "Coach review", move || {
            actions::annotate_last_move(state)
        }),
        widgets::ghost_button(state, "Analyze game", {
            let handles = handles.clone();
            move || actions::analyze_game(state, &handles)
        }),
        pane_title("Moves"),
        workspace::move_list(state),
        workspace::ply_nav(state),
    ))
    .style(|s| s.flex_col().row_gap(8.0).width_full().min_height(0.0))
}

fn pane_title(label: &'static str) -> impl IntoView {
    Label::new(label).style(|s| s.font_size(12.0).font_bold())
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
            .style(|s| s.font_size(16.0).font_bold()),
            Label::new(
                "Search by player, event, ECO, or Elo. Click a game to replay it on the board.",
            )
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
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
                    .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary)))
                    .into_any();
                }
                results
                    .into_iter()
                    .take(100)
                    .map(|summary| {
                        let id = summary.id.clone();
                        let (title, detail) = logic::game_summary_label(&summary);
                        Button::new(
                            Stack::vertical((
                                Label::new(title).style(|s| s.font_size(13.0).font_bold()),
                                Label::new(detail).style(move |s| {
                                    s.font_size(11.0).color(theme::rgba(pal().text_secondary))
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
                                .padding(8.0)
                                .border_radius(10.0)
                                .border(0.0)
                                .background(theme::rgba(pal.bg))
                                .color(theme::rgba(pal.text_primary))
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_view()
                    .style(|s| s.width_full().row_gap(6.0).flex_col().max_height(220.0))
                    .scroll()
                    .into_any()
            }),
        )),
    )
}

fn coaching_card(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Coach & Review", pal),
            Label::new("Move-quality vocabulary ready").style(|s| s.font_size(16.0).font_bold()),
            Label::new(
                "Aura !!!, Brilliant !!, Great !, Best, Excellent, Good, OK, Book, Novelty, Inaccuracy, Mistake, and Blunder share one review model.",
            )
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            Label::new("Click a move in the list to jump the board. Annotation badges paint on the destination square.")
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
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
            .style(|s| s.font_size(16.0).font_bold()),
            Label::new("Legal puzzle replay with persisted spaced-repetition scheduling.")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
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
                        .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary)))
                        .into_any();
                    }
                    due.into_iter()
                        .take(20)
                        .map(|item| {
                            let id = item.puzzle.id.clone();
                            let themes = item.puzzle.themes.join(", ");
                            let rating = format!("{} Elo", item.puzzle.rating);
                            Button::new(
                                Stack::horizontal((
                                    Label::new(themes)
                                        .style(|s| s.font_size(12.0).flex_grow(1.0f32)),
                                    Label::new(rating).style(move |s| {
                                        s.font_size(11.0).color(theme::rgba(pal().text_secondary))
                                    }),
                                ))
                                .style(|s| s.width_full().col_gap(8.0).items_center()),
                            )
                            .action({
                                let handles = handles.clone();
                                move || actions::start_training(state, &handles, id.clone())
                            })
                            .style(move |s| {
                                let pal = pal();
                                s.width_full()
                                    .padding(8.0)
                                    .border(0.0)
                                    .border_radius(10.0)
                                    .background(theme::rgba(pal.bg))
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col().max_height(180.0))
                        .scroll()
                        .into_any()
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
            .style(|s| s.font_size(16.0).font_bold()),
            Label::new(
                "Play a book move to stay on this board. White / draw / Black bars match the library sample.",
            )
            .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            widgets::ghost_button(state, "Index openings", {
                let handles = handles.clone();
                move || actions::index_openings(state, &handles)
            }),
            dyn_view({
                let handles = handles.clone();
                move || {
                    types::init();
                    let fen = logic::displayed_study_fen(
                        &state.initial_fen.get(),
                        &state.move_log.get(),
                        state.review_ply.get(),
                        state.game.get().map(|game| game.board.to_fen()),
                    );
                    let board = types::Board::from_fen(&fen).ok();
                    let rows: Vec<(String, String, u64, u64, u64, u64)> = {
                        let explorer = handles.explorer.borrow();
                        explorer
                            .moves(&fen)
                            .into_iter()
                            .take(16)
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
                            .collect()
                    };
                    if rows.is_empty() {
                        return Label::new("No library games reach this position.")
                            .style(move |s| {
                                s.font_size(12.0)
                                    .color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    rows.into_iter()
                        .map(|(uci, san, games, white, draws, black)| {
                            let label = uci.clone();
                            let score = white
                                .saturating_mul(100)
                                .saturating_add(draws.saturating_mul(50))
                                .checked_div(games)
                                .unwrap_or(0);
                            Button::new(
                                Stack::vertical((
                                    Stack::horizontal((
                                        Label::new(san).style(|s| s.font_size(14.0).font_bold()),
                                        Label::new(format!("{games} · {score}% White")).style(
                                            move |s| {
                                                s.font_size(11.0)
                                                    .flex_grow(1.0f32)
                                                    .color(theme::rgba(pal().text_secondary))
                                            },
                                        ),
                                    ))
                                    .style(|s| {
                                        s.width_full().col_gap(8.0).items_center().min_width(0.0)
                                    }),
                                    result_bar(white, draws, black),
                                ))
                                .style(|s| s.width_full().row_gap(4.0).min_width(0.0)),
                            )
                            .action(move || actions::study_opening_move(state, label.clone()))
                            .style(move |s| {
                                let pal = pal();
                                s.width_full()
                                    .padding(8.0)
                                    .border(0.0)
                                    .border_radius(10.0)
                                    .background(theme::rgba(pal.bg))
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_view()
                        .style(|s| s.width_full().row_gap(6.0).flex_col().max_height(260.0))
                        .scroll()
                        .into_any()
                }
            }),
        )),
    )
}

fn result_bar(white: u64, draws: u64, black: u64) -> impl IntoView {
    let total = white.saturating_add(draws).saturating_add(black).max(1);
    Stack::horizontal((
        Empty::new().style(move |s| {
            s.height(7.0)
                .flex_grow((white as f32 / total as f32).max(0.001))
                .border_radius(4.0)
                .background(Color::from_rgb8(236, 236, 236))
        }),
        Empty::new().style(move |s| {
            s.height(7.0)
                .flex_grow((draws as f32 / total as f32).max(0.001))
                .background(Color::from_rgb8(128, 128, 128))
        }),
        Empty::new().style(move |s| {
            s.height(7.0)
                .flex_grow((black as f32 / total as f32).max(0.001))
                .border_radius(4.0)
                .background(Color::from_rgb8(48, 48, 48))
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
            Label::new("Name the current line, pick a side, and save it to your repertoire.")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
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
                                s.font_size(12.0).color(theme::rgba(pal().text_secondary))
                            })
                            .into_any();
                    }
                    lines
                        .into_iter()
                        .map(|line| {
                            let id = line.id.clone();
                            let load_id = id.clone();
                            let title =
                                format!("{} · {} · {} ply", line.name, line.side, line.moves.len());
                            Stack::vertical((
                                Label::new(title).style(|s| s.font_size(13.0).font_bold()),
                                Label::new(if line.notes.is_empty() {
                                    "No notes".to_owned()
                                } else {
                                    line.notes
                                })
                                .style(move |s| {
                                    s.font_size(11.0).color(theme::rgba(pal().text_secondary))
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
                                .style(|s| s.col_gap(6.0)),
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
                        .style(|s| s.width_full().row_gap(6.0).flex_col().max_height(200.0))
                        .scroll()
                        .into_any()
                }
            }),
        )),
    )
}

fn gambit_card(state: AppState) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    widgets::card(
        state,
        Stack::vertical((
            widgets::section_label("Gambit Laboratory", pal),
            Label::new("Interactive lines with numbered coaching arrows.")
                .style(move |s| s.font_size(12.0).color(theme::rgba(pal().text_secondary))),
            gambit::catalog()
                .iter()
                .map(|lesson| {
                    let id = lesson.id.to_owned();
                    Stack::horizontal((
                        Stack::vertical((
                            Label::new(format!("{} ({})", lesson.name, lesson.eco))
                                .style(|s| s.font_size(13.0).font_bold()),
                            Label::new(lesson.summary).style(move |s| {
                                s.font_size(11.0).color(theme::rgba(pal().text_secondary))
                            }),
                        ))
                        .style(|s| s.flex_grow(1.0f32).row_gap(2.0).min_width(0.0)),
                        widgets::ghost_button(state, "Learn", {
                            move || actions::start_gambit_lesson(state, id.clone())
                        }),
                    ))
                    .style(|s| s.width_full().col_gap(8.0).items_center())
                })
                .collect::<Vec<_>>()
                .into_view()
                .style(|s| s.width_full().row_gap(6.0).flex_col().max_height(200.0))
                .scroll(),
        )),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn study_hub_covers_iced_workspace_panels() {
        let src = include_str!("study.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        for needle in [
            "Game Library",
            "Coach & Review",
            "Training Queue",
            "Opening Explorer",
            "Gambit Laboratory",
            "Install starter set",
            "Save current",
            "Preparations",
            "Save line",
            "result_bar",
            "uci_to_san",
            "study_opening_move",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
    }
}
