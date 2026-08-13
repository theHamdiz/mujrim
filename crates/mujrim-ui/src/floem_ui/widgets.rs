//! Shared Floem chrome: cards, buttons, in-flow pickers, and overlay frames.

use floem::prelude::*;

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

pub fn glass_card(state: AppState, child: impl IntoView + 'static) -> impl IntoView {
    child.style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.padding(18.0)
            .row_gap(10.0)
            .border_radius(18.0)
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

/// In-flow expander. Never creates an OS window or Floem overlay toplevel.
pub fn picker<T>(
    state: AppState,
    active: impl Fn() -> T + 'static,
    items: impl IntoIterator<Item = T> + 'static,
    on_accept: impl Fn(T) + 'static,
) -> impl IntoView
where
    T: Clone + PartialEq + std::fmt::Display + std::fmt::Debug + 'static,
{
    let open = RwSignal::new(false);
    let items: Vec<T> = items.into_iter().collect();
    let active = std::rc::Rc::new(active);
    let on_accept = std::rc::Rc::new(on_accept);
    Stack::vertical((
        Button::new({
            let active = active.clone();
            Label::derived(move || format!("{}  {}", active(), if open.get() { "▴" } else { "▾" }))
        })
        .action(move || open.update(|value| *value = !*value))
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            s.width_full()
                .min_width(0.0)
                .padding_horiz(10.0)
                .padding_vert(8.0)
                .border_radius(10.0)
                .background(theme::rgba(pal.bg))
                .border(1.0)
                .border_color(theme::rgba(pal.border))
                .color(theme::rgba(pal.text_primary))
        }),
        dyn_view({
            let items = items.clone();
            let on_accept = on_accept.clone();
            let active = active.clone();
            move || {
                if !open.get() {
                    return Empty::new().into_any();
                }
                let list = items
                    .iter()
                    .map(|item| {
                        let selected = active() == item.clone();
                        let on_accept = on_accept.clone();
                        Button::new(item.to_string())
                            .action({
                                let item = item.clone();
                                move || {
                                    on_accept(item.clone());
                                    open.set(false);
                                }
                            })
                            .style(move |s| {
                                let pal = theme::palette(state.settings.get().board_theme);
                                s.width_full()
                                    .padding_horiz(10.0)
                                    .padding_vert(6.0)
                                    .border(0.0)
                                    .border_radius(8.0)
                                    .background(if selected {
                                        theme::rgba(pal.accent)
                                    } else {
                                        Color::TRANSPARENT
                                    })
                                    .color(theme::rgba(pal.text_primary))
                                    .hover(|s| s.background(theme::rgba(pal.panel)))
                            })
                    })
                    .collect::<Vec<_>>();
                Stack::vertical(list)
                    .style(move |s| {
                        let pal = theme::palette(state.settings.get().board_theme);
                        s.width_full()
                            .row_gap(2.0)
                            .padding(4.0)
                            .border_radius(10.0)
                            .background(theme::rgba(pal.bg))
                            .border(1.0)
                            .border_color(theme::rgba(pal.border))
                    })
                    .into_any()
            }
        }),
    ))
    .style(|s| s.width_full().row_gap(4.0).min_width(0.0))
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

pub fn stepper_row(
    state: AppState,
    label: &'static str,
    unit: &'static str,
    value: impl Fn() -> i32 + Copy + 'static,
    on_change: impl Fn(i32) + Clone + 'static,
    min: i32,
    max: i32,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let on_minus = on_change.clone();
    let on_plus = on_change;
    Stack::horizontal((
        Label::new(label).style(move |s| {
            s.width(110.0)
                .font_size(13.0)
                .color(theme::rgba(pal().text_secondary))
        }),
        Button::new("−").action(move || on_minus((value() - 1).clamp(min, max))),
        Label::derived(move || {
            if unit.is_empty() {
                value().to_string()
            } else {
                format!("{} {unit}", value())
            }
        })
        .style(move |s| {
            s.width(72.0)
                .font_size(13.0)
                .color(theme::rgba(pal().text_primary))
        }),
        Button::new("+").action(move || on_plus((value() + 1).clamp(min, max))),
    ))
    .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0))
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

pub fn overlay_layer_style(style: floem::style::Style) -> floem::style::Style {
    style
        .size_full()
        .absolute()
        .inset_left(0.0)
        .inset_top(0.0)
        .inset_right(0.0)
        .inset_bottom(0.0)
        .z_index(80)
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
                    .min_width(280.0)
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
                    .min_width(280.0)
                    .min_height(160.0)
            }),
    ))
    .style(|s| s.size_full().items_center().justify_center().padding(16.0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn picker_stays_in_the_view_tree() {
        let src = include_str!("widgets.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(
            !production.contains("Dropdown"),
            "in-flow picker must not use Dropdown overlays"
        );
        assert!(
            production.contains("z_index(80)"),
            "modal overlay must sit above the shell"
        );
        assert!(
            production.contains("min_height(160.0)"),
            "modal panel must have a visible minimum size"
        );
    }
}
