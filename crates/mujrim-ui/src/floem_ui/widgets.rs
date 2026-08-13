//! Shared Floem chrome: cards, buttons, pickers, and overlay frames.

use floem::prelude::*;
use floem::views::dropdown::Dropdown;

use crate::app_core::layout;
use crate::app_core::palette::GuiPalette;

use super::state::AppState;
use super::theme;

pub fn curious_title(text: impl Into<String>, size: f32) -> impl IntoView {
    Label::new(text.into()).style(move |s| {
        s.font_size(size)
            .font_bold()
            .font_family(theme::CURIOUS_FAMILY.to_owned())
    })
}

pub fn section_label(
    text: &'static str,
    pal: impl Fn() -> GuiPalette + Copy + 'static,
) -> impl IntoView {
    Label::new(text).style(move |s| {
        s.font_size(12.0)
            .font_bold()
            .color(theme::rgba(pal().accent_alt))
    })
}

pub fn card(state: AppState, child: impl IntoView + 'static) -> impl IntoView {
    child.style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.padding(18.0)
            .row_gap(10.0)
            .border_radius(16.0)
            .background(theme::rgba(pal.panel))
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .min_width(0.0)
    })
}

pub fn side_badge(label: &'static str, light: bool) -> impl IntoView {
    Label::new(label).style(move |s| {
        s.size(32.0, 32.0)
            .items_center()
            .justify_center()
            .border_radius(8.0)
            .font_size(13.0)
            .font_bold()
            .background(if light {
                Color::from_rgb8(240, 224, 194)
            } else {
                Color::from_rgb8(36, 31, 41)
            })
            .color(if light {
                Color::from_rgb8(51, 38, 26)
            } else {
                Color::from_rgb8(204, 204, 218)
            })
    })
}

pub fn primary_button(
    state: AppState,
    label: &'static str,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Button::new(label).action(action).style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.padding_horiz(16.0)
            .padding_vert(10.0)
            .border_radius(12.0)
            .border(0.0)
            .font_size(13.0)
            .font_bold()
            .background(theme::rgba(pal.accent))
            .color(theme::rgba(pal.text_primary))
            .hover(|s| s.background(theme::rgba(pal.accent_alt)))
    })
}

pub fn ghost_button(
    state: AppState,
    label: &'static str,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Button::new(label).action(action).style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.padding_horiz(12.0)
            .padding_vert(8.0)
            .border_radius(10.0)
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .font_size(12.0)
            .background(Color::TRANSPARENT)
            .color(theme::rgba(pal.text_primary))
            .hover(|s| s.background(theme::rgba(pal.panel)))
    })
}

pub fn picker<T>(
    state: AppState,
    active: impl Fn() -> T + 'static,
    items: impl IntoIterator<Item = T> + 'static,
    on_accept: impl Fn(T) + 'static,
) -> impl IntoView
where
    T: Clone + PartialEq + std::fmt::Display + std::fmt::Debug + 'static,
{
    Dropdown::new(active, items)
        .on_accept(on_accept)
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.width_full()
                .min_width(0.0)
                .border_radius(10.0)
                .background(theme::rgba(pal.bg))
                .border(1.0)
                .border_color(theme::rgba(pal.border))
                .color(theme::rgba(pal.text_primary))
        })
}

pub fn picker_row<T>(
    state: AppState,
    label: &'static str,
    active: impl Fn() -> T + Copy + 'static,
    items: impl IntoIterator<Item = T> + 'static,
    on_accept: impl Fn(T) + 'static,
) -> impl IntoView
where
    T: Clone + PartialEq + std::fmt::Display + std::fmt::Debug + 'static,
{
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::horizontal((
        Label::new(label).style(move |s| {
            s.width(150.0)
                .font_size(13.0)
                .color(theme::rgba(pal().text_secondary))
        }),
        picker(state, active, items, on_accept).style(|s| s.flex_grow(1.0f32).min_width(0.0)),
    ))
    .style(|s| s.width_full().col_gap(12.0).items_center().min_width(0.0))
}

pub fn toggle_row(
    state: AppState,
    label: impl Into<String>,
    value: impl Fn() -> bool + 'static,
    on_toggle: impl Fn(bool) + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let label = label.into();
    Stack::horizontal((
        Label::new(label).style(move |s| {
            s.flex_grow(1.0f32)
                .min_width(0.0)
                .font_size(13.0)
                .color(theme::rgba(pal().text_primary))
        }),
        ToggleButton::new(value)
            .on_event_stop(ToggleChanged::listener(), move |_, next| on_toggle(*next))
            .style(move |s| s.color(theme::rgba(pal().accent))),
    ))
    .style(|s| s.width_full().col_gap(12.0).items_center())
}

pub fn overlay_frame(
    state: AppState,
    on_close: impl Fn() + 'static,
    child: impl IntoView + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::new((
        Empty::new()
            .style(|s| s.size_full().absolute().background(theme::overlay_scrim()))
            .on_event_stop(el::PointerDown, move |_, _| on_close()),
        child
            .style(move |s| {
                let pal = pal();
                s.width(layout::OVERLAY_MAX_WIDTH)
                    .max_width_pct(92.0)
                    .min_width(0.0)
                    .padding(layout::OVERLAY_PAD)
                    .border_radius(18.0)
                    .background(theme::rgba(pal.panel))
                    .border(1.0)
                    .border_color(theme::rgba(pal.border))
            })
            .scroll()
            .style(|s| {
                s.max_width_pct(92.0)
                    .max_height_pct(88.0)
                    .min_width(0.0)
                    .min_height(0.0)
            }),
    ))
    .style(|s| {
        s.size_full()
            .absolute()
            .items_center()
            .justify_center()
            .z_index(40)
            .padding(16.0)
    })
}
