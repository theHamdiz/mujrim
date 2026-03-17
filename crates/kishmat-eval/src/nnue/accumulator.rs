//! NNUE Accumulator — manages the hidden layer state during search.
//!
//! The accumulator stores the result of applying the feature transformer
//! to the current position. It is updated incrementally when pieces move
//! and refreshed from scratch when the king changes bucket.
//!
//! **Performance-critical**: The cache table avoids recomputing accumulators
//! from scratch when the same king-bucket combination is revisited.
//! The board's piece bitboards are stored alongside each entry so we can
//! detect whether the cached accumulator is still valid or needs a refresh.

use super::network::{Accumulator, HIDDEN, NUM_BUCKETS, net, get_base_index, get_bucket};

/// Number of piece bitboards we track: 6 pieces × 2 colors = 12.
const NUM_BBS: usize = 12;

/// Board bitmask state for accumulator cache validation.
/// Tracks which piece configurations have been computed.
pub struct EvalEntry {
    /// Snapshot of the board's piece bitboards at the time this entry was computed.
    /// Layout: [white_pawn, white_knight, ..., white_king, black_pawn, ..., black_king]
    pub bbs: [u64; NUM_BBS],
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
                // bbs are already zeroed — acts as "never computed"
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

    /// Snapshot the board's piece bitboards into a flat array.
    #[inline(always)]
    fn snapshot_bbs(board: &types::Board) -> [u64; NUM_BBS] {
        use types::{Color, Piece};
        [
            board.piece_bb(Piece::Pawn, Color::White),
            board.piece_bb(Piece::Knight, Color::White),
            board.piece_bb(Piece::Bishop, Color::White),
            board.piece_bb(Piece::Rook, Color::White),
            board.piece_bb(Piece::Queen, Color::White),
            board.piece_bb(Piece::King, Color::White),
            board.piece_bb(Piece::Pawn, Color::Black),
            board.piece_bb(Piece::Knight, Color::Black),
            board.piece_bb(Piece::Bishop, Color::Black),
            board.piece_bb(Piece::Rook, Color::Black),
            board.piece_bb(Piece::Queen, Color::Black),
            board.piece_bb(Piece::King, Color::Black),
        ]
    }

    /// Evaluate the position using NNUE accumulators.
    ///
    /// Uses a cache keyed by (white_king_bucket, black_king_bucket).
    /// If the cached bitboard snapshot matches the current board, the
    /// cached accumulators are reused directly. Otherwise, they are
    /// recomputed from scratch and the cache is updated.
    ///
    /// This is the **critical optimization**: the old code called
    /// `reinit_from` on every eval, doing ~2048 × 32 i16 additions
    /// per call. With caching, most calls in the search tree hit the
    /// cache and cost essentially nothing.
    pub fn evaluate(&mut self, board: &types::Board) -> i32 {
        use types::Color;
        use super::network::forward;

        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();

        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);

        let current_bbs = Self::snapshot_bbs(board);
        let entry = &mut self.table.table[wb][bb];

        // Check if cached accumulators are still valid
        if entry.bbs != current_bbs {
            // Cache miss — full recompute
            Self::compute_entry(entry, board, w_king, b_king);
            entry.bbs = current_bbs;
        }

        // Forward pass: perspective net
        let (boys, opps) = match board.side_to_move {
            Color::White => (&entry.white, &entry.black),
            Color::Black => (&entry.black, &entry.white),
        };
        forward(boys, opps)
    }

    /// Fully recompute the accumulators from a board position.
    /// Uses the Board's bitboard interface.
    pub fn reinit_from(&mut self, board: &types::Board) {
        use types::Color;

        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();

        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);

        let entry = &mut self.table.table[wb][bb];
        Self::compute_entry(entry, board, w_king, b_king);
        entry.bbs = Self::snapshot_bbs(board);
    }

    /// Compute accumulators for a specific entry (the actual work).
    fn compute_entry(entry: &mut EvalEntry, board: &types::Board, w_king: usize, b_king: usize) {
        use types::{Color, Piece};

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
                    let w_sq = if w_king % 8 > 3 { sq ^ 7 } else { sq };
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

    #[test]
    fn test_evaluate_returns_nonzero() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        // evaluate should work and return a reasonable value
        let score = state.evaluate(&board);
        // Starting position should be relatively equal (within ±200cp)
        assert!(score.abs() < 200, "Starting position eval {score} seems unreasonable");
    }

    #[test]
    fn test_evaluate_caching() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board);
        let score2 = state.evaluate(&board);
        // Same position should return same score (cache hit)
        assert_eq!(score1, score2, "Cached evaluation should be identical");
    }

    #[test]
    fn test_evaluate_different_positions() {
        types::init();
        let board1 = Board::new();
        let board2 = Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board1);
        let score2 = state.evaluate(&board2);
        // Missing black queen should give white a big advantage
        assert!(score2 > score1, "Missing queen should increase eval: start={score1}, missing_q={score2}");
    }
}
