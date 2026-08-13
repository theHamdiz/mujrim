//! In-app tab strip. Compositor decorations on Linux; CSD drag only on undecorated hosts.

use floem::action::{drag_resize_window, minimize_window, toggle_window_maximized};
use floem::prelude::*;
use floem::taffy::style::{Display, FlexWrap};
use floem::views::drag_window_area;
use floem::window::{ResizeDirection, WindowId};

use crate::app_core::layout;
use crate::app_core::settings::Screen;
use crate::app_core::windowing::WindowPolicy;

use super::actions;
use super::icons;
use super::state::{AppHandles, AppState};
use super::theme;

pub fn shell(
    window_id: WindowId,
    state: AppState,
    handles: AppHandles,
    content: impl IntoView + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let policy = WindowPolicy::current();
    let title = title_bar(window_id, state, handles.clone(), policy);
    let body = Stack::vertical((
        title,
        content.style(|s| s.flex_grow(1.0f32).min_width(0.0).min_height(0.0)),
    ))
    .style(move |s| {
        s.size_full()
            .min_width(0.0)
            .min_height(0.0)
            .background(theme::rgba(pal().bg))
    });
    if policy.client_resize_edges {
        Stack::new((body, resize_edges()))
            .style(|s| s.size_full())
            .into_any()
    } else {
        body.style(|s| s.size_full()).into_any()
    }
}

fn title_bar(
    window_id: WindowId,
    state: AppState,
    handles: AppHandles,
    policy: WindowPolicy,
) -> impl IntoView {
    let logo_bytes = handles.logo.clone();
    let logo = img(move || logo_bytes.clone()).style(|s| s.size(24, 24));
    let title_block = Stack::vertical((
        Label::new("Mujrim").style(|s| s.font_size(14.0).font_bold()),
        Label::new("Chess Engine • v1.0.0").style(move |s| {
            s.font_size(10.0).color(theme::rgba(
                theme::palette(state.settings.get().board_theme).accent,
            ))
        }),
    ))
    .style(|s| s.row_gap(1.0));
    let brand = Stack::horizontal((logo, title_block))
        .style(|s| s.col_gap(8.0f32).items_center().padding_left(12.0));
    let brand = if policy.undecorated {
        drag_window_area(brand).into_any()
    } else {
        brand.into_any()
    };

    let nav = nav_pills(state, handles);
    let trailing = if policy.client_window_controls {
        window_controls(window_id, state).into_any()
    } else {
        Empty::new()
            .style(|s| s.width(12.0).height(24.0))
            .into_any()
    };

    Stack::horizontal((
        brand,
        nav.style(|s| {
            s.flex_grow(1.0f32)
                .min_width(0.0)
                .justify_center()
                .items_center()
        }),
        trailing,
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.width_full()
            .height(layout::TITLE_BAR_PX)
            .min_width(0.0)
            .items_center()
            .background(theme::rgba(pal.sidebar))
            .border_bottom(1.0)
            .border_color(theme::rgba(pal.border))
    })
}

fn nav_pills(state: AppState, handles: AppHandles) -> impl IntoView {
    let board = move || {
        matches!(
            state.screen.get(),
            Screen::Playing | Screen::Analysis | Screen::Study | Screen::Learn
        )
    };
    Stack::horizontal((
        pill(
            state,
            icons::HOUSE,
            "Home",
            move || matches!(state.screen.get(), Screen::Menu),
            move || state.screen.set(Screen::Menu),
        ),
        pill(
            state,
            icons::SETTINGS,
            "Options",
            move || state.show_options.get(),
            move || {
                let open = !state.show_options.get_untracked();
                state.show_options.set(open);
                if open {
                    actions::refresh_updater_status(state);
                }
            },
        ),
        pill(
            state,
            icons::SEARCH,
            "Analyze",
            move || matches!(state.screen.get(), Screen::Analysis),
            move || state.screen.set(Screen::Analysis),
        ),
        pill(
            state,
            icons::DATABASE,
            "Study",
            move || matches!(state.screen.get(), Screen::Study),
            move || {
                actions::ensure_study_board(state);
                state.screen.set(Screen::Study);
            },
        ),
        pill(
            state,
            icons::SPARKLES,
            "Learn",
            move || matches!(state.screen.get(), Screen::Learn),
            move || {
                actions::ensure_study_board(state);
                state.screen.set(Screen::Learn);
            },
        ),
        pill(
            state,
            icons::TROPHY,
            "Tournaments",
            move || matches!(state.screen.get(), Screen::Tournaments),
            {
                let handles = handles.clone();
                move || {
                    state.screen.set(Screen::Tournaments);
                    if !state.tournament_snapshot.get_untracked().running {
                        actions::open_tournament_setup(state, &handles);
                    }
                }
            },
        ),
        Stack::horizontal((
            pill(state, icons::CAMERA, "Shot", || false, {
                let handles = handles.clone();
                move || actions::screenshot(state, &handles)
            }),
            pill(state, icons::PLUS, "New", || false, {
                let handles = handles.clone();
                move || actions::new_game(state, &handles)
            }),
            pill(
                state,
                icons::ARROW_UP_DOWN,
                "Flip",
                || false,
                move || {
                    state.game.update(|game| {
                        if let Some(game) = game.as_mut() {
                            game.flipped = !game.flipped;
                        }
                    });
                },
            ),
            pill(state, icons::FLAG, "Resign", || true, {
                let handles = handles.clone();
                move || actions::resign(state, &handles)
            }),
            pill(
                state,
                icons::CLIPBOARD,
                "PGN",
                || false,
                move || actions::export_pgn(state),
            ),
            pill(state, icons::DATABASE, "Library", || false, {
                let handles = handles.clone();
                move || actions::save_to_library(state, &handles)
            }),
            pill(state, icons::SPARKLES, "Review", || false, {
                let handles = handles.clone();
                move || actions::analyze_game(state, &handles)
            }),
            pill(
                state,
                icons::FILM,
                "GIF",
                || false,
                move || actions::export_gif(state),
            ),
            pill(
                state,
                icons::CIRCLE,
                "Rec",
                move || state.recording_label.get() != "Record",
                {
                    let handles = handles.clone();
                    move || actions::toggle_recording(state, &handles)
                },
            ),
        ))
        .style(move |s| {
            let s = s.col_gap(3.0).items_center();
            if board() { s } else { s.display(Display::None) }
        }),
    ))
    .style(|s| {
        s.col_gap(3.0)
            .items_center()
            .min_width(0.0)
            .flex_wrap(FlexWrap::Wrap)
    })
}

fn pill(
    state: AppState,
    icon: &'static str,
    label: &'static str,
    active: impl Fn() -> bool + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Button::new(
        Stack::horizontal((
            svg(move || icon.to_owned()).style(|s| s.size(13, 13)),
            Label::new(label).style(|s| s.font_size(11.0)),
        ))
        .style(|s| {
            s.col_gap(5.0)
                .items_center()
                .padding_horiz(8.0)
                .padding_vert(4.0)
        }),
    )
    .action(action)
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.border_radius(10.0)
            .background(if active() {
                theme::rgba(pal.accent)
            } else {
                Color::TRANSPARENT
            })
            .color(if active() {
                theme::rgba(pal.text_primary)
            } else {
                theme::rgba(pal.text_secondary)
            })
            .border(0.0)
            .hover(|s| {
                s.background(theme::rgba(pal.panel))
                    .color(theme::rgba(pal.text_primary))
            })
    })
}

fn window_controls(window_id: WindowId, state: AppState) -> impl IntoView {
    Stack::horizontal((
        icon_btn(state, icons::MINUS, minimize_window),
        icon_btn(state, icons::SQUARE, toggle_window_maximized),
        icon_btn(state, icons::X, move || floem::close_window(window_id)),
    ))
    .style(|s| s.col_gap(2.0).padding_right(8.0).items_center())
}

fn icon_btn(state: AppState, icon: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(svg(move || icon.to_owned()).style(|s| s.size(12, 12)))
        .action(action)
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.size(28, 24)
                .background(Color::TRANSPARENT)
                .color(theme::rgba(pal.text_secondary))
                .border(0.0)
                .items_center()
                .justify_center()
                .hover(|s| {
                    s.background(theme::rgba(pal.panel))
                        .color(theme::rgba(pal.text_primary))
                })
        })
}

fn resize_edges() -> impl IntoView {
    let edge = |dir: ResizeDirection, style: fn(floem::style::Style) -> floem::style::Style| {
        Empty::new()
            .style(move |s| style(s.absolute().z_index(20)))
            .on_event_stop(el::PointerDown, move |_, _| {
                drag_resize_window(dir);
            })
    };
    Stack::new((
        edge(ResizeDirection::North, |s| {
            s.inset_top(0).width_full().height(6)
        }),
        edge(ResizeDirection::South, |s| {
            s.inset_bottom(0).width_full().height(6)
        }),
        edge(ResizeDirection::West, |s| {
            s.inset_left(0).height_full().width(6)
        }),
        edge(ResizeDirection::East, |s| {
            s.inset_right(0).height_full().width(6)
        }),
        edge(ResizeDirection::NorthWest, |s| {
            s.inset_top(0).inset_left(0).size(10, 10)
        }),
        edge(ResizeDirection::NorthEast, |s| {
            s.inset_top(0).inset_right(0).size(10, 10)
        }),
        edge(ResizeDirection::SouthWest, |s| {
            s.inset_bottom(0).inset_left(0).size(10, 10)
        }),
        edge(ResizeDirection::SouthEast, |s| {
            s.inset_bottom(0).inset_right(0).size(10, 10)
        }),
    ))
    .style(|s| s.size_full().absolute())
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_bar_does_not_drag_on_pointer_move() {
        let src = include_str!("chrome.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(
            !production.contains("PointerMove"),
            "title-bar PointerMove drag detaches niri windows"
        );
        assert!(!production.contains("new_window"));
        assert!(!production.contains("drag_window()"));
        assert!(
            production.contains("open_tournament_setup"),
            "Tournaments nav must open the setup overlay"
        );
        assert!(
            production.contains("display(Display::None)"),
            "board tools must stay mounted and hidden instead of swapping Empty views"
        );
        assert!(
            !production.contains("dyn_view"),
            "creating nav widgets inside dyn_view leaves them without a window root"
        );
    }
}
