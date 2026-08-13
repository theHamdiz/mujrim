//! Palette conversion at the Floem view boundary.

use floem::prelude::Color;

use crate::app_core::palette::{GuiPalette, Rgba};

pub const CURIOUS_FAMILY: &str = "Curious Track";
pub const TYPE_CAPTION: f32 = 11.0;
pub const TYPE_BODY: f32 = 13.0;
pub const TYPE_TITLE: f32 = 16.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 18.0;

pub fn rgba(color: Rgba) -> Color {
    let [r, g, b, a] = color.to_u8();
    Color::from_rgba8(r, g, b, a)
}

pub fn palette(settings_theme: crate::app_core::palette::BoardTheme) -> GuiPalette {
    settings_theme.gui_palette()
}

pub fn overlay_scrim() -> Color {
    Color::from_rgba8(4, 6, 10, 168)
}
