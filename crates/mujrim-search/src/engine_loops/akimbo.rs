//! Official jw1912/akimbo `pvs` (main/src/search.rs), on Mujrim Board/TT/eval.

use super::dedicated_qs::quiescence;
use super::{
    INF, MATE_SCORE, MAX_PLY, SearchContext, SearchNode, ThreadState, captured_piece_index,
    draw_score, hybrid_eval, make_search_move, nmp_material_ok, piece_index_on, score_from_tt,
    score_to_tt, search_time_exceeded, store_killer, undo_search_eval, usable_tt_move,
};
use crate::move_picker::MovePicker;
use crate::policy::{
    AkimboLmpPolicy, AkimboLmrPolicy, AkimboRfpPolicy, LmpContext, LmpPolicy, LmrContext,
    LmrPolicy, RfpContext, RfpPolicy,
};
use crate::see;
use crate::tt::{NodeType, TTData};
use std::sync::atomic::Ordering;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

pub(crate) fn pvs(
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

    if state.nodes & 1023 == 0 {
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
    if ply > state.seldepth {
        state.seldepth = ply;
    }
    if is_root {
        state.in_check[ply_usize] = board.in_check();
    }

    if !is_root && board.is_search_draw(ply_usize) {
        return draw_score(state.nodes);
    }
    if !is_root {
        alpha = alpha.max(ply - MATE_SCORE);
        beta = beta.min(MATE_SCORE - ply - 1);
        if alpha >= beta {
            return alpha;
        }
        depth += i32::from(state.in_check[ply_usize]);
    }

    if depth <= 0 || ply >= MAX_PLY as i32 - 1 {
        return quiescence(board, state, context, alpha, beta, ply);
    }

    let in_check = state.in_check[ply_usize];
    let singular = excluded_move.is_some();
    let mut tt_move = None;
    let mut tt_score = None;
    let mut tt_depth = -1;
    let mut tt_bound = NodeType::Exact;
    let mut raw_eval = None;

    if !singular && let Some(entry) = tt.probe(board.tt_hash()) {
        tt_move = usable_tt_move(entry.best_move);
        tt_score = Some(score_from_tt(entry.score, ply));
        tt_depth = entry.depth;
        tt_bound = entry.node_type;
        raw_eval = entry.raw_eval;
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

    let static_eval = if in_check || singular {
        state.static_evals[ply_usize]
    } else {
        let raw = hybrid_eval(board, state, use_nnue);
        raw_eval = Some(raw);
        raw + state.correction(board, move_ordering)
    };
    if !in_check && !singular {
        state.static_evals[ply_usize] = static_eval;
        state.eval_valid[ply_usize] = true;
    }
    let mut eval = static_eval;
    if let Some(ts) = tt_score {
        let replace = match tt_bound {
            NodeType::LowerBound => eval <= ts,
            NodeType::UpperBound => eval >= ts,
            NodeType::Exact => true,
        };
        if replace {
            eval = ts;
        }
    }

    let improving = ply_usize > 1
        && state.eval_valid[ply_usize - 2]
        && static_eval > state.static_evals[ply_usize - 2];

    if !is_pv && !in_check && !singular && beta.abs() < MATE_SCORE - 100 {
        let rfp = AkimboRfpPolicy.cutoff_score(
            eval,
            beta,
            &RfpContext {
                depth,
                improving,
                improvement: 0,
                correction_abs: 0,
                tt_was_pv: false,
                own_pieces_threatened: false,
                stock_margin: 0,
            },
        );
        if let Some(score) = rfp {
            return score;
        }

        if allow_null
            && ply_usize >= state.min_nmp_ply
            && depth >= params.nmp_depth_min
            && nmp_material_ok(board, board.side_to_move)
            && eval >= beta
        {
            let r = params.null_move_r(depth, eval, beta) + i32::from(improving);
            if state.use_nnue {
                state.nnue_state.push_null();
            }
            let null_snap = board.snapshot();
            board.make_null_move();
            state.prev_move[ply_usize] = NULL_MOVE;
            if ply_usize + 1 < MAX_PLY {
                state.in_check[ply_usize + 1] = false;
            }
            let nw = -pvs(
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
            board.restore_snapshot(null_snap);
            if state.use_nnue {
                state.nnue_state.pop_move();
            }
            if stopped.load(Ordering::Relaxed) {
                return 0;
            }
            if nw >= beta {
                if depth < params.nmp_min_verif_depth || state.min_nmp_ply > 0 {
                    return if nw > MATE_SCORE - 100 { beta } else { nw };
                }
                state.min_nmp_ply = ply_usize + ((depth - r) * params.nmp_verif_frac / 16) as usize;
                let verif = pvs(
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
                        total_extensions: 0,
                        nominal_depth: depth,
                        allow_null: false,
                    },
                );
                state.min_nmp_ply = 0;
                if verif >= beta {
                    return verif;
                }
            }
        }
    }

    if depth >= 4 && tt_move.is_none() {
        depth -= 1;
    }

    if !is_pv && !in_check && !singular && depth > 5 && beta.abs() < MATE_SCORE - 100 {
        let pc_beta = super::akimbo_probcut_beta(beta);
        let can_probcut = !(tt_depth >= depth - 3 && tt_score.is_some_and(|s| s < pc_beta));
        if can_probcut {
            let pc_capture =
                |b: &Board, mv: Move| super::capture_score(b, mv, tt_move, move_ordering);
            let pc_quiet = |_: &Board, _: Move| 0;
            let mut pc_picker = MovePicker::new(board, tt_move, [NULL_MOVE; 2], NULL_MOVE)
                .with_move_ordering(move_ordering);
            pc_picker.skip_quiets();
            while let Some(mv) = pc_picker.next(board, &pc_capture, &pc_quiet) {
                if !see::see_ge(board, mv, 1) {
                    continue;
                }
                let pc_snap = board.snapshot();
                make_search_move(board, state, mv);
                if ply_usize + 1 < MAX_PLY {
                    state.in_check[ply_usize + 1] = board.in_check();
                }
                let mut pc_score =
                    -quiescence(board, state, context, -pc_beta, -pc_beta + 1, ply + 1);
                if pc_score >= pc_beta {
                    pc_score = -pvs(
                        board,
                        state,
                        context,
                        SearchNode {
                            depth: depth - 4,
                            alpha: -pc_beta,
                            beta: -pc_beta + 1,
                            ply: ply + 1,
                            is_pv: false,
                            is_root: false,
                            excluded_move: None,
                            total_extensions: 0,
                            nominal_depth: depth,
                            allow_null: false,
                        },
                    );
                }
                board.restore_snapshot(pc_snap);
                undo_search_eval(state);
                if stopped.load(Ordering::Relaxed) {
                    return 0;
                }
                if pc_score >= pc_beta {
                    tt.store(
                        board.tt_hash(),
                        TTData::new(
                            depth - 3,
                            score_to_tt(pc_beta, ply),
                            NodeType::LowerBound,
                            mv,
                            false,
                            raw_eval,
                        ),
                    );
                    return pc_beta;
                }
            }
        }
    }

    let us = board.side_to_move;
    let threats = opponent_all(board);
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
    let mut best_move = tt_move.unwrap_or(NULL_MOVE);
    let mut bound = NodeType::UpperBound;
    let mut legal = 0usize;
    let mut quiets: [Move; 32] = [NULL_MOVE; 32];
    let mut quiets_len = 0usize;
    let can_lmr = depth > 1 && !in_check;
    let lmp_margin = 2 + depth * depth / if improving { 1 } else { 2 };
    let try_singular = !is_root
        && !singular
        && depth >= 8
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
        if !is_pv && !in_check && best_score.abs() < MATE_SCORE - 100 {
            if is_quiet && legal as i32 > lmp_margin {
                break;
            }
            if is_quiet
                && AkimboLmpPolicy
                    .decision(&LmpContext {
                        depth,
                        move_count: legal + 1,
                        improvement: 0,
                        improving,
                        is_root,
                        is_pv,
                        in_check,
                        is_quiet: true,
                        best_score,
                        stock_depth_limit: 8,
                        stock_move_threshold: 99,
                    })
                    .is_some()
            {
                break;
            }
            let margin = if mv.is_capture() { -148 } else { -64 };
            if depth < 7 && !see::see_ge(board, mv, margin * depth) {
                continue;
            }
        }

        let mut extension = 0;
        if try_singular && tt_move.is_some_and(|ttm| same_move(mv, ttm)) {
            let tt_sc = tt_score.expect("singular has tt score");
            let s_beta = tt_sc - depth * params.se_margin(depth).max(1);
            let s_score = pvs(
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
                if !is_pv && s_score < s_beta - 25 && state.dbl_exts[ply_usize] < 5 {
                    state.dbl_exts[ply_usize] += 1;
                    extension += 1;
                }
            } else if tt_sc >= beta || (tt_sc <= alpha && allow_null) {
                extension = -1;
            }
        }

        let moved_piece = piece_index_on(board, mv.from);
        let captured = captured_piece_index(board, mv);
        let child_snap = board.snapshot();
        make_search_move(board, state, mv);
        let gives_check = board.in_check();
        if ply_usize + 1 < MAX_PLY {
            state.in_check[ply_usize + 1] = gives_check;
        }
        state.prev_move[ply_usize] = mv;
        state.prev_piece[ply_usize] = moved_piece;
        legal += 1;

        let mut reduce = 0;
        if can_lmr && is_quiet && legal > 1 {
            reduce = AkimboLmrPolicy.adjust_reduction(
                0,
                &LmrContext {
                    depth,
                    move_count: legal,
                    is_quiet: true,
                    is_pv,
                    improving,
                    improvement: 0,
                    alpha_raises: 0,
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
                    tt_was_pv: false,
                    tt_score_above_alpha: false,
                    tt_score_below_alpha: false,
                    tt_depth_sufficient: false,
                    tt_move_missing: tt_move.is_none(),
                    tt_capture: tt_move.is_some_and(|m| m.is_capture()),
                    hist_lmr_div: 8192,
                    lmr_corr_mul: 0,
                    lmr_cut_node_bonus: 0,
                    child_cutoffs: state.cutoffs[(ply_usize + 1).min(MAX_PLY - 1)],
                },
            );
        }

        let score = if legal == 1 {
            -pvs(
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
            )
        } else {
            let mut zw = -pvs(
                board,
                state,
                context,
                SearchNode {
                    depth: (depth - 1 - reduce).max(0),
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
            if zw > alpha && (is_pv || reduce > 0) {
                zw = -pvs(
                    board,
                    state,
                    context,
                    SearchNode {
                        depth: depth - 1,
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

        board.restore_snapshot(child_snap);
        undo_search_eval(state);
        if stopped.load(Ordering::Relaxed) {
            return 0;
        }

        if score > best_score {
            best_score = score;
        }
        if score > alpha {
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
        return if in_check {
            -MATE_SCORE + ply
        } else {
            draw_score(state.nodes)
        };
    }

    if !singular
        && !in_check
        && best_move.is_quiet()
        && !(bound == NodeType::LowerBound && best_score <= static_eval
            || bound == NodeType::UpperBound && best_score >= static_eval)
    {
        state.update_correction(board, depth, best_score, static_eval, move_ordering);
    }

    if !singular {
        tt.store(
            board.tt_hash(),
            TTData::new(
                depth,
                score_to_tt(best_score, ply),
                bound,
                best_move,
                is_pv,
                raw_eval,
            ),
        );
    }
    best_score
}

#[inline(always)]
fn same_move(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}

#[inline(always)]
fn opponent_all(board: &Board) -> u64 {
    super::opponent_threats(board).all
}
