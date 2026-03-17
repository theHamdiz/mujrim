//! NNUE Accumulator — manages the hidden layer state during search.
//!
//! The accumulator stores the result of applying the feature transformer
//! to the current position. It is updated incrementally when pieces move
//! and refreshed from scratch when the king changes bucket.

use super::network::{Accumulator, HIDDEN, NUM_BUCKETS, net, get_base_index, get_bucket};

/// Board bitmask state for accumulator cache validation.
/// Tracks which piece configurations have been computed.
pub struct EvalEntry {
    pub bbs: [u64; 8],
    pub white: Accumulator,
    pub black: Accumulator,
}

/// Accumulator cache indexed by [white_king_bucket][black_king_bucket].
/// This avoids recomputing the accumulator from scratch when the king
/// hasn't changed bucket — we can just look up the cached state.
pub struct EvalTable {
    pub table: Box<[[EvalEntry; 2 * NUM_BUCKETS]; 2 * NUM_BUCKETS]>,
}

impl Default for EvalTable {
    fn default() -> Self {
        // Allocate zeroed memory and fill with default accumulators
        let mut table: Box<[[EvalEntry; 2 * NUM_BUCKETS]; 2 * NUM_BUCKETS]> =
            unsafe { boxed_and_zeroed() };

        for row in table.iter_mut() {
            for entry in row.iter_mut() {
                entry.white = Accumulator::default();
                entry.black = Accumulator::default();
            }
        }

        Self { table }
    }
}

/// NNUE state for use in the search. Wraps the accumulator table.
pub struct NNUEState {
    pub table: EvalTable,
}

impl NNUEState {
    pub fn new() -> Self {
        Self {
            table: EvalTable::default(),
        }
    }

    /// Fully recompute the accumulators from a board position.
    /// Uses the Board's bitboard interface.
    pub fn reinit_from(&mut self, board: &types::Board) {
        use types::{Color, Piece};

        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();

        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);

        let entry = &mut self.table.table[wb][bb];

        // Reset to bias
        entry.white = Accumulator::default();
        entry.black = Accumulator::default();

        // For each piece on the board, add its feature
        let pieces = [Piece::Pawn, Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen, Piece::King];
        let colors = [Color::White, Color::Black];

        let net = net();

        for (pc_idx, &piece) in pieces.iter().enumerate() {
            for (side_idx, &color) in colors.iter().enumerate() {
                let mut bb_pieces = board.piece_bb(piece, color);
                while bb_pieces != 0 {
                    let sq = bb_pieces.trailing_zeros() as usize;
                    bb_pieces &= bb_pieces - 1;

                    // White perspective feature
                    let w_base = get_base_index::<0>(side_idx, pc_idx, w_king);
                    let w_sq = sq; // No flip for white perspective
                    let w_sq = if w_king % 8 > 3 { w_sq ^ 7 } else { w_sq };
                    let w_feat = w_base + w_sq;
                    add_feature(&mut entry.white, &net.feature_weights[w_feat]);

                    // Black perspective feature
                    let b_base = get_base_index::<1>(side_idx, pc_idx, b_king);
                    let b_sq = sq ^ 56; // Rank flip for black perspective
                    let b_sq = if (b_king ^ 56) % 8 > 3 { b_sq ^ 7 } else { b_sq };
                    let b_feat = b_base + b_sq;
                    add_feature(&mut entry.black, &net.feature_weights[b_feat]);
                }
            }
        }

        // Store bitboard state for cache validation
        entry.bbs = [0u64; 8];
    }

    /// Get the current accumulator entry for the given king positions.
    pub fn get_entry(&self, w_king: usize, b_king: usize) -> &EvalEntry {
        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);
        &self.table.table[wb][bb]
    }
}

/// Add a feature's weights to an accumulator (non-SIMD for clarity).
#[inline]
fn add_feature(acc: &mut Accumulator, weights: &Accumulator) {
    for i in 0..HIDDEN {
        acc.vals[i] += weights.vals[i];
    }
}

/// Allocate a boxed, zeroed value of any type.
/// # Safety
/// Type must be valid when all bytes are zero or must be overwritten before use.
unsafe fn boxed_and_zeroed<T>() -> Box<T> {
    unsafe {
        let layout = std::alloc::Layout::new::<T>();
        let ptr = std::alloc::alloc_zeroed(layout) as *mut T;
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Board;

    #[test]
    fn test_reinit_no_panic() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        state.reinit_from(&board);
    }

    #[test]
    fn test_accumulator_not_all_zeros_after_reinit() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        state.reinit_from(&board);

        let w_king = board.king_square(types::Color::White).index();
        let b_king = board.king_square(types::Color::Black).index();
        let entry = state.get_entry(w_king, b_king);

        // After adding piece features, values should not all be zero
        let sum: i64 = entry.white.vals.iter().map(|&v| v as i64).sum();
        assert!(sum != 0, "White accumulator is all zeros after reinit");
    }
}
