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

use super::adapter::{ActiveNetwork, NnueNetworkInfo, NnueNetworkParameters, NnueNetworkSource};
use super::network::{
    self as nn, Accumulator, NUM_BUCKETS, Network, forward_with_network, get_base_index, get_bucket,
};
use std::mem::MaybeUninit;
use std::sync::Arc;
use types::{Board, Color, Move, Piece};

/// Sentinel: `EvalEntry::king_sq` unset or entry never written (valid squares are 0..64).
const NO_KING_SQ: u8 = u8::MAX;

/// Number of bitboards we track for cache comparison.
/// Layout: [white_occ, black_occ, pawn..king×2] = 14 total.
/// But simpler: use 12 per-piece bitboards like before.
const NUM_BBS: usize = 12;

/// Max feature indices per flush; one NNUE refresh touches ≤32 pieces — 64 leaves headroom.
const DELTA_BATCH: usize = 64;

#[inline(always)]
fn initialized_prefix<T>(buffer: &[MaybeUninit<T>], len: usize) -> &[T] {
    debug_assert!(len <= buffer.len());
    // SAFETY: every caller writes each element below `len` before exposing the
    // prefix, and the returned slice cannot outlive the backing buffer.
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<T>(), len) }
}

/// SEE piece values used for material scaling (matching Akimbo).
const SEE_VALS: [i32; 6] = [100, 450, 450, 650, 1250, 0];

/// Board bitmask state for accumulator cache validation.
pub struct EvalEntry {
    /// Snapshot of the board's 12 piece bitboards (pieces[2][6]).
    pub bbs: [u64; NUM_BBS],
    /// King squares used to build `white` / `black` (HalfKP indices depend on both kings).
    pub king_sq: [u8; 2],
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
            // SAFETY: `EvalEntry` contains only integer arrays, all of which
            // accept the zero bit pattern; semantic sentinels are set below.
            unsafe { boxed_and_zeroed() };
        for row in table.iter_mut() {
            for entry in row.iter_mut() {
                entry.white = Accumulator::default();
                entry.black = Accumulator::default();
                entry.king_sq = [NO_KING_SQ; 2];
            }
        }
        Self { table }
    }
}

/// NNUE state for use in the search.
pub struct NNUEState {
    table: Option<EvalTable>,
    source: Arc<ActiveNetwork>,
    #[cfg(feature = "reckless-nnue")]
    reckless: Option<super::reckless_format::RecklessAccumulatorState>,
    #[cfg(feature = "stockfish-nnue")]
    stockfish: Option<super::stockfish_format::StockfishAccumulatorState>,
}

impl Default for NNUEState {
    fn default() -> Self {
        Self::new()
    }
}

impl NNUEState {
    pub fn new() -> Self {
        Self::with_network(Arc::new(super::adapter::default_embedded_network()))
    }

    pub fn with_network(source: Arc<ActiveNetwork>) -> Self {
        let parameters = source.parameters();
        let table = match &parameters {
            NnueNetworkParameters::Akimbo(_) => Some(EvalTable::default()),
            #[cfg(feature = "reckless-nnue")]
            NnueNetworkParameters::Reckless(_) => None,
            #[cfg(feature = "stockfish-nnue")]
            NnueNetworkParameters::Stockfish(_) => None,
        };
        #[cfg(feature = "reckless-nnue")]
        let reckless = matches!(parameters, NnueNetworkParameters::Reckless(_))
            .then(super::reckless_format::RecklessAccumulatorState::new);
        #[cfg(feature = "stockfish-nnue")]
        let stockfish = matches!(parameters, NnueNetworkParameters::Stockfish(_))
            .then(super::stockfish_format::StockfishAccumulatorState::new);
        Self {
            table,
            source,
            #[cfg(feature = "reckless-nnue")]
            reckless,
            #[cfg(feature = "stockfish-nnue")]
            stockfish,
        }
    }

    pub fn network_info(&self) -> NnueNetworkInfo {
        self.source.info()
    }

    /// Start an accumulator frame for a real search move.
    #[inline]
    pub fn push_move(&mut self, board: &Board, mv: Move) {
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_move(board, mv);
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_move(board, mv);
        }
        #[cfg(not(feature = "reckless-nnue"))]
        let _ = (board, mv);
    }

    /// Applies a search move through the evaluator-specific update path.
    #[inline]
    pub fn make_move(&mut self, board: &mut Board, mv: Move) {
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_move_observed(board, mv);
            board.make_move_observed(mv, state);
            return;
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_move(board, mv);
        }
        board.make_move(mv);
    }

    /// Start an accumulator frame for a null move.
    #[inline]
    pub fn push_null(&mut self) {
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.push_null();
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.push_null();
        }
    }

    /// Restore the parent accumulator frame after unmaking a search move.
    #[inline]
    pub fn pop_move(&mut self) {
        #[cfg(feature = "reckless-nnue")]
        if let Some(state) = &mut self.reckless {
            state.pop();
        }
        #[cfg(feature = "stockfish-nnue")]
        if let Some(state) = &mut self.stockfish {
            state.pop();
        }
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
        match self.source.as_ref() {
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::EmbeddedReckless => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate(board, super::reckless_format::embedded());
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::ExternalReckless { network, .. } => {
                return self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state")
                    .evaluate(board, network);
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::EmbeddedStockfish => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate(board, super::stockfish_format::embedded());
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::ExternalStockfish { network, .. } => {
                return self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state")
                    .evaluate(board, network);
            }
            ActiveNetwork::Embedded => {}
            ActiveNetwork::ExternalAkimbo { .. } => {}
        }

        let NnueNetworkParameters::Akimbo(net) = self.source.parameters() else {
            unreachable!("non-Akimbo backends are handled above")
        };
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();

        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);

        let current_bbs = Self::snapshot_bbs(board);
        let entry = &mut self
            .table
            .as_mut()
            .expect("Akimbo source has matching accumulator table")
            .table[wb][bb];

        // Check if cached accumulators are still valid
        let is_fresh = entry.bbs == [0u64; NUM_BBS];
        let kings_changed = entry.king_sq[0] != w_king as u8
            || entry.king_sq[1] != b_king as u8
            || entry.king_sq[0] == NO_KING_SQ;
        if is_fresh {
            // Fresh entry — full compute
            Self::compute_entry(entry, net, board, w_king, b_king);
            entry.bbs = current_bbs;
            entry.king_sq = [w_king as u8, b_king as u8];
        } else if kings_changed {
            // King moved (or sq cache invalid): every piece's HalfKP index can change — no incremental path.
            Self::compute_entry(entry, net, board, w_king, b_king);
            entry.bbs = current_bbs;
            entry.king_sq = [w_king as u8, b_king as u8];
        } else if entry.bbs != current_bbs {
            // Diff-based incremental update (kings fixed; only pieces changed).
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

    /// Incremental diff-based update: batched row applies (one pass over `HIDDEN` per flush).
    fn update_entry_diff(
        entry: &mut EvalEntry,
        net: &Network,
        new_bbs: &[u64; NUM_BBS],
        w_king: usize,
        b_king: usize,
    ) {
        let old_bbs = entry.bbs;
        let weights = nn::feature_weights_flat(net);

        let wflip: usize = if w_king % 8 > 3 { 7 } else { 0 };
        let bflip: usize = if b_king % 8 > 3 { 7 } else { 0 } ^ 56;

        let mut w_add = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut b_add = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut w_sub = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut b_sub = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut na = 0usize;
        let mut ns = 0usize;

        #[inline(always)]
        fn flush_adds(
            entry: &mut EvalEntry,
            weights: &[i16],
            w_add: &[MaybeUninit<usize>],
            b_add: &[MaybeUninit<usize>],
            n: usize,
        ) {
            if n == 0 {
                return;
            }
            super::simd::accum_apply_deltas(
                &mut entry.white.vals,
                weights,
                initialized_prefix(w_add, n),
                &[],
            );
            super::simd::accum_apply_deltas(
                &mut entry.black.vals,
                weights,
                initialized_prefix(b_add, n),
                &[],
            );
        }

        #[inline(always)]
        fn flush_subs(
            entry: &mut EvalEntry,
            weights: &[i16],
            w_sub: &[MaybeUninit<usize>],
            b_sub: &[MaybeUninit<usize>],
            n: usize,
        ) {
            if n == 0 {
                return;
            }
            super::simd::accum_apply_deltas(
                &mut entry.white.vals,
                weights,
                &[],
                initialized_prefix(w_sub, n),
            );
            super::simd::accum_apply_deltas(
                &mut entry.black.vals,
                weights,
                &[],
                initialized_prefix(b_sub, n),
            );
        }

        for side_idx in 0..2usize {
            for piece_idx in 0..6usize {
                let bb_idx = side_idx * 6 + piece_idx;
                let old_bb = old_bbs[bb_idx];
                let new_bb = new_bbs[bb_idx];

                if old_bb == new_bb {
                    continue;
                }

                let wbase = get_base_index::<0>(side_idx, piece_idx, w_king);
                let bbase = get_base_index::<1>(side_idx, piece_idx, b_king);

                let mut add_diff = new_bb & !old_bb;
                while add_diff != 0 {
                    let sq = add_diff.trailing_zeros() as usize;
                    add_diff &= add_diff - 1;
                    w_add[na].write(wbase + (sq ^ wflip));
                    b_add[na].write(bbase + (sq ^ bflip));
                    na += 1;
                    if na == DELTA_BATCH {
                        flush_adds(entry, weights, &w_add, &b_add, DELTA_BATCH);
                        na = 0;
                    }
                }

                let mut sub_diff = old_bb & !new_bb;
                while sub_diff != 0 {
                    let sq = sub_diff.trailing_zeros() as usize;
                    sub_diff &= sub_diff - 1;
                    w_sub[ns].write(wbase + (sq ^ wflip));
                    b_sub[ns].write(bbase + (sq ^ bflip));
                    ns += 1;
                    if ns == DELTA_BATCH {
                        flush_subs(entry, weights, &w_sub, &b_sub, DELTA_BATCH);
                        ns = 0;
                    }
                }
            }
        }

        flush_adds(entry, weights, &w_add, &b_add, na);
        flush_subs(entry, weights, &w_sub, &b_sub, ns);
    }

    /// Fully recompute the accumulators from a board position.
    pub fn reinit_from(&mut self, board: &Board) {
        match self.source.as_ref() {
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::EmbeddedStockfish => {
                let state = self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, super::stockfish_format::embedded());
                return;
            }
            #[cfg(feature = "stockfish-nnue")]
            ActiveNetwork::ExternalStockfish { network, .. } => {
                let state = self
                    .stockfish
                    .as_mut()
                    .expect("Stockfish source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::EmbeddedReckless => {
                let state = self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, super::reckless_format::embedded());
                return;
            }
            #[cfg(feature = "reckless-nnue")]
            ActiveNetwork::ExternalReckless { network, .. } => {
                let state = self
                    .reckless
                    .as_mut()
                    .expect("Reckless source has matching accumulator state");
                state.clear();
                let _ = state.evaluate(board, network);
                return;
            }
            ActiveNetwork::Embedded | ActiveNetwork::ExternalAkimbo { .. } => {}
        }

        let NnueNetworkParameters::Akimbo(net) = self.source.parameters() else {
            unreachable!("non-Akimbo backends are handled above")
        };
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);
        let entry = &mut self
            .table
            .as_mut()
            .expect("Akimbo source has matching accumulator table")
            .table[wb][bb];
        Self::compute_entry(entry, net, board, w_king, b_king);
        entry.bbs = Self::snapshot_bbs(board);
        entry.king_sq = [w_king as u8, b_king as u8];
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
        let weights = nn::feature_weights_flat(net);
        let wflip: usize = if w_king % 8 > 3 { 7 } else { 0 };
        let bflip: usize = if b_king % 8 > 3 { 7 } else { 0 } ^ 56;

        let mut w_idx = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut b_idx = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        let mut n = 0usize;

        for side_idx in 0..2usize {
            for piece_idx in 0..6usize {
                let w_base = get_base_index::<0>(side_idx, piece_idx, w_king);
                let b_base = get_base_index::<1>(side_idx, piece_idx, b_king);
                let mut bb_pieces = board.pieces[side_idx][piece_idx];
                while bb_pieces != 0 {
                    let sq = bb_pieces.trailing_zeros() as usize;
                    bb_pieces &= bb_pieces - 1;
                    w_idx[n].write(w_base + (sq ^ wflip));
                    b_idx[n].write(b_base + (sq ^ bflip));
                    n += 1;
                    if n == DELTA_BATCH {
                        super::simd::accum_apply_deltas(
                            &mut entry.white.vals,
                            weights,
                            initialized_prefix(&w_idx, DELTA_BATCH),
                            &[],
                        );
                        super::simd::accum_apply_deltas(
                            &mut entry.black.vals,
                            weights,
                            initialized_prefix(&b_idx, DELTA_BATCH),
                            &[],
                        );
                        n = 0;
                    }
                }
            }
        }
        if n > 0 {
            super::simd::accum_apply_deltas(
                &mut entry.white.vals,
                weights,
                initialized_prefix(&w_idx, n),
                &[],
            );
            super::simd::accum_apply_deltas(
                &mut entry.black.vals,
                weights,
                initialized_prefix(&b_idx, n),
                &[],
            );
        }
    }

    /// Get the current accumulator entry for the given king positions.
    pub fn get_entry(&self, w_king: usize, b_king: usize) -> &EvalEntry {
        let wb = get_bucket::<0>(w_king);
        let bb = get_bucket::<1>(b_king);
        &self
            .table
            .as_ref()
            .expect("Akimbo source has matching accumulator table")
            .table[wb][bb]
    }
}

/// Allocate a boxed, zeroed value of any type.
/// # Safety
/// Type must be valid when all bytes are zero or must be overwritten before use.
unsafe fn boxed_and_zeroed<T>() -> Box<T> {
    // SAFETY: the caller guarantees that zero is a valid representation for
    // `T`; allocation failure is handled before ownership is constructed.
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
    fn initialized_prefix_exposes_only_written_entries() {
        let mut buffer = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        buffer[0].write(17);
        buffer[1].write(29);
        assert_eq!(initialized_prefix(&buffer, 2), [17, 29]);
    }

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
        let mut state = NNUEState::with_network(Arc::new(ActiveNetwork::Embedded));
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

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn reckless_evaluate_uses_concrete_active_network() {
        types::init();
        let mut state = NNUEState::with_network(Arc::new(ActiveNetwork::EmbeddedReckless));
        let score = state.evaluate(&Board::new());
        assert!(score.abs() < 500);
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn reckless_backend_does_not_allocate_an_akimbo_table() {
        let state = NNUEState::with_network(Arc::new(ActiveNetwork::EmbeddedReckless));
        assert!(state.table.is_none());
        assert!(state.reckless.is_some());
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
    fn king_move_incremental_matches_full_refresh() {
        types::init();
        // White king a1 vs b1: same king bucket (BUCKETS[0]==BUCKETS[1]==0), same black king cell — exercises stale HalfKP base.
        let board_ka1 = Board::from_fen("4k3/8/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let board_kb1 = Board::from_fen("4k3/8/8/8/8/8/8/1K6 w - - 0 1").unwrap();

        let mut warm = NNUEState::new();
        let _ = warm.evaluate(&board_ka1);
        let after_king_move = warm.evaluate(&board_kb1);

        let mut fresh = NNUEState::new();
        let from_scratch = fresh.evaluate(&board_kb1);

        assert_eq!(
            after_king_move, from_scratch,
            "After king move, NNUE must match full refresh: warm={after_king_move}, scratch={from_scratch}"
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
