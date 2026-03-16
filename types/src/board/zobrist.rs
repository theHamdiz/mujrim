use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::piece::Piece;

/// Zobrist hashing tables for incrementally computing position hashes.
pub struct Zobrist {
    /// piece_keys[color][piece][square]
    pub piece_keys: [[[u64; 64]; 6]; 2],
    /// One key per castling combination (4 bits → 16 possibilities).
    pub castling_keys: [u64; 16],
    /// One key per en-passant file (0-7).
    pub en_passant_keys: [u64; 8],
    /// XOR'd when it's Black's turn.
    pub side_to_move_key: u64,
}

impl Zobrist {
    /// Deterministic initialization with a fixed seed for reproducibility.
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF_CAFE_BABE);

        let mut piece_keys = [[[0u64; 64]; 6]; 2];
        for color in 0..2 {
            for piece in 0..Piece::COUNT {
                for square in 0..64 {
                    piece_keys[color][piece][square] = rng.random();
                }
            }
        }

        let mut castling_keys = [0u64; 16];
        for key in castling_keys.iter_mut() {
            *key = rng.random();
        }

        let mut en_passant_keys = [0u64; 8];
        for key in en_passant_keys.iter_mut() {
            *key = rng.random();
        }

        let side_to_move_key = rng.random();

        Self {
            piece_keys,
            castling_keys,
            en_passant_keys,
            side_to_move_key,
        }
    }
}

impl Default for Zobrist {
    fn default() -> Self {
        Self::new()
    }
}

// Global singleton using OnceLock (safe, no `static mut`)
static ZOBRIST: std::sync::OnceLock<Zobrist> = std::sync::OnceLock::new();

/// Returns a reference to the global Zobrist tables (initialized once).
#[inline(always)]
pub fn zobrist() -> &'static Zobrist {
    ZOBRIST.get_or_init(Zobrist::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zobrist_deterministic() {
        let z1 = Zobrist::new();
        let z2 = Zobrist::new();
        assert_eq!(z1.side_to_move_key, z2.side_to_move_key);
        assert_eq!(z1.piece_keys[0][0][0], z2.piece_keys[0][0][0]);
    }

    #[test]
    fn test_zobrist_singleton() {
        let z1 = zobrist();
        let z2 = zobrist();
        assert!(std::ptr::eq(z1, z2));
    }
}
