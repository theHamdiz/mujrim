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
//! **Key optimization**: All legal moves are generated once upfront, but
//! captures are scored and yielded BEFORE quiets. If a TT move or good
//! capture causes a beta cutoff, quiet moves are never scored — saving
//! the expensive stat_score computation on ~40% of nodes.

use crate::see;
use types::chess_move::NULL_MOVE;
use types::{Board, Move, MoveList, Piece};

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

    // Quiets: scored lazily (only when we reach quiet stage)
    quiets: MoveList,
    quiet_scores: [i32; MAX_SCORED],
    quiet_idx: usize,

    // Track which moves were already yielded (to avoid duplicates)
    tt_yielded: bool,
    killer0_yielded: bool,
    killer1_yielded: bool,
    cm_yielded: bool,
    skip_quiets: bool,
}

impl MovePicker {
    /// Create a new MovePicker for the main search.
    pub fn new(tt_move: Option<Move>, killers: [Move; 2], countermove: Move) -> Self {
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
            skip_quiets: false,
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
            skip_quiets: false,
        }
    }

    /// Skip all quiet move stages from now on.
    #[inline]
    pub fn skip_quiets(&mut self) {
        self.skip_quiets = true;
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

    /// Check if a move matches the TT move.
    #[inline]
    fn is_tt_move(&self, mv: Move) -> bool {
        if let Some(ttm) = self.tt_move {
            same_move_key(mv, ttm)
        } else {
            false
        }
    }

    /// Check if a move matches any killer or countermove.
    #[inline]
    fn is_killer_or_cm(&self, mv: Move) -> bool {
        let k0 = self.killers[0];
        let k1 = self.killers[1];
        let cm = self.countermove;
        (k0 != NULL_MOVE && same_move_key(mv, k0))
            || (k1 != NULL_MOVE && same_move_key(mv, k1))
            || (cm != NULL_MOVE && same_move_key(mv, cm))
    }

    /// Get the next move in priority order.
    ///
    /// `board` must be mutable for legal move generation.
    /// `score_capture` and `score_quiet` are closures that score a move.
    ///
    /// **Key optimization**: Quiet moves are only scored when we reach the
    /// quiet stage. If a TT move or good capture causes a beta cutoff,
    /// we skip the expensive stat_score computation entirely.
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
                        let found = (0..self.legal_moves.len())
                            .find(|&i| same_move_key(self.legal_moves[i], ttm));
                        if let Some(idx) = found {
                            self.tt_yielded = true;
                            return Some(self.legal_moves[idx]);
                        }
                    }
                }

                // ── Stage 2: Generate captures + score + split ──
                // Only captures are scored here — quiets deferred to stage 6.
                Stage::GenerateCaptures => {
                    self.stage = Stage::GoodCaptures;
                    self.ensure_legal_moves(board);
                    let mut good_count = 0usize;
                    let mut bad_count = 0usize;

                    for i in 0..self.legal_moves.len() {
                        let mv = self.legal_moves[i];
                        let is_capture = mv.is_capture();
                        let is_queen_promo =
                            mv.is_promotion() && mv.promotion == Some(Piece::Queen);

                        if !is_capture && !is_queen_promo {
                            continue;
                        }

                        // Skip TT move (already yielded)
                        if self.tt_yielded && self.is_tt_move(mv) {
                            continue;
                        }

                        let score = if is_capture {
                            score_capture(board, mv)
                        } else {
                            900_000 // Queen promotion bonus
                        };

                        if !is_capture || see::see_ge(board, mv, 0) {
                            // Good capture or queen promotion
                            if good_count < MAX_SCORED {
                                self.captures.push(mv);
                                self.capture_scores[good_count] = score;
                                good_count += 1;
                            }
                        } else {
                            // Losing capture (SEE < 0)
                            if bad_count < MAX_SCORED {
                                self.bad_captures.push(mv);
                                self.bad_capture_scores[bad_count] = score;
                                bad_count += 1;
                            }
                        }
                    }
                }

                // ── Stage 3: Yield good captures in score order ──
                Stage::GoodCaptures => {
                    if self.capture_idx < self.captures.len() {
                        // Incremental sort: find best remaining
                        let mut best = self.capture_idx;
                        for j in (self.capture_idx + 1)..self.captures.len() {
                            if self.capture_scores[j] > self.capture_scores[best] {
                                best = j;
                            }
                        }
                        self.captures.swap(self.capture_idx, best);
                        self.capture_scores.swap(self.capture_idx, best);
                        let mv = self.captures[self.capture_idx];
                        self.capture_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::Killers;
                }

                // ── Stage 4: Killer moves ──
                Stage::Killers => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    if !self.killer0_yielded {
                        self.killer0_yielded = true;
                        let k = self.killers[0];
                        if k != NULL_MOVE
                            && !self.is_tt_move(k)
                            && !k.is_capture()
                            && !k.is_promotion()
                        {
                            // Find the matching legal move (with correct flags)
                            let legal = (0..self.legal_moves.len())
                                .find(|&i| same_move_key(self.legal_moves[i], k));
                            if let Some(idx) = legal {
                                return Some(self.legal_moves[idx]);
                            }
                        }
                    }
                    if !self.killer1_yielded {
                        self.killer1_yielded = true;
                        let k = self.killers[1];
                        if k != NULL_MOVE
                            && !self.is_tt_move(k)
                            && !k.is_capture()
                            && !k.is_promotion()
                        {
                            let legal = (0..self.legal_moves.len())
                                .find(|&i| same_move_key(self.legal_moves[i], k));
                            if let Some(idx) = legal {
                                return Some(self.legal_moves[idx]);
                            }
                        }
                    }
                    self.stage = Stage::Countermove;
                }

                // ── Stage 5: Countermove ──
                Stage::Countermove => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    self.stage = Stage::GenerateQuiets;
                    if !self.cm_yielded {
                        self.cm_yielded = true;
                        let cm = self.countermove;
                        if cm != NULL_MOVE
                            && !self.is_tt_move(cm)
                            && !cm.is_capture()
                            && !cm.is_promotion()
                            && !same_move_key(cm, self.killers[0])
                            && !same_move_key(cm, self.killers[1])
                        {
                            let legal = (0..self.legal_moves.len())
                                .find(|&i| same_move_key(self.legal_moves[i], cm));
                            if let Some(idx) = legal {
                                return Some(self.legal_moves[idx]);
                            }
                        }
                    }
                }

                // ── Stage 6: Generate + score quiets (DEFERRED scoring) ──
                // This is the key optimization: quiet scoring only happens
                // if we didn't cut off on captures, TT, or killers.
                Stage::GenerateQuiets => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    self.stage = Stage::Quiets;
                    let mut quiet_count = 0usize;

                    for i in 0..self.legal_moves.len() {
                        let mv = self.legal_moves[i];
                        // Skip captures and queen promotions (already handled)
                        if mv.is_capture() {
                            continue;
                        }
                        if mv.is_promotion() && mv.promotion == Some(Piece::Queen) {
                            continue;
                        }

                        // Skip already-yielded moves
                        if self.tt_yielded && self.is_tt_move(mv) {
                            continue;
                        }
                        if self.is_killer_or_cm(mv) {
                            continue;
                        }

                        if quiet_count < MAX_SCORED {
                            let score = score_quiet(board, mv);
                            self.quiets.push(mv);
                            self.quiet_scores[quiet_count] = score;
                            quiet_count += 1;
                        }
                    }
                }

                // ── Stage 7: Yield quiets in score order ──
                Stage::Quiets => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    if self.quiet_idx < self.quiets.len() {
                        let mut best = self.quiet_idx;
                        for j in (self.quiet_idx + 1)..self.quiets.len() {
                            if self.quiet_scores[j] > self.quiet_scores[best] {
                                best = j;
                            }
                        }
                        self.quiets.swap(self.quiet_idx, best);
                        self.quiet_scores.swap(self.quiet_idx, best);
                        let mv = self.quiets[self.quiet_idx];
                        self.quiet_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::BadCaptures;
                }

                // ── Stage 8: Bad captures (SEE < 0) ──
                Stage::BadCaptures => {
                    if self.bad_capture_idx < self.bad_captures.len() {
                        let mut best = self.bad_capture_idx;
                        for j in (self.bad_capture_idx + 1)..self.bad_captures.len() {
                            if self.bad_capture_scores[j] > self.bad_capture_scores[best] {
                                best = j;
                            }
                        }
                        self.bad_captures.swap(self.bad_capture_idx, best);
                        self.bad_capture_scores.swap(self.bad_capture_idx, best);
                        let mv = self.bad_captures[self.bad_capture_idx];
                        self.bad_capture_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::Done;
                }

                Stage::Done => return None,
            }
        }
    }
}

#[inline(always)]
fn same_move_key(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Board;

    fn setup() {
        types::init();
    }

    #[test]
    fn test_picker_yields_all_legal() {
        setup();
        let mut board = Board::new();
        let mut picker = MovePicker::new(None, [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            moves.push(mv);
        }

        let legal = board.generate_legal_moves();
        assert_eq!(
            moves.len(),
            legal.len(),
            "Picker yielded {} moves but {} legal moves exist",
            moves.len(),
            legal.len()
        );
    }

    #[test]
    fn test_picker_tt_move_first() {
        setup();
        let mut board = Board::new();
        let legal = board.generate_legal_moves();
        let tt = legal[0]; // Use first legal move as TT move

        let mut picker = MovePicker::new(Some(tt), [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let first = picker.next(&mut board, &score_cap, &score_quiet);
        assert!(first.is_some());
        let first = first.unwrap();
        assert_eq!(first.from, tt.from);
        assert_eq!(first.to, tt.to);
    }

    #[test]
    fn test_picker_no_duplicates() {
        setup();
        let mut board = Board::new();
        let legal = board.generate_legal_moves();
        let tt = legal[0];

        let mut picker = MovePicker::new(Some(tt), [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            let count = moves
                .iter()
                .filter(|m: &&Move| {
                    m.from == mv.from && m.to == mv.to && m.promotion == mv.promotion
                })
                .count();
            assert_eq!(count, 0, "Duplicate move: {mv}");
            moves.push(mv);
        }
    }

    #[test]
    fn test_picker_complex_position() {
        setup();
        let mut board =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let mut picker = MovePicker::new(None, [NULL_MOVE; 2], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            moves.push(mv);
        }

        let legal = board.generate_legal_moves();
        assert_eq!(
            moves.len(),
            legal.len(),
            "KiwiPete: picker={} vs legal={}",
            moves.len(),
            legal.len()
        );
    }

    #[test]
    fn test_picker_keeps_underpromotions_when_killer_exists() {
        setup();
        let mut board = Board::from_fen("7k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let killer = Move::promotion(types::Square::A7, types::Square::A8, Piece::Queen);
        let mut picker = MovePicker::new(None, [killer, NULL_MOVE], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            moves.push(mv);
        }

        let legal = board.generate_legal_moves();
        assert_eq!(moves.len(), legal.len());
        assert!(moves.iter().any(|m| m.to_uci() == "a7a8n"));
        assert!(moves.iter().any(|m| m.to_uci() == "a7a8b"));
        assert!(moves.iter().any(|m| m.to_uci() == "a7a8r"));
    }
}
