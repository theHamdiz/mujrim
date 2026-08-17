//! Custom client-side title bar and in-app navigation.

use floem::action::{minimize_window, toggle_window_maximized};
use floem::prelude::*;
use floem::taffy::style::Display;
use floem::views::{drag_resize_window_area, drag_window_area};
use floem::window::{ResizeDirection, WindowId};

use crate::app_core::layout;
use crate::app_core::settings::Screen;

use super::actions;
use super::icons;
use super::state::{AppHandles, AppState, offer_ateed_index, refresh_ateed_cli};
use super::theme;

pub fn shell(
    window_id: WindowId,
    state: AppState,
    handles: AppHandles,
    content: impl IntoView + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let title = title_bar(window_id, state, handles.clone());
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
    Stack::new((body, resize_edges()))
        .style(|s| s.size_full())
        .on_event_cont(el::WindowClosed, move |_, _| {
            actions::shutdown_ui_resources(state, &handles);
        })
        .into_any()
}

fn title_bar(window_id: WindowId, state: AppState, handles: AppHandles) -> impl IntoView {
    let logo_bytes = handles.logo.clone();
    let logo = img(move || logo_bytes.clone()).style(|s| s.size(22, 22));
    let title_block = Stack::vertical((
        Label::new("Mujrim").style(|s| s.font_size(13.0).font_bold()),
        Label::new("Chess Engine • v1.0.0").style(move |s| {
            s.font_size(10.0).color(theme::rgba(
                theme::palette(state.settings.get().board_theme).accent,
            ))
        }),
    ))
    .style(|s| s.row_gap(1.0));
    let brand = Stack::horizontal((logo, title_block)).style(|s| {
        s.col_gap(8.0f32)
            .items_center()
            .padding_horiz(10.0)
            .height_full()
            .pointer_events_none()
    });
    Stack::new((
        drag_window_area(Empty::new()).style(|s| s.size_full().absolute()),
        nav_pills(state, handles.clone()).style(|s| {
            s.absolute()
                .inset_left(0.0)
                .inset_right(0.0)
                .height_full()
                .min_width(0.0)
                .justify_center()
                .items_center()
                .pointer_events_none()
                .z_index(1)
        }),
        brand.style(|s| s.absolute().inset_left(0.0).height_full().z_index(2)),
        window_controls(window_id, state, handles).style(|s| {
            s.absolute()
                .inset_right(0.0)
                .height_full()
                .items_center()
                .z_index(3)
        }),
    ))
    .style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.width_full()
            .min_height(layout::TITLE_BAR_PX)
            .min_width(0.0)
            .padding_vert(4.0)
            .items_center()
            .background(theme::rgba(pal.sidebar))
            .border_bottom(1.0)
            .border_color(theme::rgba(pal.border))
    })
}

fn nav_pills(state: AppState, handles: AppHandles) -> impl IntoView {
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
            {
                let handles = handles.clone();
                move || {
                    actions::ensure_study_board(state, &handles);
                    actions::index_lichess_downloads(state, &handles);
                    actions::refresh_studies(state, &handles);
                    state.screen.set(Screen::Study);
                }
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
                    actions::open_tournaments_screen(state, &handles);
                }
            },
        ),
        pill(
            state,
            icons::FLAME,
            "Ateed",
            move || matches!(state.screen.get(), Screen::Ateed),
            {
                let handles = handles.clone();
                move || {
                    refresh_ateed_cli(state);
                    offer_ateed_index(state, &handles);
                    state.screen.set(Screen::Ateed);
                }
            },
        ),
    ))
    .style(|s| s.col_gap(3.0).items_center().min_width(0.0))
}

pub fn screen_tools(state: AppState, handles: AppHandles) -> impl IntoView {
    let playing = move || matches!(state.screen.get(), Screen::Playing);
    let analysis = move || matches!(state.screen.get(), Screen::Analysis);
    let study = move || matches!(state.screen.get(), Screen::Study);
    Stack::horizontal((
        Stack::horizontal((
            pill(state, icons::PLUS, "New", || false, {
                let handles = handles.clone();
                move || actions::new_game(state, &handles)
            }),
            pill(state, icons::FLAG, "Resign", || true, {
                let handles = handles.clone();
                move || actions::resign(state, &handles)
            }),
            pill(state, icons::CAMERA, "Shot", || false, {
                let handles = handles.clone();
                move || actions::screenshot(state, &handles)
            }),
            pill(state, icons::FILM, "GIF", || false, {
                move || actions::export_gif(state)
            }),
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
            if playing() {
                s
            } else {
                s.display(Display::None)
            }
        }),
        Stack::horizontal((
            pill(state, icons::SEARCH, "Review", || false, {
                let handles = handles.clone();
                move || actions::analyze_game(state, &handles)
            }),
            pill(state, icons::CLIPBOARD, "PGN", || false, {
                move || actions::export_pgn(state)
            }),
        ))
        .style(move |s| {
            let s = s.col_gap(3.0).items_center();
            if analysis() {
                s
            } else {
                s.display(Display::None)
            }
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
        )
        .style(move |s| {
            if playing() || analysis() || study() {
                s
            } else {
                s.display(Display::None)
            }
        }),
    ))
    .style(|s| s.col_gap(6.0).items_center().min_width(0.0))
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
            svg(icon).style(|s| s.size(13, 13)),
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
            .transition(
                floem::style::Background,
                floem::style::Transition::ease_in_out(std::time::Duration::from_millis(140)),
            )
            .pointer_events_auto()
    })
}

fn window_controls(window_id: WindowId, state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        icon_btn(state, icons::MINUS, minimize_window),
        icon_btn(state, icons::SQUARE, toggle_window_maximized),
        icon_btn(state, icons::X, move || {
            actions::shutdown_ui_resources(state, &handles);
            floem::close_window(window_id);
        }),
    ))
    .style(|s| s.col_gap(2.0).padding_right(8.0).items_center())
}

fn icon_btn(state: AppState, icon: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(svg(icon).style(|s| s.size(12, 12)))
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
                .pointer_events_auto()
        })
}

fn resize_edges() -> impl IntoView {
    let edge = |dir: ResizeDirection, style: fn(floem::style::Style) -> floem::style::Style| {
        drag_resize_window_area(dir, Empty::new())
            .style(move |s| style(s.absolute().z_index(20).pointer_events_auto()))
    };
    Stack::new((
        edge(ResizeDirection::North, |s| {
            s.inset_top(0).width_full().height(4)
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
            production.contains("open_tournaments_screen"),
            "Tournaments nav must resume a paused event or open setup"
        );
        assert!(
            production.contains("shutdown_ui_resources"),
            "closing the window must stop engines and persist a paused event"
        );
        assert!(
            production.contains("el::WindowClosed"),
            "Alt+F4 / compositor close must use the same shutdown path as the X button"
        );
        assert!(
            production.contains("ensure_study_board"),
            "Study nav must seed the shared study board"
        );
        assert!(
            production.contains("index_lichess_downloads"),
            "opening Study must index Lichess PGNs from Downloads"
        );
        assert!(
            !production.contains("\"Learn\""),
            "Learn is a Study tab, not a title-bar pill"
        );
        assert!(
            !production.contains("\"Library\""),
            "Library is a Study tab, not a title-bar pill"
        );
        assert!(
            production.contains("\"Ateed\""),
            "Ateed studio must have a title-bar nav pill"
        );
        assert!(
            production.contains("icons::FLAME"),
            "Ateed nav must use the flame icon"
        );
        assert!(
            production.contains("refresh_ateed_cli"),
            "opening Ateed must rescan the Mujrim CLI"
        );
        assert!(
            production.contains("offer_ateed_index"),
            "opening Ateed must offer to index new tournament games"
        );
        assert!(
            !production.contains("FlexWrap::Wrap"),
            "title bar must not wrap board tools onto a second row"
        );
        assert!(
            !production
                .split("pub fn screen_tools")
                .next()
                .unwrap()
                .contains("\"Shot\""),
            "screenshot belongs on the playing toolbar, not the global nav"
        );
        assert!(
            production.contains("pub fn screen_tools"),
            "board actions must live in a per-screen toolbar"
        );
        assert!(
            !production.contains("icon.to_owned()"),
            "chrome icons should reuse interned SVG strings"
        );
        assert!(
            production.contains("drag_window_area"),
            "custom title bar must drag the undecorated window"
        );
        assert!(
            !production.contains("drag_window_area(nav_pills"),
            "nav pills inside drag_window_area never receive Click"
        );
        assert!(
            production.contains("justify_center()"),
            "nav cluster must sit in the middle of the title bar"
        );
        assert!(
            production.contains("inset_right(0.0)"),
            "window controls must pin to the right edge"
        );
        assert!(
            production.contains("pointer_events_none"),
            "resize overlay must let title-bar clicks pass through"
        );
        assert!(
            production.contains("pointer_events_auto"),
            "resize handles must still receive pointer events"
        );
        assert!(
            production.contains("window_controls"),
            "custom title bar must own minimize/maximize/close"
        );
    }
}
