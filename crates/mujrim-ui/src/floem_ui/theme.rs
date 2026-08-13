//! Palette conversion at the Floem view boundary.

use floem::prelude::Color;

use crate::app_core::palette::{GuiPalette, Rgba};

pub fn rgba(color: Rgba) -> Color {
    let [r, g, b, a] = color.to_u8();
    Color::from_rgba8(r, g, b, a)
}

pub fn palette(settings_theme: crate::app_core::palette::BoardTheme) -> GuiPalette {
    settings_theme.gui_palette()
}
