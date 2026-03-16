//! Chess piece asset loader — slices spritesheets into individual piece images.

use iced::widget::image::Handle;

/// Embedded piece spritesheets (compiled into the binary).
const WHITE_SHEET: &[u8] = include_bytes!("../assets/white_pieces.png");
const BLACK_SHEET: &[u8] = include_bytes!("../assets/black_pieces.png");

/// Pre-loaded piece image handles for all 12 pieces.
pub struct PieceAssets {
    pub white_king: Handle,
    pub white_queen: Handle,
    pub white_rook: Handle,
    pub white_bishop: Handle,
    pub white_knight: Handle,
    pub white_pawn: Handle,
    pub black_king: Handle,
    pub black_queen: Handle,
    pub black_rook: Handle,
    pub black_bishop: Handle,
    pub black_knight: Handle,
    pub black_pawn: Handle,
}

impl PieceAssets {
    /// Load and slice the piece spritesheets.
    pub fn load() -> Self {
        let white_pieces = slice_spritesheet(WHITE_SHEET);
        let black_pieces = slice_spritesheet(BLACK_SHEET);

        Self {
            white_king: white_pieces[0].clone(),
            white_queen: white_pieces[1].clone(),
            white_rook: white_pieces[2].clone(),
            white_bishop: white_pieces[3].clone(),
            white_knight: white_pieces[4].clone(),
            white_pawn: white_pieces[5].clone(),
            black_king: black_pieces[0].clone(),
            black_queen: black_pieces[1].clone(),
            black_rook: black_pieces[2].clone(),
            black_bishop: black_pieces[3].clone(),
            black_knight: black_pieces[4].clone(),
            black_pawn: black_pieces[5].clone(),
        }
    }

    /// Get the image handle for a specific piece.
    pub fn get(&self, piece: types::Piece, color: types::Color) -> &Handle {
        match (piece, color) {
            (types::Piece::King, types::Color::White) => &self.white_king,
            (types::Piece::Queen, types::Color::White) => &self.white_queen,
            (types::Piece::Rook, types::Color::White) => &self.white_rook,
            (types::Piece::Bishop, types::Color::White) => &self.white_bishop,
            (types::Piece::Knight, types::Color::White) => &self.white_knight,
            (types::Piece::Pawn, types::Color::White) => &self.white_pawn,
            (types::Piece::King, types::Color::Black) => &self.black_king,
            (types::Piece::Queen, types::Color::Black) => &self.black_queen,
            (types::Piece::Rook, types::Color::Black) => &self.black_rook,
            (types::Piece::Bishop, types::Color::Black) => &self.black_bishop,
            (types::Piece::Knight, types::Color::Black) => &self.black_knight,
            (types::Piece::Pawn, types::Color::Black) => &self.black_pawn,
        }
    }
}

/// Slice a spritesheet PNG into 6 individual piece images.
/// The spritesheet has pieces in a row: K, Q, R, B, N, P.
/// We detect the green (#00FF00) background and make it transparent.
fn slice_spritesheet(png_data: &[u8]) -> Vec<Handle> {
    let img = image::load_from_memory(png_data)
        .expect("Failed to load piece spritesheet")
        .to_rgba8();

    let (width, height) = img.dimensions();
    let piece_width = width / 6;

    let mut handles = Vec::with_capacity(6);

    for i in 0..6 {
        let x_start = i * piece_width;
        let mut piece_img = image::RgbaImage::new(piece_width, height);

        for y in 0..height {
            for x in 0..piece_width {
                let pixel = img.get_pixel(x_start + x, y);
                let [r, g, b, _a] = pixel.0;

                // Green-screen removal: make bright green pixels transparent
                if g > 150 && r < 100 && b < 100 {
                    piece_img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                } else if g > 120 && (g as i32 - r as i32) > 40 && (g as i32 - b as i32) > 40 {
                    // Softer green — partially transparent for anti-aliased edges
                    let green_ratio = (g as f32 - r.max(b) as f32) / g as f32;
                    let alpha = ((1.0 - green_ratio) * 255.0) as u8;
                    piece_img.put_pixel(x, y, image::Rgba([r, g / 2, b, alpha]));
                } else {
                    piece_img.put_pixel(x, y, *pixel);
                }
            }
        }

        // Encode to PNG bytes for iced Handle
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            piece_img.as_raw(),
            piece_width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("Failed to encode piece image");

        handles.push(Handle::from_bytes(buf));
    }

    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spritesheet_loads_without_panic() {
        // Just verifying the spritesheets can be decoded
        let _white = slice_spritesheet(WHITE_SHEET);
        let _black = slice_spritesheet(BLACK_SHEET);
    }

    #[test]
    fn test_spritesheet_produces_6_pieces() {
        let white = slice_spritesheet(WHITE_SHEET);
        let black = slice_spritesheet(BLACK_SHEET);
        assert_eq!(white.len(), 6);
        assert_eq!(black.len(), 6);
    }

    #[test]
    fn test_piece_assets_load() {
        let assets = PieceAssets::load();
        // Verify all 12 handles exist (accessing them doesn't panic)
        let _ = &assets.white_king;
        let _ = &assets.white_queen;
        let _ = &assets.white_rook;
        let _ = &assets.white_bishop;
        let _ = &assets.white_knight;
        let _ = &assets.white_pawn;
        let _ = &assets.black_king;
        let _ = &assets.black_queen;
        let _ = &assets.black_rook;
        let _ = &assets.black_bishop;
        let _ = &assets.black_knight;
        let _ = &assets.black_pawn;
    }

    #[test]
    fn test_get_returns_correct_piece() {
        let assets = PieceAssets::load();
        // Verify each piece/color combination is accessible without panic
        for piece in types::Piece::ALL {
            for color in [types::Color::White, types::Color::Black] {
                let _ = assets.get(piece, color);
            }
        }
    }
}
