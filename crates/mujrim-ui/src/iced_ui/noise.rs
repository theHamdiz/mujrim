//! Iced image handles wrapping shared PNG bytes.

use iced::widget::image::Handle;

use crate::app_core::noise::{self as core_noise, PngImage};

fn to_handle(image: PngImage) -> Handle {
    Handle::from_bytes(image.bytes)
}

#[allow(dead_code)]
pub fn generate_noise_texture(
    size: u32,
    opacity: f32,
    base_r: u8,
    base_g: u8,
    base_b: u8,
) -> Handle {
    to_handle(core_noise::generate_noise_texture(
        size, opacity, base_r, base_g, base_b,
    ))
}

#[allow(dead_code)]
pub fn macos_grain_light() -> Handle {
    to_handle(core_noise::macos_grain_light())
}

#[allow(dead_code)]
pub fn macos_grain_dark() -> Handle {
    to_handle(core_noise::macos_grain_dark())
}

#[allow(dead_code)]
pub fn macos_grain_panel() -> Handle {
    to_handle(core_noise::macos_grain_panel())
}

pub fn pharaonic_pattern(size: u32) -> Handle {
    to_handle(core_noise::pharaonic_pattern(size))
}

pub fn chess_blur_background(w: u32, h: u32) -> Handle {
    to_handle(core_noise::chess_blur_background(w, h))
}
