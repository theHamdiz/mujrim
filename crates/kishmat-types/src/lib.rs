pub mod square;
pub mod piece;
pub mod bitboard;
pub mod chess_move;
pub mod board;

// Re-export core types for ergonomic use.
pub use square::Square;
pub use piece::{Color, Piece};
pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveList};
pub use board::Board;
pub use board::zobrist;

/// Initializes all static tables (attack tables, Zobrist keys).
/// Must be called once at program startup before any Board operations.
pub fn init() {
    board::attack_tables::init_attack_tables();
    let _ = zobrist::zobrist(); // Force Zobrist init
}