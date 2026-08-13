//! Staged move ordering with incremental legality checks.
//!
//! Stages:
//! 1. TT move — resolved against legal captures / quiets so [`Move::flag`] matches the board.
//! 2. Good captures (SEE ≥ 0)
//! 3. Killer moves
//! 4. Countermove
//! 5. Quiets — from a bucket filled early (in check / no captures) or from
//!    pseudo-legal generation only when this stage is reached.
//! 6. Bad captures (SEE < 0)
//!
//! On a typical all-quiet node, if a good capture or TT move causes a beta cutoff,
//! quiet generation is skipped entirely.

use crate::policy::MoveOrderingProfile;
use crate::see;
use std::mem::MaybeUninit;
use types::chess_move::NULL_MOVE;
use types::{Board, Move, MoveList};

/// Maximum number of moves we track scores for.
const MAX_SCORED: usize = 256;
type ScoreBuffer = [MaybeUninit<i32>; MAX_SCORED];
type EligibilityBuffer = [MaybeUninit<bool>; MAX_SCORED];

/// Select the highest-scoring remaining move without ordering unused moves.
#[inline]
fn pick_best(moves: &mut MoveList, scores: &mut ScoreBuffer, index: usize) -> Option<Move> {
    let len = moves.len();
    if index >= len {
        return None;
    }

    let mut best = index;
    // SAFETY: callers initialize the complete active move prefix before selection.
    let mut best_score = unsafe { scores[index].assume_init() };
    for (candidate, entry) in scores.iter().enumerate().take(len).skip(index + 1) {
        // SAFETY: `candidate < len`, which is inside the initialized prefix.
        let score = unsafe { entry.assume_init() };
        // `Iterator::max_by_key`, used by the former implementation, selects
        // the last equal maximum. Preserve that deterministic tie-break.
        if score >= best_score {
            best = candidate;
            best_score = score;
        }
    }
    moves.swap(index, best);
    scores.swap(index, best);
    Some(moves[index])
}

#[inline]
fn pick_best_matching(
    moves: &mut MoveList,
    scores: &mut ScoreBuffer,
    eligible: &mut EligibilityBuffer,
    index: usize,
) -> Option<Move> {
    let len = moves.len();
    let mut best = None;
    let mut best_score = i32::MIN;
    for candidate in index.min(len)..len {
        // SAFETY: both buffers are initialized for the complete active prefix.
        let is_eligible = unsafe { eligible[candidate].assume_init() };
        if is_eligible {
            // SAFETY: `candidate < len`, which is inside the initialized prefix.
            let score = unsafe { scores[candidate].assume_init() };
            if best.is_none() || score >= best_score {
                best = Some(candidate);
                best_score = score;
            }
        }
    }
    let best = best?;
    moves.swap(index, best);
    scores.swap(index, best);
    eligible.swap(index, best);
    Some(moves[index])
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

    captures: MoveList,
    capture_scores: ScoreBuffer,
    capture_good: EligibilityBuffer,
    capture_idx: usize,

    /// Quiets are generated lazily unless terminal detection needed them early.
    quiets: Option<MoveList>,
    quiet_scores: ScoreBuffer,
    quiet_idx: usize,

    tt_yielded: bool,
    killer0_yielded: bool,
    killer1_yielded: bool,
    cm_yielded: bool,
    killer0_emitted: bool,
    killer1_emitted: bool,
    cm_emitted: bool,
    skip_quiets: bool,
    skip_bad_captures: bool,
    move_ordering: MoveOrderingProfile,
}

const _: () = assert!(std::mem::size_of::<MovePicker>() <= 5 * 1024);

impl MovePicker {
    /// Builds a picker without validating moves that search may never consume.
    #[inline]
    pub fn new(
        board: &mut Board,
        tt_move: Option<Move>,
        killers: [Move; 2],
        countermove: Move,
    ) -> Self {
        let captures = board.generate_captures(board.side_to_move);

        Self {
            stage: Stage::TtMove,
            tt_move,
            killers,
            countermove,
            captures,
            capture_scores: [MaybeUninit::uninit(); MAX_SCORED],
            capture_good: [MaybeUninit::uninit(); MAX_SCORED],
            capture_idx: 0,
            quiets: None,
            quiet_scores: [MaybeUninit::uninit(); MAX_SCORED],
            quiet_idx: 0,
            tt_yielded: false,
            killer0_yielded: false,
            killer1_yielded: false,
            cm_yielded: false,
            killer0_emitted: false,
            killer1_emitted: false,
            cm_emitted: false,
            skip_quiets: false,
            skip_bad_captures: false,
            move_ordering: MoveOrderingProfile::StockLike,
        }
    }

    /// Test helper matching [`Self::new`].
    #[cfg(test)]
    fn new_for_test(
        board: &mut Board,
        tt_move: Option<Move>,
        killers: [Move; 2],
        countermove: Move,
    ) -> Self {
        Self::new(board, tt_move, killers, countermove)
    }

    /// Create a simpler structure for qsearch-only experiments (not used by main QS path).
    pub fn new_qsearch(tt_move: Option<Move>) -> Self {
        Self {
            stage: Stage::TtMove,
            tt_move,
            killers: [NULL_MOVE; 2],
            countermove: NULL_MOVE,
            captures: MoveList::new(),
            capture_scores: [MaybeUninit::uninit(); MAX_SCORED],
            capture_good: [MaybeUninit::uninit(); MAX_SCORED],
            capture_idx: 0,
            quiets: None,
            quiet_scores: [MaybeUninit::uninit(); MAX_SCORED],
            quiet_idx: 0,
            tt_yielded: false,
            killer0_yielded: false,
            killer1_yielded: false,
            cm_yielded: false,
            killer0_emitted: false,
            killer1_emitted: false,
            cm_emitted: false,
            skip_quiets: false,
            skip_bad_captures: false,
            move_ordering: MoveOrderingProfile::StockLike,
        }
    }

    #[inline]
    pub fn with_move_ordering(mut self, profile: MoveOrderingProfile) -> Self {
        self.move_ordering = profile;
        self
    }

    #[inline]
    pub fn skip_quiets(&mut self) {
        self.skip_quiets = true;
    }

    #[inline]
    pub fn skip_bad_captures(&mut self) {
        self.skip_bad_captures = true;
    }

    #[inline(always)]
    pub fn is_bad_capture_stage(&self) -> bool {
        self.stage == Stage::BadCaptures
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
        for i in 0..self.captures.len() {
            let m = self.captures[i];
            if same_move_key(m, key) {
                return Some(m);
            }
        }
        None
    }

    /// Lazily fill the quiet list when it is first needed.
    fn ensure_quiets(&mut self, board: &mut Board) {
        if self.quiets.is_none() {
            self.quiets = Some(board.generate_pseudo_legal_quiets(board.side_to_move));
        }
    }

    fn find_matching_quiet(&mut self, board: &mut Board, key: Move) -> Option<Move> {
        self.ensure_quiets(board);
        let q = self.quiets.as_ref()?;
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
                        if let Some(mv) = self.find_matching_capture(ttm)
                            && board.is_legal_move(mv)
                            && (!self.skip_bad_captures
                                || !mv.is_capture()
                                || see::see_ge(board, mv, 0))
                        {
                            self.tt_yielded = true;
                            return Some(mv);
                        }
                        if !self.skip_quiets
                            && let Some(mv) = self.find_matching_quiet(board, ttm)
                            && board.is_legal_move(mv)
                        {
                            self.tt_yielded = true;
                            return Some(mv);
                        }
                    }
                }

                Stage::GenerateCaptures => {
                    self.stage = Stage::GoodCaptures;
                    let mut write = 0usize;
                    let capture_count = self.captures.len();

                    for i in 0..capture_count {
                        let mv = self.captures[i];
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

                        self.captures.as_mut_slice()[write] = mv;
                        self.capture_scores[write].write(score);
                        let threshold = self.move_ordering.noisy_see_threshold(score);
                        self.capture_good[write]
                            .write(!is_capture || see::see_ge(board, mv, threshold));
                        write += 1;
                    }
                    self.captures.truncate(write);
                }

                Stage::GoodCaptures => {
                    if let Some(mv) = pick_best_matching(
                        &mut self.captures,
                        &mut self.capture_scores,
                        &mut self.capture_good,
                        self.capture_idx,
                    ) {
                        self.capture_idx += 1;
                        if board.is_legal_move(mv) {
                            return Some(mv);
                        }
                        continue;
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
                            && board.is_legal_move(mv)
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
                            && board.is_legal_move(mv)
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
                            && board.is_legal_move(mv)
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
                    let mut quiets = self
                        .quiets
                        .take()
                        .unwrap_or_else(|| board.generate_pseudo_legal_quiets(board.side_to_move));

                    let mut quiet_count = 0usize;
                    let generated_count = quiets.len();
                    for i in 0..generated_count {
                        let mv = quiets[i];

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
                            quiets.as_mut_slice()[quiet_count] = mv;
                            self.quiet_scores[quiet_count].write(score);
                            quiet_count += 1;
                        }
                    }
                    quiets.truncate(quiet_count);
                    self.quiets = Some(quiets);
                }

                Stage::Quiets => {
                    if self.skip_quiets {
                        self.stage = Stage::BadCaptures;
                        continue;
                    }
                    let next = self.quiets.as_mut().and_then(|quiets| {
                        pick_best(quiets, &mut self.quiet_scores, self.quiet_idx)
                    });
                    if let Some(mv) = next {
                        self.quiet_idx += 1;
                        if board.is_legal_move(mv) {
                            return Some(mv);
                        }
                        continue;
                    }
                    self.stage = Stage::BadCaptures;
                }

                Stage::BadCaptures => {
                    if self.skip_bad_captures {
                        self.stage = Stage::Done;
                        continue;
                    }
                    if let Some(mv) = pick_best(
                        &mut self.captures,
                        &mut self.capture_scores,
                        self.capture_idx,
                    ) {
                        self.capture_idx += 1;
                        if board.is_legal_move(mv) {
                            return Some(mv);
                        }
                        continue;
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
    fn picker_stays_small_enough_for_recursive_search() {
        assert!(std::mem::size_of::<MovePicker>() <= 5 * 1024);
    }

    #[test]
    fn move_ordering_profile_is_selected_without_allocation() {
        setup();
        let mut board = Board::new();
        let picker = MovePicker::new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE)
            .with_move_ordering(MoveOrderingProfile::Reckless);
        assert_eq!(picker.move_ordering, MoveOrderingProfile::Reckless);
    }

    #[test]
    fn picker_reports_bad_capture_stage_for_losing_noisy_moves() {
        setup();
        let mut board = Board::from_fen("r3k3/p7/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        let score_capture = |_: &Board, _: Move| 0;
        let score_quiet = |_: &Board, _: Move| 0;
        let mut picker = MovePicker::new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
        while let Some(mv) = picker.next(&mut board, &score_capture, &score_quiet) {
            if mv.to_uci() == "a1a7" {
                assert!(picker.is_bad_capture_stage());
                return;
            }
        }
        panic!("expected losing queen capture");
    }

    #[test]
    fn test_pick_best_orders_moves_incrementally() {
        let mut ml = MoveList::new();
        ml.push(Move::quiet(Square::A1, Square::A2));
        ml.push(Move::quiet(Square::B1, Square::B2));
        ml.push(Move::quiet(Square::C1, Square::C2));
        let mut scores = [MaybeUninit::uninit(); MAX_SCORED];
        scores[0].write(10);
        scores[1].write(30);
        scores[2].write(20);
        assert_eq!(pick_best(&mut ml, &mut scores, 0).unwrap().from, Square::B1);
        assert_eq!(pick_best(&mut ml, &mut scores, 1).unwrap().from, Square::C1);
        assert_eq!(pick_best(&mut ml, &mut scores, 2).unwrap().from, Square::A1);
        assert!(pick_best(&mut ml, &mut scores, 3).is_none());
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
    fn qsearch_mode_omits_losing_captures() {
        setup();
        let mut board = Board::from_fen("r3k3/p7/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        let losing = Move::capture(types::Square::A1, types::Square::A7);
        assert!(!see::see_ge(&board, losing, 0));

        let mut picker = MovePicker::new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
        picker.skip_quiets();
        picker.skip_bad_captures();
        let mut moves = Vec::new();
        while let Some(mv) = picker.next(&mut board, &|_, _| 0, &|_, _| 0) {
            moves.push(mv);
        }
        assert!(!moves.iter().any(|mv| same_move_key(*mv, losing)));

        let mut tt_picker = MovePicker::new(&mut board, Some(losing), [NULL_MOVE; 2], NULL_MOVE);
        tt_picker.skip_quiets();
        tt_picker.skip_bad_captures();
        assert!(tt_picker.next(&mut board, &|_, _| 0, &|_, _| 0).is_none());

        let mut start = Board::new();
        let quiet = Move::quiet(types::Square::E2, types::Square::E4);
        let mut quiet_picker = MovePicker::new(&mut start, Some(quiet), [NULL_MOVE; 2], NULL_MOVE);
        quiet_picker.skip_quiets();
        quiet_picker.skip_bad_captures();
        assert!(
            quiet_picker
                .next(&mut start, &|_, _| 0, &|_, _| 0)
                .is_none()
        );
    }

    #[test]
    fn checkmate_position_yields_no_moves() {
        setup();
        let mut board =
            Board::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .unwrap();
        let mut picker = MovePicker::new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
        assert!(picker.next(&mut board, &|_, _| 0, &|_, _| 0).is_none());
    }

    #[test]
    fn stalemate_position_yields_no_moves() {
        setup();
        let mut board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        let mut picker = MovePicker::new(&mut board, None, [NULL_MOVE; 2], NULL_MOVE);
        assert!(picker.next(&mut board, &|_, _| 0, &|_, _| 0).is_none());
    }
}
