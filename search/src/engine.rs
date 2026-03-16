//! The search engine: iterative deepening with alpha-beta, quiescence search,
//! null-move pruning, late-move reductions, PVS, aspiration windows,
//! killer moves, history heuristic, countermove heuristic, LMP,
//! check extensions, singular extensions, razoring, ProbCut,
//! IID, SEE-based pruning, futility/delta pruning.
//! Supports Lazy SMP multi-threaded search via shared transposition table.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use types::{Board, Move, MoveList, Piece};
use types::chess_move::NULL_MOVE;
use crate::tt::{TranspositionTable, NodeType};
use crate::see;
use crate::nnue::{self, NNUEState};

/// Infinity score sentinel.
const INF: i32 = 30_000;
/// Checkmate score base (mate in N = MATE_SCORE - N).
const MATE_SCORE: i32 = 29_000;
/// Maximum search ply depth.
const MAX_PLY: usize = 128;
/// Aspiration window initial width — Stockfish uses 10cp.
const ASPIRATION_WINDOW: i32 = 10;
/// Delta pruning margin in quiescence — standard queen value.
const DELTA_MARGIN: i32 = 200;
/// Maximum history score (Stockfish uses 16384).
const MAX_HISTORY: i32 = 16384;

/// Reverse futility pruning margin — Stockfish: `futilityMult * depth`.
/// futilityMult = 77 (adjusted for TT hit).
#[inline(always)]
fn rfp_margin(depth: i32, improving: bool) -> i32 {
    77 * depth - if improving { 74 } else { 0 }
}

/// Razoring margin — Stockfish: `507 + 312 * depth * depth`.
#[inline(always)]
fn razoring_margin(depth: i32) -> i32 {
    507 + 312 * depth * depth
}

/// Futility margin — Stockfish: `77 * depth` with improving correction.
#[inline(always)]
fn futility_margin(depth: i32, improving: bool) -> i32 {
    77 * depth - if improving { 46 } else { 0 }
}

/// Singular extension margin — Stockfish: `3 * depth`.
#[inline(always)]
fn se_margin(depth: i32) -> i32 {
    3 * depth
}

/// Late Move Pruning threshold — Stockfish formula: `(3 + depth²) / (2 - improving)`.
#[inline(always)]
fn lmp_threshold(depth: i32, improving: bool) -> usize {
    ((3 + depth * depth) / if improving { 1 } else { 2 }) as usize
}

/// Null-move reduction — Stockfish: `5 + depth/5`.
#[inline(always)]
fn null_move_r(depth: i32, eval: i32, beta: i32) -> i32 {
    5 + depth / 5 + ((eval - beta) / 200).min(3)
}

/// Precomputed LMR reduction table — Stockfish formula.
static LMR_TABLE: std::sync::OnceLock<[[i32; 64]; 64]> = std::sync::OnceLock::new();

#[inline(always)]
fn lmr_table() -> &'static [[i32; 64]; 64] {
    LMR_TABLE.get_or_init(|| {
        let mut table = [[0i32; 64]; 64];
        for depth in 1..64 {
            for moves in 1..64 {
                // Stockfish: 0.77 + ln(depth) * ln(moveCount) / 2.36
                table[depth][moves] = (0.77 + (depth as f64).ln() * (moves as f64).ln() / 2.36) as i32;
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

/// Search result returned to the caller.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best_move: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub elapsed: Duration,
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
        Self { max_depth: 64, time_limit: None, stopped: false }
    }
}

/// Per-thread search state. Each Lazy SMP worker has its own.
struct ThreadState {
    nodes: u64,
    killers: [[Move; 2]; MAX_PLY],
    history: [[[i32; 64]; 64]; 2],
    countermoves: [[Move; 64]; 64],
    /// Static eval at each ply (for improving detection).
    static_evals: [i32; MAX_PLY],
    /// NNUE evaluation state.
    nnue_state: NNUEState,
}

impl ThreadState {
    fn new() -> Self {
        Self {
            nodes: 0,
            killers: [[NULL_MOVE; 2]; MAX_PLY],
            history: [[[0i32; 64]; 64]; 2],
            countermoves: [[NULL_MOVE; 64]; 64],
            static_evals: [0; MAX_PLY],
            nnue_state: NNUEState::new(),
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
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn clear(&mut self) {
        self.killers = [[NULL_MOVE; 2]; MAX_PLY];
        self.history = [[[0; 64]; 64]; 2];
        self.countermoves = [[NULL_MOVE; 64]; 64];
    }
}

/// The search engine, owning a shared TT and managing Lazy SMP threads.
pub struct SearchEngine {
    pub tt: Arc<TranspositionTable>,
    pub num_threads: usize,
    stopped: Arc<AtomicBool>,
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
        // Initialize NNUE
        nnue::network::init_nnue();
        Self {
            tt: Arc::new(TranspositionTable::new(tt_size_mb)),
            num_threads: num_threads.max(1),
            stopped: Arc::new(AtomicBool::new(false)),
        }
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

            handles.push(std::thread::Builder::new()
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let mut state = ThreadState::new();
                    // Helper threads search with depth offsets for diversity
                    let depth_offset = match thread_id % 4 {
                        1 => 1,
                        2 => -1i32,
                        3 => 2,
                        _ => 0,
                    };

                    for depth in 1..=max_depth {
                        let actual_depth = (depth as i32 + depth_offset).max(1).min(max_depth);

                        // Check time/stop
                        if stopped.load(Ordering::Relaxed) { break; }
                        if let Some(tl) = time_limit {
                            if start.elapsed() >= tl { break; }
                        }

                        let _ = search_ab(
                            &mut board_clone, &tt, &stopped, &mut state,
                            actual_depth, -INF, INF, 0, true,
                            time_limit, start, true,
                        );
                    }
                    state.nodes
                }).unwrap());
        }

        // Main thread search with reporting
        let mut state = ThreadState::new();
        let mut best_move = NULL_MOVE;
        let mut best_score = -INF;
        let mut prev_best_move = NULL_MOVE;

        for depth in 1..=limits.max_depth {
            if self.stopped.load(Ordering::Relaxed) { break; }

            // Aspiration windows after depth 5
            if depth >= 5 && best_score.abs() < MATE_SCORE - 100 {
                let mut delta = ASPIRATION_WINDOW;
                let mut alpha = best_score - delta;
                let mut beta = best_score + delta;

                loop {
                    let s = search_ab(
                        board, &self.tt, &self.stopped, &mut state,
                        depth, alpha, beta, 0, true,
                        limits.time_limit, start_time, true,
                    );
                    if self.stopped.load(Ordering::Relaxed) { break; }

                    if s <= alpha {
                        alpha = (s - delta).max(-INF);
                        beta = (s + delta).min(INF); // Widen both sides slightly
                        delta *= 2;
                    } else if s >= beta {
                        beta = (s + delta).min(INF);
                        delta *= 2;
                    } else {
                        best_score = s;
                        break;
                    }

                    if delta > 500 {
                        let s = search_ab(
                            board, &self.tt, &self.stopped, &mut state,
                            depth, -INF, INF, 0, true,
                            limits.time_limit, start_time, true,
                        );
                        if !self.stopped.load(Ordering::Relaxed) { best_score = s; }
                        break;
                    }
                }
                if self.stopped.load(Ordering::Relaxed) { break; }
            } else {
                let s = search_ab(
                    board, &self.tt, &self.stopped, &mut state,
                    depth, -INF, INF, 0, true,
                    limits.time_limit, start_time, true,
                );
                if self.stopped.load(Ordering::Relaxed) { break; }
                best_score = s;
            }

            // Get best move from TT
            if let Some(entry) = self.tt.probe(board.hash) {
                prev_best_move = best_move;
                best_move = entry.best_move;
            }

            let elapsed = start_time.elapsed();
            let elapsed_ms = elapsed.as_millis().max(1) as u64;
            let nps = state.nodes * 1000 / elapsed_ms;

            // Report info
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
                let _ = writeln!(
                    out,
                    "info depth {depth} score {score_str} nodes {} nps {nps} time {elapsed_ms} pv {best_move}",
                    state.nodes,
                );
                let _ = out.flush();
            }

            if best_score.abs() > MATE_SCORE - 100 { break; }

            // Check time — use soft limit (half of hard limit)
            if let Some(tl) = limits.time_limit {
                let elapsed_now = start_time.elapsed();
                // Soft time: stop if we've used more than half of our time
                // Unless best move is unstable (changed from last iteration)
                let soft_limit = if prev_best_move != NULL_MOVE && best_move != prev_best_move {
                    // Best move changed — use more time
                    tl.mul_f64(0.75)
                } else {
                    tl.mul_f64(0.5)
                };

                if elapsed_now >= soft_limit {
                    self.stopped.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        // Stop helper threads
        self.stopped.store(true, Ordering::SeqCst);

        // Collect helper thread nodes
        let mut total_nodes = state.nodes;
        for h in handles {
            if let Ok(helper_nodes) = h.join() {
                total_nodes += helper_nodes;
            }
        }

        SearchResult {
            best_move,
            score: best_score,
            depth: limits.max_depth,
            nodes: total_nodes,
            elapsed: start_time.elapsed(),
        }
    }

    /// Convenience: search to a fixed depth.
    pub fn search_depth(&mut self, board: &mut Board, depth: i32) -> SearchResult {
        self.search(board, SearchLimits { max_depth: depth, time_limit: None, stopped: false })
    }

    /// Convenience: search with a time limit.
    pub fn search_time(&mut self, board: &mut Board, time: Duration, max_depth: i32) -> SearchResult {
        self.search(board, SearchLimits { max_depth, time_limit: Some(time), stopped: false })
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
#[inline(never)]
fn search_ab(
    board: &mut Board,
    tt: &TranspositionTable,
    stopped: &AtomicBool,
    state: &mut ThreadState,
    mut depth: i32,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    is_pv: bool,
    time_limit: Option<Duration>,
    start_time: Instant,
    is_root: bool,
) -> i32 {
    // Check stop periodically
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) { return 0; }
        if let Some(tl) = time_limit {
            if start_time.elapsed() >= tl {
                stopped.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    // Draw detection (repetition, 50-move, insufficient material)
    if !is_root && board.is_draw() { return 0; }

    let in_check = board.in_check();

    // Check extension — always extend when in check
    if in_check { depth += 1; }

    // TT probe
    let mut tt_move = None;
    let mut tt_score = None;
    let mut tt_depth = -1;
    let mut tt_node_type = NodeType::Exact;

    if let Some(entry) = tt.probe(board.hash) {
        tt_move = Some(entry.best_move);
        tt_score = Some(entry.score);
        tt_depth = entry.depth;
        tt_node_type = entry.node_type;

        if !is_pv && entry.depth >= depth {
            match entry.node_type {
                NodeType::Exact => return entry.score,
                NodeType::LowerBound => {
                    if entry.score >= beta { return entry.score; }
                }
                NodeType::UpperBound => {
                    if entry.score <= alpha { return entry.score; }
                }
            }
        }
    }

    // Leaf → quiescence
    if depth <= 0 {
        return quiescence(board, tt, stopped, state, alpha, beta, ply, time_limit, start_time);
    }

    state.nodes += 1;
    let ply_usize = (ply as usize).min(MAX_PLY - 1);
    let us = board.side_to_move;

    // Static eval — KishMat hybrid: classical eval + NNCorrL correction
    let static_eval = {
        let classical = eval::evaluate(board);
        if nnue::network::is_nnue_ready() {
            let correction = nnue::evaluate_nnue(board, &mut state.nnue_state);
            classical + correction
        } else {
            classical
        }
    };
    state.static_evals[ply_usize] = static_eval;

    // "Improving" flag: is our static eval better than 2 plies ago?
    let improving = ply >= 2
        && !in_check
        && static_eval > state.static_evals[(ply_usize).saturating_sub(2)];

    // ── Pruning techniques (non-PV, non-check) ─────────────────────────

    if !is_pv && !in_check {
        // Reverse Futility Pruning — Stockfish: static_eval - rfp_margin >= beta
        if depth <= 8 {
            let margin = rfp_margin(depth, improving);
            if static_eval - margin >= beta {
                return static_eval;
            }
        }

        // Razoring — Stockfish: alpha - 507 - 312 * depth² (quadratic)
        if depth <= 3 && static_eval <= alpha - razoring_margin(depth) {
            return quiescence(board, tt, stopped, state, alpha, beta, ply, time_limit, start_time);
        }

        // Null move pruning — Stockfish: R = 5 + depth/5 + eval correction
        if depth > 2 && !board.is_endgame() && static_eval >= beta {
            let r = null_move_r(depth, static_eval, beta);

            board.make_null_move();
            let score = -search_ab(board, tt, stopped, state, depth - 1 - r, -beta, -beta + 1, ply + 1, false, time_limit, start_time, false);
            board.unmake_null_move();

            if stopped.load(Ordering::Relaxed) { return 0; }

            if score >= beta {
                // Verification search at higher depths to avoid zugzwang issues
                if depth > 12 {
                    let v_score = search_ab(board, tt, stopped, state, depth - 7, beta - 1, beta, ply, false, time_limit, start_time, false);
                    if stopped.load(Ordering::Relaxed) { return 0; }
                    if v_score >= beta {
                        return beta;
                    }
                } else {
                    return beta;
                }
            }
        }

        // ProbCut: at medium depth, if reduced-depth search finds score way above beta, prune
        if depth >= 5 && beta.abs() < MATE_SCORE - 100 {
            let pb_beta = beta + 200;
            // Try captures first
            let mut caps = board.generate_legal_captures();
            order_captures(board, &mut caps, tt_move);

            for i in 0..caps.len() {
                let mv = caps[i];
                if !see::see_ge(board, mv, 0) { continue; }

                board.make_move(mv);
                // Reduced search
                let score = -search_ab(board, tt, stopped, state, depth - 4, -pb_beta, -pb_beta + 1, ply + 1, false, time_limit, start_time, false);
                board.unmake_move(mv);

                if stopped.load(Ordering::Relaxed) { return 0; }
                if score >= pb_beta { return score; }
            }
        }
    }

    // Internal Iterative Deepening (IID) — if no TT move in PV node
    let tt_move = if tt_move.is_none() && is_pv && depth >= 4 {
        let _ = search_ab(board, tt, stopped, state, depth - 2, alpha, beta, ply, false, time_limit, start_time, false);
        if stopped.load(Ordering::Relaxed) { return 0; }
        tt.probe(board.hash).map(|e| e.best_move)
    } else {
        tt_move
    };

    let mut moves = board.generate_legal_moves();

    if moves.is_empty() {
        return if in_check { -MATE_SCORE + ply } else { 0 };
    }

    // Move ordering: score all moves, then use pick-best during iteration
    let move_scores = score_moves(board, &moves, tt_move, &state.killers[ply_usize], &state.history[us.index()]);

    // TT prefetch for first move
    if !moves.is_empty() {
        let best_idx = pick_best_index(&move_scores, 0);
        let first_mv = moves[best_idx];
        board.make_move(first_mv);
        tt.prefetch(board.hash);
        board.unmake_move(first_mv);
    }

    let mut best_move = moves[0];
    let mut best_score = -INF;
    let mut node_type = NodeType::UpperBound;
    let mut moves_searched = 0;
    let mut move_scores = move_scores;

    // Singular extension data
    let can_do_singular = !is_root && depth >= 8
        && tt_move.is_some()
        && tt_depth >= depth - 3
        && tt_node_type != NodeType::UpperBound
        && tt_score.map_or(false, |s| s.abs() < MATE_SCORE - 100);

    for i in 0..moves.len() {
        // Pick best move from remaining (incremental sort)
        let best_idx = pick_best_index(&move_scores, i);
        moves.swap(i, best_idx);
        move_scores.swap(i, best_idx);

        let mv = moves[i];

        // Singular extension: if TT move is significantly better than alternatives
        let mut extension = 0;
        if can_do_singular && i == 0 {
            if let Some(ttm) = tt_move {
                if mv.from == ttm.from && mv.to == ttm.to {
                    if let Some(tt_sc) = tt_score {
                        let se_beta = tt_sc - se_margin(depth);
                        // Search all other moves at reduced depth with tight window
                        let mut has_alternative = false;
                        for j in 1..moves.len().min(16) {
                            let alt = moves[j];
                            board.make_move(alt);
                            let alt_score = -search_ab(board, tt, stopped, state, depth / 2 - 1, -se_beta - 1, -se_beta, ply + 1, false, time_limit, start_time, false);
                            board.unmake_move(alt);
                            if stopped.load(Ordering::Relaxed) { return 0; }
                            if alt_score >= se_beta {
                                has_alternative = true;
                                break;
                            }
                        }
                        if !has_alternative {
                            extension = 1;
                        }
                    }
                }
            }
        }

        // Late Move Pruning — Stockfish formula: (3 + depth²) / (2 - improving)
        if !is_pv && !in_check && depth <= 8
            && moves_searched >= lmp_threshold(depth, improving)
            && !mv.is_capture() && !mv.is_promotion()
            && best_score > -MATE_SCORE + 100
        {
            continue;
        }

        // Futility pruning — Stockfish: 77 * depth
        if !is_pv && !in_check && depth <= 6
            && !mv.is_capture() && !mv.is_promotion()
            && moves_searched > 0
            && static_eval + futility_margin(depth, improving) <= alpha
            && best_score > -MATE_SCORE + 100
        {
            continue;
        }

        // SEE pruning for losing captures and quiet moves at low depth
        if depth <= 4 && !is_pv && moves_searched > 0
            && best_score > -MATE_SCORE + 100
        {
            if mv.is_capture() && !see::see_ge(board, mv, -20 * depth * depth) {
                continue;
            }
            if !mv.is_capture() && !mv.is_promotion() && !see::see_ge(board, mv, -60 * depth) {
                continue;
            }
        }

        board.make_move(mv);

        // Prefetch next move's TT entry
        if i + 1 < moves.len() {
            let next_idx = pick_best_index(&move_scores, i + 1);
            let next = moves[next_idx];
            board.unmake_move(mv);
            board.make_move(next);
            tt.prefetch(board.hash);
            board.unmake_move(next);
            board.make_move(mv);
        }

        let score;
        let gives_check = board.in_check();
        let effective_depth = depth - 1 + extension;

        if moves_searched == 0 {
            // Full window search for the first move
            score = -search_ab(board, tt, stopped, state, effective_depth, -beta, -alpha, ply + 1, is_pv, time_limit, start_time, false);
        } else {
            // LMR: Late Move Reductions — Stockfish-calibrated
            let mut reduction = 0;
            if moves_searched >= 2 && depth >= 3
                && !in_check && !gives_check
                && !mv.is_capture() && !mv.is_promotion()
            {
                let d = (depth as usize).min(63);
                let m = moves_searched.min(63);
                reduction = lmr_table()[d][m];

                // PV nodes get less reduction
                if is_pv { reduction -= 1; }
                // Non-improving positions get more reduction
                if !improving { reduction += 1; }
                // Killer moves get less reduction
                if is_killer(mv, &state.killers[ply_usize]) { reduction -= 1; }

                // Stockfish stat-score adjustment: statScore * 454 / 4096
                let hist_score = state.history[us.index()][mv.from.index()][mv.to.index()];
                reduction -= hist_score * 454 / (4096 * 1024 / MAX_HISTORY);

                // Cut node gets more reduction (Stockfish)
                if !is_pv && node_type == NodeType::UpperBound { reduction += 2; }

                reduction = reduction.clamp(0, effective_depth - 1);
            }

            // PVS null-window search with reduction
            let mut s = -search_ab(board, tt, stopped, state, effective_depth - reduction, -alpha - 1, -alpha, ply + 1, false, time_limit, start_time, false);

            // Re-search if reduced search fails high
            if s > alpha && reduction > 0 {
                s = -search_ab(board, tt, stopped, state, effective_depth, -alpha - 1, -alpha, ply + 1, false, time_limit, start_time, false);
            }
            // Re-search with full window in PV
            if s > alpha && s < beta {
                s = -search_ab(board, tt, stopped, state, effective_depth, -beta, -alpha, ply + 1, true, time_limit, start_time, false);
            }
            score = s;
        }

        board.unmake_move(mv);
        moves_searched += 1;

        if stopped.load(Ordering::Relaxed) { return 0; }

        if score > best_score {
            best_score = score;
            best_move = mv;

            if score > alpha {
                alpha = score;
                node_type = NodeType::Exact;

                if score >= beta {
                    // Update heuristics for quiet moves causing beta cutoff
                    if !mv.is_capture() {
                        store_killer(&mut state.killers, mv, ply_usize);
                        let ci = us.index();
                        // Stockfish: bonus = min(depth * depth, 2000)
                        let bonus = (depth * depth).min(2000);

                        // History gravity — Stockfish formula:
                        // entry += bonus - entry * |bonus| / MAX_HISTORY
                        update_history(&mut state.history[ci][mv.from.index()][mv.to.index()], bonus);

                        // Penalize quiet moves searched before the cutoff
                        for j in 0..i {
                            let prev = moves[j];
                            if !prev.is_capture() {
                                update_history(&mut state.history[ci][prev.from.index()][prev.to.index()], -bonus);
                            }
                        }

                        if ply > 0 {
                            state.countermoves[mv.from.index()][mv.to.index()] = mv;
                        }
                    }
                    tt.store(board.hash, depth, score, NodeType::LowerBound, best_move);
                    return beta;
                }
            }
        }
    }

    tt.store(board.hash, depth, best_score, node_type, best_move);
    best_score
}

/// Quiescence search — stabilize the evaluation at leaf nodes.
/// Handles check evasions, captures, and promotions.
#[inline(never)]
fn quiescence(
    board: &mut Board,
    tt: &TranspositionTable,
    stopped: &AtomicBool,
    state: &mut ThreadState,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    time_limit: Option<Duration>,
    start_time: Instant,
) -> i32 {
    if state.nodes & 2047 == 0 {
        if stopped.load(Ordering::Relaxed) { return 0; }
        if let Some(tl) = time_limit {
            if start_time.elapsed() >= tl {
                stopped.store(true, Ordering::Relaxed);
                return 0;
            }
        }
    }

    state.nodes += 1;
    let in_check = board.in_check();

    // TT probe in qsearch (important for stability!)
    if let Some(entry) = tt.probe(board.hash) {
        if entry.depth >= 0 {
            match entry.node_type {
                NodeType::Exact => return entry.score,
                NodeType::LowerBound => {
                    if entry.score >= beta { return entry.score; }
                    if entry.score > alpha { alpha = entry.score; }
                }
                NodeType::UpperBound => {
                    if entry.score <= alpha { return entry.score; }
                }
            }
        }
    }

    // When in check, search ALL moves (not just captures) to find escape
    if in_check {
        let moves = board.generate_legal_moves();
        if moves.is_empty() { return -MATE_SCORE + ply; }

        let mut best = -INF;
        for i in 0..moves.len() {
            board.make_move(moves[i]);
            let score = -quiescence(board, tt, stopped, state, -beta, -alpha, ply + 1, time_limit, start_time);
            board.unmake_move(moves[i]);
            if stopped.load(Ordering::Relaxed) { return 0; }
            if score > best { best = score; }
            if score > alpha { alpha = score; }
            if score >= beta { return beta; }
        }
        return best;
    }

    let stand_pat = eval::evaluate(board);
    if stand_pat >= beta { return beta; }

    // Big delta pruning: if we're hopelessly behind, give up
    if stand_pat + DELTA_MARGIN + 1100 < alpha { return alpha; }
    if stand_pat > alpha { alpha = stand_pat; }

    let mut captures = board.generate_legal_captures();

    // Order captures by MVV-LVA for qsearch
    order_captures(board, &mut captures, None);

    for i in 0..captures.len() {
        let mv = captures[i];

        // Per-capture delta pruning
        let cv = estimate_capture_value(board, mv);
        if stand_pat + cv + DELTA_MARGIN < alpha { continue; }

        // SEE pruning: skip losing captures
        if !see::see_ge(board, mv, 0) { continue; }

        board.make_move(mv);
        let score = -quiescence(board, tt, stopped, state, -beta, -alpha, ply + 1, time_limit, start_time);
        board.unmake_move(mv);

        if stopped.load(Ordering::Relaxed) { return 0; }
        if score >= beta { return beta; }
        if score > alpha { alpha = score; }
    }

    alpha
}

#[inline(always)]
fn estimate_capture_value(board: &Board, mv: Move) -> i32 {
    if mv.is_promotion() { return 900; }
    if let Some((piece, _)) = board.piece_on(mv.to) {
        piece_value(piece)
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        100
    } else {
        0
    }
}

/// Score all moves for ordering. Returns a parallel Vec of scores.
#[inline]
fn score_moves(board: &Board, moves: &MoveList, tt_move: Option<Move>, killers: &[Move; 2], history: &[[i32; 64]; 64]) -> Vec<i32> {
    let mut scores = Vec::with_capacity(moves.len());
    for i in 0..moves.len() {
        scores.push(move_score(board, moves[i], tt_move, killers, history));
    }
    scores
}

/// Pick the index of the best move from position `start` to end.
#[inline]
fn pick_best_index(scores: &[i32], start: usize) -> usize {
    let mut best_idx = start;
    let mut best_score = scores[start];
    for i in (start + 1)..scores.len() {
        if scores[i] > best_score {
            best_score = scores[i];
            best_idx = i;
        }
    }
    best_idx
}

/// Order captures by MVV-LVA.
#[inline]
fn order_captures(board: &Board, moves: &mut MoveList, tt_move: Option<Move>) {
    if moves.len() <= 1 { return; }
    moves.sort_by(|a, b| {
        let sa = capture_score(board, *a, tt_move);
        let sb = capture_score(board, *b, tt_move);
        sb.cmp(&sa)
    });
}

#[inline(always)]
fn capture_score(board: &Board, mv: Move, tt_move: Option<Move>) -> i32 {
    if let Some(ttm) = tt_move {
        if mv.from == ttm.from && mv.to == ttm.to { return 10_000_000; }
    }
    let victim = if let Some((p, _)) = board.piece_on(mv.to) { piece_value(p) }
    else if mv.flag == types::chess_move::MoveFlag::EnPassant { 100 } else { 0 };
    let attacker = if let Some((p, _)) = board.piece_on(mv.from) { piece_value(p) } else { 0 };
    victim * 10 - attacker + if mv.is_promotion() { 900 } else { 0 }
}

#[inline(always)]
fn move_score(board: &Board, mv: Move, tt_move: Option<Move>, killers: &[Move; 2], history: &[[i32; 64]; 64]) -> i32 {
    if let Some(ttm) = tt_move {
        if mv.from == ttm.from && mv.to == ttm.to { return 10_000_000; }
    }
    let mut score = 0i32;
    if mv.is_capture() {
        let victim = if let Some((p, _)) = board.piece_on(mv.to) { piece_value(p) }
        else if mv.flag == types::chess_move::MoveFlag::EnPassant { 100 } else { 0 };
        let attacker = if let Some((p, _)) = board.piece_on(mv.from) { piece_value(p) } else { 0 };
        score += 1_000_000 + victim * 10 - attacker;
    }
    if mv.is_promotion() { score += 900_000; }
    if !mv.is_capture() {
        if mv.from == killers[0].from && mv.to == killers[0].to { score += 800_000; }
        else if mv.from == killers[1].from && mv.to == killers[1].to { score += 700_000; }
        else { score += history[mv.from.index()][mv.to.index()].max(0); }
    }
    score
}

#[inline(always)]
fn is_killer(mv: Move, killers: &[Move; 2]) -> bool {
    (mv.from == killers[0].from && mv.to == killers[0].to)
        || (mv.from == killers[1].from && mv.to == killers[1].to)
}

#[inline(always)]
fn store_killer(killers: &mut [[Move; 2]; MAX_PLY], mv: Move, ply: usize) {
    if mv.from == killers[ply][0].from && mv.to == killers[ply][0].to { return; }
    killers[ply][1] = killers[ply][0];
    killers[ply][0] = mv;
}

#[inline(always)]
fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100, Piece::Knight => 320, Piece::Bishop => 330,
        Piece::Rook => 500, Piece::Queen => 900, Piece::King => 20000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() { types::init(); }

    #[test]
    fn test_search_returns_legal_move() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_depth(&mut board, 3);
        let legal = board.generate_legal_moves();
        assert!(legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to));
    }

    #[test]
    fn test_search_complex_position() {
        setup();
        let mut board = Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1").unwrap();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_depth(&mut board, 4);
        let legal = board.generate_legal_moves();
        assert!(legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to));
    }

    #[test]
    fn test_mate_in_1() {
        setup();
        let mut board = Board::from_fen("r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4").unwrap();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_depth(&mut board, 4);
        assert!(result.score > MATE_SCORE - 10);
    }

    #[test]
    fn test_material_advantage() {
        setup();
        let mut board = Board::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_depth(&mut board, 3);
        assert!(result.score > 500);
    }

    #[test]
    fn test_time_limit() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_time(&mut board, Duration::from_millis(100), 64);
        assert!(result.nodes > 0);
        assert!(result.elapsed <= Duration::from_millis(500));
    }

    #[test]
    fn test_preserves_board() {
        setup();
        let mut board = Board::new();
        let original_fen = board.to_fen();
        let original_hash = board.hash;
        let mut engine = SearchEngine::new(1, 1);
        let _ = engine.search_depth(&mut board, 5);
        assert_eq!(board.to_fen(), original_fen);
        assert_eq!(board.hash, original_hash);
    }

    #[test]
    fn test_multithreaded_search() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(8, 4);
        let result = engine.search_depth(&mut board, 6);
        assert!(result.nodes > 0);
        let legal = board.generate_legal_moves();
        assert!(legal.iter().any(|m| m.from == result.best_move.from && m.to == result.best_move.to));
    }

    #[test]
    fn test_tt_stores_during_search() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        engine.search_depth(&mut board, 4);
        assert!(engine.tt.probe(board.hash).is_some());
    }

    #[test]
    fn test_tt_clear() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        engine.search_depth(&mut board, 3);
        assert!(engine.tt.probe(board.hash).is_some());
        engine.clear();
        assert!(engine.tt.probe(board.hash).is_none());
    }

    #[test]
    fn test_depth_1_bounded() {
        setup();
        let mut board = Board::new();
        let mut engine = SearchEngine::new(1, 1);
        let result = engine.search_depth(&mut board, 1);
        assert!(result.nodes > 0 && result.nodes < 1000);
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
        assert!(board.has_repetition(), "Should detect repetition after Nf3 Nf6 Ng1 Ng8");
    }
}
