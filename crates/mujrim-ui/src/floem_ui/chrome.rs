//! Undecorated title bar, nav pills, window controls, and edge resize.

use floem::action::{drag_resize_window, drag_window, minimize_window, toggle_window_maximized};
use floem::prelude::*;
use floem::window::{ResizeDirection, WindowId};

use crate::app_core::recording::RecordState;
use crate::app_core::settings::Screen;

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
    let title = title_bar(window_id, state, handles.clone());
    let edges = resize_edges(window_id);
    Stack::new((
        Stack::vertical((
            title,
            content.style(|s| s.flex_grow(1.0f32).min_height(0.0)),
        ))
        .style(move |s| s.size_full().background(theme::rgba(pal().bg))),
        edges,
    ))
    .style(|s| s.size_full())
}

fn title_bar(window_id: WindowId, state: AppState, handles: AppHandles) -> impl IntoView {
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

    let nav = nav_pills(state, handles.clone());
    let controls = window_controls(window_id, state);

    Stack::horizontal((
        Stack::horizontal((logo, title_block))
            .style(|s| s.col_gap(8.0f32).items_center().padding_left(12.0)),
        nav.style(|s| s.flex_grow(1.0f32).justify_center().items_center()),
        controls,
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.width_full()
            .height(44.0)
            .items_center()
            .background(theme::rgba(pal.sidebar))
            .border_bottom(1.0)
            .border_color(theme::rgba(pal.border))
    })
    .on_event_stop(
        el::PointerMove,
        move |_, event: &floem::ui_events::pointer::PointerUpdate| {
            if event
                .current
                .buttons
                .contains(floem::ui_events::pointer::PointerButton::Primary)
            {
                drag_window();
            }
        },
    )
}

fn nav_pills(state: AppState, handles: AppHandles) -> impl IntoView {
    let playing = move || matches!(state.screen.get(), Screen::Playing | Screen::Analysis);
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
                state.show_options.update(|open| *open = !*open);
            },
        ),
        dyn_view(move || {
            if playing() {
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
                        if handles.recorder.state() == RecordState::Recording {
                            icons::CIRCLE_STOP
                        } else {
                            icons::CIRCLE
                        },
                        "Rec",
                        || false,
                        {
                            let handles = handles.clone();
                            move || actions::toggle_recording(state, &handles)
                        },
                    ),
                ))
                .style(|s| s.col_gap(3.0).items_center())
                .into_any()
            } else {
                Stack::horizontal((
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
                        move || state.screen.set(Screen::Study),
                    ),
                    pill(
                        state,
                        icons::TROPHY,
                        "Tournaments",
                        move || matches!(state.screen.get(), Screen::Tournaments),
                        move || state.screen.set(Screen::Tournaments),
                    ),
                ))
                .style(|s| s.col_gap(3.0).items_center())
                .into_any()
            }
        }),
    ))
    .style(|s| s.col_gap(3.0).items_center())
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
            svg(move || icon.to_owned()).style(|s| s.size(14, 14)),
            Label::new(label).style(|s| s.font_size(12.0)),
        ))
        .style(|s| {
            s.col_gap(6.0)
                .items_center()
                .padding_horiz(10.0)
                .padding_vert(6.0)
        }),
    )
    .action(action)
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.border_radius(16.0)
            .background(if active() {
                theme::rgba(pal.accent)
            } else {
                theme::rgba(pal.panel)
            })
            .color(theme::rgba(pal.text_primary))
            .border(0.0)
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
            s.size(32, 28)
                .background(Color::TRANSPARENT)
                .color(theme::rgba(pal.text_secondary))
                .border(0.0)
                .items_center()
                .justify_center()
        })
}

fn resize_edges(_window_id: WindowId) -> impl IntoView {
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
    .style(|s| s.size_full().absolute().pointer_events_none())
}
