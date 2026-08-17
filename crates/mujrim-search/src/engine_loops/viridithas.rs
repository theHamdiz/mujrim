//! Official cosmobobak/viridithas `alpha_beta` (v20.0.0 src/search.rs).

use super::dedicated_qs::quiescence_snapshot as quiescence;
use super::{
    INF, MATE_SCORE, MAX_PLY, SearchContext, SearchNode, ThreadState, captured_piece_index,
    draw_score, gives_direct_check, hybrid_eval, make_search_move_no_undo, nmp_material_ok,
    piece_index_on, score_from_tt, score_to_tt, search_time_exceeded, store_killer,
    undo_search_eval, usable_tt_move,
};
use crate::move_picker::MovePicker;
use crate::policy::{
    FutilityContext, FutilityPolicy, LmpContext, LmpPolicy, LmrContext, LmrPolicy,
    ViridithasFutilityPolicy, ViridithasLmpPolicy, ViridithasLmrPolicy,
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
    state.dbl_exts[ply_usize] = if is_root {
        0
    } else {
        state.dbl_exts[ply_usize.saturating_sub(1)]
    };
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

    let static_eval = if in_check {
        0
    } else if singular {
        state.static_evals[ply_usize]
    } else if let Some(raw) = raw_eval {
        super::corrected_network_eval(
            board,
            raw,
            state.correction(board, move_ordering),
            state.optimism[board.side_to_move.index()],
            eval_mode,
        )
    } else {
        let raw = hybrid_eval(board, state, use_nnue);
        raw_eval = Some(raw);
        acc_ready = use_nnue;
        super::corrected_network_eval(
            board,
            raw,
            state.correction(board, move_ordering),
            state.optimism[board.side_to_move.index()],
            eval_mode,
        )
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

        let tt_tactical = tt_move.is_some_and(|mv| !mv.is_quiet());
        let margin = 65 * depth - i32::from(improving) * 76;
        if !tt_was_pv
            && depth < 9
            && eval >= beta
            && beta.abs() < MATE_SCORE - 100
            && (tt_move.is_none() || tt_tactical)
            && eval - margin >= beta
        {
            return beta + (eval - beta) / 3;
        }

        if allow_null
            && depth > 2
            && nmp_material_ok(board, board.side_to_move)
            && !state.nmp_banned[board.side_to_move.index()]
            && super::viridithas_nmp_static_gate(static_eval, beta, depth, improving)
        {
            hint_common_access_once(board, state, use_nnue, &mut acc_ready);
            let r = 4
                + depth / 3
                + ((static_eval - beta) / 200).min(4)
                + i32::from(tt_move.is_some_and(|mv| mv.is_capture()));
            if state.use_nnue {
                state.nnue_state.push_null();
            }
            let null_snap = board.snapshot();
            board.make_null_move_without_undo();
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
            board.restore_snapshot(null_snap);
            if state.use_nnue {
                state.nnue_state.pop_move();
            }
            if stopped.load(Ordering::Relaxed) {
                return 0;
            }
            if nw >= beta {
                if !super::viridithas_nmp_needs_verify(depth, beta) {
                    return if nw.abs() > MATE_SCORE - 100 {
                        beta
                    } else {
                        nw
                    };
                }
                let us = board.side_to_move.index();
                state.nmp_banned[us] = true;
                let veri = alpha_beta(
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
                state.nmp_banned[us] = false;
                if stopped.load(Ordering::Relaxed) {
                    return 0;
                }
                if veri >= beta {
                    return veri;
                }
            }
        }

        if allow_null && depth >= 3 && beta.abs() < MATE_SCORE - 100 {
            hint_common_access_once(board, state, use_nnue, &mut acc_ready);
            let mut pc_beta = super::viridithas_probcut_beta(beta, improving);
            if tt_score.is_none_or(|ts| ts >= pc_beta) {
                let depth_base = super::viridithas_probcut_depth_base(depth, static_eval, beta);
                let see_pivot = super::viridithas_probcut_see_pivot(pc_beta, static_eval);
                let pc_capture =
                    |b: &Board, mv: Move| super::capture_score(b, mv, tt_move, move_ordering);
                let pc_quiet = |_: &Board, _: Move| 0;
                let mut pc_picker = MovePicker::new(board, tt_move, [NULL_MOVE; 2], NULL_MOVE)
                    .with_move_ordering(move_ordering);
                pc_picker.skip_quiets();
                while let Some(mv) = pc_picker.next(board, &pc_capture, &pc_quiet) {
                    if !see::see_ge(board, mv, see_pivot) {
                        continue;
                    }
                    tt.prefetch(board.tt_hash_after(mv));
                    let pc_snap = board.snapshot();
                    make_search_move_no_undo(board, state, mv);
                    if ply_usize + 1 < MAX_PLY {
                        state.in_check[ply_usize + 1] = board.in_check();
                    }
                    let mut value =
                        -quiescence(board, state, context, -pc_beta, -pc_beta + 1, ply + 1);
                    let mut pc_depth =
                        super::viridithas_adaptive_pc_depth(depth_base, value, pc_beta)
                            .clamp(0, (depth - 1).max(0));
                    let base_pc_depth = depth_base.clamp(0, (depth - 1).max(0));
                    let ada_beta = super::viridithas_ada_beta(pc_beta, base_pc_depth, pc_depth)
                        .clamp(-MATE_SCORE + 101, MATE_SCORE - 101);
                    if value >= pc_beta && pc_depth > 0 {
                        value = -alpha_beta(
                            board,
                            state,
                            context,
                            SearchNode {
                                depth: pc_depth,
                                alpha: -ada_beta,
                                beta: -ada_beta + 1,
                                ply: ply + 1,
                                is_pv: false,
                                is_root: false,
                                excluded_move: None,
                                total_extensions: 0,
                                nominal_depth: depth,
                                allow_null: false,
                            },
                        );
                        if value < ada_beta && pc_beta < ada_beta {
                            pc_depth = base_pc_depth;
                            value = -alpha_beta(
                                board,
                                state,
                                context,
                                SearchNode {
                                    depth: pc_depth,
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
                        } else {
                            pc_beta = ada_beta;
                        }
                    }
                    board.restore_snapshot(pc_snap);
                    undo_search_eval(state);
                    if stopped.load(Ordering::Relaxed) {
                        return 0;
                    }
                    if value >= pc_beta {
                        if value.abs() > MATE_SCORE - 100 {
                            return value;
                        }
                        return value - (pc_beta - beta);
                    }
                }
            }
        }
    }

    if allow_null && !is_pv && tt_move.is_none() && depth >= 8 {
        depth -= 1;
    }

    hint_common_access_once(board, state, use_nnue, &mut acc_ready);
    let us = board.side_to_move;
    let threats = std::cell::Cell::new(0u64);
    let threats_ready = std::cell::Cell::new(false);
    let resolve_threats = |b: &Board| {
        if !threats_ready.get() {
            let all = super::opponent_attacks_all(b);
            threats.set(all);
            threats_ready.set(true);
        }
        threats.get()
    };
    let killers = state.killers[ply_usize];
    let mut picker =
        MovePicker::new(board, tt_move, killers, NULL_MOVE).with_move_ordering(move_ordering);
    let score_capture = |b: &Board, mv: Move| super::capture_score(b, mv, tt_move, move_ordering);
    let history_ptr = std::ptr::from_ref(&state.history);
    let cont_ptr = std::ptr::from_ref(&state.cont_hist);
    let cont2_ptr = std::ptr::from_ref(&state.cont_hist2);
    let prev_move = state.prev_move;
    let prev_piece = state.prev_piece;
    let us_idx = us.index();
    let score_quiet = |b: &Board, mv: Move| {
        let piece = piece_index_on(b, mv.from);
        let threats = resolve_threats(b);
        // SAFETY: history tables are only read while the picker scores, before make.
        let mut score = unsafe { (*history_ptr).get(threats, us_idx, mv) };
        if ply_usize > 0 && prev_move[ply_usize - 1] != NULL_MOVE {
            let pp = prev_piece[ply_usize - 1];
            let pt = prev_move[ply_usize - 1].to.index();
            score += i32::from(unsafe { (*cont_ptr)[pp][pt][piece][mv.to.index()] });
        }
        if ply_usize > 1 && prev_move[ply_usize - 2] != NULL_MOVE {
            let pp = prev_piece[ply_usize - 2];
            let pt = prev_move[ply_usize - 2].to.index();
            score += i32::from(unsafe { (*cont2_ptr)[pp][pt][piece][mv.to.index()] });
        }
        score
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
        && depth >= 6 + i32::from(tt_was_pv)
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
        let lmr_depth = super::viridithas_preview_lmr_depth(depth, legal, tt_was_pv);
        if !is_root && !is_pv && !in_check && best_score > -MATE_SCORE + 100 {
            if lmr_depth < 9
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
                picker.skip_quiets();
                if is_quiet {
                    continue;
                }
            }
            let hist = state.stat_score(
                mv,
                us_idx,
                piece_index_on(board, mv.from),
                ply_usize,
                move_ordering,
            );
            if is_quiet
                && !super::is_killer(mv, &killers)
                && lmr_depth < 7
                && hist / 32 < -3186 * (depth - 1)
            {
                picker.skip_quiets();
                continue;
            }
            if is_quiet
                && ViridithasFutilityPolicy
                    .decision(&FutilityContext {
                        depth: lmr_depth,
                        eval: static_eval,
                        alpha,
                        history: hist,
                        improving,
                        is_root,
                        is_pv,
                        in_check,
                        is_quiet: true,
                        move_count: legal + 1,
                        best_score,
                        gives_direct_check: gives_direct_check(board, mv),
                        stock_depth_limit: 6,
                        stock_margin: 0,
                    })
                    .is_some()
            {
                picker.skip_quiets();
                continue;
            }
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
            let s_beta = super::viridithas_singular_beta(tt_sc, depth);
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
                    allow_null,
                },
            );
            match super::viridithas_singular_verdict(super::ViridithasSingularInput {
                s_score,
                s_beta,
                tt_score: tt_sc,
                beta,
                is_pv,
                is_cut: allow_null,
                is_quiet,
                dextensions: state.dbl_exts[ply_usize],
            }) {
                super::ViridithasSingular::MultiCut(score) => return score,
                super::ViridithasSingular::Extend(ext) => {
                    extension = ext;
                    if ext >= 2 {
                        state.dbl_exts[ply_usize] += 1;
                    }
                }
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

        let lmr_ready = depth > 2
            && legal > 1 + usize::from(is_root)
            && (is_quiet || ViridithasLmrPolicy.reduce_noisy_moves());
        let mut whole_reduction = 0;
        let child_depth = if legal == 1 {
            state.reductions[ply_usize] = 0;
            depth + extension - 1
        } else {
            let table_1024 = if lmr_ready {
                ViridithasLmrPolicy
                    .reduction_1024ths(&LmrContext {
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
                    })
                    .max(0)
            } else {
                0
            };
            let (stored, whole) = super::viridithas_later_move_reduction(lmr_ready, table_1024);
            whole_reduction = whole;
            state.reductions[ply_usize] = stored;
            super::viridithas_lmr_search_depth(depth, extension, whole)
        };

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
            state.reductions[ply_usize] = 1024;
            let mut new_depth = depth + extension;
            if zw > alpha && whole_reduction > 1 {
                new_depth =
                    super::viridithas_research_depth(new_depth, whole_reduction, zw, best_score);
                if new_depth - 1 > child_depth {
                    zw = -alpha_beta(
                        board,
                        state,
                        context,
                        SearchNode {
                            depth: new_depth - 1,
                            alpha: -alpha - 1,
                            beta: -alpha,
                            ply: ply + 1,
                            is_pv: false,
                            is_root: false,
                            excluded_move: None,
                            total_extensions: 0,
                            nominal_depth: depth,
                            allow_null: !allow_null,
                        },
                    );
                }
            } else if zw > alpha {
                new_depth =
                    super::viridithas_research_depth(new_depth, whole_reduction, zw, best_score);
            }
            if zw > alpha && zw < beta {
                zw = -alpha_beta(
                    board,
                    state,
                    context,
                    SearchNode {
                        depth: new_depth - 1,
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

        board.restore_snapshot(child_snap);
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
                    let threats = resolve_threats(board);
                    state.history.update(threats, us.index(), mv, bonus);
                    for quiet in quiets[..quiets_len].iter() {
                        state.history.update(threats, us.index(), *quiet, -malus);
                    }
                } else if let Some(cap) = captured {
                    let threats = resolve_threats(board);
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
