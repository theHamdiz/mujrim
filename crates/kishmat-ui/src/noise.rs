//! Procedural noise texture generation — macOS-style grain effect.

use iced::widget::image::Handle;
use rand::Rng;

/// Generate a tileable noise texture image for background overlays.
/// Returns an iced image Handle that can be tiled across the UI.
///
/// Parameters:
/// - `size`: texture dimensions (e.g. 128 for 128x128)
/// - `opacity`: noise opacity (0.0 to 1.0), typically 0.03-0.05 for macOS feel
/// - `base_r/g/b`: base color components (0-255)
pub fn generate_noise_texture(size: u32, opacity: f32, base_r: u8, base_g: u8, base_b: u8) -> Handle {
    let mut rng = rand::rng();
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);

    for _y in 0..size {
        for _x in 0..size {
            // Generate monochrome noise
            let noise: f32 = rng.random::<f32>(); // 0.0 to 1.0
            let noise_val = ((noise - 0.5) * 255.0 * opacity) as i16;

            let r = (base_r as i16 + noise_val).clamp(0, 255) as u8;
            let g = (base_g as i16 + noise_val).clamp(0, 255) as u8;
            let b = (base_b as i16 + noise_val).clamp(0, 255) as u8;

            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(255); // fully opaque
        }
    }

    // Encode to PNG for iced
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    image::ImageEncoder::write_image(
        encoder,
        &pixels,
        size,
        size,
        image::ExtendedColorType::Rgba8,
    )
    .expect("Failed to encode noise texture");

    Handle::from_bytes(buf)
}

/// macOS-style subtle grain background — light gray with ~3% noise.
#[allow(dead_code)]
pub fn macos_grain_light() -> Handle {
    generate_noise_texture(128, 0.035, 236, 236, 236) // #ECECEC base
}

/// Darker variant for title bars / sidebars.
pub fn macos_grain_dark() -> Handle {
    generate_noise_texture(128, 0.04, 45, 45, 48) // Dark sidebar
}

/// Medium gray variant for panels.
#[allow(dead_code)]
pub fn macos_grain_panel() -> Handle {
    generate_noise_texture(128, 0.03, 52, 52, 56)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_noise_texture_does_not_panic() {
        let _ = generate_noise_texture(64, 0.03, 200, 200, 200);
    }

    #[test]
    fn test_noise_texture_small_size() {
        // Even a 1x1 texture should work
        let _ = generate_noise_texture(1, 0.1, 128, 128, 128);
    }

    #[test]
    fn test_noise_texture_zero_opacity() {
        // Zero opacity = solid color (no noise)
        let _ = generate_noise_texture(16, 0.0, 100, 100, 100);
    }

    #[test]
    fn test_noise_texture_full_opacity() {
        // Full opacity should not panic (extreme noise)
        let _ = generate_noise_texture(16, 1.0, 128, 128, 128);
    }

    #[test]
    fn test_macos_grain_presets() {
        // All preset functions should produce valid textures
        let _ = macos_grain_light();
        let _ = macos_grain_dark();
        let _ = macos_grain_panel();
    }
}
