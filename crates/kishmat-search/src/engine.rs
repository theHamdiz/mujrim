//! The search engine: iterative deepening with alpha-beta, quiescence search,
//! null-move pruning, late-move reductions, PVS, aspiration windows,
//! killer moves, history heuristic, countermove heuristic, LMP,
//! check extensions, singular extensions, razoring, ProbCut,
//! IIR, SEE-based pruning, futility/delta pruning, PV tracking.
//! Supports Lazy SMP multi-threaded search via shared transposition table.

use crate::move_picker::MovePicker;
use crate::policy::{
    DepthScoreVoteRootSelection, LmrContext, LmrPolicy, RootSelectionPolicy, StockLikeLmrPolicy,
    ThreadOutcome,
};
use crate::search_params::SearchParams;
use crate::see;
use crate::tt::{NodeType, TranspositionTable};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use types::chess_move::NULL_MOVE;
use types::{Board, Move, MoveList, Piece};

use eval::nnue::NNUEState;

/// Infinity score sentinel.
const INF: i32 = 30_000;
/// Checkmate score base (mate in N = MATE_SCORE - N).
const MATE_SCORE: i32 = 29_000;
/// Maximum search ply depth.
const MAX_PLY: usize = 128;
/// Delta pruning margin in quiescence — standard queen value.
const DELTA_MARGIN: i32 = 400;
/// Maximum quiescence search depth to prevent explosion in tactical chaos.
const MAX_QS_PLY: i32 = 8;
/// Threshold for TT mate score normalization.
const MATE_TT_THRESHOLD: i32 = MATE_SCORE - MAX_PLY as i32;
/// Maximum history score (Stockfish uses 16384).
const MAX_HISTORY: i32 = 16384;
/// Number of piece types for indexing.
const NUM_PIECES: usize = 6;
/// Number of squares.
const NUM_SQUARES: usize = 64;
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

/// Precomputed LMR reduction table — Stockfish formula.
static LMR_TABLE: std::sync::OnceLock<[[i32; 128]; 128]> = std::sync::OnceLock::new();

#[inline(always)]
fn lmr_table() -> &'static [[i32; 128]; 128] {
    LMR_TABLE.get_or_init(|| {
        let mut table = [[0i32; 128]; 128];
        for depth in 1..128 {
            for moves in 1..128 {
                // Stockfish formula: 0.77 + ln(depth) * ln(moveCount) / 2.36
                table[depth][moves] =
                    (0.77 + (depth as f64).ln() * (moves as f64).ln() / 2.36) as i32;
            }
        }
        table
    })
}

/// Update history with gravity — Stockfish formula:
/// `entry += bonus - entry * |bonus| / MAX_HISTORY`
#[inline(always)]
fn update_history(entry: &mut i32, bonus: i32) {
    *entry += bonus - *entry * bonus.abs() / MAX_HISTORY;
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

/// Compute the evaluation for a position using NNUE.
/// Uses the cached accumulator table — avoids full recompute when the
/// board's piece bitboards haven't changed since the last eval in the
/// same king-bucket pair.
#[inline(always)]
fn hybrid_eval(board: &Board, nnue_state: &mut NNUEState) -> i32 {
    nnue_state.evaluate(board)
}

/// Get the piece index of the piece on `sq`. Returns 0 (pawn) if no piece.
#[inline(always)]
fn piece_index_on(board: &Board, sq: types::Square) -> usize {
    board.piece_on(sq).map_or(0, |(p, _)| p.index())
}

/// Search result returned to the caller.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best_move: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub elapsed: Duration,
    /// Principal variation line.
    pub pv: Vec<Move>,
}

/// Configuration for a search.
#[derive(Clone, Debug)]
pub struct SearchLimits {
    pub max_depth: i32,
    pub time_limit: Option<Duration>,
    pub stopped: bool,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            time_limit: None,
            stopped: false,
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
    killers: [[Move; 2]; MAX_PLY],
    /// Main quiet history: [color][from][to] — 32KB
    history: Box<[[[i32; 64]; 64]; 2]>,
    /// Continuation history (1-ply back): [prev_piece][prev_to][cur_piece][cur_to]
    cont_hist: Box<[[[[i32; NUM_SQUARES]; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES]>,
    /// Continuation history (2-ply back): same layout
    cont_hist2: Box<[[[[i32; NUM_SQUARES]; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES]>,
    /// Capture history: [moved_piece][to_square][captured_piece_type] — 9KB
    cap_hist: Box<[[[i32; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES]>,
    /// Countermoves: [from][to] — 32KB
    countermoves: Box<[[Move; 64]; 64]>,
    /// Static eval at each ply (for improving detection).
    static_evals: [i32; MAX_PLY],
    /// NNUE evaluation state.
    nnue_state: NNUEState,
    /// Triangular PV table — 128KB, must be heap-allocated.
    pv: Box<[[Move; MAX_PLY]; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],
    /// Previous move at each ply (for countermove/continuation indexing).
    prev_move: [Move; MAX_PLY],
    /// Piece type of the move at each ply (for continuation history indexing).
    prev_piece: [usize; MAX_PLY],
    /// Correction history tables (search_score - static_eval differences).
    pawn_corr: Box<[i32; CORR_HIST_SIZE]>,
    material_corr: Box<[i32; CORR_HIST_SIZE]>,
    minor_corr: Box<[i32; CORR_HIST_SIZE]>,
    /// Double extension count per path (Akimbo anti-explosion).
    dbl_exts: [i32; MAX_PLY],
    /// Minimum ply before NMP is allowed again (Akimbo anti-recursion).
    min_nmp_ply: usize,
}

impl ThreadState {
    fn new() -> Self {
        // Use helper to allocate large arrays DIRECTLY on the heap.
        // `Box::new([val; N])` creates the array on the stack first — for arrays
        // >100KB this overflows. These helpers avoid that.
        Self {
            nodes: 0,
            killers: [[NULL_MOVE; 2]; MAX_PLY],
            history: boxed_zeroed(),
            cont_hist: boxed_zeroed(),
            cont_hist2: boxed_zeroed(),
            cap_hist: boxed_zeroed(),
            countermoves: boxed_zeroed(),
            static_evals: [0; MAX_PLY],
            nnue_state: NNUEState::new(),
            pv: boxed_zeroed(),
            pv_len: [0; MAX_PLY],
            prev_move: [NULL_MOVE; MAX_PLY],
            prev_piece: [0; MAX_PLY],
            pawn_corr: boxed_zeroed(),
            material_corr: boxed_zeroed(),
            minor_corr: boxed_zeroed(),
            dbl_exts: [0; MAX_PLY],
            min_nmp_ply: 0,
        }
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn age_history(&mut self) {
        for color_hist in self.history.iter_mut() {
            for row in color_hist.iter_mut() {
                for val in row.iter_mut() {
                    *val /= 2;
                }
            }
        }
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
        for a in self.cap_hist.iter_mut() {
            for b in a.iter_mut() {
                for v in b.iter_mut() {
                    *v /= 2;
                }
            }
        }
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn clear(&mut self) {
        self.killers = [[NULL_MOVE; 2]; MAX_PLY];
        self.history = boxed_zeroed();
        self.cont_hist = boxed_zeroed();
        self.cont_hist2 = boxed_zeroed();
        self.cap_hist = boxed_zeroed();
        self.countermoves = boxed_zeroed();
        self.pawn_corr = boxed_zeroed();
        self.material_corr = boxed_zeroed();
        self.minor_corr = boxed_zeroed();
    }

    /// Compute the combined correction for a position.
    #[inline(always)]
    fn correction(&self, board: &Board) -> i32 {
        let ph = pawn_hash(board) & CORR_HIST_MASK;
        let mh = material_hash(board) & CORR_HIST_MASK;
        let nh = minor_hash(board) & CORR_HIST_MASK;
        let raw = self.pawn_corr[ph] as i64 * PAWN_CORR_WEIGHT as i64
            + self.material_corr[mh] as i64 * MATERIAL_CORR_WEIGHT as i64
            + self.minor_corr[nh] as i64 * MINOR_CORR_WEIGHT as i64;
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
    ) {
        if search_score.abs() > MATE_SCORE - 100 {
            return;
        }
        let error = search_score - static_eval;
        let weight = (depth as i32).min(16);
        let ph = pawn_hash(board) & CORR_HIST_MASK;
        let mh = material_hash(board) & CORR_HIST_MASK;
        let nh = minor_hash(board) & CORR_HIST_MASK;
        // Exponential moving average update
        self.pawn_corr[ph] = ((self.pawn_corr[ph] as i64 * (256 - weight as i64)
            + error as i64 * weight as i64)
            / 256) as i32;
        self.pawn_corr[ph] = self.pawn_corr[ph].clamp(-MAX_CORR_ENTRY, MAX_CORR_ENTRY);
        self.material_corr[mh] = ((self.material_corr[mh] as i64 * (256 - weight as i64)
            + error as i64 * weight as i64)
            / 256) as i32;
        self.material_corr[mh] = self.material_corr[mh].clamp(-MAX_CORR_ENTRY, MAX_CORR_ENTRY);
        self.minor_corr[nh] = ((self.minor_corr[nh] as i64 * (256 - weight as i64)
            + error as i64 * weight as i64)
            / 256) as i32;
        self.minor_corr[nh] = self.minor_corr[nh].clamp(-MAX_CORR_ENTRY, MAX_CORR_ENTRY);
    }

    /// Compute stat_score for a quiet move: main history + continuation histories.
    #[inline(always)]
    fn stat_score(&self, mv: Move, us: usize, piece: usize, ply: usize) -> i32 {
        let mut score = self.history[us][mv.from.index()][mv.to.index()];
        // Add 1-ply continuation history
        if ply > 0 {
            let pp = self.prev_piece[ply.saturating_sub(1)];
            let pt = self.prev_move[ply.saturating_sub(1)].to.index();
            score += self.cont_hist[pp][pt][piece][mv.to.index()];
        }
        // Add 2-ply continuation history
        if ply > 1 {
            let pp2 = self.prev_piece[ply.saturating_sub(2)];
            let pt2 = self.prev_move[ply.saturating_sub(2)].to.index();
            score += self.cont_hist2[pp2][pt2][piece][mv.to.index()];
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
    /// Per-network search parameters (swapped when NNUE net changes).
    pub params: SearchParams,
    lmr_policy: Arc<dyn LmrPolicy + Send + Sync>,
    root_selection_policy: Arc<dyn RootSelectionPolicy + Send + Sync>,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new(4096, 32)
    }
}

impl SearchEngine {
    /// Creates a new search engine with the given TT size and thread count.
    pub fn new(tt_size_mb: usize, num_threads: usize) -> Self {
        let _ = lmr_table();
        // NNUE is always available (embedded trained net)
        Self {
            tt: Arc::new(TranspositionTable::new(tt_size_mb)),
            num_threads: num_threads.max(1),
            stopped: Arc::new(AtomicBool::new(false)),
            params: SearchParams::default(),
            lmr_policy: Arc::new(StockLikeLmrPolicy),
            root_selection_policy: Arc::new(DepthScoreVoteRootSelection),
        }
    }

    /// Set the search parameters (e.g. when switching NNUE networks).
    pub fn set_params(&mut self, params: SearchParams) {
        self.params = params;
    }

    /// Configure search params for a given network preset name.
    pub fn set_params_for_preset(&mut self, preset: &str) {
        self.params = SearchParams::for_preset(preset);
    }

    /// Set a custom LMR policy implementation.
    pub fn set_lmr_policy(&mut self, policy: Arc<dyn LmrPolicy + Send + Sync>) {
        self.lmr_policy = policy;
    }

    /// Set a custom root move selection policy for Lazy SMP.
    pub fn set_root_selection_policy(
        &mut self,
        policy: Arc<dyn RootSelectionPolicy + Send + Sync>,
    ) {
        self.root_selection_policy = policy;
    }

    /// Performs search with Lazy SMP.
    pub fn search(&mut self, board: &mut Board, limits: SearchLimits) -> SearchResult {
        self.stopped.store(false, Ordering::SeqCst);
        self.tt.new_generation();

        let start_time = Instant::now();

        // Spawn helper threads for Lazy SMP (threads > 1)
        let mut handles = Vec::new();
        for thread_id in 1..self.num_threads {
            let tt = Arc::clone(&self.tt);
            let stopped = Arc::clone(&self.stopped);
            let mut board_clone = board.clone();
            let max_depth = limits.max_depth;
            let time_limit = limits.time_limit;
            let start = start_time;
            let params_clone = self.params.clone();
            let lmr_policy_clone = Arc::clone(&self.lmr_policy);

            handles.push(
                std::thread::Builder::new()
                    .stack_size(16 * 1024 * 1024)
                    .spawn(move || {
                        let mut state = ThreadState::new();
                        // Helper threads search with depth offsets for diversity
                        let depth_offset = match thread_id % 4 {
                            1 => 1,
                            2 => -1i32,
                            3 => 2,
                            _ => 0,
                        };

                        let mut best_score = -INF;
                        let mut best_move = NULL_MOVE;
                        let mut completed_depth = 0i32;

                        for depth in 1..=max_depth {
                            let actual_depth = (depth as i32 + depth_offset).max(1).min(max_depth);

                            // Check time/stop
                            if stopped.load(Ordering::Relaxed) {
                                break;
                            }
                            if let Some(tl) = time_limit {
                                if start.elapsed() >= tl {
                                    break;
                                }
                            }

                            let s = search_ab(
                                &mut board_clone,
                                &tt,
                                &stopped,
                                &mut state,
                                actual_depth,
                                -INF,
                                INF,
                                0,
                                true,
                                time_limit,
                                start,
                                true,
                                None,
                                0,
                                actual_depth,
                                &params_clone,
                                lmr_policy_clone.as_ref(),
                            );
                            if !stopped.load(Ordering::Relaxed) {
                                best_score = s;
                                completed_depth = actual_depth;
                                if let Some(entry) = tt.probe(board_clone.hash) {
                                    best_move = entry.best_move;
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
        let mut state = ThreadState::new();
        let mut best_move = NULL_MOVE;
        let mut best_score = -INF;
        let mut prev_best_move = NULL_MOVE;
        let mut best_pv = Vec::new();
        let mut main_completed_depth = 0i32;

        // 5.x: Time management state
        let mut stability = 0i32; // consecutive iterations with same best move
        let mut prev_score = -INF; // score from previous iteration (for trend)
        let mut prev_prev_score = -INF; // score from 2 iterations ago
        // Node counts per root move for node-based TM
        let mut root_node_counts: std::collections::HashMap<(usize, usize), u64> =
            std::collections::HashMap::new();

        for depth in 1..=limits.max_depth {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }

            let nodes_before = state.nodes;

            // Aspiration windows after depth 5
            if depth >= 5 && best_score.abs() < MATE_SCORE - 100 {
                // 4.8: Eval-based aspiration narrowing (Viridithas)
                let mut delta = self.params.aspiration_window + best_score.abs() / 256;
                let mut alpha = best_score - delta;
                let mut beta = best_score + delta;

                loop {
                    let s = search_ab(
                        board,
                        &self.tt,
                        &self.stopped,
                        &mut state,
                        depth,
                        alpha,
                        beta,
                        0,
                        true,
                        limits.time_limit,
                        start_time,
                        true,
                        None,
                        0,
                        depth,
                        &self.params,
                        self.lmr_policy.as_ref(),
                    );
                    if self.stopped.load(Ordering::Relaxed) {
                        break;
                    }

                    if s <= alpha {
                        alpha = (s - delta).max(-INF);
                        delta *= 2;
                    } else if s >= beta {
                        beta = (s + delta).min(INF);
                        delta *= 2;
                    } else {
                        best_score = s;
                        break;
                    }

                    // Stockfish never falls back to full-width search.
                    // Just keep widening — the loop terminates naturally
                    // when the score falls within [alpha, beta].
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
                    &self.tt,
                    &self.stopped,
                    &mut state,
                    depth,
                    -INF,
                    INF,
                    0,
                    true,
                    limits.time_limit,
                    start_time,
                    true,
                    None,
                    0,
                    depth,
                    &self.params,
                    self.lmr_policy.as_ref(),
                );
                if self.stopped.load(Ordering::Relaxed) {
                    break;
                }
                best_score = s;
            }

            // Get best move from TT
            if let Some(entry) = self.tt.probe(board.hash) {
                prev_best_move = best_move;
                best_move = entry.best_move;
            }
            main_completed_depth = depth;

            // 5.1: Track nodes spent on this iteration's best move
            let nodes_this_iter = state.nodes - nodes_before;
            let key = (best_move.from.index(), best_move.to.index());
            *root_node_counts.entry(key).or_insert(0) += nodes_this_iter;

            // 5.2: Best-move stability
            if best_move != NULL_MOVE && prev_best_move != NULL_MOVE {
                if best_move.from == prev_best_move.from && best_move.to == prev_best_move.to {
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
                let score_str = if best_score.abs() > MATE_SCORE - 100 {
                    let mate_in = if best_score > 0 {
                        (MATE_SCORE - best_score + 1) / 2
                    } else {
                        -(MATE_SCORE + best_score + 1) / 2
                    };
                    format!("mate {mate_in}")
                } else {
                    format!("cp {best_score}")
                };

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
                    "info depth {depth} score {score_str} nodes {} nps {nps} time {elapsed_ms} pv {pv_str}",
                    state.nodes,
                );
                let _ = out.flush();
            }

            if best_score.abs() > MATE_SCORE - 100 {
                break;
            }

            // ── Smart time management ──
            if let Some(tl) = limits.time_limit {
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

                // Fail-low emergency: if best move changed this iteration, extend time
                if depth >= 6
                    && prev_best_move != NULL_MOVE
                    && best_move != NULL_MOVE
                    && (best_move.from != prev_best_move.from || best_move.to != prev_best_move.to)
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
            if let Some(choice) = outcomes.get(selected) {
                if choice.best_move != NULL_MOVE {
                    best_move = choice.best_move;
                    best_score = choice.score;
                }
            }
        }

        SearchResult {
            best_move,
            score: best_score,
            depth: main_completed_depth.max(0),
            nodes: total_nodes,
            elapsed: start_time.elapsed(),
            pv: best_pv,
        }
    }

    /// Convenience: search to a fixed depth.
    pub fn search_depth(&mut self, board: &mut Board, depth: i32) -> SearchResult {
        self.search(
            board,
            SearchLimits {
                max_depth: depth,
                time_limit: None,
                stopped: false,
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
                stopped: false,
            },
        )
    }

    /// Externally stop the search.
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Clears TT (for new game).
    pub fn clear(&mut self) {
        self.tt.clear();
    }
}

/// Alpha-beta search (free function so it can be called from any thread).
///
/// `excluded_move`: if Some, this move is skipped during the move loop.
/// Used for singular extension verification searches.
#[inline(never)]
fn search_ab(
    board: &mut Board,
    tt: &TranspositionTable,
    stopped: &AtomicBool,
    state: &mut ThreadState,
    mut depth: i32,
    mut alpha: i32,
    mut beta: i32,
    ply: i32,
    is_pv: bool,
    time_limit: Option<Duration>,
    start_time: Instant,
    is_root: bool,
    excluded_move: Option<Move>,
    total_extensions: i32,
    nominal_depth: i32,
    params: &SearchParams,
    lmr_policy: &dyn LmrPolicy,
) -> i32 {
    // Check stop periodically
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }
        if let Some(tl) = time_limit {
            if start_time.elapsed() >= tl {
                stopped.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    let ply_usize = (ply as usize).min(MAX_PLY - 1);

    // Initialize PV length for this ply
    state.pv_len[ply_usize] = 0;

    // Draw detection (repetition, 50-move, insufficient material)
    // Return 0 for draws — contempt should only be applied at root level,
    // not inside the tree where it poisons minimax scoring.
    if !is_root && board.is_draw() {
        return 0;
    }

    let in_check = board.in_check();

    // Hard ply limit — prevent unbounded search from extensions
    if ply >= MAX_PLY as i32 - 1 {
        return if in_check {
            0
        } else {
            hybrid_eval(board, &mut state.nnue_state)
        };
    }

    // Check extension — extend when in check, budgeted to prevent explosion.
    // KishMat's Lazy SMP creates more check-extension pressure than single-threaded.
    let check_ext_budget = nominal_depth * 2;
    if in_check && total_extensions < check_ext_budget {
        depth += 1;
    }

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
    let mut tt_depth = -1;
    let mut tt_node_type = NodeType::Exact;

    let mut tt_was_pv = is_pv; // 7.2: Track if this position was ever a PV node

    if excluded_move.is_none() {
        if let Some(entry) = tt.probe(board.hash) {
            let probed_score = score_from_tt(entry.score, ply);
            tt_move = Some(entry.best_move);
            tt_score = Some(probed_score);
            tt_depth = entry.depth;
            tt_node_type = entry.node_type;
            tt_was_pv = tt_was_pv || entry.was_pv; // Inherit PV flag from TT

            if !is_pv && entry.depth >= depth {
                match entry.node_type {
                    NodeType::Exact => return probed_score,
                    NodeType::LowerBound => {
                        if probed_score >= beta {
                            return probed_score;
                        }
                    }
                    NodeType::UpperBound => {
                        if probed_score <= alpha {
                            return probed_score;
                        }
                    }
                }
            }
        }
    }

    // Leaf → quiescence
    if depth <= 0 {
        return quiescence(
            board, tt, stopped, state, alpha, beta, ply, 0, time_limit, start_time,
        );
    }

    state.nodes += 1;
    let us = board.side_to_move;

    // Static eval — NNUE + correction history
    let raw_eval = hybrid_eval(board, &mut state.nnue_state);
    // Apply correction history adjustment
    let corr = state.correction(board);
    let mut static_eval = raw_eval + corr;

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
    state.static_evals[ply_usize] = static_eval;

    // "Improving" flag: is our static eval better than 2 plies ago?
    let improving =
        ply >= 2 && !in_check && static_eval > state.static_evals[(ply_usize).saturating_sub(2)];

    // ── Pruning techniques (non-PV, non-check, no excluded move) ─────

    if !is_pv && !in_check && excluded_move.is_none() {
        // 4.4: Reverse Futility Pruning with TT guards
        // Skip when TT was a PV node or TT move is a capture
        let tt_move_is_capture = tt_move.map_or(false, |m| m.is_capture());
        let rfp_tt_was_pv = tt_was_pv || (tt_node_type == NodeType::Exact);
        if depth <= 8 && !rfp_tt_was_pv && !tt_move_is_capture {
            let margin = params.rfp_margin(depth, improving);
            if static_eval - margin >= beta {
                // Return averaged score to avoid leaking raw static eval
                return (static_eval + beta) / 2;
            }
        }

        // Razoring — quadratic margin
        if depth <= 3 && static_eval <= alpha - params.razoring_margin(depth) {
            return quiescence(
                board, tt, stopped, state, alpha, beta, ply, 0, time_limit, start_time,
            );
        }

        // 4.5: Null move pruning with TT guards (Akimbo anti-recursion)
        // Don't NMP if TT says position is bad (fail-low at beta)
        let nmp_tt_ok = !tt_score.is_some_and(|s| tt_node_type == NodeType::UpperBound && s < beta);
        if depth >= 4
            && !board.is_endgame()
            && static_eval >= beta
            && nmp_tt_ok
            && ply_usize >= state.min_nmp_ply
        {
            let r = params
                .null_move_r(depth, static_eval, beta)
                .clamp(1, (depth - 2).max(1));

            board.make_null_move();
            state.prev_move[ply_usize] = NULL_MOVE;
            state.prev_piece[ply_usize] = 0;
            let score = -search_ab(
                board,
                tt,
                stopped,
                state,
                depth - 1 - r,
                -beta,
                -beta + 1,
                ply + 1,
                false,
                time_limit,
                start_time,
                false,
                None,
                total_extensions,
                nominal_depth,
                params,
                lmr_policy,
            );
            board.unmake_null_move();

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
                    tt,
                    stopped,
                    state,
                    depth - r,
                    beta - 1,
                    beta,
                    ply,
                    false,
                    time_limit,
                    start_time,
                    false,
                    None,
                    total_extensions,
                    nominal_depth,
                    params,
                    lmr_policy,
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

        // 4.6: ProbCut with eval guard — only when static_eval is high enough
        if depth >= 5 && beta.abs() < MATE_SCORE - 100 && static_eval >= beta - 200 {
            let pb_beta = beta + 200;
            // Generate captures, score them, and pick best incrementally (no full sort)
            let mut caps = board.generate_legal_captures();
            let mut cap_scores: Vec<i32> = (0..caps.len())
                .map(|i| capture_score(board, caps[i], tt_move))
                .collect();

            for idx in 0..caps.len() {
                // Find best remaining capture
                let mut best = idx;
                for j in (idx + 1)..caps.len() {
                    if cap_scores[j] > cap_scores[best] {
                        best = j;
                    }
                }
                caps.swap(idx, best);
                cap_scores.swap(idx, best);

                let mv = caps[idx];
                if !see::see_ge(board, mv, 0) {
                    continue;
                }

                let moved_piece = piece_index_on(board, mv.from);
                board.make_move(mv);
                state.prev_move[ply_usize] = mv;
                state.prev_piece[ply_usize] = moved_piece;
                // Reduced search
                let score = -search_ab(
                    board,
                    tt,
                    stopped,
                    state,
                    depth - 4,
                    -pb_beta,
                    -pb_beta + 1,
                    ply + 1,
                    false,
                    time_limit,
                    start_time,
                    false,
                    None,
                    total_extensions,
                    nominal_depth,
                    params,
                    lmr_policy,
                );
                board.unmake_move(mv);

                if stopped.load(Ordering::Relaxed) {
                    return 0;
                }
                if score >= pb_beta {
                    return score;
                }
            }
        }
    }

    // ── IIR ────────
    if tt_move.is_none() && depth >= 4 {
        // Internal Iterative Reduction
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
    let mut picker = MovePicker::new(tt_move, killers_copy, countermove);

    // Extract raw pointers to scoring data to avoid borrow conflicts.
    // Safety: These tables are only read by the closures during `picker.next()`
    // and only written after the call returns (in the loop body below).
    let cap_hist_ptr = &*state.cap_hist as *const [[[i32; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES];
    let history_ptr = &*state.history as *const [[[i32; 64]; 64]; 2];
    let cont_hist_ptr =
        &*state.cont_hist as *const [[[[i32; NUM_SQUARES]; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES];
    let cont_hist2_ptr =
        &*state.cont_hist2 as *const [[[[i32; NUM_SQUARES]; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES];
    let prev_piece_snap = state.prev_piece;
    let prev_move_snap = state.prev_move;
    let ply_snap = ply_usize;

    let score_capture = |b: &Board, mv: Move| -> i32 {
        let piece = b.piece_on(mv.from).map_or(0, |(p, _)| p.index());
        let victim = if let Some((p, _)) = b.piece_on(mv.to) {
            piece_value(p)
        } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
            100
        } else {
            0
        };
        let attacker = if let Some((p, _)) = b.piece_on(mv.from) {
            piece_value(p)
        } else {
            0
        };
        let cap_hist_score = if let Some((cap_p, _)) = b.piece_on(mv.to) {
            unsafe { (*cap_hist_ptr)[piece][mv.to.index()][cap_p.index()] }
        } else {
            0
        };
        victim * 10 - attacker + cap_hist_score / 16 + if mv.is_promotion() { 900 } else { 0 }
    };
    let score_quiet = |b: &Board, mv: Move| -> i32 {
        let piece = b.piece_on(mv.from).map_or(0, |(p, _)| p.index());
        let mut score = unsafe { (*history_ptr)[us_idx][mv.from.index()][mv.to.index()] };
        if ply_snap > 0 {
            let pp = prev_piece_snap[ply_snap.saturating_sub(1)];
            let pt = prev_move_snap[ply_snap.saturating_sub(1)].to.index();
            score += unsafe { (*cont_hist_ptr)[pp][pt][piece][mv.to.index()] };
        }
        if ply_snap > 1 {
            let pp2 = prev_piece_snap[ply_snap.saturating_sub(2)];
            let pt2 = prev_move_snap[ply_snap.saturating_sub(2)].to.index();
            score += unsafe { (*cont_hist2_ptr)[pp2][pt2][piece][mv.to.index()] };
        }
        score
    };

    // Check for no legal moves (checkmate / stalemate)
    picker.ensure_legal_moves(board);
    if picker.total_legal() == 0 {
        return if in_check { -MATE_SCORE + ply } else { 0 };
    }

    let mut best_move = NULL_MOVE;
    let mut best_score = -INF;
    let mut node_type = NodeType::UpperBound;
    let mut moves_searched = 0;

    // Track searched quiet moves for history malus (Stockfish-style)
    let mut searched_quiets: Vec<Move> = Vec::with_capacity(32);
    // Track searched captures for capture history malus
    let mut searched_captures: Vec<Move> = Vec::with_capacity(16);

    // Singular extension data
    let can_do_singular = excluded_move.is_none()
        && !is_root
        && depth >= params.se_depth_min
        && tt_move.is_some()
        && tt_depth >= depth - 3
        && tt_node_type != NodeType::UpperBound
        && tt_score.is_some_and(|s| s.abs() < MATE_SCORE - 100);

    while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
        // Skip the excluded move (for singular extension verification)
        if excluded_move
            .is_some_and(|em| em.from == mv.from && em.to == mv.to && em.promotion == mv.promotion)
        {
            continue;
        }

        // ── Singular extension (with double + negative extensions) ──────
        let mut extension = 0;
        if can_do_singular {
            if let Some(ttm) = tt_move {
                if mv.from == ttm.from && mv.to == ttm.to {
                    if let Some(tt_sc) = tt_score {
                        let se_beta = tt_sc - params.se_margin(depth);
                        let se_score = search_ab(
                            board,
                            tt,
                            stopped,
                            state,
                            (depth - 1) / 2,
                            se_beta - 1,
                            se_beta,
                            ply,
                            false,
                            time_limit,
                            start_time,
                            false,
                            Some(mv),
                            total_extensions,
                            nominal_depth,
                            params,
                            lmr_policy,
                        );
                        if stopped.load(Ordering::Relaxed) {
                            return 0;
                        }
                        if se_score < se_beta {
                            extension = 1; // TT move is singular — extend
                            // Double extension — per-path counter (Akimbo: dbl_exts < 5)
                            if !is_pv
                                && se_score < se_beta - 25
                                && state.dbl_exts[ply_usize] < params.max_dbl_exts
                            {
                                state.dbl_exts[ply_usize] += 1;
                                extension = 2;
                            }
                        } else if tt_sc >= beta {
                            // Negative extension — TT move isn't singular
                            extension = -1;
                        }
                    }
                }
            }
        }

        // Hindsight extension removed — the condition `static_eval + eval_2_back < 0`
        // fires on nearly every node in disadvantageous positions, causing unbounded
        // search tree growth that makes the engine hang at higher depths.

        // Get moved piece index for this move
        let moved_piece = piece_index_on(board, mv.from);

        // Compute stat_score for quiet moves
        let mv_stat_score = if !mv.is_capture() {
            state.stat_score(mv, us.index(), moved_piece, ply_usize)
        } else {
            0
        };

        // Late Move Pruning — Stockfish formula: (3 + depth²) / (2 - improving)
        if !is_pv
            && !in_check
            && depth <= params.lmp_depth_limit
            && moves_searched >= params.lmp_threshold(depth, improving)
            && !mv.is_capture()
            && !mv.is_promotion()
            && best_score > -MATE_SCORE + 100
        {
            picker.skip_quiets();
            continue;
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
        if !is_pv
            && !in_check
            && depth <= params.futility_depth_limit
            && !mv.is_capture()
            && !mv.is_promotion()
            && moves_searched > 0
            && static_eval + params.futility_margin(depth, improving) <= alpha
            && best_score > -MATE_SCORE + 100
        {
            continue;
        }

        // SEE pruning for losing captures and quiet moves at low depth
        // Margins adjusted by stat_score for quiets
        if depth <= params.see_prune_depth_limit
            && !is_pv
            && moves_searched > 0
            && best_score > -MATE_SCORE + 100
        {
            if mv.is_capture() && !see::see_ge(board, mv, params.see_capture_margin * depth) {
                continue;
            }
            let see_quiet_margin = params.see_quiet_margin * depth + mv_stat_score / 300;
            if !mv.is_capture() && !mv.is_promotion() && !see::see_ge(board, mv, see_quiet_margin) {
                continue;
            }
        }

        let captured_piece_idx = if mv.is_capture() {
            board.piece_on(mv.to).map(|(p, _)| p.index())
        } else {
            None
        };
        let is_losing_capture = mv.is_capture() && !see::see_ge(board, mv, 0);

        board.make_move(mv);

        // Store the move we're searching for countermove/continuation tracking
        state.prev_move[ply_usize] = mv;
        state.prev_piece[ply_usize] = moved_piece;

        // Prefetch TT for the position we're about to search
        tt.prefetch(board.hash);

        let score;
        let gives_check = board.in_check();
        // Cap extensions at remaining depth to prevent going negative
        let extension = extension.min(depth.max(0));
        let new_total_extensions = total_extensions + extension.max(0);
        let effective_depth = depth - 1 + extension;

        if moves_searched == 0 {
            // Full window search for the first move
            score = -search_ab(
                board,
                tt,
                stopped,
                state,
                effective_depth,
                -beta,
                -alpha,
                ply + 1,
                is_pv,
                time_limit,
                start_time,
                false,
                None,
                new_total_extensions,
                nominal_depth,
                params,
                lmr_policy,
            );
        } else {
            // LMR: Late Move Reductions — enhanced with stat_score
            let mut reduction = 0;
            if moves_searched >= 2
                && depth >= 3
                && (!mv.is_capture() || is_losing_capture)
                && !mv.is_promotion()
            {
                let d = (depth as usize).min(127);
                let m = moves_searched.min(127);
                let base = lmr_table()[d][m];
                let cap_hist_score = captured_piece_idx
                    .map(|idx| state.cap_hist[moved_piece][mv.to.index()][idx])
                    .unwrap_or(0);
                let lmr_ctx = LmrContext {
                    is_capture: mv.is_capture(),
                    is_losing_capture,
                    is_pv,
                    improving,
                    is_killer: is_killer(mv, &state.killers[ply_usize]),
                    gives_check,
                    mv_stat_score,
                    cap_hist_score,
                    corr_abs: corr.abs(),
                    is_cut_node: !is_pv && node_type == NodeType::UpperBound,
                    tt_was_pv,
                    hist_lmr_div: params.hist_lmr_div,
                    lmr_corr_mul: params.lmr_corr_mul,
                    lmr_cut_node_bonus: params.lmr_cut_node_bonus,
                };
                reduction = lmr_policy.adjust_reduction(base, &lmr_ctx);
                reduction = reduction.clamp(0, effective_depth - 1);
            }

            // PVS null-window search with reduction
            let mut s = -search_ab(
                board,
                tt,
                stopped,
                state,
                effective_depth - reduction,
                -alpha - 1,
                -alpha,
                ply + 1,
                false,
                time_limit,
                start_time,
                false,
                None,
                new_total_extensions,
                nominal_depth,
                params,
                lmr_policy,
            );

            // Re-search if reduced search fails high
            if s > alpha && reduction > 0 {
                s = -search_ab(
                    board,
                    tt,
                    stopped,
                    state,
                    effective_depth,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    false,
                    time_limit,
                    start_time,
                    false,
                    None,
                    new_total_extensions,
                    nominal_depth,
                    params,
                    lmr_policy,
                );
            }
            // Re-search with full window in PV
            if s > alpha && s < beta {
                s = -search_ab(
                    board,
                    tt,
                    stopped,
                    state,
                    effective_depth,
                    -beta,
                    -alpha,
                    ply + 1,
                    true,
                    time_limit,
                    start_time,
                    false,
                    None,
                    new_total_extensions,
                    nominal_depth,
                    params,
                    lmr_policy,
                );
            }
            score = s;
        }

        board.unmake_move(mv);
        moves_searched += 1;

        if stopped.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
            best_move = mv;

            if score > alpha {
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
                    // Stockfish-style linear bonus: 300 * depth - 300
                    // Better scaling than quadratic — prevents all deep nodes
                    // from getting the same capped bonus.
                    let bonus = (300 * depth - 300).clamp(0, 2000);
                    let ci = us.index();

                    if !mv.is_capture() {
                        // Update heuristics for quiet moves causing beta cutoff
                        store_killer(&mut state.killers, mv, ply_usize);

                        // Main history
                        update_history(
                            &mut state.history[ci][mv.from.index()][mv.to.index()],
                            bonus,
                        );

                        // Continuation history (1-ply back)
                        if ply > 0 {
                            let pp = state.prev_piece[ply_usize.saturating_sub(1)];
                            let pt = state.prev_move[ply_usize.saturating_sub(1)].to.index();
                            update_history(
                                &mut state.cont_hist[pp][pt][moved_piece][mv.to.index()],
                                bonus,
                            );
                        }
                        // Continuation history (2-ply back)
                        if ply > 1 {
                            let pp2 = state.prev_piece[ply_usize.saturating_sub(2)];
                            let pt2 = state.prev_move[ply_usize.saturating_sub(2)].to.index();
                            update_history(
                                &mut state.cont_hist2[pp2][pt2][moved_piece][mv.to.index()],
                                bonus,
                            );
                        }

                        // Penalize quiet moves searched before the cutoff
                        for prev in &searched_quiets {
                            if excluded_move
                                .is_some_and(|em| em.from == prev.from && em.to == prev.to)
                            {
                                continue;
                            }
                            let prev_piece_idx = piece_index_on(board, prev.from);
                            update_history(
                                &mut state.history[ci][prev.from.index()][prev.to.index()],
                                -bonus,
                            );
                            // Penalize in continuation history too
                            if ply > 0 {
                                let pp = state.prev_piece[ply_usize.saturating_sub(1)];
                                let pt = state.prev_move[ply_usize.saturating_sub(1)].to.index();
                                update_history(
                                    &mut state.cont_hist[pp][pt][prev_piece_idx][prev.to.index()],
                                    -bonus,
                                );
                            }
                        }

                        // Countermove: index by PREVIOUS move
                        if prev_mv != NULL_MOVE {
                            state.countermoves[prev_mv.from.index()][prev_mv.to.index()] = mv;
                        }
                    } else {
                        // Capture history update for captures causing cutoff
                        if let Some((cap_piece, _)) = board.piece_on(mv.to) {
                            update_history(
                                &mut state.cap_hist[moved_piece][mv.to.index()][cap_piece.index()],
                                bonus,
                            );
                        }
                        // Capture history malus: penalize captures that were searched before the cutoff capture
                        for prev_cap in &searched_captures {
                            let prev_piece_idx = piece_index_on(board, prev_cap.from);
                            if let Some((prev_cap_piece, _)) = board.piece_on(prev_cap.to) {
                                update_history(
                                    &mut state.cap_hist[prev_piece_idx][prev_cap.to.index()]
                                        [prev_cap_piece.index()],
                                    -bonus,
                                );
                            }
                        }
                    }
                    if excluded_move.is_none() {
                        tt.store(
                            board.hash,
                            depth,
                            score_to_tt(score, ply),
                            NodeType::LowerBound,
                            best_move,
                            tt_was_pv,
                        );
                    }
                    return best_score;
                }
            }
        }

        // Track searched quiet moves for history malus
        if !mv.is_capture() && !mv.is_promotion() {
            searched_quiets.push(mv);
        }
        // Track searched captures for capture history malus
        if mv.is_capture() {
            searched_captures.push(mv);
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
        return if in_check { -MATE_SCORE + ply } else { 0 };
    }

    if excluded_move.is_none() {
        tt.store(
            board.hash,
            depth,
            score_to_tt(best_score, ply),
            node_type,
            best_move,
            tt_was_pv,
        );
        // Update correction history with search outcome vs static eval
        state.update_correction(board, depth, best_score, static_eval);
    }
    best_score
}

/// Quiescence search — stabilize the evaluation at leaf nodes.
/// Handles check evasions, captures, and promotions.
/// Uses fail-soft and the hybrid eval (classical + NNUE) for consistency.
#[inline(never)]
fn quiescence(
    board: &mut Board,
    tt: &TranspositionTable,
    stopped: &AtomicBool,
    state: &mut ThreadState,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    qs_ply: i32,
    time_limit: Option<Duration>,
    start_time: Instant,
) -> i32 {
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }
        if let Some(tl) = time_limit {
            if start_time.elapsed() >= tl {
                stopped.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    state.nodes += 1;
    let in_check = board.in_check();

    // Hard depth limit — prevents stack overflow and infinite check chains.
    // This MUST apply even when in check (check evasions can chain indefinitely).
    if ply >= MAX_PLY as i32 - 1 {
        return hybrid_eval(board, &mut state.nnue_state);
    }
    if qs_ply >= MAX_QS_PLY {
        return hybrid_eval(board, &mut state.nnue_state);
    }

    // TT probe in qsearch (important for stability!)
    let mut qs_tt_move = None;
    if let Some(entry) = tt.probe(board.hash) {
        let probed_score = score_from_tt(entry.score, ply);
        qs_tt_move = Some(entry.best_move);
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
    }

    // When in check, search ALL moves (not just captures) to find escape
    // Score them by TT move priority + capture value for better ordering.
    if in_check {
        let mut moves = board.generate_legal_moves();
        if moves.is_empty() {
            return -MATE_SCORE + ply;
        }

        // Score check evasions: TT move first, then captures by MVV-LVA, then quiets
        let mut move_scores = Vec::with_capacity(moves.len());
        for i in 0..moves.len() {
            let mv = moves[i];
            let score = if qs_tt_move.is_some_and(|ttm| mv.from == ttm.from && mv.to == ttm.to) {
                1_000_000 // TT move first
            } else if mv.is_capture() {
                // MVV-LVA
                let victim = if let Some((p, _)) = board.piece_on(mv.to) {
                    piece_value(p)
                } else {
                    0
                };
                let attacker = if let Some((p, _)) = board.piece_on(mv.from) {
                    piece_value(p)
                } else {
                    0
                };
                100_000 + victim * 10 - attacker
            } else if mv.is_promotion() {
                200_000
            } else {
                0
            };
            move_scores.push(score);
        }

        let mut best_score = -INF;
        let mut best_move = moves[0];
        let alpha_orig = alpha;
        for idx in 0..moves.len() {
            // Incremental selection sort: find best remaining
            let mut best_idx = idx;
            for j in (idx + 1)..moves.len() {
                if move_scores[j] > move_scores[best_idx] {
                    best_idx = j;
                }
            }
            moves.swap(idx, best_idx);
            move_scores.swap(idx, best_idx);

            let mv = moves[idx];
            board.make_move(mv);
            let score = -quiescence(
                board,
                tt,
                stopped,
                state,
                -beta,
                -alpha,
                ply + 1,
                qs_ply + 1,
                time_limit,
                start_time,
            );
            board.unmake_move(mv);
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
                    board.hash,
                    0,
                    score_to_tt(best_score, ply),
                    NodeType::LowerBound,
                    best_move,
                    false,
                );
                return best_score;
            }
        }
        let nt = if best_score > alpha_orig {
            NodeType::Exact
        } else {
            NodeType::UpperBound
        };
        tt.store(
            board.hash,
            0,
            score_to_tt(best_score, ply),
            nt,
            best_move,
            false,
        );
        return best_score;
    }

    // ── Fail-soft stand-pat using hybrid eval + correction history ──
    let raw_eval = hybrid_eval(board, &mut state.nnue_state);
    let corr = state.correction(board);
    let mut stand_pat = raw_eval + corr;

    // TT score adjustment in QS (Akimbo technique):
    // Use TT score to refine stand-pat when TT bound agrees with direction.
    if let Some(entry) = tt.probe(board.hash) {
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
        return best_score;
    }

    // Big delta pruning: if we're hopelessly behind, give up
    if stand_pat + DELTA_MARGIN + 1100 < alpha {
        return best_score;
    }
    if stand_pat > alpha {
        alpha = stand_pat;
    }

    let mut captures = board.generate_legal_captures();
    // Note: generate_legal_captures() already includes non-capture promotions
    // via gen_pawn_captures() — no need to generate all legal moves.

    // Order captures by MVV-LVA + capture history for qsearch
    order_captures_with_history(board, &mut captures, qs_tt_move, &state.cap_hist);

    if captures.is_empty() {
        return best_score;
    }

    let mut best_move = captures[0];
    let alpha_orig = alpha;

    for i in 0..captures.len() {
        let mv = captures[i];

        // Per-capture delta pruning (skip for promotions)
        if mv.is_capture() {
            let cv = estimate_capture_value(board, mv);
            if stand_pat + cv + DELTA_MARGIN < alpha {
                continue;
            }

            // SEE pruning: skip losing captures
            if !see::see_ge(board, mv, 0) {
                continue;
            }
        }

        board.make_move(mv);
        let score = -quiescence(
            board,
            tt,
            stopped,
            state,
            -beta,
            -alpha,
            ply + 1,
            qs_ply + 1,
            time_limit,
            start_time,
        );
        board.unmake_move(mv);

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
                board.hash,
                0,
                score_to_tt(best_score, ply),
                NodeType::LowerBound,
                best_move,
                false,
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
        board.hash,
        0,
        score_to_tt(best_score, ply),
        nt,
        best_move,
        false,
    );

    best_score
}

#[inline(always)]
fn estimate_capture_value(board: &Board, mv: Move) -> i32 {
    if mv.is_promotion() {
        return 900;
    }
    if let Some((piece, _)) = board.piece_on(mv.to) {
        piece_value(piece)
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        100
    } else {
        0
    }
}

/// Order captures by MVV-LVA + capture history (for qsearch).
/// Uses capture history to break ties and improve move ordering quality.
#[inline]
fn order_captures_with_history(
    board: &Board,
    moves: &mut MoveList,
    tt_move: Option<Move>,
    cap_hist: &[[[i32; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES],
) {
    if moves.len() <= 1 {
        return;
    }
    moves.sort_by(|a, b| {
        let sa = capture_score_with_history(board, *a, tt_move, cap_hist);
        let sb = capture_score_with_history(board, *b, tt_move, cap_hist);
        sb.cmp(&sa)
    });
}

#[inline(always)]
fn capture_score_with_history(
    board: &Board,
    mv: Move,
    tt_move: Option<Move>,
    cap_hist: &[[[i32; NUM_PIECES]; NUM_SQUARES]; NUM_PIECES],
) -> i32 {
    if let Some(ttm) = tt_move {
        if mv.from == ttm.from && mv.to == ttm.to {
            return 10_000_000;
        }
    }
    let victim_val = if let Some((p, _)) = board.piece_on(mv.to) {
        piece_value(p)
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        100
    } else {
        0
    };
    let attacker_val = if let Some((p, _)) = board.piece_on(mv.from) {
        piece_value(p)
    } else {
        0
    };
    let base = victim_val * 10 - attacker_val + if mv.is_promotion() { 900 } else { 0 };

    // Add capture history score — moved piece × to square × captured piece
    let moved_piece = piece_index_on(board, mv.from);
    let ch_score = if let Some((cap_p, _)) = board.piece_on(mv.to) {
        cap_hist[moved_piece][mv.to.index()][cap_p.index()]
    } else {
        0
    };

    base + ch_score / 32
}

#[inline(always)]
fn capture_score(board: &Board, mv: Move, tt_move: Option<Move>) -> i32 {
    if let Some(ttm) = tt_move {
        if mv.from == ttm.from && mv.to == ttm.to {
            return 10_000_000;
        }
    }
    let victim = if let Some((p, _)) = board.piece_on(mv.to) {
        piece_value(p)
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        100
    } else {
        0
    };
    let attacker = if let Some((p, _)) = board.piece_on(mv.from) {
        piece_value(p)
    } else {
        0
    };
    victim * 10 - attacker + if mv.is_promotion() { 900 } else { 0 }
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

    fn setup() {
        types::init();
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
                assert!(engine.tt.probe(board.hash).is_some());
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
                assert!(engine.tt.probe(board.hash).is_some());
                engine.clear();
                assert!(engine.tt.probe(board.hash).is_none());
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
}
