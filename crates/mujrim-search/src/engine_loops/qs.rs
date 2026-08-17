//! Capture QS for the official Viridithas loop.
//! Official qsearch has no reverse-quiet re-entry into the generic PVS.

use super::{
    INF, MATE_SCORE, MAX_PLY, SearchContext, ThreadState, capture_score, draw_score, hybrid_eval,
    make_search_move, make_search_move_no_undo, score_from_tt, score_to_tt, search_time_exceeded,
    undo_search_eval, usable_tt_move,
};
use crate::move_picker::MovePicker;
use crate::see;
use crate::tt::{NodeType, TTData};
use std::sync::atomic::Ordering;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn quiescence(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    alpha: i32,
    beta: i32,
    ply: i32,
) -> i32 {
    quiescence_ex::<false>(board, state, context, alpha, beta, ply)
}

/// Akimbo PVS uses snapshot restore; QS should match that make path.
pub(super) fn quiescence_snapshot(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    alpha: i32,
    beta: i32,
    ply: i32,
) -> i32 {
    quiescence_ex::<true>(board, state, context, alpha, beta, ply)
}

fn quiescence_ex<const SNAPSHOT: bool>(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    mut alpha: i32,
    beta: i32,
    ply: i32,
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
        deadline_ms,
        ..
    } = *context;

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
        if search_time_exceeded(start_time, time_limit, deadline_ms) {
            stopped.store(true, Ordering::Relaxed);
            return 0;
        }
    }

    let ply_usize = (ply as usize).min(MAX_PLY - 1);
    if ply >= MAX_PLY as i32 - 1 {
        return hybrid_eval(board, state, use_nnue);
    }
    if board.is_search_draw(ply_usize) {
        return draw_score(state.nodes);
    }

    let in_check = state.in_check[ply_usize];
    let mut raw_eval = None;
    let mut tt_score = None;
    let mut tt_bound = None;
    let tt_move = if let Some(entry) = tt.probe(board.tt_hash()) {
        let probed = score_from_tt(entry.score, ply);
        match entry.node_type {
            NodeType::Exact => return probed,
            NodeType::LowerBound if probed >= beta => return probed,
            NodeType::UpperBound if probed <= alpha => return probed,
            _ => {}
        }
        raw_eval = entry.raw_eval;
        tt_score = Some(probed);
        tt_bound = Some(entry.node_type);
        usable_tt_move(entry.best_move)
    } else {
        None
    };

    let mut acc_ready = false;
    let mut best_score = if in_check {
        -INF
    } else {
        let raw = if let Some(raw) = raw_eval {
            raw
        } else {
            acc_ready = use_nnue;
            let raw = hybrid_eval(board, state, use_nnue);
            raw_eval = Some(raw);
            raw
        };
        let eval = super::corrected_network_eval(
            board,
            raw,
            state.correction(board, move_ordering),
            state.optimism[board.side_to_move.index()],
            eval_mode,
        );
        let stand_pat = if let (Some(ts), Some(bound)) = (tt_score, tt_bound) {
            let replace = match bound {
                NodeType::LowerBound => eval <= ts,
                NodeType::UpperBound => eval >= ts,
                NodeType::Exact => true,
            };
            if replace { ts } else { eval }
        } else {
            eval
        };
        if stand_pat >= beta {
            return i32::midpoint(stand_pat, beta);
        }
        alpha = alpha.max(stand_pat);
        stand_pat
    };
    let mut picker = MovePicker::new(board, tt_move, [NULL_MOVE; 2], NULL_MOVE)
        .with_move_ordering(move_ordering);
    if !in_check {
        picker.skip_quiets();
        picker.skip_bad_captures();
    }
    let score_capture = |b: &Board, mv: Move| capture_score(b, mv, tt_move, move_ordering);
    let score_quiet = |_: &Board, _: Move| 0;
    let mut best_move = NULL_MOVE;
    let mut bound = NodeType::UpperBound;

    while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
        if !in_check && !see::see_ge(board, mv, 1) {
            continue;
        }
        if use_nnue && !acc_ready {
            state.nnue_state.hint_common_access(board);
            acc_ready = true;
        }
        context.tt.prefetch(board.tt_hash_after(mv));
        let score = if SNAPSHOT {
            let snap = board.snapshot();
            make_search_move_no_undo(board, state, mv);
            let child_ply = ply_usize + 1;
            if child_ply < MAX_PLY {
                state.in_check[child_ply] = board.in_check();
            }
            let score = -quiescence_ex::<true>(board, state, context, -beta, -alpha, ply + 1);
            board.restore_snapshot(snap);
            score
        } else {
            make_search_move(board, state, mv);
            let child_ply = ply_usize + 1;
            if child_ply < MAX_PLY {
                state.in_check[child_ply] = board.in_check();
            }
            let score = -quiescence_ex::<false>(board, state, context, -beta, -alpha, ply + 1);
            board.unmake_move(mv);
            score
        };
        undo_search_eval(state);
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }
        if score > best_score {
            best_score = score;
            best_move = mv;
            if score > alpha {
                alpha = score;
                bound = NodeType::Exact;
                if score >= beta {
                    bound = NodeType::LowerBound;
                    break;
                }
            }
        }
    }

    if in_check && best_score == -INF {
        return -MATE_SCORE + ply;
    }

    if !in_check && best_score > beta {
        best_score = i32::midpoint(best_score, beta);
    }

    tt.store(
        board.tt_hash(),
        TTData::new(
            0,
            score_to_tt(best_score, ply),
            bound,
            best_move,
            false,
            raw_eval,
        ),
    );
    let _ = params;
    best_score
}
