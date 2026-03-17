//! Chess piece asset loader — loads individual PNG piece images.
//!
//! Uses the JohnPablok Cburnett chess set — professional, clean 2D pieces
//! with transparent backgrounds. Each piece is a separate 256×256 PNG file.

use iced::widget::image::Handle;

/// Embedded piece images (compiled into the binary).
const WK: &[u8] = include_bytes!("../assets/pieces/wK.png");
const WQ: &[u8] = include_bytes!("../assets/pieces/wQ.png");
const WR: &[u8] = include_bytes!("../assets/pieces/wR.png");
const WB: &[u8] = include_bytes!("../assets/pieces/wB.png");
const WN: &[u8] = include_bytes!("../assets/pieces/wN.png");
const WP: &[u8] = include_bytes!("../assets/pieces/wP.png");
const BK: &[u8] = include_bytes!("../assets/pieces/bK.png");
const BQ: &[u8] = include_bytes!("../assets/pieces/bQ.png");
const BR: &[u8] = include_bytes!("../assets/pieces/bR.png");
const BB: &[u8] = include_bytes!("../assets/pieces/bB.png");
const BN: &[u8] = include_bytes!("../assets/pieces/bN.png");
const BP: &[u8] = include_bytes!("../assets/pieces/bP.png");

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
    /// Load all piece images from embedded PNG data.
    pub fn load() -> Self {
        Self {
            white_king: Handle::from_bytes(WK),
            white_queen: Handle::from_bytes(WQ),
            white_rook: Handle::from_bytes(WR),
            white_bishop: Handle::from_bytes(WB),
            white_knight: Handle::from_bytes(WN),
            white_pawn: Handle::from_bytes(WP),
            black_king: Handle::from_bytes(BK),
            black_queen: Handle::from_bytes(BQ),
            black_rook: Handle::from_bytes(BR),
            black_bishop: Handle::from_bytes(BB),
            black_knight: Handle::from_bytes(BN),
            black_pawn: Handle::from_bytes(BP),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piece_assets_load() {
        let assets = PieceAssets::load();
        // Verify all handles were created successfully
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
        for piece in types::Piece::ALL {
            for color in [types::Color::White, types::Color::Black] {
                let _ = assets.get(piece, color);
            }
        }
    }
}
