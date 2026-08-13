//! Procedural noise texture and pattern generation (PNG bytes).

use rand::Rng;

pub struct PngImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn generate_noise_texture(
    size: u32,
    opacity: f32,
    base_r: u8,
    base_g: u8,
    base_b: u8,
) -> PngImage {
    let mut rng = rand::rng();
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);

    for _y in 0..size {
        for _x in 0..size {
            let noise: f32 = rng.random::<f32>();
            let noise_val = ((noise - 0.5) * 255.0 * opacity) as i16;
            let r = (base_r as i16 + noise_val).clamp(0, 255) as u8;
            let g = (base_g as i16 + noise_val).clamp(0, 255) as u8;
            let b = (base_b as i16 + noise_val).clamp(0, 255) as u8;
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    encode_png(&pixels, size, size)
}

#[allow(dead_code)]
pub fn macos_grain_light() -> PngImage {
    generate_noise_texture(128, 0.035, 236, 236, 236)
}

#[allow(dead_code)]
pub fn macos_grain_dark() -> PngImage {
    generate_noise_texture(128, 0.04, 45, 45, 48)
}

#[allow(dead_code)]
pub fn macos_grain_panel() -> PngImage {
    generate_noise_texture(128, 0.03, 52, 52, 56)
}

pub fn pharaonic_pattern(size: u32) -> PngImage {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let base = (26u8, 26u8, 46u8);
    let gold = (180u8, 145u8, 60u8);
    let dim_gold = (90u8, 72u8, 30u8);
    let teal = (45u8, 85u8, 70u8);
    let cell = size / 4;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let lx = x % cell;
            let ly = y % cell;
            let cx = cell / 2;
            let cy = cell / 2;
            let nx = lx as f32 / cell as f32;
            let ny = ly as f32 / cell as f32;
            let mut r = base.0;
            let mut g = base.1;
            let mut b = base.2;
            let dx = (lx as i32 - cx as i32).unsigned_abs();
            let dy = (ly as i32 - cy as i32).unsigned_abs();
            let diamond_dist = dx + dy;
            let diamond_r1 = cell * 3 / 8;
            let diamond_r2 = cell * 3 / 8 + 2;
            if diamond_dist >= diamond_r1 && diamond_dist <= diamond_r2 {
                r = gold.0;
                g = gold.1;
                b = gold.2;
            }
            let inner_r1 = cell / 5;
            let inner_r2 = cell / 5 + 1;
            if diamond_dist >= inner_r1 && diamond_dist <= inner_r2 {
                r = dim_gold.0;
                g = dim_gold.1;
                b = dim_gold.2;
            }
            if dx <= 1 && dy <= 1 && diamond_dist <= 2 {
                r = gold.0;
                g = gold.1;
                b = gold.2;
            }
            let band_width = 1;
            let band_pos_1 = 0;
            let band_pos_2 = cell - 1;
            if (ly == band_pos_1
                || ly == band_pos_2
                || ly.abs_diff(band_pos_1) <= band_width
                || ly.abs_diff(band_pos_2) <= band_width)
                && (lx / 3).is_multiple_of(2)
            {
                r = dim_gold.0;
                g = dim_gold.1;
                b = dim_gold.2;
            }
            if (lx == band_pos_1
                || lx == band_pos_2
                || lx.abs_diff(band_pos_1) <= band_width
                || lx.abs_diff(band_pos_2) <= band_width)
                && (ly / 3).is_multiple_of(2)
            {
                r = dim_gold.0;
                g = dim_gold.1;
                b = dim_gold.2;
            }
            let corners = [
                (0u32, 0u32),
                (0, cell - 1),
                (cell - 1, 0),
                (cell - 1, cell - 1),
            ];
            for &(corner_x, corner_y) in &corners {
                let dist_sq =
                    (lx as i32 - corner_x as i32).pow(2) + (ly as i32 - corner_y as i32).pow(2);
                let radius = cell / 6;
                let dist = (dist_sq as f32).sqrt() as u32;
                if dist >= radius && dist <= radius + 1 {
                    r = teal.0;
                    g = teal.1;
                    b = teal.2;
                }
            }
            let global_band_y = y % (cell * 2);
            if global_band_y == cell || global_band_y == cell + 1 {
                let zigzag_x = x % (cell / 2);
                let zz_half = cell / 4;
                let expected_y = if zigzag_x < zz_half {
                    cell + zigzag_x / 2
                } else {
                    cell + (cell / 2 - zigzag_x) / 2
                };
                if y.abs_diff(expected_y) <= 1 {
                    r = dim_gold.0;
                    g = dim_gold.1;
                    b = dim_gold.2;
                }
            }
            let noise = ((nx * 7919.0 + ny * 104729.0).sin() * 43_758.547).fract();
            let noise_val = ((noise - 0.5) * 8.0) as i8;
            r = (r as i16 + noise_val as i16).clamp(0, 255) as u8;
            g = (g as i16 + noise_val as i16).clamp(0, 255) as u8;
            b = (b as i16 + noise_val as i16).clamp(0, 255) as u8;
            pixels[idx] = r;
            pixels[idx + 1] = g;
            pixels[idx + 2] = b;
            pixels[idx + 3] = 255;
        }
    }
    encode_png(&pixels, size, size)
}

pub fn chess_blur_background(w: u32, h: u32) -> PngImage {
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 4) as usize;
            let nx = x as f32 / w as f32;
            let ny = y as f32 / h as f32;
            let dist = ((nx - 0.5).powi(2) + (ny - 0.5).powi(2)).sqrt();
            let vignette = 1.0 - (dist * 0.8).min(0.4);
            pixels[idx] = (20.0 * vignette) as u8;
            pixels[idx + 1] = (22.0 * vignette) as u8;
            pixels[idx + 2] = (42.0 * vignette) as u8;
            pixels[idx + 3] = 255;
        }
    }
    let board_size = w.min(h) * 3 / 5;
    let sq = board_size / 8;
    let ox = (w - board_size) / 2;
    let oy = (h - board_size) / 2;
    for row in 0..8u32 {
        for col in 0..8u32 {
            let is_light = (row + col) % 2 == 0;
            let alpha: f32 = if is_light { 0.06 } else { 0.10 };
            let sq_r: f32 = if is_light { 200.0 } else { 100.0 };
            let sq_g: f32 = if is_light { 190.0 } else { 85.0 };
            let sq_b: f32 = if is_light { 170.0 } else { 65.0 };
            let sx = ox + col * sq;
            let sy = oy + row * sq;
            for py in sy..(sy + sq).min(h) {
                for px in sx..(sx + sq).min(w) {
                    let idx = ((py * w + px) * 4) as usize;
                    let bg_r = pixels[idx] as f32;
                    let bg_g = pixels[idx + 1] as f32;
                    let bg_b = pixels[idx + 2] as f32;
                    pixels[idx] = (bg_r * (1.0 - alpha) + sq_r * alpha) as u8;
                    pixels[idx + 1] = (bg_g * (1.0 - alpha) + sq_g * alpha) as u8;
                    pixels[idx + 2] = (bg_b * (1.0 - alpha) + sq_b * alpha) as u8;
                }
            }
            let piece_alpha = 0.04_f32;
            let piece_center_x = sx + sq / 2;
            let piece_center_y = sy + sq / 2;
            let piece_r = sq * 3 / 8;
            let has_piece =
                !(2..=5).contains(&row) || ((row == 3 || row == 4) && (col == 3 || col == 4));
            if has_piece {
                for py in sy..(sy + sq).min(h) {
                    for px in sx..(sx + sq).min(w) {
                        let dx = px as f32 - piece_center_x as f32;
                        let dy = py as f32 - piece_center_y as f32;
                        let d = (dx * dx + dy * dy).sqrt();
                        if d < piece_r as f32 {
                            let falloff = 1.0 - (d / piece_r as f32);
                            let a = piece_alpha * falloff * falloff;
                            let idx = ((py * w + px) * 4) as usize;
                            let bg_r = pixels[idx] as f32;
                            let bg_g = pixels[idx + 1] as f32;
                            let bg_b = pixels[idx + 2] as f32;
                            pixels[idx] = (bg_r * (1.0 - a) + 200.0 * a) as u8;
                            pixels[idx + 1] = (bg_g * (1.0 - a) + 165.0 * a) as u8;
                            pixels[idx + 2] = (bg_b * (1.0 - a) + 80.0 * a) as u8;
                        }
                    }
                }
            }
        }
    }
    let mut blurred = pixels.clone();
    let radius = 3_i32;
    for y in 0..h {
        for x in 0..w {
            let mut sum_r = 0u32;
            let mut sum_g = 0u32;
            let mut sum_b = 0u32;
            let mut count = 0u32;
            for ky in -radius..=radius {
                for kx in -radius..=radius {
                    let sx = x as i32 + kx;
                    let sy = y as i32 + ky;
                    if sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32 {
                        let si = ((sy as u32 * w + sx as u32) * 4) as usize;
                        sum_r += pixels[si] as u32;
                        sum_g += pixels[si + 1] as u32;
                        sum_b += pixels[si + 2] as u32;
                        count += 1;
                    }
                }
            }
            let idx = ((y * w + x) * 4) as usize;
            blurred[idx] = (sum_r / count) as u8;
            blurred[idx + 1] = (sum_g / count) as u8;
            blurred[idx + 2] = (sum_b / count) as u8;
            blurred[idx + 3] = 255;
        }
    }
    encode_png(&blurred, w, h)
}

fn encode_png(pixels: &[u8], width: u32, height: u32) -> PngImage {
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    image::ImageEncoder::write_image(
        encoder,
        pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .expect("Failed to encode texture");
    PngImage {
        bytes: buf,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_noise_texture_does_not_panic() {
        let img = generate_noise_texture(64, 0.03, 200, 200, 200);
        assert!(!img.bytes.is_empty());
        assert_eq!(img.width, 64);
    }

    #[test]
    fn test_noise_texture_small_size() {
        let _ = generate_noise_texture(1, 0.1, 128, 128, 128);
    }

    #[test]
    fn test_macos_grain_presets() {
        let _ = macos_grain_light();
        let _ = macos_grain_dark();
        let _ = macos_grain_panel();
    }

    #[test]
    fn test_pharaonic_pattern() {
        let _ = pharaonic_pattern(128);
    }

    #[test]
    fn test_chess_blur_background() {
        let img = chess_blur_background(128, 96);
        assert!(img.bytes.len() > 8);
        assert_eq!(&img.bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
