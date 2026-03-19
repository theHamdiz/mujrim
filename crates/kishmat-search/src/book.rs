//! Polyglot opening book reader.
//!
//! Reads a `.bin` polyglot book file and probes it for moves
//! based on the current board position's hash.
//!
//! The polyglot format:
//! - Each entry is 16 bytes: 8 bytes key, 2 bytes move, 2 bytes weight, 4 bytes learn
//! - Entries are sorted by key for binary search
//! - The key is a Zobrist hash computed with polyglot's own hashing scheme

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

/// A single entry from a polyglot opening book.
#[derive(Clone, Copy, Debug)]
struct BookEntry {
    key: u64,
    raw_move: u16,
    weight: u16,
}

/// Polyglot opening book.
pub struct OpeningBook {
    entries: Vec<BookEntry>,
}

impl OpeningBook {
    /// Loads a polyglot book from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() % 16 != 0 {
            return Err("Invalid polyglot book: size not multiple of 16".to_string());
        }
        let count = data.len() / 16;
        let mut entries = Vec::with_capacity(count);

        for i in 0..count {
            let offset = i * 16;
            let key = u64::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let raw_move = u16::from_be_bytes([data[offset + 8], data[offset + 9]]);
            let weight = u16::from_be_bytes([data[offset + 10], data[offset + 11]]);
            // learn field at offset+12..16 is ignored

            entries.push(BookEntry {
                key,
                raw_move,
                weight,
            });
        }

        Ok(Self { entries })
    }

    /// Loads the embedded opening book (compiled into the binary).
    pub fn load_embedded() -> Result<Self, String> {
        let data = include_bytes!("../book/book.bin");
        if data.is_empty() {
            return Err("Embedded book is empty".to_string());
        }
        Self::from_bytes(data)
    }

    /// Probes the book for a move in the given position.
    /// Uses polyglot hashing to find the position, then picks
    /// a weighted random move from all matching entries.
    pub fn probe(&self, board: &Board) -> Option<Move> {
        let key = polyglot_hash(board);
        let matches = self.find_entries(key);
        if matches.is_empty() {
            return None;
        }

        // Pick the highest-weight move (deterministic, strongest line)
        let best = matches.iter().max_by_key(|e| e.weight)?;
        decode_polyglot_move(board, best.raw_move)
    }

    /// Binary search to find all entries matching the key.
    fn find_entries(&self, key: u64) -> Vec<BookEntry> {
        let mut result = Vec::new();

        // Binary search for first match
        let idx = match self.entries.binary_search_by_key(&key, |e| e.key) {
            Ok(i) => i,
            Err(_) => return result,
        };

        // Scan backwards for more matches
        let mut i = idx;
        while i > 0 && self.entries[i - 1].key == key {
            i -= 1;
        }

        // Collect all matches
        while i < self.entries.len() && self.entries[i].key == key {
            result.push(self.entries[i]);
            i += 1;
        }

        result
    }
}

/// Decode a polyglot raw move into a Move struct.
/// Polyglot encoding: bits 0-5: to_file*8 + to_rank (but reversed)
///   bits 0-2: to file, bits 3-5: to rank, bits 6-8: from file, bits 9-11: from rank
///   bits 12-14: promotion piece (0=none, 1=knight, 2=bishop, 3=rook, 4=queen)
fn decode_polyglot_move(board: &Board, raw: u16) -> Option<Move> {
    let mut board_clone = board.clone();
    let to_file = (raw & 0x07) as usize;
    let to_rank = ((raw >> 3) & 0x07) as usize;
    let from_file = ((raw >> 6) & 0x07) as usize;
    let from_rank = ((raw >> 9) & 0x07) as usize;
    let promotion = ((raw >> 12) & 0x07) as usize;

    let from_sq = Square::from_file_rank(from_file as u8, from_rank as u8);
    let to_sq = Square::from_file_rank(to_file as u8, to_rank as u8);

    // Handle castling: polyglot encodes castling as king captures rook
    if let Some((Piece::King, _color)) = board.piece_on(from_sq) {
        let from_f = from_file;
        let to_f = to_file;

        // Kingside castling
        if from_f == 4 && to_f == 7 {
            let castle_to = Square::from_file_rank(6, from_rank as u8);
            let legal = board_clone.generate_legal_moves();
            return legal
                .iter()
                .find(|m| m.from == from_sq && m.to == castle_to && m.flag == MoveFlag::KingCastle)
                .copied();
        }
        // Queenside castling
        if from_f == 4 && to_f == 0 {
            let castle_to = Square::from_file_rank(2, from_rank as u8);
            let legal = board_clone.generate_legal_moves();
            return legal
                .iter()
                .find(|m| m.from == from_sq && m.to == castle_to && m.flag == MoveFlag::QueenCastle)
                .copied();
        }
    }

    // Handle promotion
    if promotion > 0 {
        let promo_piece = match promotion {
            1 => Piece::Knight,
            2 => Piece::Bishop,
            3 => Piece::Rook,
            4 => Piece::Queen,
            _ => Piece::Queen,
        };
        let legal = board_clone.generate_legal_moves();
        return legal
            .iter()
            .find(|m| m.from == from_sq && m.to == to_sq && m.promotion == Some(promo_piece))
            .copied();
    }

    // Normal move — match against legal moves
    let legal = board_clone.generate_legal_moves();
    legal
        .iter()
        .find(|m| m.from == from_sq && m.to == to_sq)
        .copied()
}

// ── Polyglot Zobrist hashing ────────────────────────────────────────────────
// Polyglot uses its own Zobrist keys, different from the engine's internal ones.

/// Compute the polyglot hash for a board position.
fn polyglot_hash(board: &Board) -> u64 {
    let mut hash = 0u64;

    // Pieces
    for sq_idx in 0..64 {
        let sq = Square::from_index(sq_idx);
        if let Some((piece, color)) = board.piece_on(sq) {
            let poly_piece = polyglot_piece_index(piece, color);
            // Polyglot squares: rank*8+file (a1=0, b1=1, ..., h8=63)
            let poly_sq = sq_idx;
            hash ^= POLY_RANDOM[64 * poly_piece + poly_sq];
        }
    }

    // Castling
    if board.castling_rights & types::board::WHITE_KING_CASTLE != 0 {
        hash ^= POLY_RANDOM[768];
    }
    if board.castling_rights & types::board::WHITE_QUEEN_CASTLE != 0 {
        hash ^= POLY_RANDOM[769];
    }
    if board.castling_rights & types::board::BLACK_KING_CASTLE != 0 {
        hash ^= POLY_RANDOM[770];
    }
    if board.castling_rights & types::board::BLACK_QUEEN_CASTLE != 0 {
        hash ^= POLY_RANDOM[771];
    }

    // En passant (only if there's actually a pawn that can capture)
    if let Some(ep_sq) = board.en_passant {
        let ep_file = ep_sq.file() as usize;
        // Verify there's a pawn that can capture en passant
        let has_ep_capture = match board.side_to_move {
            Color::White => {
                let ep_idx = ep_sq.index();
                let pawns = board.piece_bb(Piece::Pawn, Color::White);
                (ep_idx > 0 && ep_idx % 8 > 0 && pawns & (1u64 << (ep_idx - 9)) != 0)
                    || (ep_idx % 8 < 7 && pawns & (1u64 << (ep_idx - 7)) != 0)
            }
            Color::Black => {
                let ep_idx = ep_sq.index();
                let pawns = board.piece_bb(Piece::Pawn, Color::Black);
                (ep_idx < 56 && ep_idx % 8 > 0 && pawns & (1u64 << (ep_idx + 7)) != 0)
                    || (ep_idx < 56 && ep_idx % 8 < 7 && pawns & (1u64 << (ep_idx + 9)) != 0)
            }
        };
        if has_ep_capture {
            hash ^= POLY_RANDOM[772 + ep_file];
        }
    }

    // Side to move
    if board.side_to_move == Color::White {
        hash ^= POLY_RANDOM[780];
    }

    hash
}

/// Maps Piece + Color to polyglot piece index.
/// Polyglot order: BlackPawn=0, WhitePawn=1, BlackKnight=2, WhiteKnight=3, ...
fn polyglot_piece_index(piece: Piece, color: Color) -> usize {
    let kind = match piece {
        Piece::Pawn => 0,
        Piece::Knight => 1,
        Piece::Bishop => 2,
        Piece::Rook => 3,
        Piece::Queen => 4,
        Piece::King => 5,
    };
    kind * 2 + if color == Color::White { 1 } else { 0 }
}

/// The 781 polyglot random keys (from PolyGlot source).
/// 64 squares * 12 piece types = 768 piece keys
/// + 4 castling keys + 8 en passant files + 1 side to move = 781 total
const POLY_RANDOM: [u64; 781] = include!("polyglot_keys.inc");

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        types::init();
    }

    #[test]
    fn test_polyglot_hash_startpos() {
        setup();
        let board = Board::new();
        let hash = polyglot_hash(&board);
        // Known polyglot hash for starting position
        assert_eq!(
            hash, 0x463b96181691fc9c,
            "Polyglot hash mismatch for startpos"
        );
    }

    #[test]
    fn test_decode_move_basic() {
        setup();
        let board = Board::new();
        // e2e4 in polyglot: from=(1,4) to=(3,4) → raw = (1<<9)|(4<<6)|(3<<3)|4
        let raw = (1u16 << 9) | (4u16 << 6) | (3u16 << 3) | 4u16;
        let mv = decode_polyglot_move(&board, raw);
        assert!(mv.is_some(), "Should decode e2e4");
        let mv = mv.unwrap();
        assert_eq!(mv.to_uci(), "e2e4");
    }

    #[test]
    fn test_empty_book() {
        setup();
        let book = OpeningBook::from_bytes(&[]).unwrap();
        let board = Board::new();
        assert!(book.probe(&board).is_none());
    }
}
