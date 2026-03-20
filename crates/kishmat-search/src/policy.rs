use std::collections::HashMap;
use types::Move;
use types::chess_move::NULL_MOVE;

/// Final result from one root-search thread.
#[derive(Clone, Copy, Debug)]
pub struct ThreadOutcome {
    pub best_move: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub is_main: bool,
}

/// Pluggable root selection policy for Lazy SMP.
pub trait RootSelectionPolicy {
    /// Returns the index of the selected outcome inside `outcomes`.
    fn select(&self, outcomes: &[ThreadOutcome]) -> usize;
}

/// Depth-first, then vote-by-move, then score, with main-thread tie-break.
#[derive(Default)]
pub struct DepthScoreVoteRootSelection;

impl RootSelectionPolicy for DepthScoreVoteRootSelection {
    fn select(&self, outcomes: &[ThreadOutcome]) -> usize {
        if outcomes.is_empty() {
            return 0;
        }

        let max_depth = outcomes.iter().map(|o| o.depth).max().unwrap_or(0);
        let mut candidates: Vec<usize> = outcomes
            .iter()
            .enumerate()
            .filter(|(_, o)| o.depth == max_depth && o.best_move != NULL_MOVE)
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            candidates = outcomes
                .iter()
                .enumerate()
                .filter(|(_, o)| o.depth == max_depth)
                .map(|(i, _)| i)
                .collect();
        }

        if candidates.len() <= 1 {
            return candidates.first().copied().unwrap_or(0);
        }

        let mut vote_map: HashMap<Move, (usize, i32, bool)> = HashMap::new();
        for &idx in &candidates {
            let outcome = outcomes[idx];
            let entry = vote_map
                .entry(outcome.best_move)
                .or_insert((0, i32::MIN, false));
            entry.0 += 1;
            entry.1 = entry.1.max(outcome.score);
            entry.2 |= outcome.is_main;
        }

        let mut voted_move = NULL_MOVE;
        let mut voted_count = 0usize;
        let mut voted_score = i32::MIN;
        let mut voted_has_main = false;
        for (mv, (count, best_score, has_main)) in vote_map {
            if count > voted_count
                || (count == voted_count && best_score > voted_score)
                || (count == voted_count
                    && best_score == voted_score
                    && has_main
                    && !voted_has_main)
            {
                voted_move = mv;
                voted_count = count;
                voted_score = best_score;
                voted_has_main = has_main;
            }
        }

        candidates
            .into_iter()
            .filter(|&idx| outcomes[idx].best_move == voted_move)
            .max_by_key(|&idx| {
                let o = outcomes[idx];
                (o.score, o.nodes, if o.is_main { 1 } else { 0 })
            })
            .unwrap_or(0)
    }
}

/// Prefer the main thread result when available.
///
/// This matches strong-engine Lazy SMP practice: helpers are primarily for TT
/// enrichment, while the principal thread drives root move stability.
#[derive(Default)]
pub struct MainThreadPreferredRootSelection;

impl RootSelectionPolicy for MainThreadPreferredRootSelection {
    fn select(&self, outcomes: &[ThreadOutcome]) -> usize {
        if outcomes.is_empty() {
            return 0;
        }

        if let Some((idx, _)) = outcomes
            .iter()
            .enumerate()
            .find(|(_, o)| o.is_main && o.best_move != NULL_MOVE)
        {
            return idx;
        }

        DepthScoreVoteRootSelection.select(outcomes)
    }
}

/// Inputs for LMR policy adjustment.
#[derive(Clone, Copy, Debug)]
pub struct LmrContext {
    pub is_capture: bool,
    pub is_losing_capture: bool,
    pub is_pv: bool,
    pub improving: bool,
    pub is_killer: bool,
    pub gives_check: bool,
    pub mv_stat_score: i32,
    pub cap_hist_score: i32,
    pub corr_abs: i32,
    pub is_cut_node: bool,
    pub tt_was_pv: bool,
    pub hist_lmr_div: i32,
    pub lmr_corr_mul: i32,
    pub lmr_cut_node_bonus: i32,
    /// Cutoff count at the next ply — reduce less when child had few cutoffs.
    pub child_cutoffs: u32,
}

/// Pluggable LMR adjustment policy.
pub trait LmrPolicy {
    /// Adjusts base reduction using search context.
    fn adjust_reduction(&self, base_reduction: i32, ctx: &LmrContext) -> i32;
}

/// Default LMR policy tuned for KishMat's current heuristics.
#[derive(Default)]
pub struct StockLikeLmrPolicy;

impl LmrPolicy for StockLikeLmrPolicy {
    fn adjust_reduction(&self, base_reduction: i32, ctx: &LmrContext) -> i32 {
        let mut reduction = base_reduction;

        if ctx.is_pv {
            reduction -= 1;
        }
        if !ctx.improving {
            reduction += 1;
        }
        if ctx.is_killer {
            reduction -= 1;
        }
        if ctx.gives_check {
            reduction -= 1;
        }

        let hist_div = ctx.hist_lmr_div.max(1);
        if ctx.is_capture {
            reduction -= ctx.cap_hist_score / (hist_div * 2);
            if ctx.is_losing_capture {
                reduction += 1;
            }
        } else {
            reduction -= ctx.mv_stat_score / hist_div;
        }

        reduction -= ctx.corr_abs / ctx.lmr_corr_mul.max(1);

        if ctx.is_cut_node {
            reduction += ctx.lmr_cut_node_bonus;
        }
        if ctx.tt_was_pv {
            reduction -= 1;
        }

        // Reduce less if the next ply had few fail-highs (Akimbo)
        if ctx.child_cutoffs < 4 {
            reduction -= 1;
        }

        reduction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Square;

    #[test]
    fn root_selector_prefers_deeper_result() {
        let policy = DepthScoreVoteRootSelection;
        let m1 = Move::quiet(Square::E2, Square::E4);
        let m2 = Move::quiet(Square::D2, Square::D4);
        let outcomes = vec![
            ThreadOutcome {
                best_move: m1,
                score: 20,
                depth: 11,
                nodes: 100,
                is_main: true,
            },
            ThreadOutcome {
                best_move: m2,
                score: 10,
                depth: 12,
                nodes: 90,
                is_main: false,
            },
        ];
        let idx = policy.select(&outcomes);
        assert_eq!(idx, 1);
    }

    #[test]
    fn root_selector_uses_vote_then_score() {
        let policy = DepthScoreVoteRootSelection;
        let m1 = Move::quiet(Square::E2, Square::E4);
        let m2 = Move::quiet(Square::D2, Square::D4);
        let outcomes = vec![
            ThreadOutcome {
                best_move: m1,
                score: 20,
                depth: 12,
                nodes: 100,
                is_main: true,
            },
            ThreadOutcome {
                best_move: m2,
                score: 200,
                depth: 12,
                nodes: 90,
                is_main: false,
            },
            ThreadOutcome {
                best_move: m1,
                score: 18,
                depth: 12,
                nodes: 95,
                is_main: false,
            },
        ];
        let idx = policy.select(&outcomes);
        assert_eq!(outcomes[idx].best_move, m1);
    }

    #[test]
    fn main_thread_policy_prefers_main_when_valid() {
        let policy = MainThreadPreferredRootSelection;
        let m1 = Move::quiet(Square::E2, Square::E4);
        let m2 = Move::quiet(Square::D2, Square::D4);
        let outcomes = vec![
            ThreadOutcome {
                best_move: m1,
                score: 10,
                depth: 10,
                nodes: 100,
                is_main: true,
            },
            ThreadOutcome {
                best_move: m2,
                score: 200,
                depth: 12,
                nodes: 100,
                is_main: false,
            },
        ];
        let idx = policy.select(&outcomes);
        assert_eq!(idx, 0);
    }

    #[test]
    fn lmr_policy_reduces_less_for_good_quiets() {
        let policy = StockLikeLmrPolicy;
        let base = 4;
        let bad = LmrContext {
            is_capture: false,
            is_losing_capture: false,
            is_pv: false,
            improving: false,
            is_killer: false,
            gives_check: false,
            mv_stat_score: -20_000,
            cap_hist_score: 0,
            corr_abs: 0,
            is_cut_node: true,
            tt_was_pv: false,
            hist_lmr_div: 4096,
            lmr_corr_mul: 448,
            lmr_cut_node_bonus: 2,
            child_cutoffs: 10,
        };
        let good = LmrContext {
            mv_stat_score: 20_000,
            ..bad
        };
        assert!(policy.adjust_reduction(base, &good) < policy.adjust_reduction(base, &bad));
    }
}
