pub mod bitboard;
pub mod board;
pub mod book;
pub mod chess_move;
pub mod piece;
pub mod square;

// Re-export core types for ergonomic use.
pub use bitboard::Bitboard;
pub use board::zobrist;
pub use board::{AkimboPos, Board, BoardSnapshot};
pub use chess_move::{Move, MoveList};
pub use piece::{Color, Piece};
pub use square::Square;

/// Initializes all static tables (attack tables, Zobrist keys).
/// Must be called once at program startup before any Board operations.
pub fn init() {
    board::attack_tables::init_attack_tables();
    let _ = zobrist::zobrist(); // Force Zobrist init
}
