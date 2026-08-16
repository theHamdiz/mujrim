//! Per-ply Akimbo accumulator stack and per-perspective Finny refresh cache.

use super::feature::{king_needs_mirror, king_needs_refresh};
use super::network::{
    self as nn, Accumulator, NUM_BUCKETS, Network, forward_with_network, get_base_index, get_bucket,
};
use std::mem::MaybeUninit;
use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece};

/// Sentinel: frame / Finny `king_sq` unset (valid squares are 0..64).
const NO_KING_SQ: u8 = u8::MAX;
const NUM_BBS: usize = 12;
const DELTA_BATCH: usize = 64;
const MOVE_DELTA: usize = 8;
const MAX_PLY: usize = 256;
const FINNY_BUCKETS: usize = 2 * NUM_BUCKETS;
const SEE_VALS: [i32; 6] = [100, 450, 450, 650, 1250, 0];

#[inline(always)]
fn initialized_prefix<T>(buffer: &[MaybeUninit<T>], len: usize) -> &[T] {
    debug_assert!(len <= buffer.len());
    // SAFETY: every caller writes each element below `len` before exposing the
    // prefix, and the returned slice cannot outlive the backing buffer.
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<T>(), len) }
}

/// Snapshot the board's piece bitboards: [white_pawn..white_king, black_pawn..black_king].
#[inline(always)]
pub(super) fn snapshot_bbs(board: &Board) -> [u64; NUM_BBS] {
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
pub(super) fn material_scale(board: &Board) -> i32 {
    material_scale_from_bbs(&snapshot_bbs(board))
}

#[inline(always)]
fn material_scale_from_bbs(bbs: &[u64; NUM_BBS]) -> i32 {
    let knights = (bbs[1] | bbs[7]).count_ones() as i32;
    let bishops = (bbs[2] | bbs[8]).count_ones() as i32;
    let rooks = (bbs[3] | bbs[9]).count_ones() as i32;
    let queens = (bbs[4] | bbs[10]).count_ones() as i32;
    let mat =
        knights * SEE_VALS[1] + bishops * SEE_VALS[2] + rooks * SEE_VALS[3] + queens * SEE_VALS[4];
    700 + mat / 32
}

#[inline(always)]
fn flip_for_king<const SIDE: usize>(king: usize) -> usize {
    let mirror = if king_needs_mirror(king) { 7 } else { 0 };
    if SIDE == 1 { mirror ^ 56 } else { mirror }
}

#[inline(always)]
fn feature_index<const SIDE: usize>(side: usize, pc: usize, sq: usize, king: usize) -> usize {
    get_base_index::<SIDE>(side, pc, king) + (sq ^ flip_for_king::<SIDE>(king))
}

/// Board bitmask state for accumulator cache validation / test inspection.
pub struct EvalEntry {
    pub bbs: [u64; NUM_BBS],
    pub king_sq: [u8; 2],
    pub white: Accumulator,
    pub black: Accumulator,
}

#[repr(C, align(64))]
struct AkimboFrame {
    white: Accumulator,
    black: Accumulator,
    bbs: [u64; NUM_BBS],
    king_sq: [u8; 2],
    accurate: [bool; 2],
    pending_null: bool,
    pending_has_move: bool,
    pending_move: Move,
    pending_mover: u8,
    pending_captured: u8,
    pending_side: u8,
}

#[repr(C, align(64))]
struct FinnyEntry {
    acc: Accumulator,
    bbs: [u64; NUM_BBS],
    king_sq: u8,
    initialized: bool,
}

pub(super) struct AkimboAccumulatorState {
    stack: Box<[AkimboFrame]>,
    finny: Box<[[FinnyEntry; FINNY_BUCKETS]; 2]>,
    stack_index: usize,
    current: EvalEntry,
}

impl AkimboAccumulatorState {
    pub(super) fn new() -> Self {
        let mut stack: Box<[AkimboFrame]> =
            // SAFETY: `AkimboFrame` is integer/bool/Option-of-Copy; zero is a
            // valid bit pattern. Semantic sentinels are written below.
            unsafe { boxed_slice_zeroed(MAX_PLY) };
        stack[0].king_sq = [NO_KING_SQ; 2];
        let finny: Box<[[FinnyEntry; FINNY_BUCKETS]; 2]> =
            // SAFETY: `FinnyEntry` is integer/bool; `initialized == false` is
            // the zero pattern and skips `acc` until the first refresh.
            unsafe { boxed_and_zeroed() };
        Self {
            stack,
            finny,
            stack_index: 0,
            current: EvalEntry {
                bbs: [0; NUM_BBS],
                king_sq: [NO_KING_SQ; 2],
                white: Accumulator {
                    vals: [0; nn::HIDDEN],
                },
                black: Accumulator {
                    vals: [0; nn::HIDDEN],
                },
            },
        }
    }

    #[cfg(test)]
    #[inline]
    pub(super) fn stack_index(&self) -> usize {
        self.stack_index
    }

    #[inline]
    pub(super) fn current_entry(&self) -> &EvalEntry {
        &self.current
    }

    #[inline]
    pub(super) fn push_move(&mut self, board: &Board, mv: Move) {
        if self.stack_index + 1 >= self.stack.len() {
            return;
        }
        let mover = board
            .piece_on(mv.from)
            .map(|(piece, color)| (piece.index() as u8, color.index() as u8));
        let captured = match mv.flag {
            MoveFlag::EnPassant => Some(Piece::Pawn.index() as u8),
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                board.piece_on(mv.to).map(|(piece, _)| piece.index() as u8)
            }
            _ => None,
        };
        self.stack_index += 1;
        let frame = &mut self.stack[self.stack_index];
        frame.accurate = [false, false];
        frame.pending_null = false;
        frame.pending_has_move = true;
        frame.pending_move = mv;
        frame.pending_mover = mover.map(|(pc, _)| pc).unwrap_or(u8::MAX);
        frame.pending_side = mover.map(|(_, side)| side).unwrap_or(u8::MAX);
        frame.pending_captured = captured.unwrap_or(u8::MAX);
        frame.king_sq = [NO_KING_SQ; 2];
    }

    #[inline]
    pub(super) fn push_null(&mut self) {
        if self.stack_index + 1 >= self.stack.len() {
            return;
        }
        self.stack_index += 1;
        let frame = &mut self.stack[self.stack_index];
        frame.accurate = [false, false];
        frame.pending_null = true;
        frame.pending_has_move = false;
        frame.pending_mover = u8::MAX;
        frame.pending_captured = u8::MAX;
        frame.pending_side = u8::MAX;
        frame.king_sq = [NO_KING_SQ; 2];
    }

    #[inline]
    pub(super) fn pop(&mut self) {
        if self.stack_index > 0 {
            self.stack_index -= 1;
        }
    }

    pub(super) fn clear(&mut self) {
        self.stack_index = 0;
        let frame = &mut self.stack[0];
        frame.accurate = [false, false];
        frame.pending_null = false;
        frame.pending_has_move = false;
        frame.king_sq = [NO_KING_SQ; 2];
        frame.bbs = [0; NUM_BBS];
        for row in self.finny.iter_mut() {
            for entry in row.iter_mut() {
                entry.initialized = false;
                entry.king_sq = NO_KING_SQ;
            }
        }
    }

    pub(super) fn evaluate(&mut self, board: &Board, net: &Network) -> i32 {
        self.ensure_accurate(board, net);
        self.finish(board, net)
    }

    pub(super) fn evaluate_search(&mut self, board: &Board, net: &Network) -> i32 {
        let frame = &self.stack[self.stack_index];
        if frame.accurate[0] && frame.accurate[1] && !frame.pending_has_move && !frame.pending_null
        {
            return self.finish(board, net);
        }
        self.evaluate(board, net)
    }

    fn finish(&self, board: &Board, net: &Network) -> i32 {
        let frame = &self.stack[self.stack_index];
        let (boys, opps) = match board.side_to_move {
            Color::White => (&frame.white, &frame.black),
            Color::Black => (&frame.black, &frame.white),
        };
        let raw = forward_with_network(net, boys, opps);
        let scale = material_scale_from_bbs(&frame.bbs);
        debug_assert_eq!(scale, material_scale(board));
        raw * scale / 1024
    }

    fn ensure_accurate(&mut self, board: &Board, net: &Network) {
        let idx = self.stack_index;
        if self.stack[idx].pending_null {
            self.apply_null();
            return;
        }
        if self.stack[idx].pending_has_move {
            self.apply_pending_move(board, net);
            return;
        }

        let current_bbs = snapshot_bbs(board);
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let frame = &self.stack[idx];
        if frame.accurate[0]
            && frame.accurate[1]
            && frame.bbs == current_bbs
            && frame.king_sq[0] == w_king as u8
            && frame.king_sq[1] == b_king as u8
        {
            return;
        }
        self.sync_from_board(board, net);
    }

    fn apply_null(&mut self) {
        let idx = self.stack_index;
        debug_assert!(idx > 0);
        let parent = &self.stack[idx - 1];
        let white = parent.white;
        let black = parent.black;
        let bbs = parent.bbs;
        let king_sq = parent.king_sq;
        let frame = &mut self.stack[idx];
        frame.white = white;
        frame.black = black;
        frame.bbs = bbs;
        frame.king_sq = king_sq;
        frame.accurate = [true, true];
        frame.pending_null = false;
    }

    fn apply_pending_move(&mut self, board: &Board, net: &Network) {
        let idx = self.stack_index;
        debug_assert!(idx > 0);
        let parent_ready = self.stack[idx - 1].accurate[0] && self.stack[idx - 1].accurate[1];
        if !parent_ready {
            self.stack[idx].pending_has_move = false;
            self.sync_from_board(board, net);
            return;
        }

        let mv = self.stack[idx].pending_move;
        let mover_pc = self.stack[idx].pending_mover as usize;
        let mover_side = self.stack[idx].pending_side as usize;
        let captured = (self.stack[idx].pending_captured != u8::MAX)
            .then_some(self.stack[idx].pending_captured as usize);
        let old_kings = self.stack[idx - 1].king_sq;
        let new_kings = [
            board.king_square(Color::White).index() as u8,
            board.king_square(Color::Black).index() as u8,
        ];
        let weights = nn::feature_weights_flat(net);
        let current_bbs = snapshot_bbs(board);

        let refresh_white = old_kings[0] == NO_KING_SQ
            || king_needs_refresh(0, old_kings[0] as usize, new_kings[0] as usize);
        let refresh_black = old_kings[1] == NO_KING_SQ
            || king_needs_refresh(1, old_kings[1] as usize, new_kings[1] as usize);

        if refresh_white {
            self.finny_refresh::<0>(net, &current_bbs, new_kings[0] as usize);
            self.stack[idx].white = self.finny[0][get_bucket::<0>(new_kings[0] as usize)].acc;
        }
        if refresh_black {
            self.finny_refresh::<1>(net, &current_bbs, new_kings[1] as usize);
            self.stack[idx].black = self.finny[1][get_bucket::<1>(new_kings[1] as usize)].acc;
        }

        if !refresh_white || !refresh_black {
            let (parents, children) = self.stack.split_at_mut(idx);
            let parent = &parents[idx - 1];
            let child = &mut children[0];
            if !refresh_white {
                apply_move_deltas_from::<0>(
                    &mut child.white,
                    &parent.white,
                    weights,
                    new_kings[0] as usize,
                    mover_side,
                    mover_pc,
                    captured,
                    mv,
                );
            }
            if !refresh_black {
                apply_move_deltas_from::<1>(
                    &mut child.black,
                    &parent.black,
                    weights,
                    new_kings[1] as usize,
                    mover_side,
                    mover_pc,
                    captured,
                    mv,
                );
            }
        }

        let frame = &mut self.stack[idx];
        frame.bbs = current_bbs;
        frame.king_sq = new_kings;
        frame.accurate = [true, true];
        frame.pending_has_move = false;
    }

    fn sync_from_board(&mut self, board: &Board, net: &Network) {
        let current_bbs = snapshot_bbs(board);
        let w_king = board.king_square(Color::White).index();
        let b_king = board.king_square(Color::Black).index();
        let idx = self.stack_index;
        let old_kings = self.stack[idx].king_sq;
        let old_bbs = self.stack[idx].bbs;
        let white_ok = self.stack[idx].accurate[0]
            && old_kings[0] != NO_KING_SQ
            && !king_needs_refresh(0, old_kings[0] as usize, w_king);
        let black_ok = self.stack[idx].accurate[1]
            && old_kings[1] != NO_KING_SQ
            && !king_needs_refresh(1, old_kings[1] as usize, b_king);

        if white_ok {
            apply_half_diff::<0>(
                &mut self.stack[idx].white,
                &old_bbs,
                &current_bbs,
                w_king,
                nn::feature_weights_flat(net),
            );
        } else {
            self.finny_refresh::<0>(net, &current_bbs, w_king);
            self.stack[idx].white = self.finny[0][get_bucket::<0>(w_king)].acc;
        }

        if black_ok {
            apply_half_diff::<1>(
                &mut self.stack[idx].black,
                &old_bbs,
                &current_bbs,
                b_king,
                nn::feature_weights_flat(net),
            );
        } else {
            self.finny_refresh::<1>(net, &current_bbs, b_king);
            self.stack[idx].black = self.finny[1][get_bucket::<1>(b_king)].acc;
        }

        let frame = &mut self.stack[idx];
        frame.bbs = current_bbs;
        frame.king_sq = [w_king as u8, b_king as u8];
        frame.accurate = [true, true];
        frame.pending_has_move = false;
        frame.pending_null = false;
    }

    fn finny_refresh<const SIDE: usize>(
        &mut self,
        net: &Network,
        bbs: &[u64; NUM_BBS],
        king: usize,
    ) {
        let bucket = get_bucket::<SIDE>(king);
        let entry = &mut self.finny[SIDE][bucket];
        if !entry.initialized
            || entry.king_sq == NO_KING_SQ
            || king_needs_refresh(SIDE, entry.king_sq as usize, king)
        {
            compute_half_from_bias::<SIDE>(&mut entry.acc, bbs, king, net);
        } else {
            apply_half_diff::<SIDE>(
                &mut entry.acc,
                &entry.bbs,
                bbs,
                king,
                nn::feature_weights_flat(net),
            );
        }
        entry.bbs = *bbs;
        entry.king_sq = king as u8;
        entry.initialized = true;
    }

    #[inline]
    pub(super) fn sync_current_from_frame(&mut self) {
        let frame = &self.stack[self.stack_index];
        self.current.white = frame.white;
        self.current.black = frame.black;
        self.current.bbs = frame.bbs;
        self.current.king_sq = frame.king_sq;
    }
}

fn collect_move_delta_indices<const SIDE: usize>(
    adds: &mut [MaybeUninit<usize>; MOVE_DELTA],
    subs: &mut [MaybeUninit<usize>; MOVE_DELTA],
    king: usize,
    mover_side: usize,
    mover_pc: usize,
    captured: Option<usize>,
    mv: Move,
) -> (usize, usize) {
    let mut na = 0usize;
    let mut ns = 0usize;
    let from = mv.from.index();
    let to = mv.to.index();
    let add_pc = mv.promotion.map(Piece::index).unwrap_or(mover_pc);

    subs[ns].write(feature_index::<SIDE>(mover_side, mover_pc, from, king));
    ns += 1;
    adds[na].write(feature_index::<SIDE>(mover_side, add_pc, to, king));
    na += 1;

    if let Some(captured_pc) = captured {
        let cap_sq = if mv.flag == MoveFlag::EnPassant {
            if mover_side == 0 {
                to.wrapping_sub(8)
            } else {
                to + 8
            }
        } else {
            to
        };
        let cap_side = mover_side ^ 1;
        subs[ns].write(feature_index::<SIDE>(cap_side, captured_pc, cap_sq, king));
        ns += 1;
    }

    if matches!(mv.flag, MoveFlag::KingCastle | MoveFlag::QueenCastle) {
        let (rook_from, rook_to) = castle_rook_squares(mv.flag, mover_side);
        subs[ns].write(feature_index::<SIDE>(
            mover_side,
            Piece::Rook.index(),
            rook_from,
            king,
        ));
        ns += 1;
        adds[na].write(feature_index::<SIDE>(
            mover_side,
            Piece::Rook.index(),
            rook_to,
            king,
        ));
        na += 1;
    }
    (na, ns)
}

#[cfg(test)]
fn apply_move_deltas<const SIDE: usize>(
    acc: &mut Accumulator,
    weights: &[i16],
    king: usize,
    mover_side: usize,
    mover_pc: usize,
    captured: Option<usize>,
    mv: Move,
) {
    let mut adds = [MaybeUninit::<usize>::uninit(); MOVE_DELTA];
    let mut subs = [MaybeUninit::<usize>::uninit(); MOVE_DELTA];
    let (na, ns) = collect_move_delta_indices::<SIDE>(
        &mut adds, &mut subs, king, mover_side, mover_pc, captured, mv,
    );
    super::simd::accum_apply_deltas(
        &mut acc.vals,
        weights,
        initialized_prefix(&adds, na),
        initialized_prefix(&subs, ns),
    );
}

#[allow(clippy::too_many_arguments)]
fn apply_move_deltas_from<const SIDE: usize>(
    dst: &mut Accumulator,
    src: &Accumulator,
    weights: &[i16],
    king: usize,
    mover_side: usize,
    mover_pc: usize,
    captured: Option<usize>,
    mv: Move,
) {
    let mut adds = [MaybeUninit::<usize>::uninit(); MOVE_DELTA];
    let mut subs = [MaybeUninit::<usize>::uninit(); MOVE_DELTA];
    let (na, ns) = collect_move_delta_indices::<SIDE>(
        &mut adds, &mut subs, king, mover_side, mover_pc, captured, mv,
    );
    super::stockfish_simd::apply_i16_from_width(
        &mut dst.vals,
        &src.vals,
        weights,
        initialized_prefix(&adds, na),
        initialized_prefix(&subs, ns),
    );
}

fn apply_half_diff<const SIDE: usize>(
    acc: &mut Accumulator,
    old_bbs: &[u64; NUM_BBS],
    new_bbs: &[u64; NUM_BBS],
    king: usize,
    weights: &[i16],
) {
    let flip = flip_for_king::<SIDE>(king);
    let mut add_idx = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
    let mut sub_idx = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
    let mut na = 0usize;
    let mut ns = 0usize;

    for side_idx in 0..2usize {
        for piece_idx in 0..6usize {
            let bb_idx = side_idx * 6 + piece_idx;
            let old_bb = old_bbs[bb_idx];
            let new_bb = new_bbs[bb_idx];
            if old_bb == new_bb {
                continue;
            }
            let base = get_base_index::<SIDE>(side_idx, piece_idx, king);

            let mut add_diff = new_bb & !old_bb;
            while add_diff != 0 {
                let sq = add_diff.trailing_zeros() as usize;
                add_diff &= add_diff - 1;
                add_idx[na].write(base + (sq ^ flip));
                na += 1;
                if na == DELTA_BATCH {
                    super::simd::accum_apply_deltas(
                        &mut acc.vals,
                        weights,
                        initialized_prefix(&add_idx, DELTA_BATCH),
                        &[],
                    );
                    na = 0;
                }
            }

            let mut sub_diff = old_bb & !new_bb;
            while sub_diff != 0 {
                let sq = sub_diff.trailing_zeros() as usize;
                sub_diff &= sub_diff - 1;
                sub_idx[ns].write(base + (sq ^ flip));
                ns += 1;
                if ns == DELTA_BATCH {
                    super::simd::accum_apply_deltas(
                        &mut acc.vals,
                        weights,
                        &[],
                        initialized_prefix(&sub_idx, DELTA_BATCH),
                    );
                    ns = 0;
                }
            }
        }
    }

    if na > 0 {
        super::simd::accum_apply_deltas(
            &mut acc.vals,
            weights,
            initialized_prefix(&add_idx, na),
            &[],
        );
    }
    if ns > 0 {
        super::simd::accum_apply_deltas(
            &mut acc.vals,
            weights,
            &[],
            initialized_prefix(&sub_idx, ns),
        );
    }
}

#[inline(never)]
#[cold]
fn compute_half_from_bias<const SIDE: usize>(
    acc: &mut Accumulator,
    bbs: &[u64; NUM_BBS],
    king: usize,
    net: &Network,
) {
    *acc = net.feature_bias;
    let weights = nn::feature_weights_flat(net);
    let flip = flip_for_king::<SIDE>(king);
    let mut idx = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
    let mut n = 0usize;

    for side_idx in 0..2usize {
        for piece_idx in 0..6usize {
            let base = get_base_index::<SIDE>(side_idx, piece_idx, king);
            let mut pieces = bbs[side_idx * 6 + piece_idx];
            while pieces != 0 {
                let sq = pieces.trailing_zeros() as usize;
                pieces &= pieces - 1;
                idx[n].write(base + (sq ^ flip));
                n += 1;
                if n == DELTA_BATCH {
                    super::simd::accum_apply_deltas(
                        &mut acc.vals,
                        weights,
                        initialized_prefix(&idx, DELTA_BATCH),
                        &[],
                    );
                    n = 0;
                }
            }
        }
    }
    if n > 0 {
        super::simd::accum_apply_deltas(&mut acc.vals, weights, initialized_prefix(&idx, n), &[]);
    }
}

#[inline(always)]
fn castle_rook_squares(flag: MoveFlag, mover_side: usize) -> (usize, usize) {
    match (mover_side, flag) {
        (0, MoveFlag::KingCastle) => (7, 5),
        (0, MoveFlag::QueenCastle) => (0, 3),
        (1, MoveFlag::KingCastle) => (63, 61),
        (1, MoveFlag::QueenCastle) => (56, 59),
        _ => (0, 0),
    }
}

/// Allocate a boxed, zeroed value of any type.
///
/// # Safety
/// Type must be valid when all bytes are zero or must be overwritten before use.
unsafe fn boxed_and_zeroed<T>() -> Box<T> {
    unsafe {
        let layout = std::alloc::Layout::new::<T>();
        let ptr = std::alloc::alloc_zeroed(layout).cast::<T>();
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(ptr)
    }
}

/// Allocate a boxed, zeroed slice of `len` elements.
///
/// # Safety
/// `T` must be valid when all bytes are zero or overwritten before use.
unsafe fn boxed_slice_zeroed<T>(len: usize) -> Box<[T]> {
    unsafe {
        let layout = std::alloc::Layout::array::<T>(len).expect("akimbo stack layout");
        let ptr = std::alloc::alloc_zeroed(layout).cast::<T>();
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Square;

    #[test]
    fn initialized_prefix_exposes_only_written_entries() {
        let mut buffer = [MaybeUninit::<usize>::uninit(); DELTA_BATCH];
        buffer[0].write(17);
        buffer[1].write(29);
        assert_eq!(initialized_prefix(&buffer, 2), [17, 29]);
    }

    #[test]
    fn castle_rook_squares_match_standard_geometry() {
        assert_eq!(
            castle_rook_squares(MoveFlag::KingCastle, 0),
            (Square::H1.index(), Square::F1.index())
        );
        assert_eq!(
            castle_rook_squares(MoveFlag::QueenCastle, 1),
            (Square::A8.index(), Square::D8.index())
        );
    }

    #[test]
    fn new_state_starts_at_ply_zero() {
        let state = AkimboAccumulatorState::new();
        assert_eq!(state.stack_index(), 0);
    }

    #[test]
    fn apply_from_matches_copy_then_in_place_for_quiet_pawn() {
        types::init();
        let mut parent = Accumulator {
            vals: [0; nn::HIDDEN],
        };
        for (index, value) in parent.vals.iter_mut().enumerate() {
            *value = (index as i16).wrapping_mul(13).wrapping_sub(40);
        }
        let mut weights = vec![0i16; 768 * NUM_BUCKETS * nn::HIDDEN];
        for (index, weight) in weights.iter_mut().enumerate() {
            *weight = ((index % 11) as i16) - 5;
        }
        let mv = Move::quiet(Square::E2, Square::E4);
        let mut copied = parent;
        apply_move_deltas::<0>(
            &mut copied,
            &weights,
            Square::E1.index(),
            0,
            Piece::Pawn.index(),
            None,
            mv,
        );
        let mut from = Accumulator {
            vals: [0; nn::HIDDEN],
        };
        apply_move_deltas_from::<0>(
            &mut from,
            &parent,
            &weights,
            Square::E1.index(),
            0,
            Piece::Pawn.index(),
            None,
            mv,
        );
        assert_eq!(from.vals, copied.vals);
    }

    #[test]
    fn material_scale_reads_snapshot_bitboards() {
        types::init();
        let board = Board::new();
        assert_eq!(
            material_scale(&board),
            material_scale_from_bbs(&snapshot_bbs(&board))
        );
        assert_eq!(
            material_scale(&board),
            700 + (4 * 450 + 4 * 450 + 4 * 650 + 2 * 1250) / 32
        );
    }
}
