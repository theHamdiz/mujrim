//! NNUE Feature Index Calculation for KishMat's NNCorrL architecture.
//!
//! 768 inputs: piece_type (0..5) × 2 relative_colors × 64 squares.
//! Per-perspective encoding: friendly pieces use indices 0-383,
//! enemy pieces use indices 384-767.

use super::network::L1_SIZE;

/// Accumulator storing hidden layer values for both perspectives.
#[derive(Clone)]
pub struct Accumulator {
    /// White perspective hidden values.
    pub white: [i16; L1_SIZE],
    /// Black perspective hidden values.
    pub black: [i16; L1_SIZE],
}

impl Default for Accumulator {
    fn default() -> Self {
        Self {
            white: [0i16; L1_SIZE],
            black: [0i16; L1_SIZE],
        }
    }
}

/// Compute feature index for HalfKP-like encoding.
///
/// For "perspective" relative encoding:
///   index = relative_color * 384 + piece_type * 64 + relative_square
///   where relative_square is flipped for the black perspective.
#[inline]
pub fn feature_index(piece_type: usize, piece_color: usize, square: usize, perspective: usize) -> usize {
    let relative_color = if piece_color == perspective { 0 } else { 1 };
    let relative_square = if perspective == 0 {
        square
    } else {
        square ^ 56 // Flip rank for black perspective
    };
    relative_color * 384 + piece_type * 64 + relative_square
}
