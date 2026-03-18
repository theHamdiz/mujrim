//! Staged move generation — lazily yields moves in priority order.
//!
//! Stages:
//! 1. TT move  (no generation needed)
//! 2. Good captures  (SEE ≥ 0), scored by MVV-LVA + capture history
//! 3. Killer moves
//! 4. Countermove
//! 5. Quiet moves, scored by stat_score (main + continuation history)
//! 6. Bad captures  (SEE < 0)
//!
//! Legal moves are generated once and cached. If the TT move or a
//! good capture causes a beta cutoff, we never score quiets at all.

use types::{Board, Move, MoveList, Piece};
use types::chess_move::NULL_MOVE;
use crate::see;

/// Maximum number of moves we track scores for.
const MAX_SCORED: usize = 256;

/// Stages of the move picker.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Stage {
    TtMove,
    GenerateCaptures,
    GoodCaptures,
    Killers,
    Countermove,
    GenerateQuiets,
    Quiets,
    BadCaptures,
    Done,
}

/// A staged move picker. Call `next()` repeatedly to get moves in
/// priority order. Each call either returns a legal move or None.
pub struct MovePicker {
    stage: Stage,
    tt_move: Option<Move>,
    killers: [Move; 2],
    countermove: Move,

    // Cached full legal move list — generated once, reused everywhere
    legal_moves: MoveList,
    legal_generated: bool,

    // Captures: scored and split into good (SEE≥0) and bad (SEE<0)
    captures: MoveList,
    capture_scores: [i32; MAX_SCORED],
    capture_idx: usize,
    bad_captures: MoveList,
    bad_capture_scores: [i32; MAX_SCORED],
    bad_capture_idx: usize,

    // Quiets: scored lazily
    quiets: MoveList,
    quiet_scores: [i32; MAX_SCORED],
    quiet_idx: usize,

    // Track which moves were already yielded (to avoid duplicates)
    tt_yielded: bool,
    killer0_yielded: bool,
    killer1_yielded: bool,
    cm_yielded: bool,
}

impl MovePicker {
    /// Create a new MovePicker for the main search.
    pub fn new(
        tt_move: Option<Move>,
        killers: [Move; 2],
        countermove: Move,
    ) -> Self {
        Self {
            stage: Stage::TtMove,
            tt_move,
            killers,
            countermove,
            legal_moves: MoveList::new(),
            legal_generated: false,
            captures: MoveList::new(),
            capture_scores: [0; MAX_SCORED],
            capture_idx: 0,
            bad_captures: MoveList::new(),
            bad_capture_scores: [0; MAX_SCORED],
            bad_capture_idx: 0,
            quiets: MoveList::new(),
            quiet_scores: [0; MAX_SCORED],
            quiet_idx: 0,
            tt_yielded: false,
            killer0_yielded: false,
            killer1_yielded: false,
            cm_yielded: false,
        }
    }

    /// Create a simpler picker for qsearch (captures only).
    pub fn new_qsearch(tt_move: Option<Move>) -> Self {
        Self {
            stage: Stage::TtMove,
            tt_move,
            killers: [NULL_MOVE; 2],
            countermove: NULL_MOVE,
            legal_moves: MoveList::new(),
            legal_generated: false,
            captures: MoveList::new(),
            capture_scores: [0; MAX_SCORED],
            capture_idx: 0,
            bad_captures: MoveList::new(),
            bad_capture_scores: [0; MAX_SCORED],
            bad_capture_idx: 0,
            quiets: MoveList::new(),
            quiet_scores: [0; MAX_SCORED],
            quiet_idx: 0,
            tt_yielded: false,
            killer0_yielded: false,
            killer1_yielded: false,
            cm_yielded: false,
        }
    }

    /// Returns the total number of legal moves.
    /// Must call `ensure_legal_moves()` first or this returns 0.
    #[inline]
    pub fn total_legal(&self) -> usize {
        self.legal_moves.len()
    }

    /// Generate and cache the full legal move list if not already done.
    #[inline]
    pub fn ensure_legal_moves(&mut self, board: &mut Board) {
        if !self.legal_generated {
            self.legal_moves = board.generate_legal_moves();
            self.legal_generated = true;
        }
    }

    /// Get the next move in priority order.
    ///
    /// `board` must be mutable for legal move generation.
    /// `score_capture` and `score_quiet` are closures that score a move.
    pub fn next<FC, FQ>(
        &mut self,
        board: &mut Board,
        score_capture: &FC,
        score_quiet: &FQ,
    ) -> Option<Move>
    where
        FC: Fn(&Board, Move) -> i32,
        FQ: Fn(&Board, Move) -> i32,
    {
        loop {
            match self.stage {
                // ── Stage 1: TT move ──
                Stage::TtMove => {
                    self.stage = Stage::GenerateCaptures;
                    if let Some(ttm) = self.tt_move {
                        self.ensure_legal_moves(board);
                        let found = (0..self.legal_moves.len()).find(|&i| {
                            self.legal_moves[i].from == ttm.from
                                && self.legal_moves[i].to == ttm.to
                                && self.legal_moves[i].promotion == ttm.promotion
                        });
                        if let Some(idx) = found {
                            self.tt_yielded = true;
                            return Some(self.legal_moves[idx]);
                        }
                    }
                }

                // ── Stage 2: Generate captures + score + split ──
                Stage::GenerateCaptures => {
                    self.stage = Stage::GoodCaptures;
                    self.ensure_legal_moves(board);
                    let mut good_count = 0usize;
                    let mut bad_count = 0usize;

                    for i in 0..self.legal_moves.len() {
                        let mv = self.legal_moves[i];
                        let is_capture = mv.is_capture();
                        let is_queen_promo = mv.is_promotion() && mv.promotion == Some(Piece::Queen);

                        if !is_capture && !is_queen_promo { continue; }

                        // Skip TT move (already yielded)
                        if self.tt_yielded {
                            if let Some(ttm) = self.tt_move {
                                if mv.from == ttm.from && mv.to == ttm.to && mv.promotion == ttm.promotion {
                                    continue;
                                }
                            }
                        }

                        let score = if is_capture {
                            score_capture(board, mv)
                        } else {
                            900_000 // Queen promotion bonus
                        };

                        if !is_capture || see::see_ge(board, mv, 0) {
                            if good_count < MAX_SCORED {
                                self.captures.push(mv);
                                self.capture_scores[good_count] = score;
                                good_count += 1;
                            }
                        } else if bad_count < MAX_SCORED {
                            self.bad_captures.push(mv);
                            self.bad_capture_scores[bad_count] = score;
                            bad_count += 1;
                        }
                    }
                }

                // ── Stage 3: Good captures (SEE ≥ 0) ──
                Stage::GoodCaptures => {
                    if self.capture_idx < self.captures.len() {
                        let idx = self.pick_best_capture();
                        let mv = self.captures[idx];
                        self.captures.swap(self.capture_idx, idx);
                        self.capture_scores.swap(self.capture_idx, idx);
                        self.capture_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::Killers;
                }

                // ── Stage 4: Killer moves ──
                Stage::Killers => {
                    self.stage = Stage::Countermove;
                    // Try killer 0
                    if !self.killer0_yielded
                        && self.killers[0] != NULL_MOVE
                        && !self.is_tt_move_inline(self.killers[0])
                    {
                        if let Some(legal_mv) = self.find_legal_quiet_cached(self.killers[0]) {
                            self.killer0_yielded = true;
                            return Some(legal_mv);
                        }
                    }
                    // Try killer 1
                    if !self.killer1_yielded
                        && self.killers[1] != NULL_MOVE
                        && !self.is_tt_move_inline(self.killers[1])
                    {
                        if let Some(legal_mv) = self.find_legal_quiet_cached(self.killers[1]) {
                            self.killer1_yielded = true;
                            return Some(legal_mv);
                        }
                    }
                }

                // ── Stage 5: Countermove ──
                Stage::Countermove => {
                    self.stage = Stage::GenerateQuiets;
                    if !self.cm_yielded
                        && self.countermove != NULL_MOVE
                        && !self.is_tt_move_inline(self.countermove)
                        && !self.is_killer_inline(self.countermove)
                    {
                        if let Some(legal_mv) = self.find_legal_quiet_cached(self.countermove) {
                            self.cm_yielded = true;
                            return Some(legal_mv);
                        }
                    }
                }

                // ── Stage 6: Generate quiets (from cached legal moves) ──
                Stage::GenerateQuiets => {
                    self.stage = Stage::Quiets;
                    let mut q_count = 0usize;
                    for i in 0..self.legal_moves.len() {
                        let mv = self.legal_moves[i];
                        if mv.is_capture() { continue; }
                        if mv.is_promotion() && mv.promotion == Some(Piece::Queen) { continue; }
                        if self.is_tt_move_inline(mv) { continue; }
                        if self.is_killer_inline(mv) { continue; }
                        if self.is_countermove_inline(mv) { continue; }
                        if q_count < MAX_SCORED {
                            let score = score_quiet(board, mv);
                            self.quiets.push(mv);
                            self.quiet_scores[q_count] = score;
                            q_count += 1;
                        }
                    }
                }

                // ── Stage 7: Quiet moves ──
                Stage::Quiets => {
                    if self.quiet_idx < self.quiets.len() {
                        let idx = self.pick_best_quiet();
                        let mv = self.quiets[idx];
                        self.quiets.swap(self.quiet_idx, idx);
                        self.quiet_scores.swap(self.quiet_idx, idx);
                        self.quiet_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::BadCaptures;
                }

                // ── Stage 8: Bad captures (SEE < 0) ──
                Stage::BadCaptures => {
                    if self.bad_capture_idx < self.bad_captures.len() {
                        let idx = self.pick_best_bad_capture();
                        let mv = self.bad_captures[idx];
                        self.bad_captures.swap(self.bad_capture_idx, idx);
                        self.bad_capture_scores.swap(self.bad_capture_idx, idx);
                        self.bad_capture_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::Done;
                }

                Stage::Done => return None,
            }
        }
    }

    // ── Helper methods (inlined, no &self borrows to avoid conflicts) ──

    #[inline(always)]
    fn is_tt_move_inline(&self, mv: Move) -> bool {
        if let Some(ttm) = self.tt_move {
            mv.from == ttm.from && mv.to == ttm.to && mv.promotion == ttm.promotion
        } else {
            false
        }
    }

    #[inline(always)]
    fn is_killer_inline(&self, mv: Move) -> bool {
        (self.killers[0] != NULL_MOVE && mv.from == self.killers[0].from && mv.to == self.killers[0].to)
            || (self.killers[1] != NULL_MOVE && mv.from == self.killers[1].from && mv.to == self.killers[1].to)
    }

    #[inline(always)]
    fn is_countermove_inline(&self, mv: Move) -> bool {
        self.countermove != NULL_MOVE && mv.from == self.countermove.from && mv.to == self.countermove.to
    }

    /// Find the matching legal quiet move from cached legal moves.
    /// Returns the actual legal move (with correct flags) or None.
    fn find_legal_quiet_cached(&self, mv: Move) -> Option<Move> {
        (0..self.legal_moves.len()).find_map(|i| {
            let lm = self.legal_moves[i];
            if lm.from == mv.from && lm.to == mv.to
                && !lm.is_capture()
                && lm.promotion == mv.promotion
            {
                Some(lm)
            } else {
                None
            }
        })
    }

    #[inline]
    fn pick_best_capture(&self) -> usize {
        let mut best = self.capture_idx;
        for i in (self.capture_idx + 1)..self.captures.len() {
            if self.capture_scores[i] > self.capture_scores[best] {
                best = i;
            }
        }
        best
    }

    #[inline]
    fn pick_best_quiet(&self) -> usize {
        let mut best = self.quiet_idx;
        for i in (self.quiet_idx + 1)..self.quiets.len() {
            if self.quiet_scores[i] > self.quiet_scores[best] {
                best = i;
            }
        }
        best
    }

    #[inline]
    fn pick_best_bad_capture(&self) -> usize {
        let mut best = self.bad_capture_idx;
        for i in (self.bad_capture_idx + 1)..self.bad_captures.len() {
            if self.bad_capture_scores[i] > self.bad_capture_scores[best] {
                best = i;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Board;

    #[test]
    fn test_movepicker_yields_all_legal_moves() {
        types::init();
        let mut board = Board::new();
        let legal = board.generate_legal_moves();
        let legal_count = legal.len();

        let mut picker = MovePicker::new(None, [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_b: &Board, _m: Move| -> i32 { 0 };
        let score_quiet = |_b: &Board, _m: Move| -> i32 { 0 };

        let mut picked = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            picked.push(mv);
        }

        assert_eq!(picked.len(), legal_count,
            "MovePicker should yield all {} legal moves, got {}", legal_count, picked.len());
    }

    #[test]
    fn test_movepicker_tt_move_first() {
        types::init();
        let mut board = Board::new();
        let legal = board.generate_legal_moves();
        let tt_mv = legal[5]; // some arbitrary legal move

        let mut picker = MovePicker::new(Some(tt_mv), [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_b: &Board, _m: Move| -> i32 { 0 };
        let score_quiet = |_b: &Board, _m: Move| -> i32 { 0 };

        let first = picker.next(&mut board, &score_cap, &score_quiet).unwrap();
        assert_eq!(first.from, tt_mv.from, "First move should be TT move");
        assert_eq!(first.to, tt_mv.to, "First move should be TT move");
    }

    #[test]
    fn test_movepicker_no_duplicates() {
        types::init();
        let mut board = Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1").unwrap();
        let legal = board.generate_legal_moves();
        let tt_mv = if !legal.is_empty() { Some(legal[0]) } else { None };

        let mut picker = MovePicker::new(tt_mv, [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_b: &Board, _m: Move| -> i32 { 0 };
        let score_quiet = |_b: &Board, _m: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            moves.push((mv.from, mv.to, mv.promotion));
        }

        let unique_count = {
            let mut sorted = moves.clone();
            sorted.sort_by_key(|m| (m.0.index(), m.1.index()));
            sorted.dedup();
            sorted.len()
        };

        assert_eq!(moves.len(), unique_count,
            "MovePicker should not yield duplicates: {} total, {} unique", moves.len(), unique_count);
    }

    #[test]
    fn test_movepicker_total_legal() {
        types::init();
        let mut board = Board::new();
        let expected = board.generate_legal_moves().len();

        let mut picker = MovePicker::new(None, [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_b: &Board, _m: Move| -> i32 { 0 };
        let score_quiet = |_b: &Board, _m: Move| -> i32 { 0 };

        // First call triggers generation
        let _ = picker.next(&mut board, &score_cap, &score_quiet);
        assert_eq!(picker.total_legal(), expected,
            "total_legal() should be {} after first next(), got {}", expected, picker.total_legal());
    }
}
