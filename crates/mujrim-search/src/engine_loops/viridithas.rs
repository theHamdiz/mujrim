//! Official cosmobobak/viridithas `alpha_beta` (v20.0.0 src/search.rs).

use super::dedicated_qs::quiescence;
use super::{
    INF, MATE_SCORE, MAX_PLY, SearchContext, SearchNode, ThreadState, captured_piece_index,
    draw_score, hybrid_eval, make_search_move, nmp_material_ok, piece_index_on, score_from_tt,
    score_to_tt, search_time_exceeded, store_killer, undo_search_eval, usable_tt_move,
};
use crate::move_picker::MovePicker;
use crate::policy::{
    LmpContext, LmpPolicy, LmrContext, LmrPolicy, ViridithasLmpPolicy, ViridithasLmrPolicy,
};
use crate::see;
use crate::tt::{NodeType, TTData};
use std::sync::atomic::Ordering;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

pub(crate) fn alpha_beta(
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
        use_nnue,
        move_ordering,
        deadline_ms,
        ..
    } = *context;
    let SearchNode {
        mut depth,
        mut alpha,
        mut beta,
        ply,
        is_pv,
        is_root,
        excluded_move,
        allow_null,
        ..
    } = node;

    if depth <= 0 {
        return quiescence(board, state, context, alpha, beta, ply);
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
    state.pv_len[ply_usize] = 0;
    state.cutoffs[ply_usize] = 0;
    if !is_root && ply > state.seldepth {
        state.seldepth = ply;
    }

    let in_check = if is_root {
        let check = board.in_check();
        state.in_check[ply_usize] = check;
        check
    } else {
        state.in_check[ply_usize]
    };

    if !is_root && board.is_search_draw(ply_usize) {
        return draw_score(state.nodes);
    }
    if ply >= MAX_PLY as i32 - 1 {
        return if in_check {
            0
        } else {
            hybrid_eval(board, state, use_nnue)
        };
    }
    if !is_root {
        alpha = alpha.max(-MATE_SCORE + ply);
        beta = beta.min(MATE_SCORE - ply - 1);
        if alpha >= beta {
            return alpha;
        }
    }

    let singular = excluded_move.is_some();
    let mut tt_move = None;
    let mut tt_score = None;
    let mut tt_depth = -1;
    let mut tt_bound = NodeType::Exact;
    let mut raw_eval = None;
    let mut tt_was_pv = is_pv;

    if !singular && let Some(entry) = tt.probe(board.tt_hash()) {
        tt_move = usable_tt_move(entry.best_move);
        tt_score = Some(score_from_tt(entry.score, ply));
        tt_depth = entry.depth;
        tt_bound = entry.node_type;
        raw_eval = entry.raw_eval;
        tt_was_pv = tt_was_pv || entry.was_pv;
        if !is_pv && entry.depth >= depth {
            match entry.node_type {
                NodeType::Exact => return tt_score.expect("probed"),
                NodeType::LowerBound if tt_score.expect("probed") >= beta => {
                    return tt_score.expect("probed");
                }
                NodeType::UpperBound if tt_score.expect("probed") <= alpha => {
                    return tt_score.expect("probed");
                }
                _ => {}
            }
        }
    }

    let static_eval = if in_check {
        0
    } else if singular {
        state.static_evals[ply_usize]
    } else {
        let raw = hybrid_eval(board, state, use_nnue);
        raw_eval = Some(raw);
        raw + state.correction(board, move_ordering)
    };
    if !in_check && !singular {
        state.static_evals[ply_usize] = static_eval;
        state.eval_valid[ply_usize] = true;
    } else {
        state.eval_valid[ply_usize] = false;
    }

    let mut eval = static_eval;
    if let Some(ts) = tt_score
        && !in_check
    {
        let replace = match tt_bound {
            NodeType::LowerBound => eval <= ts,
            NodeType::UpperBound => eval >= ts,
            NodeType::Exact => true,
        };
        if replace {
            eval = ts;
        }
    }

    let improving = if in_check {
        false
    } else if ply_usize >= 2 && state.eval_valid[ply_usize - 2] {
        static_eval > state.static_evals[ply_usize - 2]
    } else {
        true
    };

    if !is_root && !is_pv && !in_check && !singular {
        let prev_red = if ply_usize > 0 {
            state.reductions[ply_usize - 1]
        } else {
            0
        };
        if ply_usize > 0 && state.eval_valid[ply_usize - 1] {
            let sum = static_eval + state.static_evals[ply_usize - 1];
            if prev_red >= 1419 && sum < 0 {
                depth += 1;
            } else if prev_red >= 2494 && sum > 128 {
                depth -= 1;
            }
        }

        if alpha < 2000 && static_eval < alpha - 123 - 295 * depth {
            let v = quiescence(board, state, context, alpha, beta, ply);
            if v <= alpha {
                return v;
            }
        }

        let margin = 65 * depth - i32::from(improving) * 76;
        if depth < 9 && eval - margin >= beta && beta.abs() < MATE_SCORE - 100 {
            return beta + (eval - beta) / 3;
        }

        if allow_null
            && depth > 2
            && nmp_material_ok(board, board.side_to_move)
            && static_eval >= beta
        {
            let r = 4 + depth / 3 + ((static_eval - beta) / 200).min(4);
            if state.use_nnue {
                state.nnue_state.push_null();
            }
            board.make_null_move();
            state.prev_move[ply_usize] = NULL_MOVE;
            if ply_usize + 1 < MAX_PLY {
                state.in_check[ply_usize + 1] = false;
            }
            let nw = -alpha_beta(
                board,
                state,
                context,
                SearchNode {
                    depth: depth - r,
                    alpha: -beta,
                    beta: -beta + 1,
                    ply: ply + 1,
                    is_pv: false,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: 0,
                    nominal_depth: depth,
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
            if nw >= beta {
                return if nw > MATE_SCORE - 100 { beta } else { nw };
            }
        }
    }

    if allow_null && !is_pv && tt_move.is_none() && depth >= 8 {
        depth -= 1;
    }

    let us = board.side_to_move;
    let threats = super::opponent_threats(board).all;
    state.threats[ply_usize] = threats;
    let killers = state.killers[ply_usize];
    let mut picker =
        MovePicker::new(board, tt_move, killers, NULL_MOVE).with_move_ordering(move_ordering);
    let score_capture = |b: &Board, mv: Move| super::capture_score(b, mv, tt_move, move_ordering);
    let history_ptr = std::ptr::from_ref(&state.history);
    let us_idx = us.index();
    let score_quiet = |_: &Board, mv: Move| {
        // SAFETY: history is only read while the picker scores, before make.
        unsafe { (*history_ptr).get(threats, us_idx, mv) }
    };

    let mut best_score = -INF;
    let mut best_move = NULL_MOVE;
    let mut bound = NodeType::UpperBound;
    let mut legal = 0usize;
    let mut quiets: [Move; 32] = [NULL_MOVE; 32];
    let mut quiets_len = 0usize;
    let mut alpha_raises = 0i32;
    let try_singular = !is_root
        && !singular
        && depth >= params.se_depth_min
        && tt_move.is_some()
        && tt_depth >= depth - 3
        && tt_bound != NodeType::UpperBound
        && tt_score.is_some_and(|s| s.abs() < MATE_SCORE - 100);

    while let Some(mv) = picker.next(board, &score_capture, &score_quiet) {
        if excluded_move.is_some_and(|em| same_move(em, mv))
            || (is_root && state.is_root_excluded(mv))
        {
            continue;
        }
        let is_quiet = mv.is_quiet();
        if !is_root
            && !is_pv
            && !in_check
            && best_score > -MATE_SCORE + 100
            && ViridithasLmpPolicy
                .decision(&LmpContext {
                    depth,
                    move_count: legal + 1,
                    improvement: 0,
                    improving,
                    is_root,
                    is_pv,
                    in_check,
                    is_quiet,
                    best_score,
                    stock_depth_limit: 8,
                    stock_move_threshold: 99,
                })
                .is_some()
        {
            break;
        }
        if !is_root && !is_pv && depth < 10 && best_score > -MATE_SCORE + 100 {
            let see_margin = if is_quiet {
                -65 * depth
            } else {
                -90 * depth * depth
            };
            if !see::see_ge(board, mv, see_margin) {
                continue;
            }
        }

        let mut extension = 0;
        if try_singular && tt_move.is_some_and(|ttm| same_move(mv, ttm)) {
            let tt_sc = tt_score.expect("singular has tt score");
            let s_beta = tt_sc - params.se_margin(depth);
            let s_score = alpha_beta(
                board,
                state,
                context,
                SearchNode {
                    depth: (depth - 1) / 2,
                    alpha: s_beta - 1,
                    beta: s_beta,
                    ply,
                    is_pv: false,
                    is_root: false,
                    excluded_move: Some(mv),
                    total_extensions: 0,
                    nominal_depth: depth,
                    allow_null: false,
                },
            );
            if s_score < s_beta {
                extension = 1;
            } else if tt_sc >= beta {
                extension = -1;
            }
        }

        let moved_piece = piece_index_on(board, mv.from);
        let captured = captured_piece_index(board, mv);
        make_search_move(board, state, mv);
        let gives_check = board.in_check();
        if ply_usize + 1 < MAX_PLY {
            state.in_check[ply_usize + 1] = gives_check;
        }
        state.prev_move[ply_usize] = mv;
        state.prev_piece[ply_usize] = moved_piece;
        legal += 1;

        let mut reduction_1024 = 0;
        let lmr_ready = depth > 2 && legal > usize::from(is_root);
        if lmr_ready && (is_quiet || ViridithasLmrPolicy.reduce_noisy_moves()) {
            let whole = ViridithasLmrPolicy.adjust_reduction(
                0,
                &LmrContext {
                    depth,
                    move_count: legal,
                    is_quiet,
                    is_pv,
                    improving,
                    improvement: 0,
                    alpha_raises,
                    is_killer: false,
                    gives_check,
                    is_recapture: false,
                    mv_stat_score: state.stat_score(
                        mv,
                        us.index(),
                        moved_piece,
                        ply_usize,
                        move_ordering,
                    ),
                    corr_abs: 0,
                    is_cut_node: allow_null,
                    winning_beta: false,
                    tt_was_pv,
                    tt_score_above_alpha: tt_score.is_some_and(|s| s > alpha),
                    tt_score_below_alpha: tt_score.is_some_and(|s| s < alpha),
                    tt_depth_sufficient: tt_score.is_some() && tt_depth >= depth,
                    tt_move_missing: tt_move.is_none(),
                    tt_capture: tt_move.is_some_and(|m| m.is_capture()),
                    hist_lmr_div: params.hist_lmr_div,
                    lmr_corr_mul: params.lmr_corr_mul,
                    lmr_cut_node_bonus: params.lmr_cut_node_bonus,
                    child_cutoffs: state.cutoffs[(ply_usize + 1).min(MAX_PLY - 1)],
                },
            );
            reduction_1024 = whole.max(0).saturating_mul(1024);
        }

        let child_depth = if legal == 1 {
            depth + extension - 1
        } else {
            (depth + extension - reduction_1024 / 1024).max(0)
        };
        state.reductions[ply_usize] = reduction_1024.max(if reduction_1024 > 0 { 1024 } else { 0 });

        let score = if legal == 1 {
            -alpha_beta(
                board,
                state,
                context,
                SearchNode {
                    depth: child_depth,
                    alpha: -beta,
                    beta: -alpha,
                    ply: ply + 1,
                    is_pv,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: 0,
                    nominal_depth: depth,
                    allow_null: !is_pv,
                },
            )
        } else {
            let mut zw = -alpha_beta(
                board,
                state,
                context,
                SearchNode {
                    depth: child_depth,
                    alpha: -alpha - 1,
                    beta: -alpha,
                    ply: ply + 1,
                    is_pv: false,
                    is_root: false,
                    excluded_move: None,
                    total_extensions: 0,
                    nominal_depth: depth,
                    allow_null: true,
                },
            );
            if zw > alpha && (is_pv || reduction_1024 > 0) {
                zw = -alpha_beta(
                    board,
                    state,
                    context,
                    SearchNode {
                        depth: depth + extension - 1,
                        alpha: -beta,
                        beta: -alpha,
                        ply: ply + 1,
                        is_pv,
                        is_root: false,
                        excluded_move: None,
                        total_extensions: 0,
                        nominal_depth: depth,
                        allow_null: false,
                    },
                );
            }
            zw
        };
        state.reductions[ply_usize] = 0;

        board.unmake_move(mv);
        undo_search_eval(state);
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
        }
        if score > alpha {
            alpha_raises += 1;
            best_move = mv;
            alpha = score;
            bound = NodeType::Exact;
            let child = (ply_usize + 1).min(MAX_PLY - 1);
            state.pv[ply_usize][0] = mv;
            let child_len = state.pv_len[child].min(MAX_PLY - 1);
            for j in 0..child_len {
                state.pv[ply_usize][j + 1] = state.pv[child][j];
            }
            state.pv_len[ply_usize] = child_len + 1;
            if score >= beta {
                bound = NodeType::LowerBound;
                if ply_usize > 0 {
                    state.cutoffs[ply_usize - 1] += 1;
                }
                if is_quiet {
                    store_killer(&mut state.killers, mv, ply_usize);
                    let bonus = params.history_bonus(depth);
                    let malus = params.history_malus(depth);
                    state.history.update(threats, us.index(), mv, bonus);
                    for quiet in quiets[..quiets_len].iter() {
                        state.history.update(threats, us.index(), *quiet, -malus);
                    }
                } else if let Some(cap) = captured {
                    state.cap_hist.update(
                        threats,
                        moved_piece,
                        mv.to,
                        cap,
                        params.history_bonus(depth),
                    );
                }
                break;
            }
        }
        if is_quiet && quiets_len < quiets.len() {
            quiets[quiets_len] = mv;
            quiets_len += 1;
        }
    }

    if legal == 0 {
        return if singular {
            alpha
        } else if in_check {
            -MATE_SCORE + ply
        } else {
            draw_score(state.nodes)
        };
    }

    if !singular {
        tt.store(
            board.tt_hash(),
            TTData::new(
                depth,
                score_to_tt(best_score, ply),
                bound,
                best_move,
                tt_was_pv,
                raw_eval,
            ),
        );
        if !in_check
            && best_move.is_quiet()
            && !(bound == NodeType::LowerBound && best_score <= static_eval
                || bound == NodeType::UpperBound && best_score >= static_eval)
        {
            state.update_correction(board, depth, best_score, static_eval, move_ordering);
        }
    }
    best_score
}

#[inline(always)]
fn same_move(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}
