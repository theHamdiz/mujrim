//! Staged move ordering with **incremental** legal generation.
//!
//! Stages:
//! 1. TT move — resolved against legal captures / quiets so [`Move::flag`] matches the board.
//! 2. Good captures (SEE ≥ 0) from [`Board::generate_legal_captures`] only.
//! 3. Killer moves
//! 4. Countermove
//! 5. Quiets — from a bucket filled early (in check / no captures) or from
//!    [`Board::generate_legal_quiets`] only when this stage is reached.
//! 6. Bad captures (SEE < 0)
//!
//! On a typical all-quiet node, if a good capture or TT move causes a beta cutoff,
//! quiet generation is skipped entirely.

use crate::see;
use types::chess_move::NULL_MOVE;
use types::{Board, Move, MoveList};

/// Maximum number of moves we track scores for.
const MAX_SCORED: usize = 256;

/// Sort `moves[..len]` and matching `scores[..len]` by score descending.
#[inline]
fn sort_moves_and_scores_desc(moves: &mut MoveList, scores: &mut [i32; MAX_SCORED], len: usize) {
    if len <= 1 {
        return;
    }
    let mut order = [0usize; MAX_SCORED];
    for i in 0..len {
        order[i] = i;
    }
    let s = &scores[..len];
    order[..len].sort_unstable_by(|&a, &b| s[b].cmp(&s[a]));

    let mut tmp_moves = [NULL_MOVE; MAX_SCORED];
    let mut tmp_scores = [0i32; MAX_SCORED];
    let mv = moves.as_mut_slice();
    for i in 0..len {
        tmp_moves[i] = mv[order[i]];
        tmp_scores[i] = scores[order[i]];
    }
    mv[..len].copy_from_slice(&tmp_moves[..len]);
    scores[..len].copy_from_slice(&tmp_scores[..len]);
}

/// Terminal position detected while constructing the picker.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PickerTerminal {
    Checkmate,
    Stalemate,
}

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

/// A staged move picker. Call `next()` repeatedly to get moves in priority order.
pub struct MovePicker {
    stage: Stage,
    tt_move: Option<Move>,
    killers: [Move; 2],
    countermove: Move,

    /// Legal captures from the position (`generate_legal_captures`).
    source_captures: MoveList,

    /// When `Some`, quiets were precomputed (check, or zero captures). When `None`,
    /// quiets are generated in `GenerateQuiets` only if reached.
    quiet_bucket: Option<MoveList>,

    captures: MoveList,
    capture_scores: [i32; MAX_SCORED],
    capture_idx: usize,
    bad_captures: MoveList,
    bad_capture_scores: [i32; MAX_SCORED],
    bad_capture_idx: usize,

    quiets: MoveList,
    quiet_scores: [i32; MAX_SCORED],
    quiet_idx: usize,

    tt_yielded: bool,
    killer0_yielded: bool,
    killer1_yielded: bool,
    cm_yielded: bool,
    killer0_emitted: bool,
    killer1_emitted: bool,
    cm_emitted: bool,
    skip_quiets: bool,
}

impl MovePicker {
    /// Build a picker: returns `Err` on checkmate / stalemate (no legal moves).
    #[inline]
    pub fn try_new(
        board: &mut Board,
        tt_move: Option<Move>,
        killers: [Move; 2],
        countermove: Move,
    ) -> Result<Self, PickerTerminal> {
        let in_check = board.in_check();
        let source_captures = board.generate_legal_captures();

        let quiet_bucket = if in_check {
            let q = board.generate_legal_quiets();
            if source_captures.is_empty() && q.is_empty() {
                return Err(PickerTerminal::Checkmate);
            }
            Some(q)
        } else if source_captures.is_empty() {
            let q = board.generate_legal_quiets();
            if q.is_empty() {
                return Err(PickerTerminal::Stalemate);
            }
            Some(q)
        } else {
            None
        };

        Ok(Self {
            stage: Stage::TtMove,
            tt_move,
            killers,
            countermove,
            source_captures,
            quiet_bucket,
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
            killer0_emitted: false,
            killer1_emitted: false,
            cm_emitted: false,
            skip_quiets: false,
        })
    }

    /// Test helper: same as [`Self::try_new`] but panics on terminal (use only on non-terminal FENs).
    #[cfg(test)]
    fn new_for_test(
        board: &mut Board,
        tt_move: Option<Move>,
        killers: [Move; 2],
        countermove: Move,
    ) -> Self {
        Self::try_new(board, tt_move, killers, countermove).expect("position must not be terminal")
    }

    /// Create a simpler structure for qsearch-only experiments (not used by main QS path).
    pub fn new_qsearch(tt_move: Option<Move>) -> Self {
        Self {
            stage: Stage::TtMove,
            tt_move,
            killers: [NULL_MOVE; 2],
            countermove: NULL_MOVE,
            source_captures: MoveList::new(),
            quiet_bucket: None,
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
            killer0_emitted: false,
            killer1_emitted: false,
            cm_emitted: false,
            skip_quiets: false,
        }
    }

    #[inline]
    pub fn skip_quiets(&mut self) {
        self.skip_quiets = true;
    }

    #[inline]
    fn is_tt_move(&self, mv: Move) -> bool {
        if let Some(ttm) = self.tt_move {
            same_move_key(mv, ttm)
        } else {
            false
        }
    }

    #[inline]
    fn find_matching_capture(&self, key: Move) -> Option<Move> {
        for i in 0..self.source_captures.len() {
            let m = self.source_captures[i];
            if same_move_key(m, key) {
                return Some(m);
            }
        }
        None
    }

    /// Lazily fill `quiet_bucket` when we need quiet list for TT / killers / scoring.
    fn ensure_quiet_bucket(&mut self, board: &mut Board) {
        if self.quiet_bucket.is_none() {
            self.quiet_bucket = Some(board.generate_legal_quiets());
        }
    }

    fn find_matching_quiet(&mut self, board: &mut Board, key: Move) -> Option<Move> {
        self.ensure_quiet_bucket(board);
        let q = self.quiet_bucket.as_ref()?;
        for i in 0..q.len() {
            let m = q[i];
            if same_move_key(m, key) {
                return Some(m);
            }
        }
        None
    }

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
                Stage::TtMove => {
                    self.stage = Stage::GenerateCaptures;
                    if let Some(ttm) = self.tt_move {
                        if let Some(mv) = self.find_matching_capture(ttm) {
                            self.tt_yielded = true;
                            return Some(mv);
                        }
                        if let Some(mv) = self.find_matching_quiet(board, ttm) {
                            self.tt_yielded = true;
                            return Some(mv);
                        }
                    }
                }

                Stage::GenerateCaptures => {
                    self.stage = Stage::GoodCaptures;
                    let mut good_count = 0usize;
                    let mut bad_count = 0usize;

                    for i in 0..self.source_captures.len() {
                        let mv = self.source_captures[i];
                        let is_capture = mv.is_capture();
                        let is_promotion = mv.is_promotion();
                        debug_assert!(is_capture || is_promotion);

                        if self.tt_yielded && self.is_tt_move(mv) {
                            continue;
                        }

                        let score = if is_capture {
                            score_capture(board, mv)
                        } else {
                            900_000
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
                    sort_moves_and_scores_desc(
                        &mut self.captures,
                        &mut self.capture_scores,
                        good_count,
                    );
                    sort_moves_and_scores_desc(
                        &mut self.bad_captures,
                        &mut self.bad_capture_scores,
                        bad_count,
                    );
                }

                Stage::GoodCaptures => {
                    if self.capture_idx < self.captures.len() {
                        let mv = self.captures.as_slice()[self.capture_idx];
                        self.capture_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::Killers;
                }

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
                            && let Some(mv) = self.find_matching_quiet(board, k)
                        {
                            self.killer0_emitted = true;
                            return Some(mv);
                        }
                    }
                    if !self.killer1_yielded {
                        self.killer1_yielded = true;
                        let k = self.killers[1];
                        if k != NULL_MOVE
                            && !self.is_tt_move(k)
                            && !k.is_capture()
                            && !k.is_promotion()
                            && let Some(mv) = self.find_matching_quiet(board, k)
                        {
                            self.killer1_emitted = true;
                            return Some(mv);
                        }
                    }
                    self.stage = Stage::Countermove;
                }

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
                            && let Some(mv) = self.find_matching_quiet(board, cm)
                        {
                            self.cm_emitted = true;
                            return Some(mv);
                        }
                    }
                }

                Stage::GenerateQuiets => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    self.stage = Stage::Quiets;
                    let quiet_src = self
                        .quiet_bucket
                        .take()
                        .unwrap_or_else(|| board.generate_legal_quiets());

                    let mut quiet_count = 0usize;
                    for i in 0..quiet_src.len() {
                        let mv = quiet_src[i];

                        if self.tt_yielded && self.is_tt_move(mv) {
                            continue;
                        }
                        if self.killer0_emitted && same_move_key(mv, self.killers[0]) {
                            continue;
                        }
                        if self.killer1_emitted && same_move_key(mv, self.killers[1]) {
                            continue;
                        }
                        if self.cm_emitted && same_move_key(mv, self.countermove) {
                            continue;
                        }

                        if quiet_count < MAX_SCORED {
                            let score = score_quiet(board, mv);
                            self.quiets.push(mv);
                            self.quiet_scores[quiet_count] = score;
                            quiet_count += 1;
                        }
                    }
                    sort_moves_and_scores_desc(
                        &mut self.quiets,
                        &mut self.quiet_scores,
                        quiet_count,
                    );
                }

                Stage::Quiets => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    if self.quiet_idx < self.quiets.len() {
                        let mv = self.quiets.as_slice()[self.quiet_idx];
                        self.quiet_idx += 1;
                        return Some(mv);
                    }
                    self.stage = Stage::BadCaptures;
                }

                Stage::BadCaptures => {
                    if self.bad_capture_idx < self.bad_captures.len() {
                        let mv = self.bad_captures.as_slice()[self.bad_capture_idx];
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
    use types::Square;
    use types::{Board, Piece};

    fn setup() {
        types::init();
    }

    #[test]
    fn test_sort_moves_and_scores_desc() {
        let mut ml = MoveList::new();
        ml.push(Move::quiet(Square::A1, Square::A2));
        ml.push(Move::quiet(Square::B1, Square::B2));
        ml.push(Move::quiet(Square::C1, Square::C2));
        let mut scores = [0i32; MAX_SCORED];
        scores[0] = 10;
        scores[1] = 30;
        scores[2] = 20;
        sort_moves_and_scores_desc(&mut ml, &mut scores, 3);
        assert_eq!(scores, {
            let mut a = [0i32; MAX_SCORED];
            a[0] = 30;
            a[1] = 20;
            a[2] = 10;
            a
        });
        assert_eq!(ml.as_slice()[0].from, Square::B1);
        assert_eq!(ml.as_slice()[1].from, Square::C1);
        assert_eq!(ml.as_slice()[2].from, Square::A1);
    }

    #[test]
    fn test_picker_yields_all_legal() {
        setup();
        let mut board = Board::new();
        let mut picker = MovePicker::new_for_test(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
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
        let tt = legal[0];

        let mut picker = MovePicker::new_for_test(&mut board, Some(tt), [NULL_MOVE; 2], NULL_MOVE);
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

        let mut picker = MovePicker::new_for_test(&mut board, Some(tt), [NULL_MOVE; 2], NULL_MOVE);
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
        let mut picker = MovePicker::new_for_test(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
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
        let mut picker = MovePicker::new_for_test(&mut board, None, [killer, NULL_MOVE], NULL_MOVE);
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

    #[test]
    fn test_picker_keeps_underpromotion_when_killer_is_same_underpromo() {
        setup();
        let mut board = Board::from_fen("7k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let underpromo = Move::promotion(types::Square::A7, types::Square::A8, Piece::Knight);
        let mut picker =
            MovePicker::new_for_test(&mut board, None, [underpromo, NULL_MOVE], NULL_MOVE);
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            moves.push(mv);
        }

        assert!(
            moves.iter().any(|m| m.to_uci() == "a7a8n"),
            "underpromotion must not be dropped by killer filtering"
        );
    }

    #[test]
    fn test_underpromotions_survive_skip_quiets() {
        setup();
        let mut board = Board::from_fen("7k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let mut picker = MovePicker::new_for_test(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
        picker.skip_quiets();
        let score_cap = |_: &Board, _: Move| -> i32 { 0 };
        let score_quiet = |_: &Board, _: Move| -> i32 { 0 };

        let mut seen = Vec::new();
        while let Some(mv) = picker.next(&mut board, &score_cap, &score_quiet) {
            seen.push(mv.to_uci());
        }

        assert!(seen.iter().any(|m| m == "a7a8n"));
        assert!(seen.iter().any(|m| m == "a7a8b"));
        assert!(seen.iter().any(|m| m == "a7a8r"));
        assert!(seen.iter().any(|m| m == "a7a8q"));
    }

    #[test]
    fn test_try_new_checkmate() {
        setup();
        let mut board =
            Board::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .unwrap();
        assert!(matches!(
            MovePicker::try_new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE),
            Err(PickerTerminal::Checkmate)
        ));
    }

    #[test]
    fn test_try_new_stalemate() {
        setup();
        let mut board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert!(matches!(
            MovePicker::try_new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE),
            Err(PickerTerminal::Stalemate)
        ));
    }
}
