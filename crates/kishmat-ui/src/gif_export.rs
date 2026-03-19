//! GIF export — renders a chess game from its PGN move log into an animated GIF.
//!
//! Each position in the game is rendered as a frame with the full board,
//! using the embedded PNG piece assets scaled to fit.
//!
//! Also exposes [`quantize_frame`] for use by the recording module.

use image::{Rgba, RgbaImage, imageops};

/// Board rendering size for GIF frames.
const GIF_BOARD_SIZE: u32 = 480;
const GIF_SQ_SIZE: u32 = GIF_BOARD_SIZE / 8; // 60px per square

// Square colors (matching board_view.rs)
const SQ_LIGHT_RGBA: Rgba<u8> = Rgba([240, 217, 181, 255]); // #F0D9B5
const SQ_DARK_RGBA: Rgba<u8> = Rgba([181, 136, 99, 255]); // #B58863

/// Pre-decoded piece images for GIF rendering.
struct GifPieceAssets {
    white_king: RgbaImage,
    white_queen: RgbaImage,
    white_rook: RgbaImage,
    white_bishop: RgbaImage,
    white_knight: RgbaImage,
    white_pawn: RgbaImage,
    black_king: RgbaImage,
    black_queen: RgbaImage,
    black_rook: RgbaImage,
    black_bishop: RgbaImage,
    black_knight: RgbaImage,
    black_pawn: RgbaImage,
}

impl GifPieceAssets {
    fn load() -> Self {
        fn decode_and_resize(bytes: &[u8]) -> RgbaImage {
            let piece_size = (GIF_SQ_SIZE as f32 * 0.82) as u32; // ~82% of square, slight padding
            let img = image::load_from_memory(bytes)
                .expect("Failed to decode piece PNG")
                .to_rgba8();
            imageops::resize(&img, piece_size, piece_size, imageops::FilterType::Lanczos3)
        }

        Self {
            white_king: decode_and_resize(include_bytes!("../assets/pieces/wK.png")),
            white_queen: decode_and_resize(include_bytes!("../assets/pieces/wQ.png")),
            white_rook: decode_and_resize(include_bytes!("../assets/pieces/wR.png")),
            white_bishop: decode_and_resize(include_bytes!("../assets/pieces/wB.png")),
            white_knight: decode_and_resize(include_bytes!("../assets/pieces/wN.png")),
            white_pawn: decode_and_resize(include_bytes!("../assets/pieces/wP.png")),
            black_king: decode_and_resize(include_bytes!("../assets/pieces/bK.png")),
            black_queen: decode_and_resize(include_bytes!("../assets/pieces/bQ.png")),
            black_rook: decode_and_resize(include_bytes!("../assets/pieces/bR.png")),
            black_bishop: decode_and_resize(include_bytes!("../assets/pieces/bB.png")),
            black_knight: decode_and_resize(include_bytes!("../assets/pieces/bN.png")),
            black_pawn: decode_and_resize(include_bytes!("../assets/pieces/bP.png")),
        }
    }

    fn get(&self, piece: types::Piece, color: types::Color) -> &RgbaImage {
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

/// Render a board position as an RGBA image.
fn render_board(board: &types::Board, assets: &GifPieceAssets) -> RgbaImage {
    let mut img = RgbaImage::new(GIF_BOARD_SIZE, GIF_BOARD_SIZE);

    // Draw squares and pieces
    for rank in 0..8u32 {
        for file in 0..8u32 {
            let sq_idx = rank * 8 + file;
            let sq = types::Square::from_index(sq_idx as usize);

            // Screen coordinates (rank 0 = bottom of board = row 7 on screen)
            let screen_row = 7 - rank;
            let px = file * GIF_SQ_SIZE;
            let py = screen_row * GIF_SQ_SIZE;

            // Square color
            let is_light = (rank + file) % 2 != 0;
            let sq_color = if is_light {
                SQ_LIGHT_RGBA
            } else {
                SQ_DARK_RGBA
            };

            // Fill square
            for dy in 0..GIF_SQ_SIZE {
                for dx in 0..GIF_SQ_SIZE {
                    img.put_pixel(px + dx, py + dy, sq_color);
                }
            }

            // Draw piece if present
            if let Some((piece, color)) = board.piece_on(sq) {
                let piece_img = assets.get(piece, color);
                let pw = piece_img.width();
                let ph = piece_img.height();
                let ox = (GIF_SQ_SIZE - pw) / 2; // center horizontally
                let oy = (GIF_SQ_SIZE - ph) / 2; // center vertically
                // Alpha-composite piece onto square
                for dy in 0..ph {
                    for dx in 0..pw {
                        let src = piece_img.get_pixel(dx, dy);
                        if src[3] > 0 {
                            let dest_x = px + ox + dx;
                            let dest_y = py + oy + dy;
                            let dst = img.get_pixel(dest_x, dest_y);
                            let alpha = src[3] as f32 / 255.0;
                            let inv_alpha = 1.0 - alpha;
                            let r = (src[0] as f32 * alpha + dst[0] as f32 * inv_alpha) as u8;
                            let g = (src[1] as f32 * alpha + dst[1] as f32 * inv_alpha) as u8;
                            let b = (src[2] as f32 * alpha + dst[2] as f32 * inv_alpha) as u8;
                            img.put_pixel(dest_x, dest_y, Rgba([r, g, b, 255]));
                        }
                    }
                }
            }
        }
    }

    img
}

/// Quantize an RGBA image to a 256-color palette and return (palette, indices).
/// Public alias for use by the recording module.
pub fn quantize_frame(img: &RgbaImage) -> (Vec<u8>, Vec<u8>) {
    quantize_image(img)
}

/// Quantize an RGBA image to a 256-color palette and return (palette, indices).
fn quantize_image(img: &RgbaImage) -> (Vec<u8>, Vec<u8>) {
    // Simple median-cut-like quantization: collect unique colors, map to 256 palette
    use std::collections::HashMap;

    let mut color_counts: HashMap<(u8, u8, u8), u32> = HashMap::new();
    for pixel in img.pixels() {
        let key = (pixel[0], pixel[1], pixel[2]);
        *color_counts.entry(key).or_insert(0) += 1;
    }

    // Sort by frequency, take top 255 colors + 1 for transparent
    let mut colors: Vec<((u8, u8, u8), u32)> = color_counts.into_iter().collect();
    colors.sort_by(|a, b| b.1.cmp(&a.1));
    colors.truncate(256);

    // Build palette (flat RGB bytes)
    let mut palette = Vec::with_capacity(256 * 3);
    let mut color_to_idx: HashMap<(u8, u8, u8), u8> = HashMap::new();

    for (i, &(color, _)) in colors.iter().enumerate() {
        palette.push(color.0);
        palette.push(color.1);
        palette.push(color.2);
        color_to_idx.insert(color, i as u8);
    }

    // Pad palette to exactly 256 entries
    while palette.len() < 256 * 3 {
        palette.push(0);
    }

    // Map pixels to indices (nearest color for unmapped pixels)
    let mut indices = Vec::with_capacity((img.width() * img.height()) as usize);
    for pixel in img.pixels() {
        let key = (pixel[0], pixel[1], pixel[2]);
        if let Some(&idx) = color_to_idx.get(&key) {
            indices.push(idx);
        } else {
            // Find nearest color in palette
            let mut best_idx = 0u8;
            let mut best_dist = u32::MAX;
            for (i, chunk) in palette.chunks(3).enumerate().take(colors.len()) {
                let dr = pixel[0] as i32 - chunk[0] as i32;
                let dg = pixel[1] as i32 - chunk[1] as i32;
                let db = pixel[2] as i32 - chunk[2] as i32;
                let dist = (dr * dr + dg * dg + db * db) as u32;
                if dist < best_dist {
                    best_dist = dist;
                    best_idx = i as u8;
                }
            }
            indices.push(best_idx);
        }
    }

    (palette, indices)
}

/// Export the game as an animated GIF.
///
/// Takes the move log (UCI format moves like "e2e4"), replays them from
/// the start position, renders each position, and encodes as animated GIF.
///
/// Returns the GIF bytes.
pub fn export_game_gif(move_log: &[String], delay_cs: u16) -> Vec<u8> {
    types::init();

    let assets = GifPieceAssets::load();
    let mut board = types::Board::new();

    // Collect all positions (starting position + after each move)
    let mut positions: Vec<RgbaImage> = Vec::with_capacity(move_log.len() + 1);
    positions.push(render_board(&board, &assets));

    for uci_move in move_log {
        // Parse UCI move string (e.g., "e2e4")
        let legal = board.generate_legal_moves();
        if let Some(mv) = parse_uci_move(uci_move, &legal) {
            board.make_move(mv);
            positions.push(render_board(&board, &assets));
        }
    }

    // Encode as animated GIF
    let mut gif_bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(
            &mut gif_bytes,
            GIF_BOARD_SIZE as u16,
            GIF_BOARD_SIZE as u16,
            &[],
        )
        .expect("Failed to create GIF encoder");

        encoder
            .set_repeat(gif::Repeat::Infinite)
            .expect("Failed to set repeat");

        for frame_img in &positions {
            let (palette, indices) = quantize_image(frame_img);

            let mut frame = gif::Frame::default();
            frame.width = GIF_BOARD_SIZE as u16;
            frame.height = GIF_BOARD_SIZE as u16;
            frame.delay = delay_cs; // delay in centiseconds
            frame.palette = Some(palette);
            frame.buffer = std::borrow::Cow::Borrowed(&indices);

            encoder
                .write_frame(&frame)
                .expect("Failed to write GIF frame");
        }
    }

    gif_bytes
}

/// Parse a UCI move string (e.g., "e2e4", "e7e8q") against legal moves.
fn parse_uci_move(uci: &str, legal: &types::MoveList) -> Option<types::Move> {
    let uci = uci.trim();
    if uci.len() < 4 {
        return None;
    }

    let from_file = uci.as_bytes()[0] as usize - b'a' as usize;
    let from_rank = uci.as_bytes()[1] as usize - b'1' as usize;
    let to_file = uci.as_bytes()[2] as usize - b'a' as usize;
    let to_rank = uci.as_bytes()[3] as usize - b'1' as usize;

    if from_file >= 8 || from_rank >= 8 || to_file >= 8 || to_rank >= 8 {
        return None;
    }

    let from_sq = types::Square::from_index(from_rank * 8 + from_file);
    let to_sq = types::Square::from_index(to_rank * 8 + to_file);

    legal
        .iter()
        .find(|m| m.from == from_sq && m.to == to_sq)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_starting_position() {
        types::init();
        let assets = GifPieceAssets::load();
        let board = types::Board::new();
        let img = render_board(&board, &assets);
        assert_eq!(img.width(), GIF_BOARD_SIZE);
        assert_eq!(img.height(), GIF_BOARD_SIZE);
    }

    #[test]
    fn test_export_empty_game() {
        let gif = export_game_gif(&[], 100);
        // Should produce a valid GIF with at least a header
        assert!(gif.len() > 10);
        assert_eq!(&gif[0..3], b"GIF");
    }

    #[test]
    fn test_export_short_game() {
        let moves = vec![
            "e2e4".to_string(),
            "e7e5".to_string(),
            "g1f3".to_string(),
            "b8c6".to_string(),
        ];
        let gif = export_game_gif(&moves, 100);
        assert!(gif.len() > 100);
        assert_eq!(&gif[0..3], b"GIF");
    }

    #[test]
    fn test_quantize_image() {
        let img = RgbaImage::from_fn(8, 8, |x, y| {
            if (x + y) % 2 == 0 {
                Rgba([240, 217, 181, 255])
            } else {
                Rgba([181, 136, 99, 255])
            }
        });
        let (palette, indices) = quantize_image(&img);
        assert_eq!(palette.len(), 256 * 3);
        assert_eq!(indices.len(), 64);
    }
}
