//! Per-ply Akimbo accumulator stack and per-perspective Finny refresh cache.

use super::feature::{king_needs_mirror, king_needs_refresh};
use super::network::{
    self as nn, Accumulator, NUM_BUCKETS, Network, forward_with_network, get_base_index, get_bucket,
};
use std::mem::MaybeUninit;
use types::chess_move::MoveFlag;
use types::{AkimboPos, Board, BoardSnapshot, Color, Move, Piece};

/// Sentinel: frame / Finny `king_sq` unset (valid squares are 0..64).
const NO_KING_SQ: u8 = u8::MAX;
const NUM_BBS: usize = 12;
const DELTA_BATCH: usize = 64;
const MOVE_DELTA: usize = 8;
const MAX_PLY: usize = 256;
const FINNY_BUCKETS: usize = 2 * NUM_BUCKETS;
const FINNY_PAIR: usize = 2 * NUM_BUCKETS;
const FILL_DELTA: usize = 32;
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
#[allow(dead_code)]
fn board_bb8(board: &Board) -> [u64; 8] {
    [
        board.occupancy[0],
        board.occupancy[1],
        board.pieces[0][0] | board.pieces[1][0],
        board.pieces[0][1] | board.pieces[1][1],
        board.pieces[0][2] | board.pieces[1][2],
        board.pieces[0][3] | board.pieces[1][3],
        board.pieces[0][4] | board.pieces[1][4],
        board.pieces[0][5] | board.pieces[1][5],
    ]
}

#[inline(always)]
fn material_scale_from_bb8(bb: &[u64; 8]) -> i32 {
    let knights = bb[3].count_ones() as i32;
    let bishops = bb[4].count_ones() as i32;
    let rooks = bb[5].count_ones() as i32;
    let queens = bb[6].count_ones() as i32;
    let mat =
        knights * SEE_VALS[1] + bishops * SEE_VALS[2] + rooks * SEE_VALS[3] + queens * SEE_VALS[4];
    700 + mat / 32
}

fn fill_diff(
    new_bb: &[u64; 8],
    old_bb: &[u64; 8],
    w_king: usize,
    b_king: usize,
    add_feats: &mut [[usize; FILL_DELTA]; 2],
    sub_feats: &mut [[usize; FILL_DELTA]; 2],
) -> (usize, usize) {
    let mut adds = 0usize;
    let mut subs = 0usize;
    let wflip = if w_king % 8 > 3 { 7 } else { 0 };
    let bflip = if b_king % 8 > 3 { 7 } else { 0 } ^ 56;
    for side in 0..2 {
        let old_boys = old_bb[side];
        let new_boys = new_bb[side];
        for piece in 0..6 {
            let old_pc = old_bb[piece + 2] & old_boys;
            let new_pc = new_bb[piece + 2] & new_boys;
            if old_pc == new_pc {
                continue;
            }
            let wbase = get_base_index::<0>(side, piece, w_king);
            let bbase = get_base_index::<1>(side, piece, b_king);
            let mut add_diff = new_pc & !old_pc;
            while add_diff != 0 {
                let sq = add_diff.trailing_zeros() as usize;
                add_diff &= add_diff - 1;
                add_feats[0][adds] = wbase + (sq ^ wflip);
                add_feats[1][adds] = bbase + (sq ^ bflip);
                adds += 1;
            }
            let mut sub_diff = old_pc & !new_pc;
            while sub_diff != 0 {
                let sq = sub_diff.trailing_zeros() as usize;
                sub_diff &= sub_diff - 1;
                sub_feats[0][subs] = wbase + (sq ^ wflip);
                sub_feats[1][subs] = bbase + (sq ^ bflip);
                subs += 1;
            }
        }
    }
    (adds, subs)
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

/// Official `EvalTable[wbucket][bbucket]`: both accs + 8-bb snapshot.
#[repr(C, align(64))]
struct FinnyPair {
    white: Accumulator,
    black: Accumulator,
    bbs: [u64; 8],
    initialized: bool,
}

pub(super) struct AkimboAccumulatorState {
    stack: Box<[AkimboFrame]>,
    finny: Box<[[FinnyEntry; FINNY_BUCKETS]; 2]>,
    pairs: Box<[[FinnyPair; FINNY_PAIR]; FINNY_PAIR]>,
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
        let pairs: Box<[[FinnyPair; FINNY_PAIR]; FINNY_PAIR]> =
            // SAFETY: `initialized == false` skips accs until the first fill.
            unsafe { boxed_and_zeroed() };
        Self {
            stack,
            finny,
            pairs,
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
        self.push_pending(mv, mover, captured);
    }

    #[inline]
    pub(super) fn push_move_pos(&mut self, pos: &AkimboPos, mv: Move) {
        let mover = pos
            .piece_on(mv.from)
            .map(|(piece, color)| (piece.index() as u8, color.index() as u8));
        let captured = match mv.flag {
            MoveFlag::EnPassant => Some(Piece::Pawn.index() as u8),
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                pos.piece_on(mv.to).map(|(piece, _)| piece.index() as u8)
            }
            _ => None,
        };
        self.push_pending(mv, mover, captured);
    }

    #[inline]
    pub(super) fn push_move_snap(&mut self, pos: &BoardSnapshot, mv: Move) {
        let mover = pos
            .piece_on(mv.from)
            .map(|(piece, color)| (piece.index() as u8, color.index() as u8));
        let captured = match mv.flag {
            MoveFlag::EnPassant => Some(Piece::Pawn.index() as u8),
            MoveFlag::Capture | MoveFlag::PromotionCapture => {
                pos.piece_on(mv.to).map(|(piece, _)| piece.index() as u8)
            }
            _ => None,
        };
        self.push_pending(mv, mover, captured);
    }

    #[inline]
    fn push_pending(&mut self, mv: Move, mover: Option<(u8, u8)>, captured: Option<u8>) {
        if self.stack_index + 1 >= self.stack.len() {
            return;
        }
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
        for row in self.pairs.iter_mut() {
            for entry in row.iter_mut() {
                entry.initialized = false;
                entry.bbs = [0; 8];
            }
        }
    }

    pub(super) fn evaluate(&mut self, board: &Board, net: &Network) -> i32 {
        self.ensure_accurate(board, net);
        self.finish(board, net)
    }

    /// Official pair-keyed `fill_diff` against a Copy position.
    pub(super) fn evaluate_search_pos(&mut self, pos: &AkimboPos, net: &Network) -> i32 {
        self.evaluate_from_bb8(
            pos.bb8(),
            pos.king_square(Color::White).index(),
            pos.king_square(Color::Black).index(),
            pos.side_to_move(),
            net,
        )
    }

    pub(super) fn evaluate_search(&mut self, board: &Board, net: &Network) -> i32 {
        let current_bbs = snapshot_bbs(board);
        let w_king = board.king_square(Color::White).index() as u8;
        let b_king = board.king_square(Color::Black).index() as u8;
        let frame = &self.stack[self.stack_index];
        if frame.accurate[0]
            && frame.accurate[1]
            && !frame.pending_has_move
            && !frame.pending_null
            && frame.bbs == current_bbs
            && frame.king_sq[0] == w_king
            && frame.king_sq[1] == b_king
        {
            return self.finish(board, net);
        }
        self.evaluate(board, net)
    }

    pub(super) fn evaluate_search_snap(&mut self, pos: &BoardSnapshot, net: &Network) -> i32 {
        let current_bbs = pos.snapshot12();
        let w_king = pos.king_square(Color::White).index() as u8;
        let b_king = pos.king_square(Color::Black).index() as u8;
        let frame = &self.stack[self.stack_index];
        if frame.accurate[0]
            && frame.accurate[1]
            && !frame.pending_has_move
            && !frame.pending_null
            && frame.bbs == current_bbs
            && frame.king_sq[0] == w_king
            && frame.king_sq[1] == b_king
        {
            return self.finish_stm(pos.side_to_move(), net);
        }
        self.ensure_accurate_snap(pos, net);
        self.finish_stm(pos.side_to_move(), net)
    }

    fn evaluate_from_bb8(
        &mut self,
        bb8: [u64; 8],
        w_king: usize,
        b_king: usize,
        stm: Color,
        net: &Network,
    ) -> i32 {
        let wbucket = get_bucket::<0>(w_king);
        let bbucket = get_bucket::<1>(b_king);
        let (white, black) = {
            let entry = &mut self.pairs[wbucket][bbucket];
            if !entry.initialized {
                entry.white = net.feature_bias;
                entry.black = net.feature_bias;
                entry.bbs = [0; 8];
                entry.initialized = true;
            }
            let mut add_feats = [[0usize; FILL_DELTA]; 2];
            let mut sub_feats = [[0usize; FILL_DELTA]; 2];
            let (adds, subs) = fill_diff(
                &bb8,
                &entry.bbs,
                w_king,
                b_king,
                &mut add_feats,
                &mut sub_feats,
            );
            if adds > 0 || subs > 0 {
                let weights = nn::feature_weights_flat(net);
                super::simd::accum_apply_deltas(
                    &mut entry.white.vals,
                    weights,
                    &add_feats[0][..adds],
                    &sub_feats[0][..subs],
                );
                super::simd::accum_apply_deltas(
                    &mut entry.black.vals,
                    weights,
                    &add_feats[1][..adds],
                    &sub_feats[1][..subs],
                );
                entry.bbs = bb8;
            }
            (entry.white, entry.black)
        };
        self.current.white = white;
        self.current.black = black;
        self.current.king_sq = [w_king as u8, b_king as u8];
        let (boys, opps) = match stm {
            Color::White => (&white, &black),
            Color::Black => (&black, &white),
        };
        forward_with_network(net, boys, opps) * material_scale_from_bb8(&bb8) / 1024
    }

    fn finish(&self, board: &Board, net: &Network) -> i32 {
        let score = self.finish_stm(board.side_to_move, net);
        debug_assert_eq!(
            material_scale_from_bbs(&self.stack[self.stack_index].bbs),
            material_scale(board)
        );
        score
    }

    fn finish_stm(&self, stm: Color, net: &Network) -> i32 {
        let frame = &self.stack[self.stack_index];
        let (boys, opps) = match stm {
            Color::White => (&frame.white, &frame.black),
            Color::Black => (&frame.black, &frame.white),
        };
        let raw = forward_with_network(net, boys, opps);
        raw * material_scale_from_bbs(&frame.bbs) / 1024
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

    #[allow(dead_code)]
    fn ensure_accurate_pos(&mut self, pos: &AkimboPos, net: &Network) {
        let idx = self.stack_index;
        if self.stack[idx].pending_null {
            self.apply_null();
            return;
        }
        if self.stack[idx].pending_has_move {
            self.apply_pending_bbs(
                pos.snapshot12(),
                pos.king_square(Color::White).index(),
                pos.king_square(Color::Black).index(),
                net,
            );
            return;
        }
        let current_bbs = pos.snapshot12();
        let w_king = pos.king_square(Color::White).index();
        let b_king = pos.king_square(Color::Black).index();
        let frame = &self.stack[idx];
        if frame.accurate[0]
            && frame.accurate[1]
            && frame.bbs == current_bbs
            && frame.king_sq[0] == w_king as u8
            && frame.king_sq[1] == b_king as u8
        {
            return;
        }
        self.sync_from_bbs(&current_bbs, w_king, b_king, net);
    }

    fn ensure_accurate_snap(&mut self, pos: &BoardSnapshot, net: &Network) {
        let idx = self.stack_index;
        if self.stack[idx].pending_null {
            self.apply_null();
            return;
        }
        if self.stack[idx].pending_has_move {
            self.apply_pending_bbs(
                pos.snapshot12(),
                pos.king_square(Color::White).index(),
                pos.king_square(Color::Black).index(),
                net,
            );
            return;
        }
        let current_bbs = pos.snapshot12();
        let w_king = pos.king_square(Color::White).index();
        let b_king = pos.king_square(Color::Black).index();
        let frame = &self.stack[idx];
        if frame.accurate[0]
            && frame.accurate[1]
            && frame.bbs == current_bbs
            && frame.king_sq[0] == w_king as u8
            && frame.king_sq[1] == b_king as u8
        {
            return;
        }
        self.sync_from_bbs(&current_bbs, w_king, b_king, net);
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
        self.apply_pending_bbs(
            snapshot_bbs(board),
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
            net,
        );
    }

    fn apply_pending_bbs(
        &mut self,
        current_bbs: [u64; NUM_BBS],
        w_king: usize,
        b_king: usize,
        net: &Network,
    ) {
        let idx = self.stack_index;
        debug_assert!(idx > 0);
        let parent_ready = self.stack[idx - 1].accurate[0] && self.stack[idx - 1].accurate[1];
        if !parent_ready {
            self.stack[idx].pending_has_move = false;
            self.sync_from_bbs(&current_bbs, w_king, b_king, net);
            return;
        }

        let mv = self.stack[idx].pending_move;
        let mover_pc = self.stack[idx].pending_mover as usize;
        let mover_side = self.stack[idx].pending_side as usize;
        let captured = (self.stack[idx].pending_captured != u8::MAX)
            .then_some(self.stack[idx].pending_captured as usize);
        let old_kings = self.stack[idx - 1].king_sq;
        let new_kings = [w_king as u8, b_king as u8];
        let weights = nn::feature_weights_flat(net);

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
        self.sync_from_bbs(
            &snapshot_bbs(board),
            board.king_square(Color::White).index(),
            board.king_square(Color::Black).index(),
            net,
        );
    }

    fn sync_from_bbs(
        &mut self,
        current_bbs: &[u64; NUM_BBS],
        w_king: usize,
        b_king: usize,
        net: &Network,
    ) {
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
                current_bbs,
                w_king,
                nn::feature_weights_flat(net),
            );
        } else {
            self.finny_refresh::<0>(net, current_bbs, w_king);
            self.stack[idx].white = self.finny[0][get_bucket::<0>(w_king)].acc;
        }

        if black_ok {
            apply_half_diff::<1>(
                &mut self.stack[idx].black,
                &old_bbs,
                current_bbs,
                b_king,
                nn::feature_weights_flat(net),
            );
        } else {
            self.finny_refresh::<1>(net, current_bbs, b_king);
            self.stack[idx].black = self.finny[1][get_bucket::<1>(b_king)].acc;
        }

        let frame = &mut self.stack[idx];
        frame.bbs = *current_bbs;
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

    #[test]
    fn evaluate_search_without_ply_matches_pushed_incremental() {
        types::init();
        let net = super::super::network::net();
        let mut board = Board::new();
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4");

        let mut pushed = AkimboAccumulatorState::new();
        let _ = pushed.evaluate(&board, net);
        pushed.push_move(&board, mv);
        board.make_move(mv);
        let with_ply = pushed.evaluate_search(&board, net);

        board.unmake_move(mv);
        let mut lazy = AkimboAccumulatorState::new();
        let _ = lazy.evaluate(&board, net);
        board.make_move(mv);
        let without_ply = lazy.evaluate_search(&board, net);
        assert_eq!(without_ply, with_ply);
    }

    #[test]
    fn evaluate_search_pos_matches_ply_stack_on_e2e4() {
        types::init();
        let net = super::super::network::net();
        let mut board = Board::new();
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4");

        let mut pushed = AkimboAccumulatorState::new();
        let _ = pushed.evaluate(&board, net);
        pushed.push_move(&board, mv);
        board.make_move(mv);
        let with_ply = pushed.evaluate_search(&board, net);

        board.unmake_move(mv);
        let parent = AkimboPos::from_board(&board);
        let mut from_pos_state = AkimboAccumulatorState::new();
        let _ = from_pos_state.evaluate_search_pos(&parent, net);
        from_pos_state.push_move_pos(&parent, mv);
        let mut child = parent;
        assert!(!child.make(mv));
        let from_pos = from_pos_state.evaluate_search_pos(&child, net);
        assert_eq!(from_pos, with_ply);
    }

    #[test]
    fn evaluate_search_snap_matches_ply_stack_on_e2e4() {
        types::init();
        let net = super::super::network::net();
        let mut board = Board::new();
        let mv = board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == "e2e4")
            .copied()
            .expect("e2e4");

        let mut pushed = AkimboAccumulatorState::new();
        let _ = pushed.evaluate(&board, net);
        pushed.push_move(&board, mv);
        board.make_move(mv);
        let with_ply = pushed.evaluate_search(&board, net);

        board.unmake_move(mv);
        let parent = board.snapshot();
        let mut snap_state = AkimboAccumulatorState::new();
        let _ = snap_state.evaluate_search_snap(&parent, net);
        snap_state.push_move_snap(&parent, mv);
        let mut child = parent;
        assert!(!child.make(mv));
        let from_snap = snap_state.evaluate_search_snap(&child, net);
        assert_eq!(from_snap, with_ply);
    }

    fn ply_stack_eval(board: &Board, net: &Network) -> i32 {
        let mut state = AkimboAccumulatorState::new();
        state.ensure_accurate(board, net);
        state.finish(board, net)
    }

    #[test]
    fn finny_fill_diff_matches_ply_stack_oracle() {
        types::init();
        let net = super::super::network::net();
        let start = Board::new();
        assert_eq!(
            AkimboAccumulatorState::new().evaluate(&start, net),
            ply_stack_eval(&start, net)
        );
        let mut after_e2e4 = start.clone();
        let e2e4 = after_e2e4
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == "e2e4")
            .expect("e2e4");
        after_e2e4.make_move(e2e4);
        let mut warm = AkimboAccumulatorState::new();
        let _ = warm.evaluate(&start, net);
        assert_eq!(
            warm.evaluate(&after_e2e4, net),
            ply_stack_eval(&after_e2e4, net)
        );
        let ke1 = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("fen");
        let kg1 = Board::from_fen("4k3/8/8/8/8/8/8/6K1 w - - 0 1").expect("fen");
        let mut king_walk = AkimboAccumulatorState::new();
        assert_eq!(king_walk.evaluate(&ke1, net), ply_stack_eval(&ke1, net));
        assert_eq!(king_walk.evaluate(&kg1, net), ply_stack_eval(&kg1, net));
    }

    #[test]
    fn finny_fill_diff_matches_scratch_after_quiet_and_king_moves() {
        types::init();
        let net = super::super::network::net();
        let mut board = Board::new();
        let mut state = AkimboAccumulatorState::new();
        assert_eq!(state.evaluate(&board, net), {
            let mut scratch = AkimboAccumulatorState::new();
            scratch.evaluate(&board, net)
        });
        let e2e4 = board
            .generate_legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.to_uci() == "e2e4")
            .expect("e2e4");
        board.make_move(e2e4);
        assert_eq!(
            state.evaluate(&board, net),
            AkimboAccumulatorState::new().evaluate(&board, net)
        );
        let ke1 = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("fen");
        let kg1 = Board::from_fen("4k3/8/8/8/8/8/8/6K1 w - - 0 1").expect("fen");
        let mut warm = AkimboAccumulatorState::new();
        let _ = warm.evaluate(&ke1, net);
        assert_eq!(
            warm.evaluate(&kg1, net),
            AkimboAccumulatorState::new().evaluate(&kg1, net)
        );
    }

    #[test]
    fn material_scale_from_bb8_matches_snapshot() {
        types::init();
        let board = Board::new();
        assert_eq!(
            material_scale_from_bb8(&board_bb8(&board)),
            material_scale(&board)
        );
    }
}
