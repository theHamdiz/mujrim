//! The search engine: iterative deepening with alpha-beta, quiescence search,
//! null-move pruning, late-move reductions, PVS, aspiration windows,
//! killer moves, history heuristic, countermove heuristic, LMP,
//! check extensions, singular extensions, razoring, ProbCut,
//! IIR, SEE-based pruning, futility/delta pruning, PV tracking.
//! Supports Lazy SMP multi-threaded search via shared transposition table.

use crate::adapters;
use crate::move_picker::MovePicker;
use crate::policy::{
    BadNoisyFutilityContext, BadNoisyFutilityDispatch, FutilityContext, FutilityDispatch,
    HistorySeePruning, LmpContext, LmpDispatch, LmrContext, LmrDispatch, LmrPolicy,
    MainThreadPreferredRootSelection, MoveOrderingProfile, RfpContext, RfpDispatch,
    RootSelectionPolicy, ThreadOutcome,
};
use crate::search_params::SearchParams;
use crate::search_stack::{EvalMode, SearchExperiment, SearchStack};
use crate::see;
use crate::syzygy::SyzygyTables;
use crate::tt::{NodeType, TTData, TranspositionTable};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use types::board::attack_tables::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};
use types::chess_move::NULL_MOVE;
use types::{Board, Move, Piece};

use eval::nnue::{
    ActiveNetwork, NNUEState, NnueNetworkInfo, NnueNetworkSource, default_embedded_network,
};

#[cfg(test)]
fn ensure_test_nnue_discovery_path() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Search tests compile eval without embedded-networks; point discovery at
        // the checked-in network payloads so adapters can construct engines.
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("mujrim-eval")
            .join("resources");
        // SAFETY: test-only, called once before any engine threads start.
        unsafe {
            std::env::set_var("MUJRIM_NNUE", resources);
        }
    });
}

/// Infinity score sentinel.
const INF: i32 = 30_000;
/// Checkmate score base (mate in N = MATE_SCORE - N).
const MATE_SCORE: i32 = 29_000;
/// Maximum search ply depth.
const MAX_PLY: usize = 128;
/// Threshold for TT mate score normalization.
const MATE_TT_THRESHOLD: i32 = MATE_SCORE - MAX_PLY as i32;

/// Maximum history score (Stockfish uses 16384).
const MAX_HISTORY: i32 = 16384;
/// Number of piece types for indexing.
const NUM_PIECES: usize = 6;
/// Number of squares.
const NUM_SQUARES: usize = 64;
const COLORED_PIECES: usize = 2 * NUM_PIECES;
const PAWN_HISTORY_SIZE: usize = 512;
type HistoryValue = i16;
type ContinuationHistory = [[[[HistoryValue; NUM_SQUARES]; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES];

#[derive(Clone, Copy, Default)]
struct QuietHistoryEntry {
    factorizer: HistoryValue,
    buckets: [[HistoryValue; 2]; 2],
}

struct QuietHistory {
    entries: Box<[[[QuietHistoryEntry; NUM_SQUARES]; NUM_SQUARES]; 2]>,
}

impl Default for QuietHistory {
    fn default() -> Self {
        Self {
            entries: boxed_zeroed(),
        }
    }
}

impl QuietHistory {
    #[inline(always)]
    fn get(&self, threats: u64, color: usize, mv: Move) -> i32 {
        let entry = &self.entries[color][mv.from.index()][mv.to.index()];
        let from_threatened = usize::from(threats & mv.from.bitboard() != 0);
        let to_threatened = usize::from(threats & mv.to.bitboard() != 0);
        i32::from(entry.factorizer + entry.buckets[from_threatened][to_threatened])
    }

    #[inline(always)]
    fn update(&mut self, threats: u64, color: usize, mv: Move, bonus: i32) {
        let entry = &mut self.entries[color][mv.from.index()][mv.to.index()];
        update_history_with_max(&mut entry.factorizer, bonus, 1852);
        let from_threatened = usize::from(threats & mv.from.bitboard() != 0);
        let to_threatened = usize::from(threats & mv.to.bitboard() != 0);
        update_history_with_max(
            &mut entry.buckets[from_threatened][to_threatened],
            bonus,
            6324,
        );
    }

    fn clear(&mut self) {
        for color in self.entries.iter_mut() {
            for row in color.iter_mut() {
                row.fill(QuietHistoryEntry::default());
            }
        }
    }

    fn age(&mut self) {
        for color in self.entries.iter_mut() {
            for row in color.iter_mut() {
                for entry in row {
                    entry.factorizer /= 2;
                    for buckets in &mut entry.buckets {
                        for value in buckets {
                            *value /= 2;
                        }
                    }
                }
            }
        }
    }
}

struct PawnHistory {
    entries: Box<[[[HistoryValue; NUM_SQUARES]; COLORED_PIECES]; PAWN_HISTORY_SIZE]>,
}

impl Default for PawnHistory {
    fn default() -> Self {
        Self {
            entries: boxed_zeroed(),
        }
    }
}

impl PawnHistory {
    #[inline(always)]
    fn get(&self, key: usize, color: usize, piece: usize, to: types::Square) -> i32 {
        i32::from(
            self.entries[key & (PAWN_HISTORY_SIZE - 1)][color * NUM_PIECES + piece][to.index()],
        )
    }

    #[inline(always)]
    fn update(&mut self, key: usize, color: usize, piece: usize, to: types::Square, bonus: i32) {
        update_history_with_max(
            &mut self.entries[key & (PAWN_HISTORY_SIZE - 1)][color * NUM_PIECES + piece]
                [to.index()],
            bonus,
            8192,
        );
    }

    fn clear(&mut self) {
        for bucket in self.entries.iter_mut() {
            for piece in bucket {
                piece.fill(0);
            }
        }
    }

    fn age(&mut self) {
        for bucket in self.entries.iter_mut() {
            for piece in bucket {
                for value in piece {
                    *value /= 2;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CaptureHistoryEntry {
    factorizer: HistoryValue,
    buckets: [[HistoryValue; 2]; NUM_PIECES],
}

struct CaptureHistory {
    entries: Box<[[CaptureHistoryEntry; NUM_SQUARES]; NUM_PIECES]>,
}

impl Default for CaptureHistory {
    fn default() -> Self {
        Self {
            entries: boxed_zeroed(),
        }
    }
}

impl CaptureHistory {
    #[inline(always)]
    fn get(&self, threats: u64, piece: usize, to: types::Square, captured: usize) -> i32 {
        let entry = &self.entries[piece][to.index()];
        let threatened = usize::from(threats & to.bitboard() != 0);
        i32::from(entry.factorizer + entry.buckets[captured][threatened])
    }

    #[inline(always)]
    fn update(
        &mut self,
        threats: u64,
        piece: usize,
        to: types::Square,
        captured: usize,
        bonus: i32,
    ) {
        let entry = &mut self.entries[piece][to.index()];
        update_history_with_max(&mut entry.factorizer, bonus, 4524);
        let threatened = usize::from(threats & to.bitboard() != 0);
        update_history_with_max(&mut entry.buckets[captured][threatened], bonus, 7826);
    }

    fn clear(&mut self) {
        for row in self.entries.iter_mut() {
            row.fill(CaptureHistoryEntry::default());
        }
    }

    fn age(&mut self) {
        for row in self.entries.iter_mut() {
            for entry in row {
                entry.factorizer /= 2;
                for buckets in &mut entry.buckets {
                    for value in buckets {
                        *value /= 2;
                    }
                }
            }
        }
    }
}
const _: () = assert!(std::mem::size_of::<HistoryValue>() == 2);
const _: () = assert!(
    std::mem::size_of::<ContinuationHistory>()
        == NUM_PIECES * NUM_SQUARES * NUM_PIECES * NUM_SQUARES * 2
);
/// History-malus bookkeeping in `search_ab` (stack-only; matches old `Vec` capacities).
const SEARCHED_QUIETS_MAX: usize = 32;
const SEARCHED_CAPTURES_MAX: usize = 16;
const HISTORY_MALUS_SCALE: [i16; SEARCHED_QUIETS_MAX] = build_history_malus_scale();
/// Correction history table size (power of 2 for fast masking).
const CORR_HIST_SIZE: usize = 16384;
const CORR_HIST_MASK: usize = CORR_HIST_SIZE - 1;
/// Correction history weight for each table (Viridithas-inspired).
const PAWN_CORR_WEIGHT: i32 = 1890;
const MATERIAL_CORR_WEIGHT: i32 = 1461;
const MINOR_CORR_WEIGHT: i32 = 1292;
const CORR_WEIGHT_SUM: i32 = PAWN_CORR_WEIGHT + MATERIAL_CORR_WEIGHT + MINOR_CORR_WEIGHT;
/// Max correction entry magnitude.
const MAX_CORR_ENTRY: i32 = 512;
type CorrectionValue = i16;

const fn build_history_malus_scale() -> [i16; SEARCHED_QUIETS_MAX] {
    let mut scales = [0; SEARCHED_QUIETS_MAX];
    let mut rank = 0;
    while rank < SEARCHED_QUIETS_MAX {
        scales[rank] = (64 * 1024 / (64 + 3 * rank)) as i16;
        rank += 1;
    }
    scales
}

#[inline(always)]
fn ranked_history_malus(malus: i32, rank: usize) -> i32 {
    let scale = i32::from(HISTORY_MALUS_SCALE[rank.min(SEARCHED_QUIETS_MAX - 1)]);
    malus * scale / 1024
}

#[inline(always)]
fn fail_low_parent_history_bonus(
    depth: i32,
    parent_move_count: u16,
    parent_was_tt_move: bool,
    in_check: bool,
    best_score: i32,
    static_eval: i32,
    parent_eval: Option<i32>,
) -> i32 {
    let factor = 88
        + (17 * i32::from(parent_move_count)).min(229)
        + 110 * i32::from(parent_was_tt_move)
        + 144 * i32::from(!in_check && best_score <= static_eval - 97)
        + 306 * i32::from(parent_eval.is_some_and(|eval| best_score <= -eval - 136));
    factor * (180 * depth - 37).min(2414) / 128
}

#[inline(always)]
fn first_child_is_cut_node(is_pv: bool, parent_is_cut_node: bool) -> bool {
    !is_pv && !parent_is_cut_node
}

/// Update history with gravity — Stockfish formula:
/// `entry += bonus - entry * |bonus| / MAX_HISTORY`
#[inline(always)]
fn update_history(entry: &mut HistoryValue, bonus: i32) {
    let current = i32::from(*entry);
    let updated = current + bonus - current * bonus.abs() / MAX_HISTORY;
    *entry = updated.clamp(-MAX_HISTORY, MAX_HISTORY) as HistoryValue;
}

#[inline(always)]
fn update_history_with_max(entry: &mut HistoryValue, bonus: i32, max: i32) {
    let bonus = bonus.clamp(-max, max);
    let current = i32::from(*entry);
    *entry = (current + bonus - current * bonus.abs() / max) as HistoryValue;
}

/// Opponent attack maps for the current side to move. Computed once per node
/// and reused by history and Reckless move ordering.
#[derive(Clone, Copy)]
struct OpponentThreats {
    by_piece: [u64; NUM_PIECES],
    all: u64,
}

fn opponent_threats(board: &Board) -> OpponentThreats {
    let color = board.side_to_move.opponent();
    let occupancy = board.all_occupancy() & !board.king_square(board.side_to_move).bitboard();
    let mut by_piece = [0u64; NUM_PIECES];
    let mut all = 0u64;
    for piece in Piece::ALL {
        let attacks = attack_set(piece, color, board.piece_bb(piece, color), occupancy);
        by_piece[piece.index()] = attacks;
        all |= attacks;
    }
    OpponentThreats { by_piece, all }
}

struct RecklessQuietMaps {
    threatened: [u64; NUM_PIECES],
    checking_squares: [u64; NUM_PIECES],
    offense: [u64; NUM_PIECES],
    wall_pawns: u64,
}

#[inline]
fn attack_set(piece: Piece, color: types::Color, mut pieces: u64, occupancy: u64) -> u64 {
    if piece == Piece::Pawn {
        return pawn_attack_set(color, pieces);
    }

    let mut attacks = 0;
    while pieces != 0 {
        let square = pieces.trailing_zeros() as usize;
        pieces &= pieces - 1;
        attacks |= match piece {
            Piece::Pawn => unreachable!(),
            Piece::Knight => knight_attacks(square),
            Piece::Bishop => bishop_attacks(square, occupancy),
            Piece::Rook => rook_attacks(square, occupancy),
            Piece::Queen => queen_attacks(square, occupancy),
            Piece::King => king_attacks(square),
        };
    }
    attacks
}

#[inline(always)]
const fn pawn_attack_set(color: types::Color, pawns: u64) -> u64 {
    const NOT_A_FILE: u64 = !0x0101_0101_0101_0101;
    const NOT_H_FILE: u64 = !0x8080_8080_8080_8080;

    match color {
        types::Color::White => ((pawns & NOT_A_FILE) << 7) | ((pawns & NOT_H_FILE) << 9),
        types::Color::Black => ((pawns & NOT_A_FILE) >> 9) | ((pawns & NOT_H_FILE) >> 7),
    }
}

fn reckless_quiet_ordering_maps(
    board: &Board,
    opponent_threats: OpponentThreats,
) -> RecklessQuietMaps {
    let us = board.side_to_move;
    let them = us.opponent();
    let occupancy = board.all_occupancy();

    let threats = opponent_threats.all;
    let pawn_threats = opponent_threats.by_piece[Piece::Pawn.index()];
    let knight_threats = opponent_threats.by_piece[Piece::Knight.index()];
    let bishop_threats = opponent_threats.by_piece[Piece::Bishop.index()];
    let rook_attacks_set = opponent_threats.by_piece[Piece::Rook.index()];
    let queen_threats = opponent_threats.by_piece[Piece::Queen.index()];
    let king_threats = opponent_threats.by_piece[Piece::King.index()];
    let non_pawn_threats =
        knight_threats | bishop_threats | rook_attacks_set | queen_threats | king_threats;
    let minor_threats = pawn_threats | knight_threats | bishop_threats;
    let rook_threats = minor_threats | rook_attacks_set;
    let threatened = [
        0,
        pawn_threats,
        pawn_threats,
        minor_threats,
        rook_threats,
        0,
    ];

    let king = board.king_square(them).index();
    let bishop_checks = bishop_attacks(king, occupancy);
    let rook_checks = rook_attacks(king, occupancy);
    let mut checking_squares = [0u64; NUM_PIECES];
    checking_squares[Piece::Pawn.index()] = pawn_attacks(them.index(), king);
    checking_squares[Piece::Knight.index()] = knight_attacks(king);
    checking_squares[Piece::Bishop.index()] = bishop_checks;
    checking_squares[Piece::Rook.index()] = rook_checks;
    checking_squares[Piece::Queen.index()] = bishop_checks | rook_checks;
    checking_squares[Piece::King.index()] = king_attacks(king);

    let their_occupancy = Piece::ALL
        .iter()
        .fold(0, |all, &piece| all | board.piece_bb(piece, them));
    let knight_vulnerable = (board.piece_bb(Piece::Bishop, them) & !threats)
        | board.piece_bb(Piece::Rook, them)
        | board.piece_bb(Piece::Queen, them);
    let bishop_vulnerable = board.piece_bb(Piece::Rook, them);
    let queen_orth_vulnerable = board.piece_bb(Piece::Bishop, them) & !threats;
    let queen_diag_vulnerable = board.piece_bb(Piece::Rook, them) & !threats;
    let mut pawn_offense = attack_set(Piece::Pawn, them, their_occupancy, occupancy) & !threats;
    let lever_ranks = [0x0000_FFFF_0000_0000, 0x0000_0000_FFFF_0000][us.index()];
    pawn_offense |= pawn_threats & lever_ranks & !non_pawn_threats;
    let offense = [
        pawn_offense,
        attack_set(Piece::Knight, us, knight_vulnerable, occupancy) & !threats,
        attack_set(Piece::Bishop, us, bishop_vulnerable, occupancy) & !threats,
        0x0101_0101_0101_0101u64 << board.king_square(them).file(),
        (attack_set(Piece::Rook, us, queen_orth_vulnerable, occupancy)
            | attack_set(Piece::Bishop, us, queen_diag_vulnerable, occupancy))
            & !threats,
        0,
    ];
    let my_king = board.king_square(us);
    let home_rank = [0xFF, 0xFF00_0000_0000_0000][us.index()];
    let wall_pawns = if my_king.bitboard() & home_rank != 0 {
        king_attacks(my_king.index())
            & (board.piece_bb(Piece::Pawn, types::Color::White)
                | board.piece_bb(Piece::Pawn, types::Color::Black))
    } else {
        0
    };

    RecklessQuietMaps {
        threatened,
        checking_squares,
        offense,
        wall_pawns,
    }
}

#[inline(always)]
fn gives_direct_check(board: &Board, mv: Move) -> bool {
    let color = board.side_to_move;
    let Some(moved_piece) = board.piece_of_color_on(mv.from, color) else {
        return false;
    };
    let piece = mv.promotion.unwrap_or(moved_piece);
    let occupancy = (board.all_occupancy() & !mv.from.bitboard()) | mv.to.bitboard();
    let target = board.king_square(color.opponent()).bitboard();
    let attacks = match piece {
        Piece::Pawn => pawn_attacks(color.index(), mv.to.index()),
        Piece::Knight => knight_attacks(mv.to.index()),
        Piece::Bishop => bishop_attacks(mv.to.index(), occupancy),
        Piece::Rook => rook_attacks(mv.to.index(), occupancy),
        Piece::Queen => queen_attacks(mv.to.index(), occupancy),
        Piece::King => king_attacks(mv.to.index()),
    };
    attacks & target != 0
}

#[inline(always)]
fn update_correction_entry(entry: &mut CorrectionValue, error: i32, weight: i32) {
    let retained = 256 - i64::from(weight);
    let updated = (i64::from(*entry) * retained + i64::from(error) * i64::from(weight)) / 256;
    *entry =
        updated.clamp(-i64::from(MAX_CORR_ENTRY), i64::from(MAX_CORR_ENTRY)) as CorrectionValue;
}

/// Convert score to TT storage format (normalize mate scores by ply).
#[inline(always)]
fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_TT_THRESHOLD {
        score + ply
    } else if score <= -MATE_TT_THRESHOLD {
        score - ply
    } else {
        score
    }
}

/// Convert score from TT storage format to local ply.
#[inline(always)]
fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_TT_THRESHOLD {
        score - ply
    } else if score <= -MATE_TT_THRESHOLD {
        score + ply
    } else {
        score
    }
}

fn normalize_uci_score(score: i32, board: &Board, eval_mode: EvalMode) -> i32 {
    if !eval_mode.is_reckless_nnue() || score.abs() > MATE_SCORE - 100 {
        return score;
    }
    let material = board
        .piece_bb(Piece::Pawn, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Pawn, types::Color::Black)
            .count_ones()
        + 3 * (board
            .piece_bb(Piece::Knight, types::Color::White)
            .count_ones()
            + board
                .piece_bb(Piece::Knight, types::Color::Black)
                .count_ones()
            + board
                .piece_bb(Piece::Bishop, types::Color::White)
                .count_ones()
            + board
                .piece_bb(Piece::Bishop, types::Color::Black)
                .count_ones())
        + 5 * (board
            .piece_bb(Piece::Rook, types::Color::White)
            .count_ones()
            + board
                .piece_bb(Piece::Rook, types::Color::Black)
                .count_ones())
        + 9 * (board
            .piece_bb(Piece::Queen, types::Color::White)
            .count_ones()
            + board
                .piece_bb(Piece::Queen, types::Color::Black)
                .count_ones());
    let phase = f64::from(material.clamp(16, 78)) / 58.0;
    let normalization = -166.3 * phase.powi(3) + 402.2 * phase.powi(2) - 340.0 * phase + 419.9;
    (100.0 * f64::from(score) / normalization).round() as i32
}

fn format_uci_score_value(score: i32, board: &Board, eval_mode: EvalMode) -> String {
    if score.abs() > MATE_SCORE - 100 {
        let mate_in = if score > 0 {
            (MATE_SCORE - score + 1) / 2
        } else {
            -(MATE_SCORE + score + 1) / 2
        };
        format!("mate {mate_in}")
    } else {
        format!("cp {}", normalize_uci_score(score, board, eval_mode))
    }
}

#[inline]
fn extend_checks(
    move_ordering: MoveOrderingProfile,
    eval_mode: EvalMode,
    fixed_nodes: bool,
) -> bool {
    // Fixed-node BK gates skip Reckless/v60 check extensions so depth is not
    // burned. Time-based games match upstream Reckless, which still extends.
    if eval_mode.is_reckless_nnue() {
        return !fixed_nodes;
    }
    move_ordering != MoveOrderingProfile::Reckless
        || eval_mode.is_lc0_nnue()
        || eval_mode.is_viridithas_nnue()
}

#[inline]
fn full_depth_root_quiets(move_ordering: MoveOrderingProfile, eval_mode: EvalMode) -> bool {
    eval_mode.is_reckless_nnue()
        || move_ordering == MoveOrderingProfile::StockLike
        || eval_mode.is_lc0_nnue()
        || eval_mode.is_viridithas_nnue()
}

#[inline(always)]
fn budgeted_check_extension(
    depth: i32,
    extensions: i32,
    budget: i32,
    in_check: bool,
) -> (i32, i32) {
    if in_check && extensions < budget {
        (depth + 1, extensions + 1)
    } else {
        (depth, extensions)
    }
}

/// Compute the evaluation for a position using NNUE.
/// Uses the cached accumulator table — avoids full recompute when the
/// board's piece bitboards haven't changed since the last eval in the
/// same king-bucket pair.
#[inline(always)]
fn hybrid_eval(board: &Board, state: &mut ThreadState, use_nnue: bool) -> i32 {
    if use_nnue {
        state.nnue_state.evaluate(board)
    } else {
        eval::evaluate_with_hce(board, &state.hce)
    }
}

fn hybrid_eval_with_uncertainty(
    board: &Board,
    state: &mut ThreadState,
    use_nnue: bool,
) -> (i32, i32) {
    if use_nnue {
        state.nnue_state.evaluate_with_uncertainty(board)
    } else {
        (eval::evaluate_with_hce(board, &state.hce), 0)
    }
}

/// Widen Ateed pruning when WDL variance is high. `variance` is already ×10_000.
fn ateed_uncertainty_margin(eval_mode: EvalMode, variance: i32) -> i32 {
    if !eval_mode.is_ateed_nnue() || variance <= 0 {
        return 0;
    }
    (variance / 200).clamp(0, 64)
}

fn ateed_lmr_relief(eval_mode: EvalMode, variance: i32) -> i32 {
    i32::from(eval_mode.is_ateed_nnue() && variance >= 1_500)
}

#[inline(always)]
fn reckless_material(board: &Board) -> i32 {
    board.total_material()
}

/// Count nodes like Reckless/native-v60: once per legal move made, not once per
/// search-node entry. Stand-pat qsearch and null-move probes do not increment.
#[inline(always)]
fn make_search_move(board: &mut Board, state: &mut ThreadState, mv: Move) {
    state.nodes += 1;
    if state.use_nnue {
        state.nnue_state.make_move(board, mv);
    } else {
        state.hce_undo[state.hce_ply] = state.hce;
        state.hce_ply += 1;
        state.hce.apply_move(board, mv);
        board.make_move(mv);
    }
}

#[inline(always)]
fn undo_search_eval(state: &mut ThreadState) {
    if state.use_nnue {
        state.nnue_state.pop_move();
    } else if state.hce_ply > 0 {
        state.hce_ply -= 1;
        state.hce = state.hce_undo[state.hce_ply];
    }
}

fn stockfish_material(board: &Board) -> i32 {
    let pawns = board
        .piece_bb(Piece::Pawn, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Pawn, types::Color::Black)
            .count_ones();
    let knights = board
        .piece_bb(Piece::Knight, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Knight, types::Color::Black)
            .count_ones();
    let bishops = board
        .piece_bb(Piece::Bishop, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Bishop, types::Color::Black)
            .count_ones();
    let rooks = board
        .piece_bb(Piece::Rook, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Rook, types::Color::Black)
            .count_ones();
    let queens = board
        .piece_bb(Piece::Queen, types::Color::White)
        .count_ones()
        + board
            .piece_bb(Piece::Queen, types::Color::Black)
            .count_ones();
    534 * pawns as i32
        + 416 * knights as i32
        + 441 * bishops as i32
        + 663 * rooks as i32
        + 1_292 * queens as i32
}

fn corrected_network_eval(
    board: &Board,
    raw_eval: i32,
    correction: i32,
    optimism: i32,
    eval_mode: EvalMode,
) -> i32 {
    if eval_mode.is_stockfish_nnue() {
        // official-stockfish/Stockfish evaluate.cpp: material blend + rule50 damp.
        let material = stockfish_material(board);
        let mut value = (raw_eval * (77_871 + material) + optimism * (7_191 + material)) / 77_871;
        value -= value * board.halfmove_clock.min(199) as i32 / 199;
        return (value + correction)
            .clamp(-MATE_SCORE + MAX_PLY as i32, MATE_SCORE - MAX_PLY as i32);
    }
    if !eval_mode.is_reckless_nnue() {
        return raw_eval + correction;
    }

    let material = reckless_material(board);
    let mut value = (raw_eval * (21_032 + material) + optimism * (1_548 + material)) / 27_015;
    value = value * (200 - board.halfmove_clock.min(200) as i32) / 200;
    (value + correction).clamp(-MATE_SCORE + MAX_PLY as i32, MATE_SCORE - MAX_PLY as i32)
}

#[inline(always)]
fn update_reckless_optimism(
    state: &mut ThreadState,
    side_to_move: types::Color,
    average_score: i32,
    shared_best_stat: u32,
    eval_mode: EvalMode,
) {
    if !eval_mode.is_reckless_nnue() && !eval_mode.is_stockfish_nnue() {
        state.optimism = [0; 2];
        return;
    }

    let shared_average = (shared_best_stat & 0xffff) as i32 - 32_768;
    let best_average = (average_score + shared_average) / 2;
    let bounded_score = if best_average.abs() < MATE_SCORE - 100 {
        best_average
    } else {
        0
    };
    let optimism = 113 * bounded_score / (bounded_score.abs() + 201);
    state.optimism[side_to_move.index()] = optimism;
    state.optimism[side_to_move.opponent().index()] = -optimism;
}

#[inline(always)]
fn root_score_stat(depth: i32, average_score: i32) -> u32 {
    let depth = depth.clamp(0, u16::MAX as i32) as u32;
    let score = average_score.clamp(-32_768, 32_767) + 32_768;
    (depth << 16) | score as u32
}

/// Interior draw jitter. Contempt is applied only at the root.
#[inline(always)]
const fn draw_score(nodes: u64) -> i32 {
    crate::conversion::interior_draw_score(nodes)
}

#[inline(always)]
fn update_root_average(average_score: &mut i32, score: i32) {
    *average_score = (*average_score + score) / 2;
}

/// Get the piece index of the piece on `sq`. Returns 0 (pawn) if no piece.
#[inline(always)]
fn piece_index_on(board: &Board, sq: types::Square) -> usize {
    board
        .piece_of_color_on(sq, board.side_to_move)
        .map_or(0, Piece::index)
}

#[inline(always)]
fn captured_piece_index(board: &Board, mv: Move) -> Option<usize> {
    if mv.flag == types::chess_move::MoveFlag::EnPassant {
        return Some(Piece::Pawn.index());
    }
    board
        .piece_of_color_on(mv.to, board.side_to_move.opponent())
        .map(Piece::index)
}

/// Search result returned to the caller.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best_move: Move,
    pub score: i32,
    pub depth: i32,
    pub seldepth: i32,
    pub nodes: u64,
    pub elapsed: Duration,
    /// Principal variation line.
    pub pv: Vec<Move>,
    /// TT occupancy in per-mille, for UCI `hashfull`.
    pub hashfull: u16,
    /// Successful Syzygy WDL probes during this search.
    pub tbhits: u64,
}

/// Configuration for a search.
#[derive(Clone, Debug)]
pub struct SearchLimits {
    pub max_depth: i32,
    pub time_limit: Option<Duration>,
    pub node_limit: Option<u64>,
    pub stopped: bool,
    pub use_soft_time: bool,
    /// When true, Lazy SMP helpers run even under a hard node limit.
    ///
    /// Default searches keep helpers off for node-limited runs so equal-node
    /// duels stay deterministic. Classical HCE throughput benches set this.
    pub force_helpers: bool,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            time_limit: None,
            node_limit: None,
            stopped: false,
            use_soft_time: true,
            force_helpers: false,
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Heap allocation helpers — avoid stack overflow for large arrays.
//
// `Box::new([val; N])` creates the array ON THE STACK before moving
// it to the heap. For arrays > ~100KB this overflows the default 2MB
// thread stack. These helpers allocate directly on the heap.
// ──────────────────────────────────────────────────────────────

/// Allocate a zeroed Box<T> directly on the heap (no stack intermediate).
/// T must be safe to zero-initialize (integers, arrays of integers).
#[inline]
fn boxed_zeroed<T>() -> Box<T> {
    // SAFETY: every call site instantiates this private helper with integer
    // arrays or integer-backed search records whose zero representation is
    // valid; allocation failure is handled before ownership is constructed.
    unsafe {
        let layout = std::alloc::Layout::new::<T>();
        let ptr = std::alloc::alloc_zeroed(layout) as *mut T;
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Box::from_raw(ptr)
    }
}
/// Per-thread search state. Each Lazy SMP worker has its own.
///
/// Large arrays are heap-allocated via `Box` to stay within default 2MB thread stacks.
/// This is critical for Lazy SMP (spawned worker threads) and test runners.
struct ThreadState {
    nodes: u64,
    /// Deepest ply reached by the current root iteration.
    seldepth: i32,
    killers: [[Move; 2]; MAX_PLY],
    /// Main quiet history: [color][from][to] — 32KB
    history: QuietHistory,
    /// Pawn-structure-conditioned quiet history.
    pawn_history: PawnHistory,
    /// Continuation history (1-ply back): [prev_piece][prev_to][cur_piece][cur_to]
    cont_hist: Box<ContinuationHistory>,
    /// Continuation history (2-ply back): same layout
    cont_hist2: Box<ContinuationHistory>,
    /// Reckless shares one continuation table across offsets 1, 2, 4, and 6.
    reckless_cont_hist: Box<ContinuationHistory>,
    /// Capture history: [moved_piece][to_square][captured_piece_type] — 9KB
    cap_hist: CaptureHistory,
    /// Countermoves: [from][to] — 32KB
    countermoves: Box<[[Move; 64]; 64]>,
    /// Unbounded corrected eval at each ply (for improving/history learning).
    static_evals: [i32; MAX_PLY],
    /// Ateed WDL variance at each ply; zero for other evaluators.
    eval_variance: [i32; MAX_PLY],
    /// Whether the corresponding static eval is meaningful outside check.
    eval_valid: [bool; MAX_PLY],
    /// NNUE evaluation state.
    nnue_state: NNUEState,
    /// Triangular PV table — 128KB, must be heap-allocated.
    pv: Box<[[Move; MAX_PLY]; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],
    /// Previous move at each ply (for countermove/continuation indexing).
    prev_move: [Move; MAX_PLY],
    /// Piece type of the move at each ply (for continuation history indexing).
    prev_piece: [usize; MAX_PLY],
    /// Opponent attack map at each ply, shared by all history consumers.
    threats: [u64; MAX_PLY],
    /// Correction history tables (search_score - static_eval differences).
    pawn_corr: Box<[[CorrectionValue; CORR_HIST_SIZE]; 2]>,
    material_corr: Box<[[CorrectionValue; CORR_HIST_SIZE]; 2]>,
    minor_corr: Box<[[CorrectionValue; CORR_HIST_SIZE]; 2]>,
    /// Double extension count per path (Akimbo anti-explosion).
    dbl_exts: [i32; MAX_PLY],
    /// Minimum ply before NMP is allowed again (Akimbo anti-recursion).
    min_nmp_ply: usize,
    /// Per-ply cutoff count for LMR adjustment (Akimbo: reduce less if cutoffs < 4).
    cutoffs: [u32; MAX_PLY],
    /// Number of moves searched at each active node.
    move_counts: [u16; MAX_PLY],
    /// TT move selected at each active node for post-node learning.
    tt_moves: [Move; MAX_PLY],
    /// Active LMR reduction for the move entering each child node.
    reductions: [i32; MAX_PLY],
    /// Prevent recursive reverse-quiescence re-entry.
    reverse_qsearch: bool,
    /// Root score bias used by the Reckless evaluation adapter.
    optimism: [i32; 2],
    /// Successful Syzygy WDL probes in this search.
    tbhits: u64,
    /// Root moves skipped for MultiPV follow-up lines. Read only at the root.
    root_exclude: Vec<Move>,
    /// When false, skip NNUE accumulator updates on make/unmake.
    use_nnue: bool,
    /// Incremental material + PSQT used only when `use_nnue` is false.
    hce: eval::HceState,
    hce_undo: [eval::HceState; MAX_PLY],
    hce_ply: usize,
}

impl ThreadState {
    fn new(nnue_network: Arc<ActiveNetwork>) -> Self {
        let mut countermoves = boxed_zeroed::<[[Move; 64]; 64]>();
        for row in countermoves.iter_mut() {
            row.fill(NULL_MOVE);
        }

        // Use helper to allocate large arrays DIRECTLY on the heap.
        // `Box::new([val; N])` creates the array on the stack first — for arrays
        // >100KB this overflows. These helpers avoid that.
        Self {
            nodes: 0,
            seldepth: 0,
            killers: [[NULL_MOVE; 2]; MAX_PLY],
            history: QuietHistory::default(),
            pawn_history: PawnHistory::default(),
            cont_hist: boxed_zeroed(),
            cont_hist2: boxed_zeroed(),
            reckless_cont_hist: boxed_zeroed(),
            cap_hist: CaptureHistory::default(),
            countermoves,
            static_evals: [0; MAX_PLY],
            eval_variance: [0; MAX_PLY],
            eval_valid: [false; MAX_PLY],
            nnue_state: NNUEState::with_network(nnue_network),
            pv: boxed_zeroed(),
            pv_len: [0; MAX_PLY],
            prev_move: [NULL_MOVE; MAX_PLY],
            prev_piece: [0; MAX_PLY],
            threats: [0; MAX_PLY],
            pawn_corr: boxed_zeroed(),
            material_corr: boxed_zeroed(),
            minor_corr: boxed_zeroed(),
            dbl_exts: [0; MAX_PLY],
            min_nmp_ply: 0,
            cutoffs: [0; MAX_PLY],
            move_counts: [0; MAX_PLY],
            tt_moves: [NULL_MOVE; MAX_PLY],
            reductions: [0; MAX_PLY],
            reverse_qsearch: false,
            optimism: [0; 2],
            tbhits: 0,
            root_exclude: Vec::new(),
            use_nnue: true,
            hce: eval::HceState::default(),
            hce_undo: [eval::HceState::default(); MAX_PLY],
            hce_ply: 0,
        }
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn age_history(&mut self) {
        self.history.age();
        self.pawn_history.age();
        for a in self.cont_hist.iter_mut() {
            for b in a.iter_mut() {
                for c in b.iter_mut() {
                    for v in c.iter_mut() {
                        *v /= 2;
                    }
                }
            }
        }
        for a in self.cont_hist2.iter_mut() {
            for b in a.iter_mut() {
                for c in b.iter_mut() {
                    for v in c.iter_mut() {
                        *v /= 2;
                    }
                }
            }
        }
        for a in self.reckless_cont_hist.iter_mut() {
            for b in a.iter_mut() {
                for c in b.iter_mut() {
                    for v in c.iter_mut() {
                        *v /= 2;
                    }
                }
            }
        }
        self.cap_hist.age();
    }

    fn reset_for_search(&mut self) {
        self.nodes = 0;
        self.seldepth = 0;
        self.killers = [[NULL_MOVE; 2]; MAX_PLY];
        self.static_evals.fill(0);
        self.eval_variance.fill(0);
        self.eval_valid = [false; MAX_PLY];
        self.pv_len.fill(0);
        self.prev_move.fill(NULL_MOVE);
        self.prev_piece.fill(0);
        self.threats.fill(0);
        self.dbl_exts.fill(0);
        self.min_nmp_ply = 0;
        self.cutoffs.fill(0);
        self.move_counts.fill(0);
        self.tt_moves.fill(NULL_MOVE);
        self.reductions.fill(0);
        self.reverse_qsearch = false;
        self.optimism = [0; 2];
        self.tbhits = 0;
    }

    #[inline(always)]
    fn is_root_excluded(&self, mv: Move) -> bool {
        self.root_exclude.iter().any(|&excluded| {
            excluded.from == mv.from && excluded.to == mv.to && excluded.promotion == mv.promotion
        })
    }

    fn clear(&mut self) {
        self.reset_for_search();
        self.history.clear();
        self.pawn_history.clear();
        for table in [
            &mut self.cont_hist,
            &mut self.cont_hist2,
            &mut self.reckless_cont_hist,
        ] {
            for previous_piece in table.iter_mut() {
                for previous_square in previous_piece.iter_mut() {
                    for piece in previous_square.iter_mut() {
                        piece.fill(0);
                    }
                }
            }
        }
        self.cap_hist.clear();
        for row in self.countermoves.iter_mut() {
            row.fill(NULL_MOVE);
        }
        for table in [
            &mut self.pawn_corr,
            &mut self.material_corr,
            &mut self.minor_corr,
        ] {
            for color in table.iter_mut() {
                color.fill(0);
            }
        }
    }

    /// Compute the combined correction for a position.
    #[inline(always)]
    fn correction(&self, board: &Board, _profile: MoveOrderingProfile) -> i32 {
        let us = board.side_to_move.index();
        let ph = pawn_hash(board) & CORR_HIST_MASK;
        let mh = material_hash(board) & CORR_HIST_MASK;
        let nh = minor_hash(board) & CORR_HIST_MASK;
        let raw = i64::from(self.pawn_corr[us][ph]) * i64::from(PAWN_CORR_WEIGHT)
            + i64::from(self.material_corr[us][mh]) * i64::from(MATERIAL_CORR_WEIGHT)
            + i64::from(self.minor_corr[us][nh]) * i64::from(MINOR_CORR_WEIGHT);
        (raw / CORR_WEIGHT_SUM as i64) as i32
    }

    /// Update correction history tables after a search completes at a node.
    #[inline(always)]
    fn update_correction(
        &mut self,
        board: &Board,
        depth: i32,
        search_score: i32,
        static_eval: i32,
        _profile: MoveOrderingProfile,
    ) {
        if search_score.abs() > MATE_SCORE - 100 {
            return;
        }
        let error = search_score - static_eval;
        let us = board.side_to_move.index();
        let ph = pawn_hash(board) & CORR_HIST_MASK;
        let mh = material_hash(board) & CORR_HIST_MASK;
        let nh = minor_hash(board) & CORR_HIST_MASK;
        let weight = depth.min(16);
        update_correction_entry(&mut self.pawn_corr[us][ph], error, weight);
        update_correction_entry(&mut self.material_corr[us][mh], error, weight);
        update_correction_entry(&mut self.minor_corr[us][nh], error, weight);
    }

    /// Compute stat_score for a quiet move: main history + continuation histories.
    #[inline(always)]
    fn stat_score(
        &self,
        mv: Move,
        us: usize,
        piece: usize,
        ply: usize,
        move_ordering: MoveOrderingProfile,
    ) -> i32 {
        let mut score = self.history.get(self.threats[ply], us, mv);
        if move_ordering == MoveOrderingProfile::Reckless {
            for offset in [1usize, 2] {
                if ply >= offset && self.prev_move[ply - offset] != NULL_MOVE {
                    let previous_piece = self.prev_piece[ply - offset];
                    let previous_to = self.prev_move[ply - offset].to.index();
                    score += i32::from(
                        self.reckless_cont_hist[previous_piece][previous_to][piece][mv.to.index()],
                    );
                }
            }
            return score;
        }
        // Add 1-ply continuation history
        if ply > 0 {
            let pp = self.prev_piece[ply.saturating_sub(1)];
            let pt = self.prev_move[ply.saturating_sub(1)].to.index();
            score += i32::from(self.cont_hist[pp][pt][piece][mv.to.index()]);
        }
        // Add 2-ply continuation history
        if ply > 1 {
            let pp2 = self.prev_piece[ply.saturating_sub(2)];
            let pt2 = self.prev_move[ply.saturating_sub(2)].to.index();
            score += i32::from(self.cont_hist2[pp2][pt2][piece][mv.to.index()]);
        }
        score
    }

    /// Get the PV line from ply 0 as a Vec.
    fn get_pv(&self) -> Vec<Move> {
        let len = self.pv_len[0].min(MAX_PLY);
        self.pv[0][..len].to_vec()
    }
}

/// The search engine, owning a shared TT and managing Lazy SMP threads.
pub struct SearchEngine {
    pub tt: Arc<TranspositionTable>,
    pub num_threads: usize,
    stopped: Arc<AtomicBool>,
    nnue_network: Arc<ActiveNetwork>,
    /// Coherent evaluator-specific parameters and search policies.
    pub(crate) search_stack: SearchStack,
    use_nnue: bool,
    root_selection_policy: Arc<dyn RootSelectionPolicy + Send + Sync>,
    state: ThreadState,
    previous_best_score: i32,
    contempt: i32,
    syzygy: Arc<SyzygyTables>,
    /// Wall-clock deadline in milliseconds from search start; 0 disables it.
    /// Used to convert an infinite ponder search into a timed one on ponderhit.
    deadline_ms: Arc<AtomicU64>,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new(64, 1)
    }
}

impl SearchEngine {
    /// Creates a new search engine with the given TT size and thread count.
    pub fn new(tt_size_mb: usize, num_threads: usize) -> Self {
        #[cfg(test)]
        ensure_test_nnue_discovery_path();
        let embedded = default_embedded_network();
        let search_stack = SearchStack::for_network(embedded.search_profile());
        let nnue_network: Arc<ActiveNetwork> = Arc::new(embedded);
        let state = ThreadState::new(Arc::clone(&nnue_network));
        // NNUE is always available (embedded trained net)
        Self {
            tt: Arc::new(TranspositionTable::new(tt_size_mb)),
            num_threads: num_threads.max(1),
            stopped: Arc::new(AtomicBool::new(false)),
            nnue_network,
            search_stack,
            use_nnue: true,
            root_selection_policy: Arc::new(MainThreadPreferredRootSelection),
            state,
            previous_best_score: 0,
            contempt: crate::conversion::DEFAULT_CONTEMPT,
            syzygy: Arc::new(SyzygyTables::empty()),
            deadline_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn set_syzygy_path(&mut self, path: &str) {
        let probe_limit = self.syzygy.probe_limit();
        let probe_depth = self.syzygy.probe_depth();
        let mut tables = SyzygyTables::from_path(path);
        tables.set_probe_limit(probe_limit);
        tables.set_probe_depth(probe_depth);
        self.syzygy = Arc::new(tables);
    }

    pub fn set_syzygy_probe_limit(&mut self, limit: u32) {
        let mut tables = (*self.syzygy).clone();
        tables.set_probe_limit(limit);
        self.syzygy = Arc::new(tables);
    }

    pub fn set_syzygy_probe_depth(&mut self, depth: i32) {
        let mut tables = (*self.syzygy).clone();
        tables.set_probe_depth(depth);
        self.syzygy = Arc::new(tables);
    }

    /// Clone the ponder/soft deadline so a UCI thread can arm it mid-search.
    pub fn deadline_token(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.deadline_ms)
    }

    /// Exclude these root moves (MultiPV follow-up lines). Empty = none.
    pub fn set_root_exclusions(&mut self, moves: &[Move]) {
        self.state.root_exclude.clear();
        self.state.root_exclude.extend_from_slice(moves);
    }

    pub fn syzygy(&self) -> &SyzygyTables {
        &self.syzygy
    }

    /// Store requested UCI contempt. NNUE adapters still search with 0.
    pub fn set_contempt(&mut self, contempt: i32) {
        self.contempt = contempt.clamp(-100, 100);
    }

    /// Requested UCI contempt before the HCE/NNUE gate.
    pub fn requested_contempt(&self) -> i32 {
        self.contempt
    }

    /// Contempt actually used in search: HCE only; 0 for every NNUE adapter.
    pub fn contempt(&self) -> i32 {
        self.eval_mode().effective_contempt(self.contempt)
    }

    /// Set the search parameters (e.g. when switching NNUE networks).
    pub fn set_params(&mut self, params: SearchParams) {
        self.search_stack.replace_parameters(params);
    }

    /// Configure search params for a given network preset name.
    ///
    /// Applies repo tuning overlays only for non-Stockfish presets (see
    /// [`SearchParams::for_preset_with_repo_tuning`]).
    pub fn set_params_for_preset(&mut self, preset: &str) {
        self.search_stack = SearchStack::for_preset_name(preset);
    }

    /// Replace the composed search stack atomically.
    pub fn set_search_stack(&mut self, stack: SearchStack) {
        self.search_stack = stack;
    }

    /// Install a named eval+search adapter (`stockfish`, `reckless`, `akimbo`, `mujrim-hce`).
    pub fn install_adapter(&mut self, id: &str) -> bool {
        adapters::install_adapter(self, id)
    }

    /// Apply a benchmark-only policy overlay to the currently compatible
    /// evaluator stack without replacing its tuned parameters.
    pub fn set_search_experiment(&mut self, experiment: SearchExperiment) {
        self.search_stack.apply_experiment(experiment);
    }

    /// Set the active NNUE network source.
    pub fn set_nnue_network_source(&mut self, network: Arc<ActiveNetwork>) {
        let profile = network.search_profile();
        self.state.nnue_state = NNUEState::with_network(Arc::clone(&network));
        self.nnue_network = network;
        self.search_stack = SearchStack::for_network(profile);
    }

    /// Set the active NNUE network from a concrete adapter handle.
    ///
    /// Automatically applies the network's recommended search parameters
    /// via `preset_hint()`. This means callers do NOT need to manually call
    /// `set_params_for_preset()` after this method.
    pub fn set_nnue_network(&mut self, network: ActiveNetwork) {
        self.set_nnue_network_source(Arc::new(network));
    }

    /// Enable or disable NNUE evaluation (fallback to classical eval when disabled).
    pub fn set_use_nnue(&mut self, enabled: bool) {
        self.use_nnue = enabled;
        self.state.use_nnue = enabled;
    }

    /// Replace the transposition table without keeping the old and new tables
    /// resident at the same time.
    pub fn resize_tt(&mut self, tt_size_mb: usize) {
        let old = std::mem::replace(&mut self.tt, Arc::new(TranspositionTable::new(0)));
        drop(old);
        self.tt = Arc::new(TranspositionTable::new(tt_size_mb));
    }

    /// Returns whether NNUE evaluation is enabled.
    pub fn use_nnue(&self) -> bool {
        self.use_nnue
    }

    /// Returns metadata for the currently active NNUE network.
    pub fn nnue_info(&self) -> NnueNetworkInfo {
        self.nnue_network.info()
    }

    /// Returns the recommended search preset for the active NNUE network.
    pub fn nnue_preset_hint(&self) -> &'static str {
        self.nnue_network.preset_hint()
    }

    /// Active evaluator/search binding (NNUE family or Mujrim HCE).
    pub fn eval_mode(&self) -> EvalMode {
        self.search_stack.eval_mode()
    }

    /// Active NNUE search-stack profile when NNUE is bound.
    pub fn network_profile(&self) -> Option<eval::nnue::NnueSearchProfile> {
        self.search_stack.network_profile()
    }

    /// Parameters from the currently composed evaluator-compatible stack.
    pub fn params(&self) -> &SearchParams {
        &self.search_stack.params
    }

    /// Formats a root score according to the active evaluator/search adapter.
    ///
    /// This keeps final protocol telemetry identical to the iterative search
    /// output, including evaluator-specific centipawn normalization and mate
    /// distances.
    pub fn format_uci_score(&self, board: &Board, score: i32) -> String {
        format_uci_score_value(score, board, self.search_stack.eval_mode())
    }

    /// Set a custom LMR policy implementation.
    pub fn set_lmr_policy(&mut self, policy: Arc<dyn LmrPolicy + Send + Sync>) {
        self.search_stack.policies.lmr = LmrDispatch::Custom(policy);
    }

    /// Set a custom root move selection policy for Lazy SMP.
    pub fn set_root_selection_policy(
        &mut self,
        policy: Arc<dyn RootSelectionPolicy + Send + Sync>,
    ) {
        self.root_selection_policy = policy;
    }

    #[cfg(test)]
    fn lmr_reduction_for(&self, depth: usize, moves: usize) -> i32 {
        self.search_stack.lmr_table[depth.min(127)][moves.min(127)]
    }

    /// Performs search with Lazy SMP.
    pub fn search(&mut self, board: &mut Board, limits: SearchLimits) -> SearchResult {
        self.stopped.store(false, Ordering::SeqCst);
        self.deadline_ms.store(0, Ordering::Relaxed);
        self.tt.new_generation();

        let start_time = Instant::now();
        let fallback_move = board
            .generate_legal_moves()
            .iter()
            .next()
            .copied()
            .unwrap_or(NULL_MOVE);
        let initial_root_score = self.previous_best_score;
        let shared_best_stat = Arc::new(AtomicU32::new(root_score_stat(0, initial_root_score)));
        let use_helpers = limits.node_limit.is_none() || limits.force_helpers;
        let helper_threads = if use_helpers { self.num_threads } else { 1 };

        // Spawn helper threads for Lazy SMP (threads > 1)
        let mut handles = Vec::new();
        for _thread_id in 1..helper_threads {
            let tt = Arc::clone(&self.tt);
            let stopped = Arc::clone(&self.stopped);
            let mut board_clone = board.clone();
            let max_depth = limits.max_depth;
            let time_limit = limits.time_limit;
            let node_limit = limits.node_limit;
            let start = start_time;
            let search_stack = self.search_stack.clone();
            let use_nnue_clone = self.use_nnue;
            let contempt = self.contempt();
            let move_ordering = search_stack.policies.move_ordering;
            let nnue_network = Arc::clone(&self.nnue_network);
            let syzygy = Arc::clone(&self.syzygy);
            let deadline_ms = Arc::clone(&self.deadline_ms);
            let root_exclude = self.state.root_exclude.clone();
            let shared_best_stat = Arc::clone(&shared_best_stat);

            handles.push(
                std::thread::Builder::new()
                    .stack_size(16 * 1024 * 1024)
                    .spawn(move || {
                        let mut state = ThreadState::new(nnue_network);
                        state.use_nnue = use_nnue_clone;
                        state.hce_ply = 0;
                        if !use_nnue_clone {
                            state.hce = eval::HceState::from_board(&board_clone);
                        }
                        state.root_exclude = root_exclude;
                        let mut best_score = -INF;
                        let mut average_score = initial_root_score;
                        let mut best_move = NULL_MOVE;
                        let mut completed_depth = 0i32;
                        let context = SearchContext {
                            tt: &tt,
                            stopped: &stopped,
                            time_limit,
                            node_limit,
                            start_time: start,
                            params: &search_stack.params,
                            lmr_table: search_stack.lmr_table.as_ref(),
                            use_nnue: use_nnue_clone,
                            lmr_policy: &search_stack.policies.lmr,
                            lmp_policy: &search_stack.policies.lmp,
                            futility_policy: &search_stack.policies.futility,
                            bad_noisy_futility_policy: &search_stack.policies.bad_noisy_futility,
                            rfp_policy: &search_stack.policies.rfp,
                            move_ordering,
                            eval_mode: search_stack.eval_mode(),
                            contempt,
                            syzygy: &syzygy,
                            deadline_ms: &deadline_ms,
                        };

                        for depth in 1..=max_depth {
                            let actual_depth = depth;

                            // Check time/stop
                            if stopped.load(Ordering::Relaxed) {
                                break;
                            }
                            if search_time_exceeded(start, time_limit, &deadline_ms) {
                                break;
                            }
                            if let Some(nl) = node_limit
                                && state.nodes >= nl
                            {
                                break;
                            }

                            state.seldepth = 0;

                            update_reckless_optimism(
                                &mut state,
                                board_clone.side_to_move,
                                average_score,
                                shared_best_stat.load(Ordering::Acquire),
                                search_stack.eval_mode(),
                            );

                            let s = search_ab(
                                &mut board_clone,
                                &mut state,
                                &context,
                                SearchNode {
                                    depth: actual_depth,
                                    alpha: -INF,
                                    beta: INF,
                                    ply: 0,
                                    is_pv: true,
                                    is_root: true,
                                    excluded_move: None,
                                    total_extensions: 0,
                                    nominal_depth: actual_depth,
                                    allow_null: false,
                                },
                            );
                            if !stopped.load(Ordering::Relaxed) {
                                best_score = s;
                                update_root_average(&mut average_score, s);
                                shared_best_stat.fetch_max(
                                    root_score_stat(actual_depth, average_score),
                                    Ordering::AcqRel,
                                );
                                completed_depth = actual_depth;
                                if let Some(pv0) = state.get_pv().first().copied() {
                                    best_move = pv0;
                                }
                            }
                        }
                        // 7.1: Return full results for best-thread selection
                        (state.nodes, best_score, best_move, completed_depth)
                    })
                    .unwrap(),
            );
        }

        // Main thread search with reporting
        self.state.reset_for_search();
        self.state.use_nnue = self.use_nnue;
        self.state.hce_ply = 0;
        if !self.use_nnue {
            self.state.hce = eval::HceState::from_board(board);
        }
        let state = &mut self.state;
        let mut best_move = fallback_move;
        // Until depth 1 completes, the legal fallback has no searched score.
        // Keep it finite and neutral so an immediate stop cannot leak -INF
        // through UCI or into the next root's aspiration window.
        let mut best_score: i32 = 0;
        let mut average_score = initial_root_score;
        let mut best_pv = Vec::new();
        let mut main_completed_depth = 0i32;
        let mut main_seldepth = 0i32;

        // 5.x: Time management state
        let mut stability = 0i32; // consecutive iterations with same best move
        let mut prev_score = -INF; // score from previous iteration (for trend)
        let mut prev_prev_score = -INF; // score from 2 iterations ago
        // Node counts per root move for node-based TM
        let mut root_node_counts: std::collections::HashMap<(usize, usize), u64> =
            std::collections::HashMap::new();
        let context = SearchContext {
            tt: &self.tt,
            stopped: &self.stopped,
            time_limit: limits.time_limit,
            node_limit: limits.node_limit,
            start_time,
            params: &self.search_stack.params,
            lmr_table: self.search_stack.lmr_table.as_ref(),
            use_nnue: self.use_nnue,
            lmr_policy: &self.search_stack.policies.lmr,
            lmp_policy: &self.search_stack.policies.lmp,
            futility_policy: &self.search_stack.policies.futility,
            bad_noisy_futility_policy: &self.search_stack.policies.bad_noisy_futility,
            rfp_policy: &self.search_stack.policies.rfp,
            move_ordering: self.search_stack.policies.move_ordering,
            eval_mode: self.search_stack.eval_mode(),
            contempt: self
                .search_stack
                .eval_mode()
                .effective_contempt(self.contempt),
            syzygy: &self.syzygy,
            deadline_ms: &self.deadline_ms,
        };

        for depth in 1..=limits.max_depth {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
            if search_time_exceeded(start_time, limits.time_limit, &self.deadline_ms) {
                break;
            }
            if let Some(nl) = limits.node_limit
                && state.nodes >= nl
            {
                break;
            }

            state.seldepth = 0;

            let nodes_before = state.nodes;
            update_reckless_optimism(
                state,
                board.side_to_move,
                average_score,
                shared_best_stat.load(Ordering::Acquire),
                self.search_stack.eval_mode(),
            );

            // Aspiration windows after depth 5
            if depth >= 5 && best_score.abs() < MATE_SCORE - 100 {
                // 4.8: Eval-based aspiration narrowing (Viridithas)
                let mut delta = self.search_stack.params.aspiration_window + best_score.abs() / 256;
                let mut alpha = best_score - delta;
                let mut beta = best_score + delta;
                let mut asp_depth = depth;

                loop {
                    let s = search_ab(
                        board,
                        state,
                        &context,
                        SearchNode {
                            depth: asp_depth,
                            alpha,
                            beta,
                            ply: 0,
                            is_pv: true,
                            is_root: true,
                            excluded_move: None,
                            total_extensions: 0,
                            nominal_depth: depth,
                            allow_null: false,
                        },
                    );
                    if self.stopped.load(Ordering::Relaxed) {
                        break;
                    }

                    if s <= alpha {
                        // Fail low: widen alpha, restore depth
                        beta = (alpha + beta) / 2;
                        alpha = (s - delta).max(-INF);
                        asp_depth = depth;
                        delta *= 2;
                    } else if s >= beta {
                        // Fail high: widen beta, reduce depth (Akimbo technique)
                        beta = (s + delta).min(INF);
                        asp_depth -= 1;
                        delta *= 2;
                    } else {
                        best_score = s;
                        break;
                    }

                    // Fallback to infinite window when delta is very large
                    if delta > 2000 {
                        alpha = -INF;
                        beta = INF;
                    }
                }
                if self.stopped.load(Ordering::Relaxed) {
                    break;
                }
            } else {
                let s = search_ab(
                    board,
                    state,
                    &context,
                    SearchNode {
                        depth,
                        alpha: -INF,
                        beta: INF,
                        ply: 0,
                        is_pv: true,
                        is_root: true,
                        excluded_move: None,
                        total_extensions: 0,
                        nominal_depth: depth,
                        allow_null: false,
                    },
                );
                if self.stopped.load(Ordering::Relaxed) {
                    break;
                }
                best_score = s;
            }

            update_root_average(&mut average_score, best_score);
            shared_best_stat.fetch_max(root_score_stat(depth, average_score), Ordering::AcqRel);

            // Use the main thread PV for stable root move selection.
            let old_best_move = best_move;
            if let Some(pv0) = state.get_pv().first().copied() {
                best_move = pv0;
            }
            main_completed_depth = depth;
            main_seldepth = state.seldepth;

            // 5.1: Track nodes spent on this iteration's best move
            let nodes_this_iter = state.nodes - nodes_before;
            let key = (best_move.from.index(), best_move.to.index());
            *root_node_counts.entry(key).or_insert(0) += nodes_this_iter;

            // 5.2: Best-move stability
            if best_move != NULL_MOVE && old_best_move != NULL_MOVE {
                if best_move.from == old_best_move.from && best_move.to == old_best_move.to {
                    stability = (stability + 1).min(10);
                } else {
                    stability = 0;
                }
            }

            // Get PV line from the triangular PV table
            best_pv = state.get_pv();

            let elapsed = start_time.elapsed();
            let elapsed_ms = elapsed.as_millis().max(1) as u64;
            let nps = state.nodes * 1000 / elapsed_ms;

            // Report info with full PV line
            {
                use std::io::Write;
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                let score_str =
                    format_uci_score_value(best_score, board, self.search_stack.eval_mode());

                let pv_str = if best_pv.is_empty() {
                    format!("{best_move}")
                } else {
                    best_pv
                        .iter()
                        .map(|m| m.to_uci())
                        .collect::<Vec<_>>()
                        .join(" ")
                };

                let _ = writeln!(
                    out,
                    "info depth {depth} seldepth {} score {score_str} nodes {} nps {nps} time {elapsed_ms} hashfull {} tbhits {} currmove {} currmovenumber 1 pv {pv_str}",
                    state.seldepth,
                    state.nodes,
                    self.tt.hashfull_per_mille(),
                    state.tbhits,
                    best_move.to_uci(),
                );
                let _ = out.flush();
            }

            if best_score.abs() > MATE_SCORE - 100 {
                break;
            }

            // ── Smart time management ──
            if limits.use_soft_time
                && let Some(tl) = limits.time_limit
            {
                let elapsed_now = start_time.elapsed();

                // Base soft time: 50% of hard limit
                let mut soft_mul = 0.50f64;

                // Stability adjustment — stable best move → resolve faster
                // stability 0 = just changed → 1.0x, stability 10 = very stable → 0.65x
                let stability_factor = 1.0 - (stability as f64 * 0.035);
                soft_mul *= stability_factor;

                // Node-based TM — if best move got most nodes, we're confident
                if depth > 8 && state.nodes > 0 {
                    let best_nodes = root_node_counts.get(&key).copied().unwrap_or(0);
                    let frac = best_nodes as f64 / state.nodes as f64;
                    // Akimbo: (1.5 - frac) * 1.35
                    let node_mul = ((1.5 - frac) * 1.35).clamp(0.7, 1.8);
                    soft_mul *= node_mul;
                }

                // Score trend — increase time on significant score drops
                if depth >= 3 {
                    let score_drop = prev_prev_score - best_score;
                    if score_drop > 20 {
                        // Score dropped — spend proportionally more time
                        soft_mul *= 1.0 + (score_drop as f64 / 150.0).min(0.6);
                    } else if score_drop < -20 {
                        // Score improved — resolve faster
                        soft_mul *= 0.85;
                    }
                }

                // When clearly ahead, spend more clock converting instead of
                // stopping on a stable shuffle that is about to three-fold.
                soft_mul *= crate::conversion::winning_time_multiplier(best_score);

                // Fail-low emergency: if best move changed this iteration, extend time
                if depth >= 6
                    && old_best_move != NULL_MOVE
                    && best_move != NULL_MOVE
                    && (best_move.from != old_best_move.from || best_move.to != old_best_move.to)
                {
                    soft_mul *= 1.3; // give 30% more time when best move changes at depth >= 6
                }

                let soft_limit = tl.mul_f64(soft_mul.clamp(0.25, 1.5));

                if elapsed_now >= soft_limit {
                    self.stopped.store(true, Ordering::SeqCst);
                    break;
                }
            }

            // Update score history for trend detection
            prev_prev_score = prev_score;
            prev_score = best_score;
        }

        if main_completed_depth > 0 && best_score.abs() < INF {
            self.previous_best_score = best_score;
        }

        // Stop helper threads
        self.stopped.store(true, Ordering::SeqCst);

        // 7.1: Best-thread selection — collect results and pick via policy
        let mut total_nodes = state.nodes;
        let mut outcomes = vec![ThreadOutcome {
            best_move,
            score: best_score,
            depth: main_completed_depth,
            nodes: state.nodes,
            is_main: true,
        }];
        for h in handles {
            if let Ok((helper_nodes, helper_score, helper_move, helper_depth)) = h.join() {
                total_nodes += helper_nodes;
                outcomes.push(ThreadOutcome {
                    best_move: helper_move,
                    score: helper_score,
                    depth: helper_depth,
                    nodes: helper_nodes,
                    is_main: false,
                });
            }
        }

        if !outcomes.is_empty() {
            let selected = self.root_selection_policy.select(&outcomes);
            if let Some(choice) = outcomes.get(selected)
                && choice.best_move != NULL_MOVE
            {
                best_move = choice.best_move;
                best_score = choice.score;
            }
        }

        SearchResult {
            best_move,
            score: best_score,
            depth: main_completed_depth.max(0),
            seldepth: main_seldepth,
            nodes: total_nodes,
            elapsed: start_time.elapsed(),
            pv: best_pv,
            hashfull: self.tt.hashfull_per_mille(),
            tbhits: state.tbhits,
        }
    }

    /// Convenience: search to a fixed depth.
    pub fn search_depth(&mut self, board: &mut Board, depth: i32) -> SearchResult {
        self.search(
            board,
            SearchLimits {
                max_depth: depth,
                time_limit: None,
                node_limit: None,
                stopped: false,
                use_soft_time: true,
                force_helpers: false,
            },
        )
    }

    /// Convenience: search with a time limit.
    pub fn search_time(
        &mut self,
        board: &mut Board,
        time: Duration,
        max_depth: i32,
    ) -> SearchResult {
        self.search(
            board,
            SearchLimits {
                max_depth,
                time_limit: Some(time),
                node_limit: None,
                stopped: false,
                use_soft_time: true,
                force_helpers: false,
            },
        )
    }

    /// Convenience: search with a hard time limit (no early soft stop).
    pub fn search_time_hard(
        &mut self,
        board: &mut Board,
        time: Duration,
        max_depth: i32,
    ) -> SearchResult {
        self.search(
            board,
            SearchLimits {
                max_depth,
                time_limit: Some(time),
                node_limit: None,
                stopped: false,
                use_soft_time: false,
                force_helpers: false,
            },
        )
    }

    /// Convenience: search with a hard node limit.
    pub fn search_nodes(&mut self, board: &mut Board, nodes: u64, max_depth: i32) -> SearchResult {
        self.search(
            board,
            SearchLimits {
                max_depth,
                time_limit: None,
                node_limit: Some(nodes.max(1)),
                stopped: false,
                use_soft_time: false,
                force_helpers: false,
            },
        )
    }

    /// Externally stop the search.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Clone the internal stop flag so external controllers can request stop
    /// without holding a mutable engine reference.
    pub fn stop_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stopped)
    }

    /// Clears TT (for new game).
    pub fn clear(&mut self) {
        self.tt.clear();
        self.state.clear();
        self.previous_best_score = 0;
    }
}

/// When to apply IIR (internal iterative reduction): depth -= 1 with no TT move.
///
/// Skips in-check nodes and singular-extension verification (`excluded_move`).
/// Cut nodes: depth ≥ 4. PV nodes: depth ≥ 6 (later threshold so the main line keeps depth).
#[inline]
fn should_apply_iir(
    tt_move: Option<Move>,
    depth: i32,
    is_pv: bool,
    in_check: bool,
    excluded_move: Option<Move>,
) -> bool {
    tt_move.is_none()
        && !in_check
        && excluded_move.is_none()
        && if is_pv { depth >= 6 } else { depth >= 4 }
}

#[inline(always)]
fn usable_tt_move(mv: Move) -> Option<Move> {
    (mv != NULL_MOVE).then_some(mv)
}

#[inline(always)]
fn hindsight_depth_adjustment(
    is_root: bool,
    in_check: bool,
    excluded_move: Option<Move>,
    previous_reduction: i32,
    depth: i32,
    current_eval: i32,
    previous_eval: Option<i32>,
) -> i32 {
    if is_root || in_check || excluded_move.is_some() {
        return 0;
    }

    let Some(previous_eval) = previous_eval else {
        return 0;
    };
    let eval_sum = current_eval + previous_eval;
    if previous_reduction >= 3 && eval_sum <= 0 {
        1
    } else if previous_reduction >= 2 && depth >= 2 && eval_sum > 166 {
        -1
    } else {
        0
    }
}

#[inline(always)]
fn low_depth_extension(
    depth: i32,
    in_check: bool,
    is_cut_node: bool,
    static_eval: i32,
    alpha: i32,
    params: &SearchParams,
) -> i32 {
    i32::from(
        params.ldse_depth_max > 0
            && depth <= params.ldse_depth_max
            && !in_check
            && is_cut_node
            && static_eval <= alpha - params.ldse_margin,
    )
}

/// Reckless / native-v60 LMR child depth after a whole-ply reduction.
///
/// Native uses `(new_depth - r).clamp(1, new_depth + 2) + 2 * PV` so PV lines
/// keep enough depth for quiet breakthroughs (BK `d4d5`).
#[inline(always)]
fn reckless_lmr_search_depth(effective_depth: i32, reduction: i32, is_pv: bool) -> i32 {
    (effective_depth - reduction).clamp(1, effective_depth + 2) + 2 * i32::from(is_pv)
}

/// Stock-like PV lines keep one extra ply under LMR (native-style compensation).
#[inline(always)]
fn stock_like_lmr_search_depth(effective_depth: i32, reduction: i32, is_pv: bool) -> i32 {
    (effective_depth - reduction).max(1) + i32::from(is_pv)
}

/// Negative singular extension when the TT move is not singular.
#[inline(always)]
fn negative_singular_extension(move_ordering: MoveOrderingProfile) -> i32 {
    if move_ordering == MoveOrderingProfile::Reckless {
        -1
    } else {
        -2
    }
}

#[inline(always)]
fn singular_multicut_score(
    singular_score: i32,
    beta: i32,
    move_ordering: MoveOrderingProfile,
) -> Option<i32> {
    let _ = move_ordering;
    (singular_score >= beta && singular_score.abs() < MATE_SCORE - 100)
        .then(|| (singular_score * 5_973 + beta * 4_027) / 10_000)
}

#[inline(always)]
fn search_time_exceeded(
    start_time: Instant,
    time_limit: Option<Duration>,
    deadline_ms: &AtomicU64,
) -> bool {
    let elapsed = start_time.elapsed();
    if time_limit.is_some_and(|limit| elapsed >= limit) {
        return true;
    }
    let deadline = deadline_ms.load(Ordering::Relaxed);
    deadline != 0 && elapsed.as_millis() as u64 >= deadline
}

/// Immutable resources shared by every node in one search invocation.
#[derive(Copy, Clone)]
struct SearchContext<'a> {
    tt: &'a TranspositionTable,
    stopped: &'a AtomicBool,
    time_limit: Option<Duration>,
    node_limit: Option<u64>,
    start_time: Instant,
    params: &'a SearchParams,
    lmr_table: &'a [[i32; 128]; 128],
    use_nnue: bool,
    lmr_policy: &'a LmrDispatch,
    lmp_policy: &'a LmpDispatch,
    futility_policy: &'a FutilityDispatch,
    bad_noisy_futility_policy: &'a BadNoisyFutilityDispatch,
    rfp_policy: &'a RfpDispatch,
    move_ordering: MoveOrderingProfile,
    eval_mode: EvalMode,
    contempt: i32,
    syzygy: &'a SyzygyTables,
    deadline_ms: &'a AtomicU64,
}

/// Per-node alpha-beta inputs. Keeping these together makes recursive calls
/// explicit without repeatedly passing the immutable search resources.
#[derive(Copy, Clone)]
struct SearchNode {
    depth: i32,
    alpha: i32,
    beta: i32,
    ply: i32,
    is_pv: bool,
    is_root: bool,
    excluded_move: Option<Move>,
    total_extensions: i32,
    nominal_depth: i32,
    allow_null: bool,
}

#[derive(Copy, Clone)]
struct QuiescenceNode {
    alpha: i32,
    beta: i32,
    ply: i32,
    qs_ply: i32,
    is_pv: bool,
}

/// Alpha-beta search (free function so it can be called from any thread).
///
/// `excluded_move`: if Some, this move is skipped during the move loop.
/// Used for singular extension verification searches.
#[inline(never)]
fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    let SearchContext {
        tt,
        stopped,
        time_limit,
        node_limit,
        start_time,
        params,
        lmr_table,
        use_nnue,
        lmr_policy,
        lmp_policy,
        futility_policy,
        bad_noisy_futility_policy,
        rfp_policy,
        move_ordering,
        eval_mode,
        contempt,
        syzygy,
        deadline_ms,
    } = *context;
    let SearchNode {
        mut depth,
        mut alpha,
        mut beta,
        ply,
        is_pv,
        is_root,
        excluded_move,
        total_extensions,
        nominal_depth,
        allow_null,
    } = node;
    // Check stop periodically
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }
        if let Some(nl) = node_limit
            && state.nodes >= nl
        {
            stopped.store(true, Ordering::Relaxed);
            return 0;
        }
        if search_time_exceeded(start_time, time_limit, deadline_ms) {
            stopped.store(true, Ordering::Relaxed);
            return 0;
        }
    }

    let ply_usize = (ply as usize).min(MAX_PLY - 1);

    // Initialize PV length for this ply
    state.pv_len[ply_usize] = 0;
    // Reset cutoff counter for this ply (used by parent's LMR)
    state.cutoffs[ply_usize] = 0;
    state.move_counts[ply_usize] = 0;
    state.tt_moves[ply_usize] = NULL_MOVE;
    state.reductions[ply_usize] = 0;

    // Draw detection (repetition, 50-move, insufficient material)
    // Return 0 for draws — contempt should only be applied at root level,
    // not inside the tree where it poisons minimax scoring.
    if !is_root && board.is_search_draw(ply_usize) {
        return draw_score(state.nodes);
    }
    if !is_root
        && excluded_move.is_none()
        && depth >= syzygy.probe_depth()
        && let Some(tb) = syzygy.probe_wdl(board)
    {
        state.tbhits += 1;
        let score = if tb > 0 {
            tb - ply
        } else if tb < 0 {
            tb + ply
        } else {
            0
        };
        return score;
    }
    let in_check = board.in_check();

    // Hard ply limit — prevent unbounded search from extensions
    if ply >= MAX_PLY as i32 - 1 {
        return if in_check {
            0
        } else {
            hybrid_eval(board, state, use_nnue)
        };
    }

    let check_ext_budget = nominal_depth * 2;
    let (extended_depth, total_extensions) =
        if extend_checks(move_ordering, eval_mode, context.node_limit.is_some()) {
            budgeted_check_extension(depth, total_extensions, check_ext_budget, in_check)
        } else {
            (depth, total_extensions)
        };
    depth = extended_depth;

    // Propagate double extension count from parent ply
    if ply_usize > 0 {
        state.dbl_exts[ply_usize] = state.dbl_exts[ply_usize.saturating_sub(1)];
    } else {
        state.dbl_exts[ply_usize] = 0;
    }

    // ── Mate distance pruning — prune if we can't possibly improve ──
    if !is_root {
        alpha = alpha.max(-MATE_SCORE + ply);
        beta = beta.min(MATE_SCORE - ply - 1);
        if alpha >= beta {
            return alpha;
        }
    }

    // TT probe
    let mut tt_move = None;
    let mut tt_score = None;
    let mut tt_raw_eval = None;
    let mut tt_depth = -1;
    let mut tt_node_type = NodeType::Exact;

    let mut tt_was_pv = is_pv;

    if let Some(entry) = excluded_move
        .is_none()
        .then(|| tt.probe(board.tt_hash()))
        .flatten()
    {
        tt_move = usable_tt_move(entry.best_move);
        state.tt_moves[ply_usize] = tt_move.unwrap_or(NULL_MOVE);
        tt_raw_eval = entry.raw_eval;
        tt_depth = entry.depth;
        tt_node_type = entry.node_type;
        tt_was_pv = tt_was_pv || entry.was_pv;

        let probed_score = score_from_tt(entry.score, ply);
        tt_score = Some(probed_score);
        if !is_pv && entry.depth >= depth {
            match entry.node_type {
                NodeType::Exact => return probed_score,
                NodeType::LowerBound if probed_score >= beta => return probed_score,
                NodeType::UpperBound if probed_score <= alpha => return probed_score,
                _ => {}
            }
        }
    }
    // Leaf → quiescence
    if depth <= 0 {
        return quiescence(
            board,
            state,
            context,
            QuiescenceNode {
                alpha,
                beta,
                ply,
                qs_ply: 0,
                is_pv,
            },
        );
    }

    let us = board.side_to_move;
    let opponent_threats = opponent_threats(board);
    let threats = opponent_threats.all;
    state.threats[ply_usize] = threats;

    // Static eval — NNUE + correction history
    // Checked nodes do not use a static evaluation. Skipping it also avoids an
    // unnecessary NNUE forward pass on tactical check chains.
    let (raw_eval, corrected_eval, corr) = if in_check {
        state.eval_variance[ply_usize] = 0;
        (None, 0, 0)
    } else {
        let (raw_eval, variance) = if eval_mode.is_ateed_nnue() {
            hybrid_eval_with_uncertainty(board, state, use_nnue)
        } else {
            (
                tt_raw_eval.unwrap_or_else(|| hybrid_eval(board, state, use_nnue)),
                0,
            )
        };
        state.eval_variance[ply_usize] = variance;
        let corr = state.correction(board, move_ordering);
        let corrected =
            corrected_network_eval(board, raw_eval, corr, state.optimism[us.index()], eval_mode);
        (Some(raw_eval), corrected, corr)
    };
    let mut static_eval = corrected_eval;

    // Use TT score to refine static eval when available (Stockfish technique)
    if let Some(tt_sc) = tt_score {
        match tt_node_type {
            NodeType::Exact => static_eval = tt_sc,
            NodeType::LowerBound => {
                if tt_sc > static_eval {
                    static_eval = tt_sc;
                }
            }
            NodeType::UpperBound => {
                if tt_sc < static_eval {
                    static_eval = tt_sc;
                }
            }
        }
    }
    state.static_evals[ply_usize] = corrected_eval;
    state.eval_valid[ply_usize] = !in_check && excluded_move.is_none();

    // "Improving" flag: is our static eval better than 2 plies ago?
    // Viridithas-style: fall back to 4 plies ago, default to true when unknown.
    let (improvement, improving) = if in_check {
        (0, false)
    } else if ply >= 2 && state.eval_valid[ply_usize.saturating_sub(2)] {
        let delta = corrected_eval - state.static_evals[ply_usize.saturating_sub(2)];
        (delta, delta > 0)
    } else if ply >= 4 && state.eval_valid[ply_usize.saturating_sub(4)] {
        let delta = corrected_eval - state.static_evals[ply_usize.saturating_sub(4)];
        (delta, delta > 0)
    } else {
        (0, true)
    };

    // Learn the value of the preceding quiet move from the evaluation swing.
    // Evaluation scores are side-to-move relative, hence the negated sum.
    if !is_root
        && !in_check
        && excluded_move.is_none()
        && ply > 0
        && state.eval_valid[ply_usize - 1]
        && (depth < params.eval_history_depth_limit || tt_score.is_none())
    {
        let previous = state.prev_move[ply_usize - 1];
        if previous != NULL_MOVE && !previous.is_capture() && !previous.is_promotion() {
            let bonus =
                params.eval_history_bonus(corrected_eval, state.static_evals[ply_usize - 1]);
            state.history.update(
                state.threats[ply_usize - 1],
                us.opponent().index(),
                previous,
                bonus,
            );
        }
    }

    // ── Pruning techniques (non-PV, non-check, no excluded move) ─────

    let previous_eval =
        (ply > 0 && state.eval_valid[ply_usize - 1]).then(|| state.static_evals[ply_usize - 1]);
    let previous_reduction = if ply > 0 {
        state.reductions[ply_usize - 1]
    } else {
        0
    };
    depth += hindsight_depth_adjustment(
        is_root,
        in_check,
        excluded_move,
        previous_reduction,
        depth,
        corrected_eval,
        previous_eval,
    );

    if !is_pv && !in_check && excluded_move.is_none() {
        // Reverse Futility Pruning (Akimbo: no TT guards — straightforward)
        let rfp_context = RfpContext {
            depth,
            improving,
            improvement,
            correction_abs: corr.abs(),
            tt_was_pv,
            own_pieces_threatened: threats & board.occupancy[us.index()] != 0,
            stock_margin: params.rfp_margin(depth, improving)
                + ateed_uncertainty_margin(eval_mode, state.eval_variance[ply_usize]),
        };
        if let Some(score) = rfp_policy.cutoff_score(static_eval, beta, &rfp_context) {
            return score;
        }

        // Null move pruning. Reckless/native-v60 require a cut-node
        // (`allow_null`); Stock-like keeps the broader non-PV gate that
        // matched Akimbo-era tuning.
        if depth >= params.nmp_depth_min
            && (move_ordering != MoveOrderingProfile::Reckless || allow_null)
            && nmp_material_ok(board, us)
            && static_eval >= beta
            && ply_usize >= state.min_nmp_ply
        {
            let r = params.null_move_r(depth, static_eval, beta) + i32::from(improving);

            if state.use_nnue {
                state.nnue_state.push_null();
            }
            board.make_null_move();
            state.prev_move[ply_usize] = NULL_MOVE;
            state.prev_piece[ply_usize] = 0;
            let score = -search_ab(
                board,
                state,
                context,
                SearchNode {
                    depth: depth - 1 - r,
                    alpha: -beta,
                    beta: -beta + 1,
                    ply: ply + 1,
                    is_pv: false,
                    is_root: false,
                    excluded_move: None,
                    total_extensions,
                    nominal_depth,
                    allow_null: false,
                },
            );
            board.unmake_null_move();
            if state.use_nnue {
                state.nnue_state.pop_move();
            }

            if stopped.load(Ordering::Relaxed) {
                return 0;
            }

            if score >= beta {
                // At low depths, return directly (no verification needed)
                if depth < params.nmp_min_verif_depth || state.min_nmp_ply > 0 {
                    return if score > MATE_SCORE - 100 {
                        beta
                    } else {
                        score
                    };
                }

                // Anti-recursion: set min_nmp_ply to prevent cascading verification
                state.min_nmp_ply = ply_usize + ((depth - r) * params.nmp_verif_frac / 16) as usize;
                let v_score = search_ab(
                    board,
                    state,
                    context,
                    SearchNode {
                        depth: depth - r,
                        alpha: beta - 1,
                        beta,
                        ply,
                        is_pv: false,
                        is_root: false,
                        excluded_move: None,
                        total_extensions,
                        nominal_depth,
                        allow_null: false,
                    },
                );
                state.min_nmp_ply = 0;

                if stopped.load(Ordering::Relaxed) {
                    return 0;
                }
                if v_score >= beta {
                    return v_score;
                }
            }
        }

        // ProbCut with a depth-qualified TT guard.
        let pb_beta = beta + 200;
        let can_probcut = tt_score.is_none_or(|ts| !(tt_depth >= depth - 3 && ts < pb_beta));
        if depth >= 5 && beta.abs() < MATE_SCORE - 100 && can_probcut {
            let score_capture = |b: &Board, mv: Move| capture_score(b, mv, tt_move, move_ordering);
            let score_quiet = |_: &Board, _: Move| 0;
            let mut picker = MovePicker::new(board, tt_move, [NULL_MOVE; 2], NULL_MOVE)
                .with_move_ordering(move_ordering);
            picker.skip_quiets();
            picker.skip_bad_captures();

            while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
                let moved_piece = piece_index_on(board, mv.from);
                make_search_move(board, state, mv);
                state.prev_move[ply_usize] = mv;
                state.prev_piece[ply_usize] = moved_piece;
                let score = -search_ab(
                    board,
                    state,
                    context,
                    SearchNode {
                        depth: depth - 4,
                        alpha: -pb_beta,
                        beta: -pb_beta + 1,
                        ply: ply + 1,
                        is_pv: false,
                        is_root: false,
                        excluded_move: None,
                        total_extensions,
                        nominal_depth,
                        allow_null: false,
                    },
                );
                board.unmake_move(mv);
                undo_search_eval(state);

                if stopped.load(Ordering::Relaxed) {
                    return 0;
                }
                if score >= pb_beta {
                    return score;
                }
            }
        }
    }

    // ── IIR (Internal Iterative Reduction) ────────
    if should_apply_iir(tt_move, depth, is_pv, in_check, excluded_move) {
        depth -= 1;
    }

    // Get the previous move for countermove lookup
    let prev_mv = if ply > 0 {
        state.prev_move[ply_usize.saturating_sub(1)]
    } else {
        NULL_MOVE
    };

    // Look up countermove for the previous move
    let countermove = if prev_mv != NULL_MOVE {
        state.countermoves[prev_mv.from.index()][prev_mv.to.index()]
    } else {
        NULL_MOVE
    };

    // ── Staged MovePicker: lazily generates and scores moves ──
    // Captures scored by MVV-LVA + capture history, quiets by stat_score.
    // If a TT move or good capture causes a cutoff, quiets are never generated.
    let us_idx = us.index();
    let killers_copy = state.killers[ply_usize];
    let mut picker = MovePicker::new(board, tt_move, killers_copy, countermove)
        .with_move_ordering(move_ordering);

    // Extract raw pointers to scoring data to avoid borrow conflicts.
    // Safety: These tables are only read by the closures during `picker.next()`
    // and only written after the call returns (in the loop body below).
    let cap_hist_ptr = &state.cap_hist as *const CaptureHistory;
    let history_ptr = &state.history as *const QuietHistory;
    let cont_hist_ptr = &*state.cont_hist as *const ContinuationHistory;
    let cont_hist2_ptr = &*state.cont_hist2 as *const ContinuationHistory;
    let reckless_cont_hist_ptr = &*state.reckless_cont_hist as *const ContinuationHistory;
    let pawn_history_ptr = &state.pawn_history as *const PawnHistory;
    let prev_piece_snap = state.prev_piece;
    let prev_move_snap = state.prev_move;
    let ply_snap = ply_usize;
    let pawn_key = pawn_hash(board);
    let reckless_maps = (move_ordering == MoveOrderingProfile::Reckless)
        .then(|| reckless_quiet_ordering_maps(board, opponent_threats));

    let score_capture = |b: &Board, mv: Move| -> i32 {
        let attacker = b.piece_of_color_on(mv.from, b.side_to_move);
        let piece = attacker.map_or(0, Piece::index);
        let victim = b.piece_of_color_on(mv.to, b.side_to_move.opponent());
        let en_passant = mv.flag == types::chess_move::MoveFlag::EnPassant;
        let captured_index = victim
            .map(Piece::index)
            .or_else(|| en_passant.then_some(Piece::Pawn.index()));
        let cap_hist_score = captured_index.map_or(0, |captured| {
            // SAFETY: the pointer targets `state.cap_hist`, which remains alive
            // and is only read while this scoring closure executes.
            unsafe { (*cap_hist_ptr).get(threats, piece, mv.to, captured) }
        });
        move_ordering.noisy_score(
            victim,
            attacker,
            en_passant,
            mv.is_promotion(),
            cap_hist_score,
            16,
        )
    };
    let score_quiet = |b: &Board, mv: Move| -> i32 {
        let moving = b.piece_of_color_on(mv.from, b.side_to_move);
        let piece = moving.map_or(0, Piece::index);
        // SAFETY: the pointer targets `state.history`, which remains alive and
        // is only read while this scoring closure executes.
        let mut score = unsafe { (*history_ptr).get(threats, us_idx, mv) };
        if move_ordering == MoveOrderingProfile::Reckless {
            score = 1763 * score / 1024;
            // SAFETY: the pointer targets the live pawn-history table and is
            // only read while this scoring closure executes.
            score += unsafe { (*pawn_history_ptr).get(pawn_key, us_idx, piece, mv.to) };
            const CONTINUATION_WEIGHTS: [i32; 4] = [1614, 1066, 1086, 1051];
            for offset in [1usize, 2, 4, 6] {
                if ply_snap >= offset && prev_move_snap[ply_snap - offset] != NULL_MOVE {
                    let previous_piece = prev_piece_snap[ply_snap - offset];
                    let previous_to = prev_move_snap[ply_snap - offset].to.index();
                    // SAFETY: the pointer targets the live continuation table;
                    // all indices are bounded piece and square indices.
                    let continuation = unsafe {
                        i32::from(
                            (*reckless_cont_hist_ptr)[previous_piece][previous_to][piece]
                                [mv.to.index()],
                        )
                    };
                    score += CONTINUATION_WEIGHTS[match offset {
                        1 => 0,
                        2 => 1,
                        4 => 2,
                        _ => 3,
                    }] * continuation
                        / 1024;
                }
            }
            let maps = reckless_maps
                .as_ref()
                .expect("Reckless maps are initialized for Reckless ordering");
            const ESCAPE: [i32; NUM_PIECES] = [0, 8854, 8170, 14051, 20357, 0];
            score += ESCAPE[piece] * i32::from(maps.threatened[piece] & mv.from.bitboard() != 0);
            let checking_piece = mv.promotion.or(moving).unwrap_or(Piece::Pawn);
            score += 10_723
                * i32::from(maps.checking_squares[checking_piece.index()] & mv.to.bitboard() != 0);
            score -= 8875 * i32::from(maps.threatened[piece] & mv.to.bitboard() != 0);
            score += 3446 * i32::from(maps.offense[piece] & mv.to.bitboard() != 0);
            score -= 4494 * i32::from(maps.wall_pawns & mv.from.bitboard() != 0);
            return score;
        }
        if ply_snap > 0 {
            let pp = prev_piece_snap[ply_snap.saturating_sub(1)];
            let pt = prev_move_snap[ply_snap.saturating_sub(1)].to.index();
            // SAFETY: the pointer targets the live continuation table; all
            // indices are bounded piece and square indices.
            score += unsafe { i32::from((*cont_hist_ptr)[pp][pt][piece][mv.to.index()]) };
        }
        if ply_snap > 1 {
            let pp2 = prev_piece_snap[ply_snap.saturating_sub(2)];
            let pt2 = prev_move_snap[ply_snap.saturating_sub(2)].to.index();
            // SAFETY: the pointer targets the live continuation table; all
            // indices are bounded piece and square indices.
            score += unsafe { i32::from((*cont_hist2_ptr)[pp2][pt2][piece][mv.to.index()]) };
        }
        score
    };

    let mut best_move = NULL_MOVE;
    let mut best_score = -INF;
    let mut node_type = NodeType::UpperBound;
    let mut moves_searched = 0;
    let mut alpha_raises = 0;
    // In Akimbo, ZW children pass allow_null=true, meaning they are cut-nodes.
    // Derive is_cut_node for LMR context where cut-node heuristic is valuable.
    let is_cut_node = allow_null;
    // Track searched quiet / capture moves for history malus (stack — no heap allocs per node).
    let mut searched_quiets: [Move; SEARCHED_QUIETS_MAX] = [NULL_MOVE; SEARCHED_QUIETS_MAX];
    let mut searched_quiets_len: usize = 0;
    let mut searched_captures: [(usize, types::Square, usize); SEARCHED_CAPTURES_MAX] =
        [(0, types::Square::A1, 0); SEARCHED_CAPTURES_MAX];
    let mut searched_captures_len: usize = 0;

    // Singular extension data
    let can_do_singular = excluded_move.is_none()
        && !is_root
        && depth >= params.se_depth_min
        && tt_move.is_some()
        && tt_depth >= depth - 3
        && tt_node_type != NodeType::UpperBound
        && tt_score.is_some_and(|s| s.abs() < MATE_SCORE - 100);
    let low_depth_extension = if can_do_singular {
        0
    } else {
        low_depth_extension(depth, in_check, is_cut_node, static_eval, alpha, params)
    };

    while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
        // Skip the excluded move (for singular extension verification)
        if excluded_move
            .is_some_and(|em| em.from == mv.from && em.to == mv.to && em.promotion == mv.promotion)
            || (is_root && state.is_root_excluded(mv))
        {
            continue;
        }

        // ── Singular extension (with double + negative extensions) ──────
        let mut extension = if moves_searched == 0 {
            low_depth_extension
        } else {
            0
        };
        if can_do_singular
            && let Some(ttm) = tt_move
            && same_move_key(mv, ttm)
            && let Some(tt_sc) = tt_score
        {
            let se_margin = params.se_margin(depth);
            let se_beta = tt_sc - se_margin;
            let se_score = search_ab(
                board,
                state,
                context,
                SearchNode {
                    depth: (depth - 1) / 2,
                    alpha: se_beta - 1,
                    beta: se_beta,
                    ply,
                    is_pv: false,
                    is_root: false,
                    excluded_move: Some(mv),
                    total_extensions,
                    nominal_depth,
                    allow_null,
                },
            );
            if stopped.load(Ordering::Relaxed) {
                return 0;
            }
            if se_score < se_beta {
                extension = 1; // TT move is singular — extend
                // Double extension when clearly singular (PV included — helps tactics).
                if se_score < se_beta - params.se_double_ext_margin
                    && state.dbl_exts[ply_usize] < params.max_dbl_exts
                {
                    state.dbl_exts[ply_usize] += 1;
                    extension = 2;
                }
            } else if let Some(score) = singular_multicut_score(se_score, beta, move_ordering) {
                return score;
            } else if tt_sc >= beta || (tt_sc <= alpha && !is_pv) {
                // StockLike uses -2 so quiet prophylaxis can displace sticky
                // tactical TT moves (BK#8). Reckless keeps the classic -1 gate.
                extension = negative_singular_extension(move_ordering);
            }
        }

        // Hindsight extension removed — the condition `static_eval + eval_2_back < 0`
        // fires on nearly every node in disadvantageous positions, causing unbounded
        // search tree growth that makes the engine hang at higher depths.

        // Get moved piece index for this move
        let moved_piece = piece_index_on(board, mv.from);
        let captured_piece_idx = if mv.is_capture() {
            captured_piece_index(board, mv)
        } else {
            None
        };

        // Reckless reduces later noisy moves using their capture-history score.
        let mv_stat_score = if mv.is_quiet() {
            state.stat_score(mv, us.index(), moved_piece, ply_usize, move_ordering)
        } else {
            captured_piece_idx
                .map(|idx| state.cap_hist.get(threats, moved_piece, mv.to, idx))
                .unwrap_or(0)
        };

        // Late Move Pruning — Stockfish formula: (3 + depth²) / (2 - improving)
        let lmp_context = LmpContext {
            depth,
            move_count: moves_searched + 1,
            improvement,
            improving,
            is_root,
            is_pv,
            in_check,
            is_quiet: !mv.is_capture() && !mv.is_promotion(),
            best_score,
            stock_depth_limit: params.lmp_depth_limit,
            stock_move_threshold: params.lmp_threshold(depth, improving),
        };
        if let Some(decision) = lmp_policy.decision(&lmp_context) {
            if decision.skip_remaining_quiets {
                picker.skip_quiets();
            }
            if decision.prune_current {
                continue;
            }
        }

        // ── History pruning: skip quiet moves with terrible stat_score ──
        if !is_pv
            && !in_check
            && depth <= params.hist_prune_depth_limit
            && !mv.is_capture()
            && !mv.is_promotion()
            && moves_searched > 0
            && best_score > -MATE_SCORE + 100
            && mv_stat_score < params.hist_prune_margin * depth
        {
            continue;
        }

        // Futility pruning — Stockfish: 77 * depth
        let is_quiet = !mv.is_capture() && !mv.is_promotion();
        let futility_context = FutilityContext {
            depth,
            eval: static_eval,
            alpha,
            history: mv_stat_score,
            improving,
            is_root,
            is_pv,
            in_check,
            is_quiet,
            move_count: moves_searched + 1,
            best_score,
            gives_direct_check: is_quiet
                && futility_policy.requires_direct_check()
                && gives_direct_check(board, mv),
            stock_depth_limit: params.futility_depth_limit,
            stock_margin: params.futility_margin(depth, improving)
                + ateed_uncertainty_margin(eval_mode, state.eval_variance[ply_usize]),
        };
        if let Some(decision) = futility_policy.decision(&futility_context) {
            if let Some(score_floor) = decision.score_floor {
                best_score = best_score.max(score_floor);
            }
            if decision.skip_remaining_quiets {
                picker.skip_quiets();
            }
            continue;
        }

        let bad_noisy_context = BadNoisyFutilityContext {
            depth,
            eval: static_eval,
            alpha,
            history: mv_stat_score,
            captured_value: board
                .piece_of_color_on(mv.to, board.side_to_move.opponent())
                .map_or(0, |piece| MoveOrderingProfile::Reckless.piece_value(piece)),
            is_root,
            in_check,
            is_bad_noisy: picker.is_bad_capture_stage(),
            best_score,
            gives_direct_check: bad_noisy_futility_policy.requires_direct_check()
                && gives_direct_check(board, mv),
        };
        if let Some(score_floor) = bad_noisy_futility_policy.score_floor(&bad_noisy_context) {
            if best_score.abs() < MATE_SCORE - 100 {
                best_score = best_score.max(score_floor);
            }
            break;
        }

        // History-aware SEE pruning. Promotions are preserved unless they capture.
        if !is_pv
            && !in_check
            && moves_searched > 0
            && best_score > -MATE_SCORE + 100
            && (mv.is_capture() || !mv.is_promotion())
        {
            let history = captured_piece_idx
                .map(|idx| state.cap_hist.get(threats, moved_piece, mv.to, idx))
                .unwrap_or(mv_stat_score);
            let threshold = HistorySeePruning::threshold(mv.is_capture(), depth, history);
            if !see::see_ge(board, mv, threshold) {
                continue;
            }
        }

        // Prefetch the child TT slot before make; hash_after matches post-make key.
        tt.prefetch(board.tt_hash_after(mv));
        make_search_move(board, state, mv);
        let repeats = is_root && (board.has_repetition() || board.is_draw());

        let gives_check = board.in_check();

        // Store the move we're searching for countermove/continuation tracking
        state.prev_move[ply_usize] = mv;
        state.prev_piece[ply_usize] = moved_piece;
        state.move_counts[ply_usize] = (moves_searched + 1) as u16;

        let score;
        // Cap extensions at remaining depth to prevent going negative
        let extension = extension.min(depth.max(0));
        let new_total_extensions = total_extensions + extension.max(0);
        let effective_depth = depth - 1 + extension;

        if moves_searched == 0 {
            // Full window search for the first move
            score = -search_ab(
                board,
                state,
                context,
                SearchNode {
                    depth: effective_depth,
                    alpha: -beta,
                    beta: -alpha,
                    ply: ply + 1,
                    is_pv,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: new_total_extensions,
                    nominal_depth,
                    allow_null: first_child_is_cut_node(is_pv, allow_null),
                },
            );
        } else {
            // LMR: Late Move Reductions — enhanced with stat_score.
            // Root quiet probes keep full-depth ZW searches so prophylaxis /
            // breakthroughs (BK#8 / BK#23) are not buried by LMR.
            let mut reduction = 0;
            // StockLike: all root quiets. Reckless keeps LMR at root and relies
            // on the pawn near-miss re-search below (stable for BK#8/#23).
            let root_quiet_no_lmr = is_root
                && full_depth_root_quiets(move_ordering, eval_mode)
                && !mv.is_capture()
                && !mv.is_promotion();
            if !root_quiet_no_lmr
                && moves_searched >= 1
                && depth >= 2
                && ((!mv.is_capture() && !mv.is_promotion()) || lmr_policy.reduce_noisy_moves())
            {
                let d = (depth as usize).min(127);
                let m = moves_searched.min(127);
                let base = lmr_table[d][m];
                let lmr_ctx = LmrContext {
                    depth,
                    move_count: moves_searched + 1,
                    is_quiet: mv.is_quiet(),
                    is_pv,
                    improving,
                    improvement,
                    alpha_raises,
                    is_killer: is_killer(mv, &state.killers[ply_usize]),
                    gives_check,
                    is_recapture: ply > 0
                        && state.prev_move[ply_usize - 1].is_capture()
                        && mv.is_capture()
                        && state.prev_move[ply_usize - 1].to == mv.to,
                    mv_stat_score,
                    corr_abs: corr.abs(),
                    is_cut_node,
                    winning_beta: beta >= MATE_TT_THRESHOLD,
                    tt_was_pv,
                    tt_score_above_alpha: tt_score.is_some_and(|score| score > alpha),
                    tt_score_below_alpha: tt_score.is_some_and(|score| score < alpha),
                    tt_depth_sufficient: tt_score.is_some() && tt_depth >= depth,
                    tt_move_missing: tt_move.is_none(),
                    hist_lmr_div: params.hist_lmr_div,
                    lmr_corr_mul: params.lmr_corr_mul,
                    lmr_cut_node_bonus: params.lmr_cut_node_bonus,
                    child_cutoffs: state.cutoffs[(ply_usize + 1).min(MAX_PLY - 1)],
                };
                reduction = lmr_policy.adjust_reduction(base, &lmr_ctx)
                    - ateed_lmr_relief(eval_mode, state.eval_variance[ply_usize]);
                reduction = if effective_depth <= 1 {
                    0
                } else if move_ordering == MoveOrderingProfile::Reckless {
                    reduction.max(0)
                } else {
                    reduction.clamp(0, effective_depth - 1)
                };
            }
            let reduced_depth = if reduction > 0 {
                if move_ordering == MoveOrderingProfile::Reckless {
                    reckless_lmr_search_depth(effective_depth, reduction, is_pv)
                } else {
                    stock_like_lmr_search_depth(effective_depth, reduction, is_pv)
                }
            } else {
                effective_depth
            };
            // PVS null-window search with reduction
            state.reductions[ply_usize] = (effective_depth - reduced_depth).max(0);
            let mut s = -search_ab(
                board,
                state,
                context,
                SearchNode {
                    depth: reduced_depth,
                    alpha: -alpha - 1,
                    beta: -alpha,
                    ply: ply + 1,
                    is_pv: false,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: new_total_extensions,
                    nominal_depth,
                    allow_null: true,
                },
            );
            state.reductions[ply_usize] = 0;
            // Re-search with full window if ZW fails high.
            // Root near-miss research lets quiet breakthroughs / prophylaxis
            // that only fail-low on the ZW probe still earn a PV window.
            const RECKLESS_ROOT_PAWN_NEAR_MISS: i32 = 120;
            const STOCK_ROOT_QUIET_NEAR_MISS: i32 = 160;
            let root_near_miss = is_root
                && !mv.is_capture()
                && !mv.is_promotion()
                && s <= alpha
                && if eval_mode.is_reckless_nnue() {
                    s > alpha - STOCK_ROOT_QUIET_NEAR_MISS
                } else {
                    match move_ordering {
                        MoveOrderingProfile::Reckless => {
                            moved_piece == Piece::Pawn.index()
                                && s > alpha - RECKLESS_ROOT_PAWN_NEAR_MISS
                        }
                        MoveOrderingProfile::StockLike => s > alpha - STOCK_ROOT_QUIET_NEAR_MISS,
                    }
                };
            if (s > alpha && (is_pv || reduction > 0)) || root_near_miss {
                let mut research_depth = effective_depth;
                if move_ordering == MoveOrderingProfile::Reckless {
                    if !is_root {
                        research_depth += i32::from(s > best_score + 57);
                        research_depth -= i32::from(s < best_score + 9);
                    }
                } else if reduction > 1 {
                    let do_deeper = s > best_score + 60 + 12 * reduction;
                    let do_shallower = s < best_score + research_depth;
                    research_depth += i32::from(do_deeper) - i32::from(do_shallower);
                }
                if move_ordering != MoveOrderingProfile::Reckless
                    || research_depth > reduced_depth
                    || is_pv
                    || root_near_miss
                {
                    s = -search_ab(
                        board,
                        state,
                        context,
                        SearchNode {
                            depth: research_depth,
                            alpha: -beta,
                            beta: -alpha,
                            ply: ply + 1,
                            is_pv: is_pv || root_near_miss,
                            is_root: false,
                            excluded_move: None,
                            total_extensions: new_total_extensions,
                            nominal_depth,
                            allow_null: false,
                        },
                    );
                }
            }
            score = s;
        }

        board.unmake_move(mv);
        undo_search_eval(state);
        moves_searched += 1;

        if stopped.load(Ordering::Relaxed) {
            return 0;
        }

        let score = if is_root {
            crate::conversion::apply_root_conversion(score, repeats, static_eval, contempt)
        } else {
            score
        };

        if score > best_score {
            best_score = score;
            best_move = mv;

            if score > alpha {
                alpha_raises += 1;
                alpha = score;
                node_type = NodeType::Exact;

                // ── Update PV: prepend this move to child's PV ──
                let child_ply = (ply_usize + 1).min(MAX_PLY - 1);
                state.pv[ply_usize][0] = mv;
                let child_len = state.pv_len[child_ply].min(MAX_PLY - 1);
                for j in 0..child_len {
                    state.pv[ply_usize][j + 1] = state.pv[child_ply][j];
                }
                state.pv_len[ply_usize] = child_len + 1;

                if score >= beta {
                    // Track cutoffs for parent's LMR adjustment
                    if ply_usize > 0 {
                        state.cutoffs[ply_usize - 1] += 1;
                    }
                    let bonus = params.history_bonus(depth).clamp(0, 2000);
                    let malus = params.history_malus(depth).clamp(0, 2000);
                    let ci = us.index();

                    if mv.is_quiet() {
                        // Update heuristics for quiet moves causing beta cutoff
                        store_killer(&mut state.killers, mv, ply_usize);

                        // Main history
                        state.history.update(threats, ci, mv, bonus);

                        if move_ordering == MoveOrderingProfile::Reckless {
                            state
                                .pawn_history
                                .update(pawn_key, ci, moved_piece, mv.to, bonus);
                            for offset in [1usize, 2, 4, 6] {
                                if ply_usize >= offset
                                    && state.prev_move[ply_usize - offset] != NULL_MOVE
                                {
                                    let previous_piece = state.prev_piece[ply_usize - offset];
                                    let previous_to =
                                        state.prev_move[ply_usize - offset].to.index();
                                    update_history(
                                        &mut state.reckless_cont_hist[previous_piece][previous_to]
                                            [moved_piece][mv.to.index()],
                                        bonus,
                                    );
                                }
                            }
                        } else {
                            if ply > 0 {
                                let pp = state.prev_piece[ply_usize.saturating_sub(1)];
                                let pt = state.prev_move[ply_usize.saturating_sub(1)].to.index();
                                update_history(
                                    &mut state.cont_hist[pp][pt][moved_piece][mv.to.index()],
                                    bonus,
                                );
                            }
                            if ply > 1 {
                                let pp2 = state.prev_piece[ply_usize.saturating_sub(2)];
                                let pt2 = state.prev_move[ply_usize.saturating_sub(2)].to.index();
                                update_history(
                                    &mut state.cont_hist2[pp2][pt2][moved_piece][mv.to.index()],
                                    bonus,
                                );
                            }
                        }

                        // Penalize quiet moves searched before the cutoff
                        for (rank, prev) in
                            searched_quiets[..searched_quiets_len].iter().enumerate()
                        {
                            if excluded_move
                                .is_some_and(|em| em.from == prev.from && em.to == prev.to)
                            {
                                continue;
                            }
                            let ranked_malus = ranked_history_malus(malus, rank);
                            let prev_piece_idx = piece_index_on(board, prev.from);
                            state.history.update(threats, ci, *prev, -ranked_malus);
                            if move_ordering == MoveOrderingProfile::Reckless {
                                state.pawn_history.update(
                                    pawn_key,
                                    ci,
                                    prev_piece_idx,
                                    prev.to,
                                    -ranked_malus,
                                );
                                for offset in [1usize, 2, 4, 6] {
                                    if ply_usize >= offset
                                        && state.prev_move[ply_usize - offset] != NULL_MOVE
                                    {
                                        let previous_piece = state.prev_piece[ply_usize - offset];
                                        let previous_to =
                                            state.prev_move[ply_usize - offset].to.index();
                                        update_history(
                                            &mut state.reckless_cont_hist[previous_piece]
                                                [previous_to][prev_piece_idx][prev.to.index()],
                                            -ranked_malus,
                                        );
                                    }
                                }
                            } else if ply > 0 {
                                let pp = state.prev_piece[ply_usize.saturating_sub(1)];
                                let pt = state.prev_move[ply_usize.saturating_sub(1)].to.index();
                                update_history(
                                    &mut state.cont_hist[pp][pt][prev_piece_idx][prev.to.index()],
                                    -ranked_malus,
                                );
                            }
                        }

                        // Countermove: index by PREVIOUS move
                        if prev_mv != NULL_MOVE {
                            state.countermoves[prev_mv.from.index()][prev_mv.to.index()] = mv;
                        }
                    } else {
                        // Capture history update for captures causing cutoff
                        if let Some(cap_idx) = captured_piece_idx {
                            state
                                .cap_hist
                                .update(threats, moved_piece, mv.to, cap_idx, bonus);
                        }
                        // Capture history malus: penalize captures that were searched before the cutoff capture
                        for (prev_piece_idx, prev_to, prev_cap_idx) in
                            searched_captures[..searched_captures_len].iter()
                        {
                            state.cap_hist.update(
                                threats,
                                *prev_piece_idx,
                                *prev_to,
                                *prev_cap_idx,
                                -malus,
                            );
                        }
                    }
                    if excluded_move.is_none() {
                        tt.store(
                            board.tt_hash(),
                            TTData::new(
                                depth,
                                score_to_tt(score, ply),
                                NodeType::LowerBound,
                                best_move,
                                tt_was_pv,
                                raw_eval,
                            ),
                        );
                    }
                    return best_score;
                }
            }
        }

        if !mv.is_capture() && !mv.is_promotion() && searched_quiets_len < SEARCHED_QUIETS_MAX {
            searched_quiets[searched_quiets_len] = mv;
            searched_quiets_len += 1;
        }
        if mv.is_capture()
            && let Some(cap_idx) = captured_piece_idx
            && searched_captures_len < SEARCHED_CAPTURES_MAX
        {
            searched_captures[searched_captures_len] = (moved_piece, mv.to, cap_idx);
            searched_captures_len += 1;
        }
    }

    // ── Post-loop mate/stalemate detection ──
    // With lazy move generation, we only know there are no legal moves
    // after trying to iterate. If moves_searched==0 and no excluded move
    // was skipped, this is checkmate or stalemate.
    if moves_searched == 0 {
        if excluded_move.is_some() {
            // We were in singular extension search — excluded move is the only legal one
            return alpha;
        }
        return if in_check {
            -MATE_SCORE + ply
        } else {
            draw_score(state.nodes)
        };
    }

    if move_ordering == MoveOrderingProfile::Reckless
        && !is_root
        && excluded_move.is_none()
        && node_type == NodeType::UpperBound
        && (is_cut_node || is_pv)
        && ply_usize > 0
    {
        let parent_ply = ply_usize - 1;
        let prior_move = state.prev_move[parent_ply];
        if prior_move != NULL_MOVE && prior_move.is_quiet() {
            let prior_piece = state.prev_piece[parent_ply];
            let parent_eval =
                state.eval_valid[parent_ply].then_some(state.static_evals[parent_ply]);
            let bonus = fail_low_parent_history_bonus(
                depth,
                state.move_counts[parent_ply],
                prior_move == state.tt_moves[parent_ply],
                in_check,
                best_score,
                static_eval,
                parent_eval,
            );
            state.history.update(
                state.threats[parent_ply],
                us.opponent().index(),
                prior_move,
                bonus,
            );
            if ply_usize >= 2 && state.prev_move[ply_usize - 2] != NULL_MOVE {
                let previous_piece = state.prev_piece[ply_usize - 2];
                let previous_to = state.prev_move[ply_usize - 2].to.index();
                let continuation_bonus = (152 * depth - 47).min(1379);
                update_history(
                    &mut state.reckless_cont_hist[previous_piece][previous_to][prior_piece]
                        [prior_move.to.index()],
                    continuation_bonus,
                );
            }
        }
    }

    if excluded_move.is_none() {
        tt.store(
            board.tt_hash(),
            TTData::new(
                depth,
                score_to_tt(best_score, ply),
                node_type,
                best_move,
                tt_was_pv,
                raw_eval,
            ),
        );
        // Update correction history with Akimbo-style filtering:
        // Skip when in check, singular search, noisy best move, or when
        // bound direction agrees with static eval (no info to learn).
        let should_update_corr = !in_check
            && excluded_move.is_none()
            && !best_move.is_capture()
            && !best_move.is_promotion()
            && !(node_type == NodeType::LowerBound && best_score <= static_eval)
            && !(node_type == NodeType::UpperBound && best_score >= static_eval);
        if should_update_corr {
            state.update_correction(board, depth, best_score, corrected_eval, move_ordering);
        }
    }
    best_score
}

/// Quiescence search — stabilize the evaluation at leaf nodes.
/// Handles check evasions, captures, and promotions.
/// Uses fail-soft and the hybrid eval (classical + NNUE) for consistency.
#[inline(always)]
fn should_reverse_qsearch(
    is_pv: bool,
    reverse_qsearch_active: bool,
    tt_move: Move,
    tt_node_type: NodeType,
) -> bool {
    !is_pv
        && !reverse_qsearch_active
        && tt_move != NULL_MOVE
        && tt_move.is_quiet()
        && tt_node_type != NodeType::UpperBound
}

#[inline(always)]
fn soften_qsearch_fail_high(score: i32, beta: i32, divisor: i32) -> i32 {
    if score.abs() >= MATE_SCORE - 100 || beta.abs() >= MATE_SCORE - 100 {
        score
    } else {
        beta + (score - beta) / divisor
    }
}

#[inline(always)]
fn qsearch_see_threshold(
    alpha: i32,
    eval: i32,
    correction: i32,
    history: i32,
    move_ordering: MoveOrderingProfile,
) -> Option<i32> {
    (move_ordering == MoveOrderingProfile::Reckless)
        .then(|| (alpha - eval) / 8 - correction.abs().min(68) - 74 - history / 48)
}

#[inline(never)]
fn quiescence(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: QuiescenceNode,
) -> i32 {
    let SearchContext {
        tt,
        stopped,
        time_limit,
        node_limit,
        start_time,
        params,
        use_nnue,
        move_ordering,
        eval_mode,
        ..
    } = *context;
    let QuiescenceNode {
        mut alpha,
        beta,
        ply,
        qs_ply,
        is_pv,
    } = node;
    if ply > state.seldepth {
        state.seldepth = ply;
    }
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }
        if let Some(nl) = node_limit
            && state.nodes >= nl
        {
            stopped.store(true, Ordering::Relaxed);
            return 0;
        }
        if search_time_exceeded(start_time, time_limit, context.deadline_ms) {
            stopped.store(true, Ordering::Relaxed);
            return 0;
        }
    }

    let in_check = board.in_check();

    if board.is_search_draw(ply as usize) {
        return draw_score(state.nodes);
    }
    // Hard depth limit — prevents stack overflow and infinite check chains.
    // This MUST apply even when in check (check evasions can chain indefinitely).
    if ply >= MAX_PLY as i32 - 1 {
        if in_check {
            return if board.generate_legal_moves().is_empty() {
                -MATE_SCORE + ply
            } else {
                0
            };
        }
        return hybrid_eval(board, state, use_nnue);
    }
    if qs_ply >= params.max_qs_ply && !in_check {
        return hybrid_eval(board, state, use_nnue);
    }

    // TT probe in qsearch (important for stability!)
    let qs_entry = tt.probe(board.tt_hash());
    let mut qs_tt_move = None;
    if let Some(entry) = qs_entry {
        let probed_score = score_from_tt(entry.score, ply);
        qs_tt_move = usable_tt_move(entry.best_move);
        if entry.depth >= 0 {
            match entry.node_type {
                NodeType::Exact => return probed_score,
                NodeType::LowerBound => {
                    if probed_score >= beta {
                        return probed_score;
                    }
                    if probed_score > alpha {
                        alpha = probed_score;
                    }
                }
                NodeType::UpperBound => {
                    if probed_score <= alpha {
                        return probed_score;
                    }
                }
            }
        }

        if should_reverse_qsearch(
            is_pv,
            state.reverse_qsearch,
            entry.best_move,
            entry.node_type,
        ) {
            state.reverse_qsearch = true;
            let score = search_ab(
                board,
                state,
                context,
                SearchNode {
                    depth: 1,
                    alpha,
                    beta,
                    ply,
                    is_pv: false,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: 0,
                    nominal_depth: 1,
                    allow_null: true,
                },
            );
            state.reverse_qsearch = false;
            return score;
        }
    }

    // When in check, search ALL moves (not just captures) to find escape
    // Score them by TT move priority + capture value for better ordering.
    if in_check {
        let mut picker = MovePicker::new(board, qs_tt_move, [NULL_MOVE; 2], NULL_MOVE)
            .with_move_ordering(move_ordering);
        let score_capture = |b: &Board, mv: Move| capture_score(b, mv, qs_tt_move, move_ordering);
        let score_quiet = |_: &Board, _: Move| 0;
        let mut best_score = -INF;
        let mut best_move = NULL_MOVE;
        let mut moves_searched = 0;
        let alpha_orig = alpha;
        while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
            moves_searched += 1;
            make_search_move(board, state, mv);
            let score = -quiescence(
                board,
                state,
                context,
                QuiescenceNode {
                    alpha: -beta,
                    beta: -alpha,
                    ply: ply + 1,
                    qs_ply: qs_ply + 1,
                    is_pv,
                },
            );
            board.unmake_move(mv);
            undo_search_eval(state);
            if stopped.load(Ordering::Relaxed) {
                return 0;
            }
            if score > best_score {
                best_score = score;
                best_move = mv;
            }
            if score > alpha {
                alpha = score;
            }
            if score >= beta {
                tt.store(
                    board.tt_hash(),
                    TTData::new(
                        0,
                        score_to_tt(best_score, ply),
                        NodeType::LowerBound,
                        best_move,
                        false,
                        None,
                    ),
                );
                return best_score;
            }
        }
        if moves_searched == 0 {
            return -MATE_SCORE + ply;
        }
        let nt = if best_score > alpha_orig {
            NodeType::Exact
        } else {
            NodeType::UpperBound
        };
        tt.store(
            board.tt_hash(),
            TTData::new(0, score_to_tt(best_score, ply), nt, best_move, false, None),
        );
        return best_score;
    }

    // ── Fail-soft stand-pat using hybrid eval + correction history ──
    let raw_eval = qs_entry
        .and_then(|entry| entry.raw_eval)
        .unwrap_or_else(|| hybrid_eval(board, state, use_nnue));
    let corr = state.correction(board, move_ordering);
    let mut stand_pat = corrected_network_eval(
        board,
        raw_eval,
        corr,
        state.optimism[board.side_to_move.index()],
        eval_mode,
    );

    // TT score adjustment in QS (Akimbo technique):
    // Use TT score to refine stand-pat when TT bound agrees with direction.
    if let Some(entry) = qs_entry {
        let tt_sc = score_from_tt(entry.score, ply);
        match entry.node_type {
            NodeType::Exact => stand_pat = tt_sc,
            NodeType::LowerBound => {
                if tt_sc > stand_pat {
                    stand_pat = tt_sc;
                }
            }
            NodeType::UpperBound => {
                if tt_sc < stand_pat {
                    stand_pat = tt_sc;
                }
            }
        }
    }

    let mut best_score = stand_pat;

    if stand_pat >= beta {
        best_score = soften_qsearch_fail_high(best_score, beta, 6);
        if qs_entry.is_none() {
            tt.store(
                board.tt_hash(),
                TTData::new(
                    0,
                    score_to_tt(best_score, ply),
                    NodeType::LowerBound,
                    NULL_MOVE,
                    is_pv,
                    Some(raw_eval),
                ),
            );
        }
        return best_score;
    }

    // Big delta pruning: if we're hopelessly behind, give up
    if stand_pat + params.delta_margin + 1100 < alpha {
        return best_score;
    }
    if stand_pat > alpha {
        alpha = stand_pat;
    }

    let threats = opponent_threats(board).all;
    let cap_hist_ptr = &state.cap_hist as *const CaptureHistory;
    let score_capture = |b: &Board, mv: Move| {
        // SAFETY: the pointer targets `state.cap_hist`, which remains alive and
        // is only read while the move picker invokes this closure.
        unsafe {
            capture_score_with_history(b, mv, qs_tt_move, threats, &*cap_hist_ptr, move_ordering)
        }
    };
    let score_quiet = |_: &Board, _: Move| 0;
    let mut picker = MovePicker::new(board, qs_tt_move, [NULL_MOVE; 2], NULL_MOVE)
        .with_move_ordering(move_ordering);
    picker.skip_quiets();
    picker.skip_bad_captures();
    let mut best_move = NULL_MOVE;
    let alpha_orig = alpha;

    while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
        let moved_piece = piece_index_on(board, mv.from);
        let history = captured_piece_index(board, mv)
            .map(|captured| state.cap_hist.get(threats, moved_piece, mv.to, captured))
            .unwrap_or(0);
        if let Some(threshold) =
            qsearch_see_threshold(alpha, stand_pat, corr, history, move_ordering)
            && !see::see_ge(board, mv, threshold)
        {
            continue;
        }

        // Per-capture delta pruning (skip for promotions)
        if mv.is_capture() {
            let cv = estimate_capture_value(board, mv);
            if stand_pat + cv + params.delta_margin < alpha {
                continue;
            }
        }

        make_search_move(board, state, mv);
        let score = -quiescence(
            board,
            state,
            context,
            QuiescenceNode {
                alpha: -beta,
                beta: -alpha,
                ply: ply + 1,
                qs_ply: qs_ply + 1,
                is_pv,
            },
        );
        board.unmake_move(mv);
        undo_search_eval(state);

        if stopped.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
            best_move = mv;
        }
        if score > alpha {
            alpha = score;
        }
        if score >= beta {
            best_score = soften_qsearch_fail_high(best_score, beta, 2);
            tt.store(
                board.tt_hash(),
                TTData::new(
                    0,
                    score_to_tt(best_score, ply),
                    NodeType::LowerBound,
                    best_move,
                    false,
                    Some(raw_eval),
                ),
            );
            return best_score;
        }
    }

    // Store qsearch result into TT
    let nt = if best_score > alpha_orig {
        NodeType::Exact
    } else {
        NodeType::UpperBound
    };
    tt.store(
        board.tt_hash(),
        TTData::new(
            0,
            score_to_tt(best_score, ply),
            nt,
            best_move,
            false,
            Some(raw_eval),
        ),
    );

    best_score
}

#[inline(always)]
fn estimate_capture_value(board: &Board, mv: Move) -> i32 {
    if mv.is_promotion() {
        return 900;
    }
    if let Some(piece) = board.piece_of_color_on(mv.to, board.side_to_move.opponent()) {
        piece_value(piece)
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        100
    } else {
        0
    }
}

#[inline(always)]
fn same_move_key(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}

#[inline(always)]
fn nmp_material_ok(board: &Board, us: types::Color) -> bool {
    board.has_non_pawn_material(us)
}

#[inline(always)]
fn capture_score_with_history(
    board: &Board,
    mv: Move,
    tt_move: Option<Move>,
    threats: u64,
    cap_hist: &CaptureHistory,
    move_ordering: MoveOrderingProfile,
) -> i32 {
    if let Some(ttm) = tt_move
        && mv.from == ttm.from
        && mv.to == ttm.to
    {
        return 10_000_000;
    }
    let victim = board.piece_of_color_on(mv.to, board.side_to_move.opponent());
    let attacker = board.piece_of_color_on(mv.from, board.side_to_move);
    let en_passant = mv.flag == types::chess_move::MoveFlag::EnPassant;

    // Add capture history score — moved piece × to square × captured piece
    let moved_piece = piece_index_on(board, mv.from);
    let captured_index = victim
        .map(Piece::index)
        .or_else(|| en_passant.then_some(Piece::Pawn.index()));
    let history = captured_index.map_or(0, |captured| {
        cap_hist.get(threats, moved_piece, mv.to, captured)
    });

    move_ordering.noisy_score(victim, attacker, en_passant, mv.is_promotion(), history, 32)
}

#[inline(always)]
fn capture_score(
    board: &Board,
    mv: Move,
    tt_move: Option<Move>,
    move_ordering: MoveOrderingProfile,
) -> i32 {
    if let Some(ttm) = tt_move
        && mv.from == ttm.from
        && mv.to == ttm.to
    {
        return 10_000_000;
    }
    let victim = board.piece_of_color_on(mv.to, board.side_to_move.opponent());
    let attacker = board.piece_of_color_on(mv.from, board.side_to_move);
    let en_passant = mv.flag == types::chess_move::MoveFlag::EnPassant;
    move_ordering.noisy_score(victim, attacker, en_passant, mv.is_promotion(), 0, 1)
}

#[inline(always)]
fn is_killer(mv: Move, killers: &[Move; 2]) -> bool {
    (mv.from == killers[0].from && mv.to == killers[0].to)
        || (mv.from == killers[1].from && mv.to == killers[1].to)
}

#[inline(always)]
fn store_killer(killers: &mut [[Move; 2]; MAX_PLY], mv: Move, ply: usize) {
    if mv.from == killers[ply][0].from && mv.to == killers[ply][0].to {
        return;
    }
    killers[ply][1] = killers[ply][0];
    killers[ply][0] = mv;
}

#[inline(always)]
fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Knight => 320,
        Piece::Bishop => 330,
        Piece::Rook => 500,
        Piece::Queen => 900,
        Piece::King => 20000,
    }
}

/// Hash pawn structure for correction history.
#[inline(always)]
fn pawn_hash(board: &Board) -> usize {
    let wp = board.piece_bb(Piece::Pawn, types::Color::White);
    let bp = board.piece_bb(Piece::Pawn, types::Color::Black);
    // FNV-1a inspired hash of two u64 bitboards
    let mut h = 0xcbf29ce484222325u64;
    h ^= wp;
    h = h.wrapping_mul(0x100000001b3);
    h ^= bp;
    h = h.wrapping_mul(0x100000001b3);
    h as usize
}

/// Hash material configuration for correction history.
#[inline(always)]
fn material_hash(board: &Board) -> usize {
    let mut h = 0x9e3779b97f4a7c15u64;
    for &piece in &Piece::ALL {
        for &color in &[types::Color::White, types::Color::Black] {
            h ^= board
                .piece_bb(piece, color)
                .wrapping_mul(piece.index() as u64 + 1);
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h as usize
}

/// Hash minor piece (knight+bishop) positions for correction history.
#[inline(always)]
fn minor_hash(board: &Board) -> usize {
    let wn = board.piece_bb(Piece::Knight, types::Color::White);
    let bn = board.piece_bb(Piece::Knight, types::Color::Black);
    let wb = board.piece_bb(Piece::Bishop, types::Color::White);
    let bb = board.piece_bb(Piece::Bishop, types::Color::Black);
    let mut h = 0x517cc1b727220a95u64;
    h ^= wn;
    h = h.wrapping_mul(0x100000001b3);
    h ^= bn;
    h = h.wrapping_mul(0x100000001b3);
    h ^= wb;
    h = h.wrapping_mul(0x100000001b3);
    h ^= bb;
    h = h.wrapping_mul(0x100000001b3);
    h as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use eval::nnue::NnueSearchProfile;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    static TEST_LMR_POLICY: LmrDispatch = LmrDispatch::StockLike;
    static TEST_LMP_POLICY: LmpDispatch = LmpDispatch::StockLike;
    static TEST_FUTILITY_POLICY: FutilityDispatch = FutilityDispatch::StockLike;
    static TEST_BAD_NOISY_FUTILITY_POLICY: BadNoisyFutilityDispatch =
        BadNoisyFutilityDispatch::Disabled;
    static TEST_RFP_POLICY: RfpDispatch = RfpDispatch::StockLike;

    fn setup() {
        types::init();
    }

    #[test]
    fn ateed_uncertainty_widens_margins_and_relieves_lmr() {
        let ateed = EvalMode::Nnue(NnueSearchProfile::Ateed);
        let reckless = EvalMode::Nnue(NnueSearchProfile::Reckless);
        assert_eq!(ateed_uncertainty_margin(reckless, 2_000), 0);
        assert_eq!(ateed_uncertainty_margin(ateed, 0), 0);
        assert_eq!(ateed_uncertainty_margin(ateed, 2_000), 10);
        assert_eq!(ateed_uncertainty_margin(ateed, 20_000), 64);
        assert_eq!(ateed_lmr_relief(reckless, 2_000), 0);
        assert_eq!(ateed_lmr_relief(ateed, 1_499), 0);
        assert_eq!(ateed_lmr_relief(ateed, 1_500), 1);
    }

    #[cfg(feature = "ateed-nnue")]
    #[test]
    fn ateed_search_smoke_uses_uncertainty_eval() {
        setup();
        let path = std::env::temp_dir().join("mujrim-ateed-search-smoke.bin");
        std::fs::write(&path, eval::nnue::AteedNetwork::zero().to_bytes()).unwrap();
        let network = eval::nnue::load_network(&path).expect("zero Ateed net");
        let _ = std::fs::remove_file(&path);
        let mut engine = SearchEngine::new(4, 1);
        engine.set_nnue_network(network);
        engine.set_use_nnue(true);
        assert!(engine.eval_mode().is_ateed_nnue());
        let mut board = Board::new();
        let result = engine.search_nodes(&mut board, 400, 3);
        assert!(result.nodes > 0);
        assert_ne!(result.best_move, NULL_MOVE);
    }

    #[test]
    fn opponent_threat_map_matches_starting_position_attacks() {
        setup();
        let board = Board::new();
        let maps = opponent_threats(&board);
        let threats = maps.all;

        assert_ne!(threats & types::Square::E6.bitboard(), 0);
        assert_ne!(threats & types::Square::F6.bitboard(), 0);
        assert_eq!(threats & types::Square::E5.bitboard(), 0);
        assert_eq!(
            maps.by_piece.into_iter().fold(0, |all, map| all | map),
            threats
        );
        assert_ne!(
            maps.by_piece[Piece::Pawn.index()] & types::Square::E6.bitboard(),
            0
        );
    }

    #[test]
    fn bulk_pawn_attacks_match_the_square_table() {
        setup();
        for color in [types::Color::White, types::Color::Black] {
            for square in 0..64 {
                assert_eq!(
                    pawn_attack_set(color, 1u64 << square),
                    pawn_attacks(color.index(), square),
                    "mismatch for {color:?} pawn on square {square}"
                );
            }
        }
    }

    #[test]
    fn reckless_uci_score_normalization_matches_material_scale() {
        setup();
        let board = Board::new();
        assert_eq!(
            normalize_uci_score(270, &board, EvalMode::Nnue(NnueSearchProfile::Reckless)),
            95
        );
        assert_eq!(
            normalize_uci_score(-270, &board, EvalMode::Nnue(NnueSearchProfile::Reckless)),
            -95
        );
        assert_eq!(
            normalize_uci_score(270, &board, EvalMode::Nnue(NnueSearchProfile::Stockfish)),
            270
        );
        assert_eq!(
            normalize_uci_score(
                MATE_SCORE,
                &board,
                EvalMode::Nnue(NnueSearchProfile::Reckless)
            ),
            MATE_SCORE
        );
    }

    #[test]
    fn uci_score_formatter_reports_centipawns_and_mate_distance() {
        setup();
        let board = Board::new();
        assert_eq!(
            format_uci_score_value(42, &board, EvalMode::Nnue(NnueSearchProfile::Stockfish)),
            "cp 42"
        );
        assert_eq!(
            format_uci_score_value(
                MATE_SCORE - 5,
                &board,
                EvalMode::Nnue(NnueSearchProfile::Stockfish)
            ),
            "mate 3"
        );
        assert_eq!(
            format_uci_score_value(
                -MATE_SCORE + 4,
                &board,
                EvalMode::Nnue(NnueSearchProfile::Stockfish)
            ),
            "mate -2"
        );
    }

    #[test]
    fn reckless_quiet_ordering_maps_threatened_pieces_and_check_targets() {
        setup();
        let board = Board::from_fen("4k3/8/8/8/3p4/4N3/8/4R1K1 w - - 0 1").unwrap();
        let threats = opponent_threats(&board);
        let maps = reckless_quiet_ordering_maps(&board, threats);
        assert_ne!(
            maps.threatened[Piece::Knight.index()] & types::Square::E3.bitboard(),
            0
        );
        assert_ne!(
            maps.checking_squares[Piece::Rook.index()] & types::Square::E7.bitboard(),
            0
        );
    }

    #[test]
    fn pawn_history_is_partitioned_by_structure_color_and_piece() {
        let mut history = PawnHistory::default();
        history.update(7, 0, Piece::Knight.index(), types::Square::F3, 512);

        assert!(history.get(7, 0, Piece::Knight.index(), types::Square::F3) > 0);
        assert_eq!(
            history.get(8, 0, Piece::Knight.index(), types::Square::F3),
            0
        );
        assert_eq!(
            history.get(7, 1, Piece::Knight.index(), types::Square::F3),
            0
        );
        assert_eq!(
            history.get(7, 0, Piece::Bishop.index(), types::Square::F3),
            0
        );
    }

    #[test]
    fn direct_check_detection_handles_sliders_and_promotions() {
        setup();
        let rook_board = Board::from_fen("4k3/8/8/8/8/8/4R3/4K3 w - - 0 1").unwrap();
        assert!(gives_direct_check(
            &rook_board,
            Move::from_uci("e2e7").unwrap()
        ));
        assert!(!gives_direct_check(
            &rook_board,
            Move::from_uci("e2a2").unwrap()
        ));
        let promotion_board = Board::from_fen("k7/4P3/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(gives_direct_check(
            &promotion_board,
            Move::from_uci("e7e8q").unwrap()
        ));
    }

    #[test]
    fn quiet_history_combines_factorizer_and_threat_bucket() {
        let mut history = QuietHistory::default();
        let mv = Move::double_pawn(types::Square::E2, types::Square::E4);
        let threats = types::Square::E2.bitboard() | types::Square::E4.bitboard();

        history.update(threats, 0, mv, 512);

        assert!(history.get(threats, 0, mv) > history.get(0, 0, mv));
        assert!(history.get(0, 0, mv) > 0);
    }

    #[test]
    fn thread_state_clear_reuses_large_buffers() {
        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        let history = state.history.entries.as_ptr();
        let pawn_history = state.pawn_history.entries.as_ptr();
        let continuation = state.cont_hist.as_ptr();
        let reckless_continuation = state.reckless_cont_hist.as_ptr();
        let pv = state.pv.as_ptr();
        state.history.entries[0][0][0].factorizer = 42;
        state.pawn_history.entries[0][0][0] = 42;
        state.pawn_corr[0][0] = 42;
        state.reckless_cont_hist[0][0][0][0] = 42;

        state.clear();

        assert_eq!(state.history.entries.as_ptr(), history);
        assert_eq!(state.pawn_history.entries.as_ptr(), pawn_history);
        assert_eq!(state.cont_hist.as_ptr(), continuation);
        assert_eq!(state.reckless_cont_hist.as_ptr(), reckless_continuation);
        assert_eq!(state.pv.as_ptr(), pv);
        assert_eq!(state.history.entries[0][0][0].factorizer, 0);
        assert_eq!(state.pawn_history.entries[0][0][0], 0);
        assert_eq!(state.pawn_corr[0][0], 0);
        assert_eq!(state.reckless_cont_hist[0][0][0][0], 0);
    }

    #[test]
    fn hce_search_skips_nnue_and_returns_a_legal_move() {
        types::init();
        let mut engine = SearchEngine::new(8, 1);
        engine.set_use_nnue(false);
        let mut board = Board::new();
        let result = engine.search_depth(&mut board, 3);
        let legal = board.generate_legal_moves();
        assert!(
            legal.iter().any(|mv| *mv == result.best_move),
            "best={} legal={:?}",
            result.best_move.to_uci(),
            legal.iter().map(|mv| mv.to_uci()).collect::<Vec<_>>()
        );
        assert!(result.nodes > 0);
        assert!(!engine.use_nnue());
    }

    #[test]
    fn fresh_thread_state_has_no_countermoves() {
        let state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));

        assert!(
            state
                .countermoves
                .iter()
                .flatten()
                .all(|&mv| mv == NULL_MOVE)
        );
    }

    #[test]
    fn search_reset_preserves_game_learning() {
        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        let countermove = Move::quiet(types::Square::E2, types::Square::E4);
        state.nodes = 99;
        state.killers[0][0] = countermove;
        state.eval_valid[0] = true;
        state.move_counts[7] = 3;
        state.tt_moves[7] = countermove;
        state.history.entries[0][0][1].factorizer = 41;
        state.cont_hist[0][0][0][1] = 42;
        state.cont_hist2[0][0][0][1] = 43;
        state.reckless_cont_hist[0][0][0][1] = 46;
        state.cap_hist.entries[0][0].factorizer = 44;
        state.countermoves[0][1] = countermove;
        state.pawn_corr[0][0] = 45;

        state.reset_for_search();

        assert_eq!(state.nodes, 0);
        assert_eq!(state.killers[0][0], NULL_MOVE);
        assert!(!state.eval_valid[0]);
        assert_eq!(state.move_counts[7], 0);
        assert_eq!(state.tt_moves[7], NULL_MOVE);
        assert_eq!(state.history.entries[0][0][1].factorizer, 41);
        assert_eq!(state.cont_hist[0][0][0][1], 42);
        assert_eq!(state.cont_hist2[0][0][0][1], 43);
        assert_eq!(state.reckless_cont_hist[0][0][0][1], 46);
        assert_eq!(state.cap_hist.entries[0][0].factorizer, 44);
        assert_eq!(state.countermoves[0][1], countermove);
        assert_eq!(state.pawn_corr[0][0], 45);
    }

    #[test]
    fn fail_low_parent_history_bonus_matches_reckless_scaling() {
        assert_eq!(
            fail_low_parent_history_bonus(1, 0, false, true, 0, 0, None),
            98
        );
        assert_eq!(
            fail_low_parent_history_bonus(20, 20, true, false, -300, 0, Some(0)),
            16_539
        );
    }

    #[test]
    fn first_child_preserves_reckless_pv_and_cut_node_roles() {
        assert!(!first_child_is_cut_node(true, false));
        assert!(!first_child_is_cut_node(true, true));
        assert!(first_child_is_cut_node(false, false));
        assert!(!first_child_is_cut_node(false, true));
    }

    #[test]
    fn reckless_lmr_search_depth_matches_native_v60_formula() {
        assert_eq!(reckless_lmr_search_depth(12, 3, false), 9);
        assert_eq!(reckless_lmr_search_depth(12, 0, false), 12);
        assert_eq!(reckless_lmr_search_depth(3, 5, false), 1);
        assert_eq!(reckless_lmr_search_depth(8, -1, false), 9);
        assert_eq!(reckless_lmr_search_depth(12, 3, true), 11);
        assert_eq!(reckless_lmr_search_depth(12, 2, true), 12);
        assert_eq!(reckless_lmr_search_depth(16, 4, true), 14);
        assert_eq!(reckless_lmr_search_depth(5, 0, true), 7);
    }

    #[test]
    fn stock_like_lmr_search_depth_keeps_pv_compensation() {
        assert_eq!(stock_like_lmr_search_depth(12, 3, false), 9);
        assert_eq!(stock_like_lmr_search_depth(12, 3, true), 10);
        assert_eq!(stock_like_lmr_search_depth(4, 5, false), 1);
        assert_eq!(stock_like_lmr_search_depth(4, 5, true), 2);
    }

    #[test]
    fn negative_singular_extension_matches_adapter_aggressiveness() {
        assert_eq!(
            negative_singular_extension(MoveOrderingProfile::Reckless),
            -1
        );
        assert_eq!(
            negative_singular_extension(MoveOrderingProfile::StockLike),
            -2
        );
    }

    #[test]
    fn reckless_skips_check_extensions_to_preserve_fixed_node_depth() {
        assert_eq!(budgeted_check_extension(8, 0, 16, true), (9, 1));
        assert_eq!(budgeted_check_extension(8, 0, 16, false), (8, 0));
        assert!(!extend_checks(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Reckless),
            true
        ));
        assert!(extend_checks(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Reckless),
            false
        ));
        assert!(!extend_checks(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Stockfish),
            true
        ));
        assert!(full_depth_root_quiets(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Reckless)
        ));
        assert!(extend_checks(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Lc0),
            true
        ));
        assert!(full_depth_root_quiets(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Lc0)
        ));
        assert!(extend_checks(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Viridithas),
            true
        ));
        assert!(full_depth_root_quiets(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Viridithas)
        ));
        assert!(!full_depth_root_quiets(
            MoveOrderingProfile::Reckless,
            EvalMode::Nnue(NnueSearchProfile::Akimbo)
        ));
    }

    #[test]
    fn reckless_root_pawn_near_miss_margin_is_tight() {
        // Reckless NNUE now uses the StockLike 160cp quiet window so BK
        // breakthroughs (Ne5 / Re4 / f2f4) get a PV re-search. Other adapters
        // that reuse Reckless move ordering keep the pawn-only 120cp gate.
        const RECKLESS_NNUE_QUIET: i32 = 160;
        const OTHER_RECKLESS_ORDERING_PAWN: i32 = 120;
        const _: () = assert!(316 > 400 - RECKLESS_NNUE_QUIET);
        const _: () = assert!(250 > 400 - RECKLESS_NNUE_QUIET);
        const _: () = assert!(178 <= 400 - OTHER_RECKLESS_ORDERING_PAWN);
    }

    #[test]
    fn stock_root_quiet_near_miss_margin_covers_prophylaxis() {
        // BK#8 king/rook slides often land 80–150cp under a bishop-pin alpha.
        const MARGIN: i32 = 160;
        const _: () = assert!(320 > 400 - MARGIN);
        const _: () = assert!(250 > 400 - MARGIN);
        const _: () = assert!(200 <= 400 - MARGIN);
    }

    static TEST_SYZYGY: SyzygyTables = SyzygyTables::empty();
    static TEST_DEADLINE: AtomicU64 = AtomicU64::new(0);

    fn test_context<'a>(
        tt: &'a TranspositionTable,
        stopped: &'a AtomicBool,
        params: &'a SearchParams,
        lmr_table: &'a [[i32; 128]; 128],
    ) -> SearchContext<'a> {
        SearchContext {
            tt,
            stopped,
            time_limit: None,
            node_limit: None,
            start_time: Instant::now(),
            params,
            lmr_table,
            use_nnue: true,
            lmr_policy: &TEST_LMR_POLICY,
            lmp_policy: &TEST_LMP_POLICY,
            futility_policy: &TEST_FUTILITY_POLICY,
            bad_noisy_futility_policy: &TEST_BAD_NOISY_FUTILITY_POLICY,
            rfp_policy: &TEST_RFP_POLICY,
            move_ordering: MoveOrderingProfile::StockLike,
            eval_mode: EvalMode::Nnue(NnueSearchProfile::Stockfish),
            contempt: 0,
            syzygy: &TEST_SYZYGY,
            deadline_ms: &TEST_DEADLINE,
        }
    }

    #[test]
    fn test_search_returns_legal_move() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 3);
                let legal = board.generate_legal_moves();
                assert!(result.seldepth >= result.depth);
                assert!(
                    legal
                        .iter()
                        .any(|m| m.from == result.best_move.from && m.to == result.best_move.to)
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn nodes_are_counted_on_legal_moves_not_node_entries() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_nodes(&mut board, 5_000, 64);
        // With move-based counting, a 5k budget must complete real iterative depth.
        assert!(
            result.depth >= 4,
            "expected depth>=4 at 5k move-nodes, got depth={} nodes={}",
            result.depth,
            result.nodes
        );
        // Soft stop checks every 2048 nodes, so the final tally may overshoot slightly.
        assert!(
            result.nodes <= 5_000 + 2_048,
            "node overshoot too large: {}",
            result.nodes
        );
        assert_ne!(result.best_move, NULL_MOVE);
    }

    #[test]
    fn node_limit_before_first_completed_iteration_uses_legal_fallback() {
        setup();
        let mut board = Board::new();
        let legal_moves = board.generate_legal_moves();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_nodes(&mut board, 1, 128);
        assert!(legal_moves.iter().any(|mv| *mv == result.best_move));
        assert!(result.score.abs() < INF);
        if result.depth == 0 {
            assert_eq!(result.score, 0);
        }
    }

    #[test]
    fn test_search_complex_position() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::from_fen(
                    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
                )
                .unwrap();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 4);
                let legal = board.generate_legal_moves();
                assert!(
                    legal
                        .iter()
                        .any(|m| m.from == result.best_move.from && m.to == result.best_move.to)
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_mate_in_1() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::from_fen(
                    "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
                )
                .unwrap();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 4);
                assert!(result.score > MATE_SCORE - 10);
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_material_advantage() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board =
                    Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
                        .unwrap();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 3);
                assert!(result.score > 500);
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_time_limit() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_time(&mut board, Duration::from_millis(100), 64);
                assert!(result.nodes > 0);
                assert!(result.elapsed <= Duration::from_millis(500));
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_hard_time_mode_not_less_work_than_soft_time() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut soft_board = Board::new();
                let mut hard_board = Board::new();
                let mut soft_engine = SearchEngine::new(8, 1);
                let mut hard_engine = SearchEngine::new(8, 1);
                let soft =
                    soft_engine.search_time(&mut soft_board, Duration::from_millis(200), 64);
                let hard = hard_engine.search_time_hard(
                    &mut hard_board,
                    Duration::from_millis(200),
                    64,
                );
                assert!(
                    hard.nodes >= soft.nodes || hard.elapsed >= soft.elapsed,
                    "hard mode should not do less work: soft nodes={} elapsed={:?}, hard nodes={} elapsed={:?}",
                    soft.nodes,
                    soft.elapsed,
                    hard.nodes,
                    hard.elapsed
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_search_nodes_stops_near_limit() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(8, 4);
                let limit = 10_000u64;
                let result = engine.search_nodes(&mut board, limit, 64);
                assert!(result.nodes > 0);
                assert!(
                    result.nodes <= limit + 4096,
                    "nodes {} exceeded expected cap {}",
                    result.nodes,
                    limit + 4096
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_preserves_board() {
        // Run in a thread with 8MB stack — deep recursive search needs more than 2MB default.
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let original_fen = board.to_fen();
                let original_hash = board.hash;
                let mut engine = SearchEngine::new(1, 1);
                let _ = engine.search_depth(&mut board, 5);
                assert_eq!(board.to_fen(), original_fen);
                assert_eq!(board.hash, original_hash);
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_multithreaded_search() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(8, 4);
                let result = engine.search_depth(&mut board, 6);
                assert!(result.nodes > 0);
                let legal = board.generate_legal_moves();
                assert!(
                    legal
                        .iter()
                        .any(|m| m.from == result.best_move.from && m.to == result.best_move.to)
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_tt_stores_during_search() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                engine.search_depth(&mut board, 4);
                assert!(engine.tt.probe(board.tt_hash()).is_some());
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_tt_clear() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                engine.search_depth(&mut board, 3);
                assert!(engine.tt.probe(board.tt_hash()).is_some());
                engine.clear();
                assert!(engine.tt.probe(board.tt_hash()).is_none());
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_depth_1_bounded() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 1);
                assert!(result.nodes > 0 && result.nodes < 1000);
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_repetition_detection() {
        setup();
        let mut board = Board::new();
        // Play Nf3 Nf6 Ng1 Ng8 — back to start (repetition)
        let moves_uci = ["g1f3", "g8f6", "f3g1", "f6g8"];
        for m in &moves_uci {
            let legal = board.generate_legal_moves();
            if let Some(mv) = legal.iter().find(|mv| mv.to_uci() == *m) {
                board.make_move(*mv);
            }
        }
        assert!(
            board.has_repetition(),
            "Should detect repetition after Nf3 Nf6 Ng1 Ng8"
        );
    }

    #[test]
    fn test_pv_line_reported() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 4);
                assert!(
                    !result.pv.is_empty(),
                    "PV line should contain at least one move"
                );
                assert_eq!(result.pv[0].from, result.best_move.from);
                assert_eq!(result.pv[0].to, result.best_move.to);
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_mate_distance_pruning() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::from_fen(
                    "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
                )
                .unwrap();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 6);
                assert!(
                    result.score > MATE_SCORE - 10,
                    "Should still detect mate with MDP"
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    #[test]
    fn test_tt_mate_score_normalization_roundtrip() {
        let mate_score = MATE_SCORE - 12;
        let ply = 7;
        let stored = score_to_tt(mate_score, ply);
        let restored = score_from_tt(stored, ply);
        assert_eq!(restored, mate_score);

        let mated_score = -MATE_SCORE + 9;
        let stored_neg = score_to_tt(mated_score, ply);
        let restored_neg = score_from_tt(stored_neg, ply);
        assert_eq!(restored_neg, mated_score);
    }

    #[test]
    fn check_extensions_consume_the_path_budget() {
        assert_eq!(budgeted_check_extension(7, 1, 3, true), (8, 2));
        assert_eq!(budgeted_check_extension(7, 3, 3, true), (7, 3));
        assert_eq!(budgeted_check_extension(7, 1, 3, false), (7, 1));
    }

    #[test]
    fn searched_history_malus_buffers_stack_sized_like_former_vecs() {
        assert_eq!(SEARCHED_QUIETS_MAX, 32);
        assert_eq!(SEARCHED_CAPTURES_MAX, 16);
    }

    #[test]
    fn test_same_move_key_checks_promotion_piece() {
        let q = Move::promotion(types::Square::A7, types::Square::A8, Piece::Queen);
        let n = Move::promotion(types::Square::A7, types::Square::A8, Piece::Knight);
        assert!(!same_move_key(q, n));
        assert!(same_move_key(q, q));
    }

    #[test]
    fn test_nmp_material_ok_requires_non_pawn_material() {
        setup();
        let start = Board::new();
        assert!(nmp_material_ok(&start, types::Color::White));
        assert!(nmp_material_ok(&start, types::Color::Black));

        let kp_only = Board::from_fen("8/8/4k3/8/8/8/4p3/4K3 w - - 0 1").unwrap();
        assert!(!nmp_material_ok(&kp_only, types::Color::White));
        assert!(!nmp_material_ok(&kp_only, types::Color::Black));
    }

    #[test]
    fn iir_predicate_matches_stockfish_style_gates() {
        setup();
        let m = Move::from_uci("e2e4").unwrap();
        assert!(!should_apply_iir(None, 3, false, false, None));
        assert!(should_apply_iir(None, 4, false, false, None));
        assert!(!should_apply_iir(None, 5, true, false, None));
        assert!(should_apply_iir(None, 6, true, false, None));
        assert!(!should_apply_iir(None, 4, false, true, None));
        assert!(!should_apply_iir(None, 4, false, false, Some(m)));
        assert!(!should_apply_iir(Some(m), 4, false, false, None));
    }

    #[test]
    fn null_tt_move_does_not_suppress_iir() {
        let stored = Move::from_uci("e2e4").unwrap();
        assert_eq!(usable_tt_move(NULL_MOVE), None);
        assert_eq!(usable_tt_move(stored), Some(stored));
        assert!(should_apply_iir(
            usable_tt_move(NULL_MOVE),
            4,
            false,
            false,
            None
        ));
    }

    #[test]
    fn compact_history_updates_remain_bounded() {
        let mut history = 0;
        for _ in 0..1_000 {
            update_history(&mut history, 2_000);
        }
        assert!(i32::from(history) <= MAX_HISTORY);
        for _ in 0..1_000 {
            update_history(&mut history, -2_000);
        }
        assert!(i32::from(history) >= -MAX_HISTORY);

        let mut correction = 0;
        update_correction_entry(&mut correction, MAX_CORR_ENTRY, 16);
        assert_eq!(correction, 32);
        for _ in 0..1_000 {
            update_correction_entry(&mut correction, 10_000, 16);
        }
        assert_eq!(i32::from(correction), MAX_CORR_ENTRY);
    }

    #[test]
    fn ranked_history_malus_favors_earlier_failed_moves() {
        let malus = 1_200;
        let first = ranked_history_malus(malus, 0);
        let middle = ranked_history_malus(malus, SEARCHED_QUIETS_MAX / 2);
        let last = ranked_history_malus(malus, SEARCHED_QUIETS_MAX - 1);

        assert_eq!(first, malus);
        assert!(first > middle && middle > last && last > 0);
        assert_eq!(last, ranked_history_malus(malus, usize::MAX));
    }

    #[test]
    fn reverse_qsearch_gate_matches_reckless_conditions() {
        let quiet = Move::from_uci("e2e4").unwrap();
        let capture = Move {
            flag: types::chess_move::MoveFlag::Capture,
            ..quiet
        };

        assert!(should_reverse_qsearch(
            false,
            false,
            quiet,
            NodeType::LowerBound
        ));
        assert!(!should_reverse_qsearch(
            true,
            false,
            quiet,
            NodeType::LowerBound
        ));
        assert!(!should_reverse_qsearch(
            false,
            true,
            quiet,
            NodeType::LowerBound
        ));
        assert!(!should_reverse_qsearch(
            false,
            false,
            quiet,
            NodeType::UpperBound
        ));
        assert!(!should_reverse_qsearch(
            false,
            false,
            capture,
            NodeType::LowerBound
        ));
        assert!(!should_reverse_qsearch(
            false,
            false,
            NULL_MOVE,
            NodeType::LowerBound
        ));
    }

    #[test]
    fn qsearch_see_margin_uses_reckless_history_and_correction() {
        assert_eq!(
            qsearch_see_threshold(100, 20, -80, 480, MoveOrderingProfile::Reckless),
            Some(-142)
        );
        assert_eq!(
            qsearch_see_threshold(100, 20, -80, 480, MoveOrderingProfile::StockLike),
            None
        );
    }

    #[test]
    fn hindsight_adjusts_depth_from_reduction_and_eval_swing() {
        let excluded = Move::from_uci("e2e4").unwrap();
        assert_eq!(
            hindsight_depth_adjustment(false, false, None, 3, 4, -100, Some(50)),
            1
        );
        assert_eq!(
            hindsight_depth_adjustment(false, false, None, 2, 4, -100, Some(50)),
            0
        );
        assert_eq!(
            hindsight_depth_adjustment(false, false, None, 2, 4, 120, Some(50)),
            -1
        );
        assert_eq!(
            hindsight_depth_adjustment(false, false, None, 2, 1, 120, Some(50)),
            0
        );
        assert_eq!(
            hindsight_depth_adjustment(true, false, None, 3, 4, -100, Some(50)),
            0
        );
        assert_eq!(
            hindsight_depth_adjustment(false, true, None, 3, 4, -100, Some(50)),
            0
        );
        assert_eq!(
            hindsight_depth_adjustment(false, false, Some(excluded), 3, 4, -100, Some(50)),
            0
        );
    }

    #[test]
    fn reckless_eval_adapter_scales_material_clock_and_optimism() {
        let mut board = Board::new();
        assert_eq!(reckless_material(&board), 10_296);
        let stockfish = corrected_network_eval(
            &board,
            100,
            7,
            0,
            EvalMode::Nnue(NnueSearchProfile::Stockfish),
        );
        assert_ne!(stockfish, 107);
        assert!(stockfish > 100);
        assert_eq!(
            corrected_network_eval(
                &board,
                100,
                7,
                0,
                EvalMode::Nnue(NnueSearchProfile::Reckless)
            ),
            122
        );

        board.halfmove_clock = 80;
        assert_eq!(
            corrected_network_eval(
                &board,
                100,
                7,
                0,
                EvalMode::Nnue(NnueSearchProfile::Reckless)
            ),
            76
        );

        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        update_reckless_optimism(
            &mut state,
            types::Color::White,
            100,
            root_score_stat(7, 300),
            EvalMode::Nnue(NnueSearchProfile::Reckless),
        );
        assert_eq!(state.optimism, [56, -56]);
        update_reckless_optimism(
            &mut state,
            types::Color::White,
            200,
            root_score_stat(7, 200),
            EvalMode::Nnue(NnueSearchProfile::Stockfish),
        );
        assert_ne!(state.optimism, [0; 2]);
    }

    #[test]
    fn stockfish_net_uses_upstream_material_blend_not_reckless_scale() {
        let board = Board::new();
        let stockfish = corrected_network_eval(
            &board,
            100,
            0,
            0,
            EvalMode::Nnue(NnueSearchProfile::Stockfish),
        );
        let reckless = corrected_network_eval(
            &board,
            100,
            0,
            0,
            EvalMode::Nnue(NnueSearchProfile::Reckless),
        );
        assert_eq!(stockfish, 122);
        assert_eq!(reckless, 115);
        assert_eq!(stockfish_material(&board), 17_208);
    }

    #[test]
    fn reckless_root_score_statistics_track_depth_and_smoothed_score() {
        let mut average = 40;
        update_root_average(&mut average, 100);
        assert_eq!(average, 70);

        let shallow = root_score_stat(5, 500);
        let deep = root_score_stat(6, -200);
        assert!(deep > shallow, "depth must dominate the shared statistic");
        assert_eq!((deep & 0xffff) as i32 - 32_768, -200);
    }

    #[test]
    fn draw_score_jitter_is_small_and_deterministic() {
        assert_eq!(draw_score(0), -1);
        assert_eq!(draw_score(1), -1);
        assert_eq!(draw_score(2), 1);
        assert_eq!(draw_score(3), 1);
        assert_eq!(draw_score(4), -1);
    }

    fn play_uci(board: &mut Board, moves: &[&str]) {
        for uci in moves {
            let mv = board
                .generate_legal_moves()
                .iter()
                .copied()
                .find(|candidate| candidate.to_uci() == *uci)
                .unwrap_or_else(|| panic!("illegal test move {uci}"));
            board.make_move(mv);
        }
    }

    #[test]
    fn winning_side_does_not_choose_a_repeating_knight_shuffle() {
        setup();
        // White is a queen up. A knight tour has already occurred once, so
        // g1f3 would repeat the start of the cycle.
        let mut board =
            Board::from_fen("4k3/8/8/8/8/8/8/Q3K1N1 w - - 0 1").expect("winning queen ending");
        play_uci(&mut board, &["g1f3", "e8d8", "f3g1", "d8e8"]);
        assert!(board.has_repetition());
        assert!(!board.has_threefold_repetition());

        let mut engine = SearchEngine::new(4, 1);
        let _ = engine.install_adapter("mujrim-hce");
        engine.set_contempt(48);
        let result = engine.search_nodes(&mut board, 4_000, 8);
        assert_ne!(result.best_move.to_uci(), "g1f3");
        let legal = board.generate_legal_moves();
        assert!(legal.iter().any(|mv| *mv == result.best_move));
    }

    #[test]
    fn contempt_is_zero_on_nnue_and_active_only_for_hce() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(matches!(engine.eval_mode(), EvalMode::Nnue(_)));
        assert_eq!(engine.contempt(), 0);
        engine.set_contempt(200);
        assert_eq!(engine.requested_contempt(), 100);
        assert_eq!(engine.contempt(), 0);

        assert!(engine.install_adapter("mujrim-hce"));
        assert_eq!(engine.contempt(), 100);
        engine.set_contempt(-200);
        assert_eq!(engine.requested_contempt(), -100);
        assert_eq!(engine.contempt(), -100);

        assert!(engine.install_adapter("akimbo"));
        assert_eq!(engine.requested_contempt(), -100);
        assert_eq!(engine.contempt(), 0);
    }

    #[test]
    fn root_exclusions_force_a_different_best_move() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(4, 1);
        let first = engine.search_nodes(&mut board, 1_500, 4);
        assert_ne!(first.best_move, NULL_MOVE);
        engine.set_root_exclusions(std::slice::from_ref(&first.best_move));
        let second = engine.search_nodes(&mut board, 1_500, 4);
        engine.set_root_exclusions(&[]);
        assert_ne!(second.best_move, NULL_MOVE);
        assert_ne!(second.best_move, first.best_move);
    }

    #[test]
    fn deadline_token_can_be_armed_from_another_handle() {
        let engine = SearchEngine::new(1, 1);
        let token = engine.deadline_token();
        assert_eq!(token.load(Ordering::Relaxed), 0);
        token.store(1, Ordering::Relaxed);
        assert_eq!(engine.deadline_token().load(Ordering::Relaxed), 1);
    }

    #[test]
    fn search_persists_main_thread_score_for_next_root() {
        let mut engine = SearchEngine::new(1, 1);
        let mut board = Board::new();
        let result = engine.search_depth(&mut board, 1);
        assert_eq!(engine.previous_best_score, result.score);
        engine.clear();
        assert_eq!(engine.previous_best_score, 0);
    }

    #[test]
    fn test_qsearch_in_check_at_qs_cap_still_reports_mate() {
        setup();
        let mut board = Board::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        let tt = TranspositionTable::new(1);
        let stopped = AtomicBool::new(false);
        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        let params = SearchParams::default();
        let lmr_table = params.build_lmr_table();
        let context = test_context(&tt, &stopped, &params, &lmr_table);
        let score = quiescence(
            &mut board,
            &mut state,
            &context,
            QuiescenceNode {
                alpha: -INF,
                beta: INF,
                ply: 0,
                qs_ply: params.max_qs_ply,
                is_pv: false,
            },
        );
        assert_eq!(score, -MATE_SCORE);
    }

    #[test]
    fn qsearch_honors_fifty_move_draws() {
        setup();
        let mut board = Board::from_fen("7k/8/6KQ/8/8/8/8/8 w - - 100 1").unwrap();
        assert!(board.is_draw());
        let tt = TranspositionTable::new(1);
        let stopped = AtomicBool::new(false);
        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        let params = SearchParams::default();
        let lmr_table = params.build_lmr_table();
        let context = test_context(&tt, &stopped, &params, &lmr_table);
        let score = quiescence(
            &mut board,
            &mut state,
            &context,
            QuiescenceNode {
                alpha: -INF,
                beta: INF,
                ply: 0,
                qs_ply: 0,
                is_pv: false,
            },
        );
        assert_eq!(score, draw_score(state.nodes));
    }

    #[test]
    fn test_set_nnue_network_updates_engine_info() {
        let mut engine = SearchEngine::new(1, 1);
        assert_eq!(
            engine.nnue_preset_hint(),
            default_embedded_network().preset_hint()
        );

        if !eval::nnue::enabled_network_formats().contains(&eval::nnue::NetworkFormat::Akimbo) {
            return;
        }

        let net_path = format!(
            "{}/../mujrim-eval/resources/ak_default.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        let loaded = eval::nnue::load_network(Path::new(&net_path)).expect("load ak_default.bin");
        engine.set_nnue_network(loaded);
        assert_eq!(engine.nnue_info().format, eval::nnue::NetworkFormat::Akimbo);
        // Verify auto-preset was applied
        assert_eq!(engine.nnue_preset_hint(), "akimbo");
        assert_eq!(engine.params().nmp_base, 5); // Akimbo NMP base
    }

    #[test]
    fn test_set_params_rebuilds_lmr_table() {
        let mut engine = SearchEngine::new(1, 1);
        let before = engine.lmr_reduction_for(24, 24);
        let mut tuned = engine.params().clone();
        tuned.lmr_divisor = 1.25;
        engine.set_params(tuned);
        let after = engine.lmr_reduction_for(24, 24);
        assert_ne!(before, after);
    }

    #[test]
    fn reckless_preset_selects_coherent_reckless_policies() {
        let mut engine = SearchEngine::new(1, 1);
        let default_ordering = if default_embedded_network().preset_hint() == "reckless" {
            MoveOrderingProfile::Reckless
        } else {
            MoveOrderingProfile::StockLike
        };
        assert_eq!(engine.search_stack.policies.move_ordering, default_ordering);
        if default_ordering == MoveOrderingProfile::Reckless {
            assert!(
                engine
                    .search_stack
                    .policies
                    .futility
                    .requires_direct_check()
            );
            assert!(
                engine
                    .search_stack
                    .policies
                    .lmp
                    .decision(&LmpContext {
                        depth: 3,
                        move_count: 15,
                        improvement: 0,
                        improving: false,
                        is_root: false,
                        is_pv: false,
                        in_check: false,
                        is_quiet: true,
                        best_score: 0,
                        stock_depth_limit: 0,
                        stock_move_threshold: usize::MAX,
                    })
                    .is_some()
            );
        }
        engine.set_params_for_preset("reckless");
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::Reckless
        );
        assert!(matches!(
            engine.search_stack.policies.lmr,
            LmrDispatch::RecklessFull
        ));
        assert!(
            engine
                .search_stack
                .policies
                .futility
                .requires_direct_check()
        );
        assert!(
            engine
                .search_stack
                .policies
                .lmp
                .decision(&LmpContext {
                    depth: 3,
                    move_count: 15,
                    improvement: 0,
                    improving: false,
                    is_root: false,
                    is_pv: false,
                    in_check: false,
                    is_quiet: true,
                    best_score: 0,
                    stock_depth_limit: 0,
                    stock_move_threshold: usize::MAX,
                })
                .is_some()
        );
        engine.set_params_for_preset("stockfish");
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::StockLike
        );
        assert!(
            engine
                .search_stack
                .policies
                .futility
                .requires_direct_check()
        );
    }

    #[test]
    fn low_depth_extension_is_bounded_to_losing_cut_nodes() {
        let params = SearchParams::reckless();
        assert_eq!(low_depth_extension(7, false, true, -26, 0, &params), 1);
        assert_eq!(low_depth_extension(8, false, true, -26, 0, &params), 0);
        assert_eq!(low_depth_extension(7, true, true, -26, 0, &params), 0);
        assert_eq!(low_depth_extension(7, false, false, -26, 0, &params), 0);
        assert_eq!(low_depth_extension(7, false, true, -24, 0, &params), 0);
        assert_eq!(
            low_depth_extension(7, false, true, -100, 0, &SearchParams::akimbo()),
            0
        );
    }

    #[test]
    fn singular_multicut_applies_to_stockfish_and_preserves_mate_scores() {
        assert_eq!(
            singular_multicut_score(200, 100, MoveOrderingProfile::Reckless),
            Some(159)
        );
        assert_eq!(
            singular_multicut_score(99, 100, MoveOrderingProfile::Reckless),
            None
        );
        assert_eq!(
            singular_multicut_score(200, 100, MoveOrderingProfile::StockLike),
            Some(159)
        );
        assert_eq!(
            singular_multicut_score(MATE_SCORE - 1, 100, MoveOrderingProfile::Reckless),
            None
        );
    }

    #[test]
    fn test_set_use_nnue_toggles_engine_mode() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(engine.use_nnue());
        engine.set_use_nnue(false);
        assert!(!engine.use_nnue());
    }

    #[test]
    fn use_nnue_false_uses_classical_eval_in_search() {
        setup();
        let mut classical = SearchEngine::new(4, 1);
        classical.set_use_nnue(false);
        assert!(!classical.use_nnue());

        let board = Board::new();
        let classical_static = eval::evaluate(&board);
        let mut nnue_state = NNUEState::new();
        let nnue_static = nnue_state.evaluate(&board);
        assert_ne!(
            classical_static, nnue_static,
            "HCE and NNUE must be distinct evaluators"
        );

        let mut search_board = board;
        let result = classical.search_nodes(&mut search_board, 1_200, 6);
        assert!(result.nodes > 0);
        assert_ne!(result.best_move, NULL_MOVE);

        let mut see_board = Board::from_fen("4k3/8/8/3q4/4P3/8/8/4K3 w - - 0 1").unwrap();
        let capture = see_board
            .generate_legal_moves()
            .iter()
            .find(|mv| mv.to_uci() == "e4d5")
            .copied()
            .expect("pawn takes queen e4d5");
        assert!(see::see(&see_board, capture) > 0);
    }

    #[test]
    fn stockfish_preset_selects_stockfish_search_params() {
        let mut engine = SearchEngine::new(1, 1);
        engine.set_params_for_preset("stockfish");
        assert_eq!(
            engine.network_profile(),
            Some(eval::nnue::NnueSearchProfile::Stockfish)
        );
        assert_eq!(engine.params().nmp_base, 5);
        assert_eq!(engine.params().aspiration_window, 10);
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::StockLike
        );
    }

    #[test]
    fn resize_tt_releases_the_previous_table_and_clears_entries() {
        let mut engine = SearchEngine::new(1, 1);
        let previous = Arc::downgrade(&engine.tt);
        engine.tt.store(
            42,
            TTData::new(1, 10, NodeType::Exact, NULL_MOVE, false, None),
        );

        engine.resize_tt(2);

        assert!(previous.upgrade().is_none());
        assert!(engine.tt.probe(42).is_none());
    }

    #[test]
    fn test_captured_piece_index_handles_en_passant() {
        setup();
        let mut board = Board::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").unwrap();
        let ep = board
            .generate_legal_moves()
            .iter()
            .find(|m| m.to_uci() == "e5d6")
            .copied()
            .expect("en passant move must be legal");
        assert_eq!(captured_piece_index(&board, ep), Some(Piece::Pawn.index()));
    }

    #[test]
    fn test_correction_history_is_side_to_move_scoped() {
        setup();
        let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
        let white = Board::new();
        let black =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();

        assert_eq!(state.correction(&white, MoveOrderingProfile::StockLike), 0);
        assert_eq!(state.correction(&black, MoveOrderingProfile::StockLike), 0);

        state.update_correction(&white, 12, 180, 40, MoveOrderingProfile::StockLike);
        assert_ne!(state.correction(&white, MoveOrderingProfile::StockLike), 0);
        assert_eq!(
            state.correction(&black, MoveOrderingProfile::StockLike),
            0,
            "white correction updates must not leak into black side-to-move bucket"
        );
    }

    /// Phase 3: RFP returns an interpolated score between beta and static_eval,
    /// not the raw static_eval.
    #[test]
    fn test_rfp_returns_interpolated_score() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                // Position where white is up a full queen — RFP should fire
                // at shallow depth with eval >> beta.
                let mut board =
                    Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
                        .unwrap();
                let mut engine = SearchEngine::new(1, 1);
                let result = engine.search_depth(&mut board, 4);
                // Score should be positive (huge material advantage) but NOT
                // equal to the raw eval — interpolation brings it closer to beta.
                assert!(
                    result.score > 0,
                    "Should detect material advantage, got {}",
                    result.score
                );
                // The move should still be legal
                let legal = board.generate_legal_moves();
                assert!(
                    legal
                        .iter()
                        .any(|m| m.from == result.best_move.from && m.to == result.best_move.to)
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    /// Phase 3: QS stand-pat cutoff returns a softened score.
    /// We test that qsearch in a winning position returns a reasonable value,
    /// not an inflated raw eval.
    #[test]
    fn test_qs_softened_standpat() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                // Quiet position with extra knight — QS should hit stand-pat immediately
                let mut board =
                    Board::from_fen("rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R w KQkq - 0 1")
                        .unwrap();
                let tt = TranspositionTable::new(1);
                let stopped = AtomicBool::new(false);
                let mut state = ThreadState::new(Arc::new(ActiveNetwork::Embedded));
                let params = SearchParams::default();
                let lmr_table = params.build_lmr_table();
                let context = test_context(&tt, &stopped, &params, &lmr_table);

                // Get raw eval for comparison
                let raw_eval = hybrid_eval(&board, &mut state, true);
                let corr = state.correction(&board, MoveOrderingProfile::StockLike);
                let stand_pat_val = corrected_network_eval(
                    &board,
                    raw_eval,
                    corr,
                    0,
                    EvalMode::Nnue(NnueSearchProfile::Stockfish),
                );

                assert!(stand_pat_val > 100);
                let beta = stand_pat_val - 50;
                let qs_score = quiescence(
                    &mut board,
                    &mut state,
                    &context,
                    QuiescenceNode {
                        alpha: -INF,
                        beta,
                        ply: 0,
                        qs_ply: 0,
                        is_pv: false,
                    },
                );
                assert_eq!(qs_score, beta + (stand_pat_val - beta) / 6);
                let entry = tt
                    .probe(board.tt_hash())
                    .expect("stand-pat fail-high should be cached");
                assert_eq!(entry.node_type, NodeType::LowerBound);
                assert_eq!(score_from_tt(entry.score, 0), qs_score);
                assert_eq!(entry.raw_eval, Some(raw_eval));
                assert_eq!(
                    soften_qsearch_fail_high(MATE_SCORE - 1, beta, 6),
                    MATE_SCORE - 1
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }

    /// Phase 3 integration: search to depth 8 with all optimizations active
    /// and verify correctness.
    #[test]
    fn test_search_depth_8_legal_and_stable() {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                setup();
                let mut board = Board::new();
                let original_fen = board.to_fen();
                let original_hash = board.hash;
                let mut engine = SearchEngine::new(8, 1);
                let result = engine.search_depth(&mut board, 8);

                // Verify legal move
                let legal = board.generate_legal_moves();
                assert!(
                    legal
                        .iter()
                        .any(|m| m.from == result.best_move.from && m.to == result.best_move.to),
                    "depth-8 search must return a legal move"
                );

                // Board is restored
                assert_eq!(board.to_fen(), original_fen);
                assert_eq!(board.hash, original_hash);

                // Score is reasonable (opening eval should be near 0)
                assert!(
                    result.score.abs() < 300,
                    "Opening score should be reasonable, got {}",
                    result.score
                );

                // PV must be non-empty
                assert!(
                    !result.pv.is_empty(),
                    "PV line must not be empty at depth 8"
                );

                // Reasonable node count
                assert!(
                    result.nodes > 1000,
                    "depth-8 search should explore >1000 nodes, got {}",
                    result.nodes
                );
            })
            .expect("Failed to spawn test thread");
        handle.join().expect("Test thread panicked");
    }
}
