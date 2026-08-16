//! Shared Floem chrome: cards, buttons, in-flow pickers, and overlay frames.

use floem::prelude::*;
use floem::style::Transition;
use floem::taffy::style::Overflow;

use crate::app_core::layout;
use crate::app_core::palette::GuiPalette;
use crate::app_core::tournament_live::PodiumTier;

use super::icons;
use super::state::{AppHandles, AppState};
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
            .min_width(0.0)
            .width_full()
            .text_ellipsis()
            .color(theme::rgba(pal().accent_alt))
    })
}

pub fn card(state: AppState, child: impl IntoView + 'static) -> impl IntoView {
    child.style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.width_full()
            .min_width(0.0)
            .padding(theme::SPACE_LG)
            .row_gap(theme::SPACE_MD)
            .border_radius(16.0)
            .background(theme::rgba(pal.panel))
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .overflow_x(Overflow::Clip)
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
        s.min_width(0.0)
            .padding_horiz(16.0)
            .padding_vert(10.0)
            .border_radius(12.0)
            .border(0.0)
            .font_size(13.0)
            .font_bold()
            .background(theme::rgba(pal.accent))
            .color(theme::rgba(pal.text_primary))
            .hover(|s| s.background(theme::rgba(pal.accent_alt)))
            .transition(
                floem::style::Background,
                Transition::ease_in_out(std::time::Duration::from_millis(140)),
            )
    })
}

pub fn primary_button_when(
    state: AppState,
    label: &'static str,
    enabled: impl Fn() -> bool + Copy + 'static,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Button::new(label)
        .action(move || {
            if enabled() {
                action();
            }
        })
        .style(move |s| {
            let pal = theme::palette(state.settings.get().board_theme);
            let on = enabled();
            let s = s
                .min_width(0.0)
                .padding_horiz(16.0)
                .padding_vert(10.0)
                .border_radius(12.0)
                .border(0.0)
                .font_size(13.0)
                .font_bold()
                .background(theme::rgba(if on { pal.accent } else { pal.panel }))
                .color(theme::rgba(if on {
                    pal.text_primary
                } else {
                    pal.text_secondary
                }));
            if on {
                s.hover(|s| s.background(theme::rgba(pal.accent_alt)))
            } else {
                s.pointer_events_none()
            }
        })
}

pub fn ghost_button(
    state: AppState,
    label: &'static str,
    action: impl Fn() + 'static,
) -> impl IntoView {
    Button::new(label).action(action).style(move |s| {
        let pal = theme::palette(state.settings.get().board_theme);
        s.min_width(0.0)
            .padding_horiz(12.0)
            .padding_vert(8.0)
            .border_radius(10.0)
            .border(1.0)
            .border_color(theme::rgba(pal.border))
            .font_size(12.0)
            .background(Color::TRANSPARENT)
            .color(theme::rgba(pal.text_primary))
            .hover(|s| s.background(theme::rgba(pal.panel)))
            .transition(
                floem::style::Background,
                Transition::ease_in_out(std::time::Duration::from_millis(140)),
            )
    })
}

pub fn game_io_bar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        ghost_button(state, "Import", {
            let handles = handles.clone();
            move || super::actions::import_games(state, &handles)
        }),
        ghost_button(state, "PGN", {
            let handles = handles.clone();
            move || {
                super::actions::export_board(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Pgn,
                )
            }
        }),
        ghost_button(state, "JSON", {
            let handles = handles.clone();
            move || {
                super::actions::export_board(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Json,
                )
            }
        }),
        ghost_button(state, "EPD", {
            let handles = handles.clone();
            move || {
                super::actions::export_board(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Epd,
                )
            }
        }),
        ghost_button(state, "Binpack", {
            let handles = handles.clone();
            move || {
                super::actions::export_board(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Binpack,
                )
            }
        }),
    ))
    .style(|s| {
        s.width_full()
            .col_gap(6.0)
            .row_gap(6.0)
            .flex_wrap(floem::taffy::style::FlexWrap::Wrap)
    })
}

pub fn results_export_bar(state: AppState, handles: AppHandles) -> impl IntoView {
    Stack::horizontal((
        ghost_button(state, "PGN", {
            let handles = handles.clone();
            move || {
                super::actions::export_results(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Pgn,
                )
            }
        }),
        ghost_button(state, "JSON", {
            let handles = handles.clone();
            move || {
                super::actions::export_results(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Json,
                )
            }
        }),
        ghost_button(state, "EPD", {
            let handles = handles.clone();
            move || {
                super::actions::export_results(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Epd,
                )
            }
        }),
        ghost_button(state, "Binpack", {
            let handles = handles.clone();
            move || {
                super::actions::export_results(
                    state,
                    &handles,
                    mujrim_study::game_export::GameExportFormat::Binpack,
                )
            }
        }),
    ))
    .style(|s| {
        s.width_full()
            .col_gap(6.0)
            .row_gap(6.0)
            .flex_wrap(floem::taffy::style::FlexWrap::Wrap)
    })
}

/// Viewport-capped scroller. Height belongs on the scroll view, not the list body.
pub fn capped_scroll(child: impl IntoView + 'static, max_height: f64) -> impl IntoView {
    child.scroll().style(move |s| {
        s.width_full()
            .min_width(0.0)
            .min_height(0.0)
            .max_height(max_height)
    })
}

/// Scroller that fills leftover column space (dock panes, sidebar standings).
pub fn filling_scroll(child: impl IntoView + 'static) -> impl IntoView {
    child.scroll().style(|s| {
        s.width_full()
            .min_width(0.0)
            .min_height(0.0)
            .flex_grow(1.0f32)
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
            Stack::horizontal((
                Label::derived(move || active().to_string())
                    .style(|s| s.flex_grow(1.0f32).min_width(0.0).text_ellipsis()),
                dyn_view(move || {
                    svg(icons::chevron(open.get()))
                        .style(|s| s.size(14, 14))
                        .into_any()
                }),
            ))
            .style(|s| s.width_full().items_center().col_gap(8.0))
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
                capped_scroll(
                    Stack::vertical(list).style(move |s| {
                        let pal = theme::palette(state.settings.get().board_theme);
                        s.width_full()
                            .row_gap(2.0)
                            .padding(4.0)
                            .border_radius(10.0)
                            .background(theme::rgba(pal.bg))
                            .border(1.0)
                            .border_color(theme::rgba(pal.border))
                    }),
                    layout::PICKER_SCROLL_PX,
                )
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
            s.width(148.0)
                .min_width(0.0)
                .font_size(13.0)
                .text_wrap()
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
            s.width(148.0)
                .min_width(0.0)
                .font_size(13.0)
                .text_wrap()
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
    let value = std::rc::Rc::new(value);
    let on_toggle = std::rc::Rc::new(on_toggle);
    let value_visual = value.clone();
    Stack::horizontal((
        Label::new(label).style(move |s| {
            s.flex_grow(1.0f32)
                .min_width(0.0)
                .font_size(13.0)
                .text_ellipsis()
                .color(theme::rgba(pal().text_primary))
        }),
        Button::new(Label::derived({
            let value = value_visual.clone();
            move || {
                if value() {
                    "On".to_owned()
                } else {
                    "Off".to_owned()
                }
            }
        }))
        .action({
            let value = value.clone();
            let on_toggle = on_toggle.clone();
            move || {
                let current = value();
                let Some(next) = crate::app_core::settings::committed_toggle(current, !current)
                else {
                    return;
                };
                on_toggle(next);
            }
        })
        .style(move |s| {
            let on = value();
            let pal = pal();
            s.min_width(52.0)
                .padding_horiz(10.0)
                .padding_vert(4.0)
                .border_radius(12.0)
                .border(0.0)
                .font_size(11.0)
                .font_bold()
                .background(if on {
                    theme::rgba(pal.accent)
                } else {
                    theme::rgba(pal.bg)
                })
                .color(theme::rgba(pal.text_primary))
        }),
    ))
    .style(|s| s.width_full().col_gap(12.0).items_center().min_width(0.0))
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
    overlay_frame_sized(state, on_close, child, layout::OVERLAY_MAX_WIDTH)
}

pub fn overlay_frame_sized(
    state: AppState,
    on_close: impl Fn() + 'static,
    child: impl IntoView + 'static,
    width: f64,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    Stack::new((
        Empty::new()
            .style(|s| s.size_full().absolute().background(theme::overlay_scrim()))
            .on_event_stop(el::PointerDown, move |_, _| on_close()),
        child
            .style(move |s| {
                let pal = pal();
                s.width(width)
                    .max_width_pct(92.0)
                    .min_width(280.0)
                    .padding(layout::OVERLAY_PAD)
                    .border_radius(18.0)
                    .background(theme::rgba(pal.panel))
                    .border(1.0)
                    .border_color(theme::rgba(pal.border))
                    .overflow_x(Overflow::Clip)
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

pub fn wrapping_label(
    text: impl Fn() -> String + 'static,
    pal: impl Fn() -> GuiPalette + Copy + 'static,
) -> impl IntoView {
    Label::derived(text).style(move |s| {
        s.font_size(theme::TYPE_BODY)
            .min_width(0.0)
            .width_full()
            .text_wrap()
            .color(theme::rgba(pal().text_secondary))
    })
}

pub fn body_copy(
    text: impl Into<String>,
    pal: impl Fn() -> GuiPalette + Copy + 'static,
) -> impl IntoView {
    let text = text.into();
    wrapping_label(move || text.clone(), pal)
}

pub fn explanation_card(
    state: AppState,
    lines: impl Fn() -> Vec<(String, Vec<types::Square>)> + 'static,
) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let lines = std::rc::Rc::new(lines);
    let spoken = lines.clone();
    card(
        state,
        Stack::vertical((
            section_label("Position explainer", pal),
            dyn_view({
                let lines = lines.clone();
                move || {
                    let items = lines();
                    if items.is_empty() {
                        return wrapping_label(
                            || "Quiet position — no immediate tactical alarms.".to_owned(),
                            pal,
                        )
                        .into_any();
                    }
                    Stack::vertical(
                        items
                            .into_iter()
                            .map(|(text, squares)| {
                                Button::new(text)
                                    .action(move || {
                                        super::actions::highlight_explain(state, squares.clone())
                                    })
                                    .style(move |s| {
                                        s.width_full()
                                            .min_width(0.0)
                                            .justify_start()
                                            .border(0.0)
                                            .padding_vert(2.0)
                                            .font_size(theme::TYPE_BODY)
                                            .color(theme::rgba(pal().text_secondary))
                                            .background(floem::peniko::Color::TRANSPARENT)
                                    })
                            })
                            .collect::<Vec<_>>(),
                    )
                    .style(|s| s.row_gap(4.0).width_full())
                    .into_any()
                }
            }),
            ghost_button(state, "Speak", move || {
                let spoken = spoken()
                    .into_iter()
                    .map(|(text, _)| text)
                    .collect::<Vec<_>>()
                    .join("\n");
                super::actions::speak_explanation(&spoken);
            }),
        ))
        .style(|s| s.row_gap(theme::SPACE_SM).width_full().min_width(0.0)),
    )
}

pub fn standing_rows_list(state: AppState, empty: &'static str) -> impl IntoView {
    let pal = move || theme::palette(state.settings.get().board_theme);
    let rows = (0..layout::STANDING_SLOTS)
        .map(|index| standing_row_slot(state, index))
        .collect::<Vec<_>>();
    Stack::vertical((
        Label::derived(move || {
            if state.tournament_snapshot.get().standings.is_empty() {
                empty.to_owned()
            } else {
                String::new()
            }
        })
        .style(move |s| {
            let empty_list = state.tournament_snapshot.get().standings.is_empty();
            let s = s
                .font_size(theme::TYPE_CAPTION)
                .min_width(0.0)
                .width_full()
                .text_wrap()
                .color(theme::rgba(pal().text_secondary));
            if empty_list {
                s
            } else {
                s.display(floem::taffy::style::Display::None)
            }
        }),
        capped_scroll(
            rows.into_view()
                .style(|s| s.width_full().row_gap(6.0).flex_col().min_width(0.0)),
            layout::LIST_SCROLL_PX,
        ),
    ))
    .style(|s| s.width_full().row_gap(2.0).min_width(0.0).min_height(0.0))
}

fn standing_row_slot(state: AppState, index: usize) -> impl IntoView {
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
            svg(icons::TROPHY).style(move |s| {
                let Some(row) = row() else {
                    return s.display(floem::taffy::style::Display::None);
                };
                let (tr, tg, tb) = row.podium().map(PodiumTier::rgb).unwrap_or((160, 160, 168));
                let s = s.size(16, 16).color(Color::from_rgb8(tr, tg, tb));
                if row.podium().is_none() {
                    s.display(floem::taffy::style::Display::None)
                } else {
                    s
                }
            }),
            Label::derived(move || row().map(|row| row.rank.to_string()).unwrap_or_default())
                .style(move |s| {
                    let (tr, tg, tb) = row()
                        .and_then(|row| row.podium())
                        .map(PodiumTier::rgb)
                        .unwrap_or((160, 160, 168));
                    s.font_size(12.0)
                        .font_bold()
                        .width(22.0)
                        .color(Color::from_rgb8(tr, tg, tb))
                }),
            Label::derived(move || row().map(|row| row.name).unwrap_or_default()).style(move |s| {
                s.flex_grow(1.0f32)
                    .min_width(0.0)
                    .font_size(theme::TYPE_BODY)
                    .font_bold()
                    .text_ellipsis()
                    .color(theme::rgba(pal().text_primary))
            }),
        ))
        .style(|s| s.width_full().col_gap(8.0).items_center().min_width(0.0)),
        Label::derived(move || row().map(|row| row.score_line()).unwrap_or_default()).style(
            move |s| {
                s.font_size(theme::TYPE_CAPTION)
                    .min_width(0.0)
                    .width_full()
                    .text_wrap()
                    .color(theme::rgba(pal().text_secondary))
            },
        ),
    ))
    .style(move |s| {
        let Some(row) = row() else {
            return s.display(floem::taffy::style::Display::None);
        };
        let s = s
            .width_full()
            .row_gap(2.0)
            .padding_horiz(8.0)
            .padding_vert(8.0)
            .border_radius(10.0)
            .min_width(0.0);
        if let Some(podium) = row.podium() {
            let (tr, tg, tb) = podium.rgb();
            s.background(Color::from_rgba8(tr, tg, tb, 28))
        } else {
            s
        }
    })
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
        assert!(
            production.contains("results_export_bar"),
            "results export buttons must stay always-mounted"
        );
        assert!(
            !production.contains("ToggleChanged"),
            "settings toggles must not use ToggleChanged cascade"
        );
        assert!(
            production.contains("committed_toggle"),
            "toggle clicks must ignore no-op events"
        );
        assert!(
            production.contains("standing_rows_list"),
            "podium standings widget must be shared"
        );
        assert!(
            production.contains("score_line"),
            "standings must show one tournament Elo line"
        );
        assert!(
            production.contains("text_wrap()"),
            "sidebar copy must wrap instead of painting past the panel"
        );
        assert!(
            production.contains("text_ellipsis()"),
            "single-line sidebar titles must ellipsize"
        );
        assert!(
            production.contains("capped_scroll"),
            "dropdown and sidebar lists must share a viewport-capped scroller"
        );
        assert!(
            production.contains("primary_button_when"),
            "CLI-backed actions must use a gated primary button"
        );
        let scroller = production
            .split("pub fn capped_scroll")
            .nth(1)
            .expect("capped_scroll")
            .split("pub fn filling_scroll")
            .next()
            .expect("filling_scroll follows capped_scroll");
        assert!(
            scroller.contains("child.scroll()"),
            "capped_scroll must wrap the list in a Floem Scroll view"
        );
        assert!(
            scroller.contains(".max_height(max_height)"),
            "list max-height must sit on the scroll viewport, not the inner stack"
        );
        assert!(
            production
                .split("pub fn picker")
                .nth(1)
                .expect("picker")
                .contains("capped_scroll"),
            "engine and theme pickers must scroll when the roster is long"
        );
        let picker = production
            .split("pub fn picker")
            .nth(1)
            .expect("picker source");
        assert!(
            picker.contains("icons::chevron"),
            "picker disclosure must use a bundled Lucide chevron, not a missing Inter glyph"
        );
        assert!(
            !picker.contains('▴') && !picker.contains('▾'),
            "picker must not depend on geometric triangles missing from Inter"
        );
        let standings = production
            .split("pub fn standing_rows_list")
            .nth(1)
            .expect("standing_rows_list");
        assert!(
            standings.contains("capped_scroll"),
            "standings must scroll instead of overflowing the pane"
        );
        assert!(
            !standings.contains("dyn_view"),
            "rebuilding standings inside dyn_view resets the scroll offset"
        );
        assert!(
            standings.contains("STANDING_SLOTS"),
            "standings must keep a mounted slot list"
        );
        assert!(
            production.contains("highlight_explain"),
            "explainer bullets must highlight referenced squares"
        );
        assert!(
            production.contains("Speak"),
            "explainer text stays visible; Speak is optional"
        );
    }
}
