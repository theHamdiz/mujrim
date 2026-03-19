//! NNUE Accumulator — manages the hidden layer state during search.
//!
//! The accumulator stores the result of applying the feature transformer
//! to the current position. It is updated incrementally when pieces move
//! and refreshed from scratch when the king changes bucket.
//!
//! **Key optimization**: Diff-based incremental updates (à la Akimbo) —
//! instead of recomputing all 32 piece features from scratch, we only
//! add/subtract the features that changed between the cached and current
//! board state. With typical moves changing 2-4 piece bitboard entries,
//! this is ~10x faster than full recompute.

use super::adapter::{ActiveNetwork, NnueNetworkInfo, NnueNetworkSource};
use super::network::{
    Accumulator, NUM_BUCKETS, Network, forward_with_network, get_base_index, get_bucket,
};
use std::sync::Arc;
use types::{Board, Color, Piece};

/// Number of bitboards we track for cache comparison.
/// Layout: [white_occ, black_occ, pawn..king×2] = 14 total.
/// But simpler: use 12 per-piece bitboards like before.
const NUM_BBS: usize = 12;

/// SEE piece values used for material scaling (matching Akimbo).
const SEE_VALS: [i32; 6] = [100, 450, 450, 650, 1250, 0];

/// Board bitmask state for accumulator cache validation.
pub struct EvalEntry {
    /// Snapshot of the board's 12 piece bitboards (pieces[2][6]).
    pub bbs: [u64; NUM_BBS],
    pub white: Accumulator,
    pub black: Accumulator,
}

/// Accumulator cache indexed by [white_king_bucket][black_king_bucket].
pub struct EvalTable {
    pub table: Box<[[EvalEntry; 2 * NUM_BUCKETS]; 2 * NUM_BUCKETS]>,
}

impl Default for EvalTable {
    fn default() -> Self {
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

/// NNUE state for use in the search.
pub struct NNUEState {
    pub table: EvalTable,
    source: Arc<dyn NnueNetworkSource + Send + Sync>,
}

impl NNUEState {
    pub fn new() -> Self {
        Self::with_network(Arc::new(ActiveNetwork::Embedded))
    }

    pub fn with_network(source: Arc<dyn NnueNetworkSource + Send + Sync>) -> Self {
        Self {
            table: EvalTable::default(),
            source,
        }
    }

    pub fn network_info(&self) -> NnueNetworkInfo {
        self.source.info()
    }

    /// Snapshot the board's piece bitboards: [white_pawn..white_king, black_pawn..black_king].
    #[inline(always)]
    fn snapshot_bbs(board: &Board) -> [u64; NUM_BBS] {
        [
            board.pieces[0][0],
            board.pieces[0][1],
            board.pieces[0][2],
            board.pieces[0][3],
            board.pieces[0][4],
            board.pieces[0][5],
            board.pieces[1][0],
            board.pieces[1][1],
            board.pieces[1][2],
            board.pieces[1][3],
            board.pieces[1][4],
            board.pieces[1][5],
        ]
    }

    /// Material scaling factor (Akimbo: `eval * (700 + mat/32) / 1024`).
    #[inline(always)]
    fn material_scale(board: &Board) -> i32 {
        let knights = (board.pieces[0][Piece::Knight.index()]
            | board.pieces[1][Piece::Knight.index()])
        .count_ones() as i32;
        let bishops = (board.pieces[0][Piece::Bishop.index()]
            | board.pieces[1][Piece::Bishop.index()])
        .count_ones() as i32;
        let rooks = (board.pieces[0][Piece::Rook.index()] | board.pieces[1][Piece::Rook.index()])
            .count_ones() as i32;
        let queens = (board.pieces[0][Piece::Queen.index()] | board.pieces[1][Piece::Queen.index()])
            .count_ones() as i32;

        let mat = knights * SEE_VALS[1]
            + bishops * SEE_VALS[2]
            + rooks * SEE_VALS[3]
            + queens * SEE_VALS[4];
        700 + mat / 32
    }

    /// Evaluate the position using NNUE with incremental accumulator updates.
    pub fn evaluate(&mut self, board: &Board) -> i32 {
        let net = self.source.network();
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();

        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);

        let current_bbs = Self::snapshot_bbs(board);
        let entry = &mut self.table.table[wb][bb];

        // Check if cached accumulators are still valid
        let is_fresh = entry.bbs.iter().all(|&b| b == 0);
        if is_fresh {
            // Fresh entry — full compute
            Self::compute_entry(entry, net, board, w_king, b_king);
            entry.bbs = current_bbs;
        } else if entry.bbs != current_bbs {
            // Diff-based incremental update
            Self::update_entry_diff(entry, net, &current_bbs, w_king, b_king);
            entry.bbs = current_bbs;
        }

        // Forward pass: perspective net
        let (boys, opps) = match board.side_to_move {
            Color::White => (&entry.white, &entry.black),
            Color::Black => (&entry.black, &entry.white),
        };
        let raw = forward_with_network(net, boys, opps);

        // Material scaling (Akimbo: eval * (700 + mat/32) / 1024)
        let scale = Self::material_scale(board);
        raw * scale / 1024
    }

    /// Incremental diff-based update: find bitboard differences and add/sub features.
    fn update_entry_diff(
        entry: &mut EvalEntry,
        net: &Network,
        new_bbs: &[u64; NUM_BBS],
        w_king: usize,
        b_king: usize,
    ) {
        let old_bbs = entry.bbs;

        let wflip: usize = if w_king % 8 > 3 { 7 } else { 0 };
        let bflip: usize = if b_king % 8 > 3 { 7 } else { 0 } ^ 56;

        // Our BBS layout: [w_pawn, w_knight, w_bishop, w_rook, w_queen, w_king,
        //                   b_pawn, b_knight, b_bishop, b_rook, b_queen, b_king]
        // Index: side_idx * 6 + piece_idx
        for side_idx in 0..2usize {
            for piece_idx in 0..6usize {
                let bb_idx = side_idx * 6 + piece_idx;
                let old_bb = old_bbs[bb_idx];
                let new_bb = new_bbs[bb_idx];

                if old_bb == new_bb {
                    continue;
                } // No change for this piece/color

                let wbase = get_base_index::<0>(side_idx, piece_idx, w_king);
                let bbase = get_base_index::<1>(side_idx, piece_idx, b_king);

                // Features to add (new but not old)
                let mut add_diff = new_bb & !old_bb;
                while add_diff != 0 {
                    let sq = add_diff.trailing_zeros() as usize;
                    add_diff &= add_diff - 1;

                    let w_feat = wbase + (sq ^ wflip);
                    let b_feat = bbase + (sq ^ bflip);

                    super::simd::vector_add(
                        &mut entry.white.vals,
                        &net.feature_weights[w_feat].vals,
                    );
                    super::simd::vector_add(
                        &mut entry.black.vals,
                        &net.feature_weights[b_feat].vals,
                    );
                }

                // Features to subtract (old but not new)
                let mut sub_diff = old_bb & !new_bb;
                while sub_diff != 0 {
                    let sq = sub_diff.trailing_zeros() as usize;
                    sub_diff &= sub_diff - 1;

                    let w_feat = wbase + (sq ^ wflip);
                    let b_feat = bbase + (sq ^ bflip);

                    super::simd::vector_sub(
                        &mut entry.white.vals,
                        &net.feature_weights[w_feat].vals,
                    );
                    super::simd::vector_sub(
                        &mut entry.black.vals,
                        &net.feature_weights[b_feat].vals,
                    );
                }
            }
        }
    }

    /// Fully recompute the accumulators from a board position.
    pub fn reinit_from(&mut self, board: &Board) {
        let net = self.source.network();
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);
        let entry = &mut self.table.table[wb][bb];
        Self::compute_entry(entry, net, board, w_king, b_king);
        entry.bbs = Self::snapshot_bbs(board);
    }

    /// Compute accumulators from scratch (the actual work).
    fn compute_entry(
        entry: &mut EvalEntry,
        net: &Network,
        board: &Board,
        w_king: usize,
        b_king: usize,
    ) {
        entry.white = net.feature_bias;
        entry.black = net.feature_bias;
        let wflip: usize = if w_king % 8 > 3 { 7 } else { 0 };
        let bflip: usize = if b_king % 8 > 3 { 7 } else { 0 } ^ 56;

        for side_idx in 0..2usize {
            for piece_idx in 0..6usize {
                let mut bb_pieces = board.pieces[side_idx][piece_idx];
                while bb_pieces != 0 {
                    let sq = bb_pieces.trailing_zeros() as usize;
                    bb_pieces &= bb_pieces - 1;

                    let w_base = get_base_index::<0>(side_idx, piece_idx, w_king);
                    let w_feat = w_base + (sq ^ wflip);
                    super::simd::vector_add(
                        &mut entry.white.vals,
                        &net.feature_weights[w_feat].vals,
                    );

                    let b_base = get_base_index::<1>(side_idx, piece_idx, b_king);
                    let b_feat = b_base + (sq ^ bflip);
                    super::simd::vector_add(
                        &mut entry.black.vals,
                        &net.feature_weights[b_feat].vals,
                    );
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
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let entry = state.get_entry(w_king, b_king);
        let sum: i64 = entry.white.vals.iter().map(|&v| v as i64).sum();
        assert!(sum != 0, "White accumulator is all zeros after reinit");
    }

    #[test]
    fn test_evaluate_returns_reasonable() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        let score = state.evaluate(&board);
        assert!(
            score.abs() < 200,
            "Starting position eval {score} seems unreasonable"
        );
    }

    #[test]
    fn test_evaluate_caching() {
        types::init();
        let board = Board::new();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board);
        let score2 = state.evaluate(&board);
        assert_eq!(score1, score2, "Cached evaluation should be identical");
    }

    #[test]
    fn test_evaluate_different_positions() {
        types::init();
        let board1 = Board::new();
        let board2 =
            Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut state = NNUEState::new();
        let score1 = state.evaluate(&board1);
        let score2 = state.evaluate(&board2);
        assert!(
            score2 > score1,
            "Missing queen should increase eval: start={score1}, missing_q={score2}"
        );
    }

    #[test]
    fn test_material_scaling() {
        types::init();
        let mg = Board::new();
        let eg = Board::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").unwrap();
        let mg_scale = NNUEState::material_scale(&mg);
        let eg_scale = NNUEState::material_scale(&eg);
        assert!(
            mg_scale > eg_scale,
            "Middlegame should have higher scale: mg={mg_scale}, eg={eg_scale}"
        );
    }

    #[test]
    fn test_incremental_update_consistency() {
        types::init();
        let board1 = Board::new();
        let board2 =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1").unwrap();

        // Method 1: Evaluate board1 first (populates cache), then board2 (incremental diff)
        let mut state1 = NNUEState::new();
        let _ = state1.evaluate(&board1);
        let score_incremental = state1.evaluate(&board2);

        // Method 2: Evaluate board2 from scratch (fresh state)
        let mut state2 = NNUEState::new();
        let score_scratch = state2.evaluate(&board2);

        assert_eq!(
            score_incremental, score_scratch,
            "Incremental ({score_incremental}) vs scratch ({score_scratch}) mismatch"
        );
    }

    #[test]
    fn nnue_weight_diagnostic() {
        types::init();
        let net = super::super::network::net();

        // Feature bias
        let bias = &net.feature_bias.vals;
        let bias_nz = bias.iter().filter(|&&v| v != 0).count();
        eprintln!(
            "Feature bias: nonzero={}/{}, range=[{}, {}]",
            bias_nz,
            bias.len(),
            bias.iter().min().unwrap(),
            bias.iter().max().unwrap()
        );

        // Output weights
        let ow0 = &net.output_weights[0].vals;
        let ow1 = &net.output_weights[1].vals;
        eprintln!(
            "Output weights[0]: nonzero={}/{}, range=[{}, {}]",
            ow0.iter().filter(|&&v| v != 0).count(),
            ow0.len(),
            ow0.iter().min().unwrap(),
            ow0.iter().max().unwrap()
        );
        eprintln!(
            "Output weights[1]: nonzero={}/{}, range=[{}, {}]",
            ow1.iter().filter(|&&v| v != 0).count(),
            ow1.len(),
            ow1.iter().min().unwrap(),
            ow1.iter().max().unwrap()
        );
        eprintln!("Output bias: {}", net.output_bias);

        // Evaluate key positions
        let mut state = NNUEState::new();
        let board = Board::new();
        let s1 = state.evaluate(&board);
        eprintln!("Starting pos: {} cp", s1);

        let b2 =
            Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let s2 = state.evaluate(&b2);
        eprintln!("W up queen: {} cp", s2);

        let b3 =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNB1KBNR w Qkq - 0 1").unwrap();
        let s3 = state.evaluate(&b3);
        eprintln!("W down queen: {} cp", s3);

        // BK position 1: Black to play, Qd1+ is winning back exchange
        let bk1 = Board::from_fen("1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1").unwrap();
        let bk1s = state.evaluate(&bk1);
        eprintln!("BK #1 (black, exp d6d1): {} cp", bk1s);

        assert!(s2 > s1, "White up queen ({s2}) should be > start ({s1})");
        assert!(s3 < s1, "White down queen ({s3}) should be < start ({s1})");
    }
}
