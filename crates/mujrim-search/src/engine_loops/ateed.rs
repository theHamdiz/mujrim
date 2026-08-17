//! Ateed MoE PVS: Reckless-shaped reductions plus expert/WDL/gate hooks.
//!
//! Mechanical opts shared with the other dedicated loops: snapshot make/restore,
//! TT prefetch, deferred `hint_common_access`, lazy threat maps, and snapshot QS.

use super::dedicated_qs::quiescence_snapshot as quiescence;
use super::{
    INF, MATE_SCORE, MAX_PLY, SearchContext, SearchNode, ThreadState, captured_piece_index,
    draw_score, gives_direct_check, make_search_move_no_undo, nmp_material_ok, piece_index_on,
    score_from_tt, score_to_tt, search_time_exceeded, store_ateed_signal, store_killer,
    undo_search_eval, usable_tt_move,
};
use crate::move_picker::MovePicker;
use crate::policy::{
    AteedBadNoisyFutilityPolicy, AteedFutilityPolicy, AteedLmpPolicy, AteedLmrPolicy,
    AteedRfpPolicy, BadNoisyFutilityContext, BadNoisyFutilityPolicy, FutilityContext,
    FutilityPolicy, LmpContext, LmpPolicy, LmrContext, LmrPolicy, MoveOrderingProfile, RfpContext,
    RfpPolicy, ateed_moe_lmr_delta,
};
use crate::see;
use crate::tt::{NodeType, TTData};
use std::sync::atomic::Ordering;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

#[inline(always)]
fn hint_common_access_once(
    board: &Board,
    state: &mut ThreadState,
    use_nnue: bool,
    ready: &mut bool,
) {
    if use_nnue && !*ready {
        state.nnue_state.hint_common_access(board);
        *ready = true;
    }
}

#[inline(always)]
fn same_move(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}

fn child_moe_signal(
    board: &Board,
    state: &mut ThreadState,
    use_nnue: bool,
    parent_expert: usize,
) -> (bool, i32, i32) {
    if !use_nnue {
        return (false, 0, 0);
    }
    #[cfg(feature = "ateed-nnue")]
    if let Some(signal) = state.nnue_state.evaluate_ateed_search_signal(board) {
        return (
            signal.expert != parent_expert,
            signal.draw_mass,
            signal.gate_margin,
        );
    }
    (false, 0, 0)
}

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
        eval_mode,
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
    let mut acc_ready = false;
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

    let (static_eval, variance, parent_expert, draw_mass, gate_margin) = if in_check || singular {
        store_ateed_signal(
            state,
            ply_usize,
            state.eval_variance[ply_usize],
            usize::from(state.eval_expert[ply_usize]),
            state.eval_draw_mass[ply_usize],
            state.eval_gate_margin[ply_usize],
        );
        (
            state.static_evals[ply_usize],
            state.eval_variance[ply_usize],
            usize::from(state.eval_expert[ply_usize]),
            state.eval_draw_mass[ply_usize],
            state.eval_gate_margin[ply_usize],
        )
    } else {
        let (raw, variance, expert, draw_mass, gate_margin) =
            super::ateed_eval_or_reuse(board, state, use_nnue, raw_eval);
        if raw_eval.is_none() {
            raw_eval = Some(raw);
            acc_ready = use_nnue;
        }
        store_ateed_signal(state, ply_usize, variance, expert, draw_mass, gate_margin);
        let corr = state.correction(board, move_ordering);
        let corrected = super::corrected_network_eval(
            board,
            raw,
            corr,
            state.optimism[board.side_to_move.index()],
            eval_mode,
        );
        (corrected, variance, expert, draw_mass, gate_margin)
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
    let improvement = if improving && ply_usize > 1 {
        static_eval - state.static_evals[ply_usize - 2]
    } else {
        0
    };
    let uncertainty = super::ateed_uncertainty_margin(eval_mode, variance);

    if !is_pv && !in_check && !singular && beta.abs() < MATE_SCORE - 100 {
        let rfp = AteedRfpPolicy.cutoff_score(
            eval,
            beta + uncertainty,
            &RfpContext {
                depth,
                improving,
                improvement,
                correction_abs: 0,
                tt_was_pv,
                own_pieces_threatened: false,
                stock_margin: params.rfp_margin(depth, improving) + uncertainty,
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
            hint_common_access_once(board, state, use_nnue, &mut acc_ready);
            let r = params.null_move_r(depth, eval, beta) + i32::from(improving);
            if state.use_nnue {
                state.nnue_state.push_null();
            }
            let null_snap = board.snapshot();
            board.make_null_move_without_undo();
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
        let pc_beta = beta + params.probcut_margin;
        let can_probcut = !(tt_depth >= depth - 3 && tt_score.is_some_and(|s| s < pc_beta));
        if can_probcut {
            hint_common_access_once(board, state, use_nnue, &mut acc_ready);
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
                tt.prefetch(board.tt_hash_after(mv));
                let pc_snap = board.snapshot();
                make_search_move_no_undo(board, state, mv);
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

    hint_common_access_once(board, state, use_nnue, &mut acc_ready);
    let us = board.side_to_move;
    let threats = std::cell::Cell::new(0u64);
    let threats_ready = std::cell::Cell::new(false);
    let resolve_threats = |b: &Board| {
        if !threats_ready.get() {
            threats.set(super::opponent_threats(b).all);
            threats_ready.set(true);
        }
        threats.get()
    };
    let killers = state.killers[ply_usize];
    let mut picker =
        MovePicker::new(board, tt_move, killers, NULL_MOVE).with_move_ordering(move_ordering);
    let score_capture = |b: &Board, mv: Move| super::capture_score(b, mv, tt_move, move_ordering);
    let history_ptr = std::ptr::from_ref(&state.history);
    let us_idx = us.index();
    let score_quiet = |b: &Board, mv: Move| {
        let threat_map = resolve_threats(b);
        // SAFETY: history is only read while the picker scores, before make.
        unsafe { (*history_ptr).get(threat_map, us_idx, mv) }
    };

    let mut best_score = -INF;
    let mut best_move = tt_move.unwrap_or(NULL_MOVE);
    let mut bound = NodeType::UpperBound;
    let mut legal = 0usize;
    let mut quiets: [Move; 32] = [NULL_MOVE; 32];
    let mut quiets_len = 0usize;
    let can_lmr = depth > 1 && !in_check;
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
        let mv_stat_score = state.stat_score(
            mv,
            us.index(),
            piece_index_on(board, mv.from),
            ply_usize,
            move_ordering,
        );
        if !is_pv && !in_check && best_score.abs() < MATE_SCORE - 100 {
            if is_quiet
                && variance < 1_500
                && AteedLmpPolicy
                    .decision(&LmpContext {
                        depth,
                        move_count: legal + 1,
                        improvement,
                        improving,
                        is_root,
                        is_pv,
                        in_check,
                        is_quiet: true,
                        best_score,
                        stock_depth_limit: params.lmp_depth_limit,
                        stock_move_threshold: 99,
                    })
                    .is_some()
            {
                picker.skip_quiets();
                continue;
            }
            if is_quiet
                && depth <= params.hist_prune_depth_limit
                && legal > 0
                && mv_stat_score < params.hist_prune_margin * depth
            {
                continue;
            }
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
                move_count: legal + 1,
                best_score,
                gives_direct_check: is_quiet && gives_direct_check(board, mv),
                stock_depth_limit: params.futility_depth_limit,
                stock_margin: params.futility_margin(depth, improving) + uncertainty,
            };
            if let Some(decision) = AteedFutilityPolicy.decision(&futility_context) {
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
                gives_direct_check: gives_direct_check(board, mv),
            };
            if let Some(score_floor) = AteedBadNoisyFutilityPolicy.score_floor(&bad_noisy_context) {
                if best_score.abs() < MATE_SCORE - 100 {
                    best_score = best_score.max(score_floor);
                }
                continue;
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
        tt.prefetch(board.tt_hash_after(mv));
        let child_snap = board.snapshot();
        make_search_move_no_undo(board, state, mv);
        let gives_check = board.in_check();
        if ply_usize + 1 < MAX_PLY {
            state.in_check[ply_usize + 1] = gives_check;
        }
        state.prev_move[ply_usize] = mv;
        state.prev_piece[ply_usize] = moved_piece;
        legal += 1;

        let mut reduce = 0;
        if can_lmr && (is_quiet || AteedLmrPolicy.reduce_noisy_moves()) && legal > 1 {
            reduce = AteedLmrPolicy.adjust_reduction(
                0,
                &LmrContext {
                    depth,
                    move_count: legal,
                    is_quiet,
                    is_pv,
                    improving,
                    improvement,
                    alpha_raises: 0,
                    is_killer: super::is_killer(mv, &killers),
                    gives_check,
                    is_recapture: ply > 0
                        && state.prev_move[ply_usize.saturating_sub(1)].is_capture()
                        && mv.is_capture()
                        && state.prev_move[ply_usize.saturating_sub(1)].to == mv.to,
                    mv_stat_score,
                    corr_abs: 0,
                    is_cut_node: allow_null,
                    winning_beta: beta >= MATE_SCORE - 100,
                    tt_was_pv,
                    tt_score_above_alpha: tt_score.is_some_and(|score| score > alpha),
                    tt_score_below_alpha: tt_score.is_some_and(|score| score < alpha),
                    tt_depth_sufficient: tt_score.is_some() && tt_depth >= depth,
                    tt_move_missing: tt_move.is_none(),
                    tt_capture: tt_move.is_some_and(|m| m.is_capture()),
                    hist_lmr_div: params.hist_lmr_div,
                    lmr_corr_mul: params.lmr_corr_mul,
                    lmr_cut_node_bonus: params.lmr_cut_node_bonus,
                    child_cutoffs: state.cutoffs[(ply_usize + 1).min(MAX_PLY - 1)],
                },
            );
            let (expert_changed, child_draw, child_gate) =
                child_moe_signal(board, state, use_nnue, parent_expert);
            reduce -= ateed_moe_lmr_delta(
                variance,
                expert_changed,
                draw_mass.max(child_draw),
                gate_margin.max(child_gate),
            );
            reduce = reduce.max(0);
        }

        let reduced_depth = if reduce > 0 {
            super::reckless_lmr_search_depth(depth + extension, reduce, is_pv)
        } else {
            (depth + extension - 1).max(0)
        };

        let score = if legal == 1 {
            -pvs(
                board,
                state,
                context,
                SearchNode {
                    depth: (depth + extension - 1).max(0),
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
                    depth: reduced_depth,
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
                        depth: (depth + extension - 1).max(0),
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
                let threat_map = resolve_threats(board);
                if is_quiet {
                    store_killer(&mut state.killers, mv, ply_usize);
                    let bonus = params.history_bonus(depth);
                    let malus = params.history_malus(depth);
                    state.history.update(threat_map, us.index(), mv, bonus);
                    for quiet in quiets[..quiets_len].iter() {
                        state.history.update(threat_map, us.index(), *quiet, -malus);
                    }
                } else if let Some(cap) = captured {
                    state.cap_hist.update(
                        threat_map,
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
