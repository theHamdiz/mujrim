use std::collections::HashMap;
use std::sync::Arc;
use types::chess_move::NULL_MOVE;
use types::{Move, Piece};

/// Allocation-free noisy-move ordering profiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveOrderingProfile {
    #[default]
    StockLike,
    Reckless,
}

impl MoveOrderingProfile {
    #[inline(always)]
    pub const fn piece_value(self, piece: Piece) -> i32 {
        match self {
            Self::StockLike => match piece {
                Piece::Pawn => 100,
                Piece::Knight => 320,
                Piece::Bishop => 330,
                Piece::Rook => 500,
                Piece::Queen => 900,
                Piece::King => 20_000,
            },
            Self::Reckless => match piece {
                Piece::Pawn => 109,
                Piece::Knight => 403,
                Piece::Bishop => 435,
                Piece::Rook => 679,
                Piece::Queen => 1_242,
                Piece::King => 0,
            },
        }
    }

    #[inline(always)]
    pub fn noisy_score(
        self,
        victim: Option<Piece>,
        attacker: Option<Piece>,
        en_passant: bool,
        promotion: bool,
        history: i32,
        stock_history_divisor: i32,
    ) -> i32 {
        match self {
            Self::StockLike => {
                let victim =
                    victim.map_or(i32::from(en_passant) * 100, |piece| self.piece_value(piece));
                let attacker = attacker.map_or(0, |piece| self.piece_value(piece));
                victim * 10 - attacker
                    + history / stock_history_divisor.max(1)
                    + i32::from(promotion) * 900
            }
            Self::Reckless => {
                let captured =
                    victim.map_or(i32::from(en_passant) * 109, |piece| self.piece_value(piece));
                16 * captured + history
            }
        }
    }

    #[inline(always)]
    pub fn noisy_see_threshold(self, score: i32) -> i32 {
        match self {
            Self::StockLike => 0,
            Self::Reckless => -score / 46 + 109,
        }
    }
}

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
    pub depth: i32,
    pub move_count: usize,
    pub is_quiet: bool,
    pub is_pv: bool,
    pub improving: bool,
    pub improvement: i32,
    pub alpha_raises: i32,
    pub is_killer: bool,
    pub gives_check: bool,
    pub is_recapture: bool,
    pub mv_stat_score: i32,
    pub corr_abs: i32,
    pub is_cut_node: bool,
    pub winning_beta: bool,
    pub tt_was_pv: bool,
    pub tt_score_above_alpha: bool,
    pub tt_score_below_alpha: bool,
    pub tt_depth_sufficient: bool,
    pub tt_move_missing: bool,
    pub hist_lmr_div: i32,
    pub lmr_corr_mul: i32,
    pub lmr_cut_node_bonus: i32,
    /// Cutoff count at the next ply — reduce less when child had few cutoffs.
    pub child_cutoffs: u32,
}

/// Pluggable LMR adjustment policy.
pub trait LmrPolicy {
    /// Whether later captures and promotions use the LMR model.
    fn reduce_noisy_moves(&self) -> bool {
        false
    }

    /// Adjusts base reduction using search context.
    fn adjust_reduction(&self, base_reduction: i32, ctx: &LmrContext) -> i32;
}

/// Default LMR policy tuned for Mujrim's current heuristics.
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
        reduction -= ctx.mv_stat_score / hist_div;

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

/// The strict-tested Reckless LMR core, expressed in whole-ply reductions.
#[derive(Default)]
pub struct RecklessLmrPolicy;

impl LmrPolicy for RecklessLmrPolicy {
    fn reduce_noisy_moves(&self) -> bool {
        true
    }

    fn adjust_reduction(&self, _base_reduction: i32, ctx: &LmrContext) -> i32 {
        let depth_log = (ctx.depth.max(1) as u32).ilog2() as i32;
        let move_log = (ctx.move_count.max(1) as u32).ilog2() as i32;
        let mut reduction = 250 * depth_log * move_log - 65 * ctx.move_count as i32;
        reduction -= 3183 * ctx.corr_abs / 1024;

        if ctx.is_quiet {
            reduction += 1972 - 154 * ctx.mv_stat_score / 1024;
        } else {
            reduction += 1452 - 109 * ctx.mv_stat_score / 1024;
        }
        if ctx.is_pv {
            reduction -= 411;
        }
        if ctx.tt_was_pv {
            reduction -= 371;
        }
        if ctx.is_cut_node && !ctx.tt_was_pv {
            reduction += 1762;
        }
        if ctx.winning_beta {
            reduction += 1024;
        }
        if !ctx.improving {
            reduction += 438;
        }
        if ctx.gives_check {
            reduction -= 966;
        }
        if ctx.child_cutoffs > 2 {
            reduction += 1604;
        }

        reduction / 1024
    }
}

/// Reckless v0.10-dev's v60-era contextual LMR model.
///
/// Kept as a separate policy so its additional context terms remain directly
/// testable against the compact model.
#[derive(Default)]
pub struct RecklessFullLmrPolicy;

impl LmrPolicy for RecklessFullLmrPolicy {
    fn reduce_noisy_moves(&self) -> bool {
        true
    }

    fn adjust_reduction(&self, _base_reduction: i32, ctx: &LmrContext) -> i32 {
        let depth_log = (ctx.depth.max(1) as u32).ilog2() as i32;
        let mut reduction = 269 * depth_log;
        reduction -= (425 * ctx.improvement / 128).clamp(-241, 1155);
        reduction -= 3417 * ctx.corr_abs / 1024;
        reduction += 1412 * i32::from(ctx.alpha_raises > 0);
        reduction += 464 * i32::from(ctx.tt_score_below_alpha);
        reduction += 1024 * i32::from(ctx.winning_beta);

        if ctx.is_quiet {
            reduction += 2171 - 179 * ctx.mv_stat_score / 1024;
        } else {
            reduction += 1426 - 130 * ctx.mv_stat_score / 1024;
        }
        if ctx.is_pv {
            reduction -= 519;
        }
        if ctx.tt_was_pv {
            reduction -= 333;
            reduction -= 611 * i32::from(ctx.tt_score_above_alpha);
            reduction -= 685 * i32::from(ctx.tt_depth_sufficient);
        } else if ctx.is_cut_node {
            reduction += 1852;
            reduction += 2204 * i32::from(ctx.tt_move_missing);
        }
        if ctx.gives_check {
            reduction -= 955;
        }
        if ctx.child_cutoffs > 2 {
            reduction += 1151;
            reduction += 400 * i32::from(!ctx.is_pv && !ctx.is_cut_node);
        }
        reduction / 1024
    }
}

/// Allocation-free dispatch for production LMR policies, with an explicit
/// dynamic escape hatch for callers that install a custom implementation.
#[derive(Clone, Default)]
pub enum LmrDispatch {
    #[default]
    StockLike,
    Reckless,
    RecklessFull,
    Custom(Arc<dyn LmrPolicy + Send + Sync>),
}

impl LmrDispatch {
    #[inline(always)]
    pub fn reduce_noisy_moves(&self) -> bool {
        match self {
            Self::StockLike => StockLikeLmrPolicy.reduce_noisy_moves(),
            Self::Reckless => RecklessLmrPolicy.reduce_noisy_moves(),
            Self::RecklessFull => RecklessFullLmrPolicy.reduce_noisy_moves(),
            Self::Custom(policy) => policy.reduce_noisy_moves(),
        }
    }

    #[inline(always)]
    pub fn adjust_reduction(&self, base_reduction: i32, context: &LmrContext) -> i32 {
        match self {
            Self::StockLike => StockLikeLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Reckless => RecklessLmrPolicy.adjust_reduction(base_reduction, context),
            Self::RecklessFull => RecklessFullLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Custom(policy) => policy.adjust_reduction(base_reduction, context),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LmpContext {
    pub depth: i32,
    pub move_count: usize,
    pub improvement: i32,
    pub improving: bool,
    pub is_root: bool,
    pub is_pv: bool,
    pub in_check: bool,
    pub is_quiet: bool,
    pub best_score: i32,
    pub stock_depth_limit: i32,
    pub stock_move_threshold: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LmpDecision {
    pub skip_remaining_quiets: bool,
    pub prune_current: bool,
}

pub trait LmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision>;
}

#[derive(Default)]
pub struct StockLikeLmpPolicy;

impl LmpPolicy for StockLikeLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        (!context.is_pv
            && !context.in_check
            && context.depth <= context.stock_depth_limit
            && context.move_count > context.stock_move_threshold
            && context.is_quiet
            && context.best_score > -29_000 + 100)
            .then_some(LmpDecision {
                skip_remaining_quiets: true,
                prune_current: true,
            })
    }
}

#[derive(Default)]
pub struct RecklessLmpPolicy;

impl LmpPolicy for RecklessLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        let threshold =
            (2818 + 78 * context.improvement / 16 + 1351 * context.depth * context.depth) / 1024;
        (!context.is_root
            && !context.in_check
            && context.best_score > -29_000 + 100
            && context.move_count as i32 >= threshold)
            .then_some(LmpDecision {
                skip_remaining_quiets: true,
                prune_current: false,
            })
    }
}

#[derive(Clone, Default)]
pub enum LmpDispatch {
    #[default]
    StockLike,
    Reckless,
    Custom(Arc<dyn LmpPolicy + Send + Sync>),
}

impl LmpDispatch {
    #[inline(always)]
    pub fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        match self {
            Self::StockLike => StockLikeLmpPolicy.decision(context),
            Self::Reckless => RecklessLmpPolicy.decision(context),
            Self::Custom(policy) => policy.decision(context),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FutilityContext {
    pub depth: i32,
    pub eval: i32,
    pub alpha: i32,
    pub history: i32,
    pub improving: bool,
    pub is_root: bool,
    pub is_pv: bool,
    pub in_check: bool,
    pub is_quiet: bool,
    pub move_count: usize,
    pub best_score: i32,
    pub gives_direct_check: bool,
    pub stock_depth_limit: i32,
    pub stock_margin: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FutilityDecision {
    pub skip_remaining_quiets: bool,
    pub score_floor: Option<i32>,
}

pub trait FutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        false
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision>;
}

#[derive(Default)]
pub struct StockLikeFutilityPolicy;

impl FutilityPolicy for StockLikeFutilityPolicy {
    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        (!context.is_pv
            && !context.in_check
            && context.depth <= context.stock_depth_limit
            && context.is_quiet
            && context.move_count > 1
            && context.eval + context.stock_margin <= context.alpha
            && context.best_score > -29_000 + 100)
            .then_some(FutilityDecision {
                skip_remaining_quiets: false,
                score_floor: None,
            })
    }
}

#[derive(Default)]
pub struct RecklessFutilityPolicy;

impl FutilityPolicy for RecklessFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        let value = context.eval
            + 88 * context.depth
            + 63 * context.history / 1024
            + 88 * i32::from(context.eval >= context.alpha)
            - 114;
        (!context.is_root
            && !context.in_check
            && context.is_quiet
            && context.depth < 14
            && value <= context.alpha
            && !context.gives_direct_check
            && context.best_score > -29_000 + 100)
            .then_some(FutilityDecision {
                skip_remaining_quiets: true,
                score_floor: (context.best_score.abs() < 29_000 - 100).then_some(value),
            })
    }
}

#[derive(Clone, Default)]
pub enum FutilityDispatch {
    #[default]
    StockLike,
    Reckless,
    Custom(Arc<dyn FutilityPolicy + Send + Sync>),
}

impl FutilityDispatch {
    #[inline(always)]
    pub fn requires_direct_check(&self) -> bool {
        match self {
            Self::StockLike => StockLikeFutilityPolicy.requires_direct_check(),
            Self::Reckless => RecklessFutilityPolicy.requires_direct_check(),
            Self::Custom(policy) => policy.requires_direct_check(),
        }
    }

    #[inline(always)]
    pub fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        match self {
            Self::StockLike => StockLikeFutilityPolicy.decision(context),
            Self::Reckless => RecklessFutilityPolicy.decision(context),
            Self::Custom(policy) => policy.decision(context),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BadNoisyFutilityContext {
    pub depth: i32,
    pub eval: i32,
    pub alpha: i32,
    pub history: i32,
    pub captured_value: i32,
    pub is_root: bool,
    pub in_check: bool,
    pub is_bad_noisy: bool,
    pub best_score: i32,
    pub gives_direct_check: bool,
}

pub trait BadNoisyFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        false
    }

    fn score_floor(&self, context: &BadNoisyFutilityContext) -> Option<i32>;
}

#[derive(Default)]
pub struct DisabledBadNoisyFutilityPolicy;

impl BadNoisyFutilityPolicy for DisabledBadNoisyFutilityPolicy {
    fn score_floor(&self, _context: &BadNoisyFutilityContext) -> Option<i32> {
        None
    }
}

#[derive(Default)]
pub struct RecklessBadNoisyFutilityPolicy;

impl BadNoisyFutilityPolicy for RecklessBadNoisyFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn score_floor(&self, context: &BadNoisyFutilityContext) -> Option<i32> {
        let value = context.eval + 84 * context.depth + 82 * context.history / 1024 + 24;
        (!context.is_root
            && !context.in_check
            && context.depth < 12
            && context.is_bad_noisy
            && value <= context.alpha
            && !context.gives_direct_check
            && context.best_score > -29_000 + 100)
            .then_some(value)
    }
}

#[derive(Clone, Default)]
pub enum BadNoisyFutilityDispatch {
    #[default]
    Disabled,
    Reckless,
    Custom(Arc<dyn BadNoisyFutilityPolicy + Send + Sync>),
}

impl BadNoisyFutilityDispatch {
    #[inline(always)]
    pub fn requires_direct_check(&self) -> bool {
        match self {
            Self::Disabled => DisabledBadNoisyFutilityPolicy.requires_direct_check(),
            Self::Reckless => RecklessBadNoisyFutilityPolicy.requires_direct_check(),
            Self::Custom(policy) => policy.requires_direct_check(),
        }
    }

    #[inline(always)]
    pub fn score_floor(&self, context: &BadNoisyFutilityContext) -> Option<i32> {
        match self {
            Self::Disabled => DisabledBadNoisyFutilityPolicy.score_floor(context),
            Self::Reckless => RecklessBadNoisyFutilityPolicy.score_floor(context),
            Self::Custom(policy) => policy.score_floor(context),
        }
    }
}

/// Inputs for reverse-futility pruning policies.
#[derive(Clone, Copy, Debug)]
pub struct RfpContext {
    pub depth: i32,
    pub improving: bool,
    pub improvement: i32,
    pub correction_abs: i32,
    pub tt_was_pv: bool,
    pub own_pieces_threatened: bool,
    pub stock_margin: i32,
}

pub trait RfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32>;
}

#[derive(Default)]
pub struct StockLikeRfpPolicy;

impl RfpPolicy for StockLikeRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        (context.depth <= 8 && eval - context.stock_margin >= beta).then_some(eval)
    }
}

#[derive(Default)]
pub struct RecklessRfpPolicy;

impl RfpPolicy for RecklessRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        let margin = (1140 * context.depth * context.depth / 128
            - 120 * context.improvement / 1024
            + 22 * context.depth
            + 669 * context.correction_abs / 1024
            - 54 * i32::from(!context.own_pieces_threatened)
            - 19)
            .max(2);
        (!context.tt_was_pv
            && eval >= beta + margin
            && beta.abs() < 29_000 - 100
            && eval.abs() < 29_000 - 100)
            .then(|| beta + 3055 * (eval - beta) / 10_000)
    }
}

#[derive(Clone, Default)]
pub enum RfpDispatch {
    #[default]
    StockLike,
    Reckless,
    Custom(Arc<dyn RfpPolicy + Send + Sync>),
}

impl RfpDispatch {
    #[inline(always)]
    pub fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        match self {
            Self::StockLike => StockLikeRfpPolicy.cutoff_score(eval, beta, context),
            Self::Reckless => RecklessRfpPolicy.cutoff_score(eval, beta, context),
            Self::Custom(policy) => policy.cutoff_score(eval, beta, context),
        }
    }
}

/// History-aware static-exchange pruning policy derived from Reckless.
pub struct HistorySeePruning;

impl HistorySeePruning {
    #[inline(always)]
    pub fn threshold(is_capture: bool, depth: i32, history: i32) -> i32 {
        if is_capture {
            (-7 * depth * depth - 36 * depth - 39 * history / 1024 + 14).min(0)
        } else {
            (-12 * depth * depth + 56 * depth - 27 * history / 1024 + 27).min(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Square;

    #[test]
    fn reckless_noisy_ordering_matches_source_scale_and_see_threshold() {
        let policy = MoveOrderingProfile::Reckless;
        let score =
            policy.noisy_score(Some(Piece::Queen), Some(Piece::Pawn), false, false, 460, 16);
        assert_eq!(score, 20_332);
        assert_eq!(policy.noisy_see_threshold(score), -333);
        assert_eq!(
            policy.noisy_score(None, Some(Piece::Pawn), true, false, 0, 16),
            1_744
        );
    }

    #[test]
    fn stock_noisy_ordering_preserves_mvv_lva_contract() {
        let policy = MoveOrderingProfile::StockLike;
        assert_eq!(
            policy.noisy_score(Some(Piece::Queen), Some(Piece::Pawn), false, false, 160, 16,),
            8_910
        );
        assert_eq!(policy.noisy_see_threshold(8_910), 0);
    }

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
            depth: 8,
            move_count: 8,
            is_quiet: false,
            is_pv: false,
            improving: false,
            improvement: 0,
            alpha_raises: 0,
            is_killer: false,
            gives_check: false,
            is_recapture: false,
            mv_stat_score: -20_000,
            corr_abs: 0,
            is_cut_node: true,
            winning_beta: false,
            tt_was_pv: false,
            tt_score_above_alpha: false,
            tt_score_below_alpha: false,
            tt_depth_sufficient: false,
            tt_move_missing: false,
            hist_lmr_div: 4096,
            lmr_corr_mul: 448,
            lmr_cut_node_bonus: 2,
            child_cutoffs: 10,
        };
        let good = LmrContext {
            mv_stat_score: 20_000,
            ..bad
        };
        let good_reduction = policy.adjust_reduction(base, &good);
        let bad_reduction = policy.adjust_reduction(base, &bad);
        assert!(
            good_reduction < bad_reduction,
            "good={good_reduction}, bad={bad_reduction}"
        );
    }

    #[test]
    fn lmr_improving_penalty_only_on_cut_nodes() {
        let policy = StockLikeLmrPolicy;
        let base = 3;
        let mut pv_quiet = LmrContext {
            depth: 8,
            move_count: 8,
            is_quiet: true,
            is_pv: true,
            improving: false,
            improvement: 0,
            alpha_raises: 0,
            is_killer: false,
            gives_check: false,
            is_recapture: false,
            mv_stat_score: 0,
            corr_abs: 0,
            is_cut_node: false,
            winning_beta: false,
            tt_was_pv: false,
            tt_score_above_alpha: false,
            tt_score_below_alpha: false,
            tt_depth_sufficient: false,
            tt_move_missing: false,
            hist_lmr_div: 8192,
            lmr_corr_mul: 448,
            lmr_cut_node_bonus: 2,
            child_cutoffs: 10,
        };
        let r_pv = policy.adjust_reduction(base, &pv_quiet);
        pv_quiet.is_cut_node = true;
        let r_cut = policy.adjust_reduction(base, &pv_quiet);
        assert!(
            r_cut > r_pv,
            "non-improving should add reduction only as cut node"
        );
    }

    #[test]
    fn reckless_lmr_uses_integer_source_model() {
        let policy = RecklessLmrPolicy;
        assert!(policy.reduce_noisy_moves());
        assert!(!StockLikeLmrPolicy.reduce_noisy_moves());
        let context = LmrContext {
            depth: 8,
            move_count: 8,
            is_quiet: true,
            is_pv: false,
            improving: true,
            improvement: 0,
            alpha_raises: 0,
            is_killer: false,
            gives_check: false,
            is_recapture: false,
            mv_stat_score: 0,
            corr_abs: 0,
            is_cut_node: true,
            winning_beta: false,
            tt_was_pv: false,
            tt_score_above_alpha: false,
            tt_score_below_alpha: false,
            tt_depth_sufficient: false,
            tt_move_missing: false,
            hist_lmr_div: 1,
            lmr_corr_mul: 1,
            lmr_cut_node_bonus: 0,
            child_cutoffs: 0,
        };
        assert_eq!(policy.adjust_reduction(99, &context), 5);
        assert_eq!(
            policy.adjust_reduction(
                99,
                &LmrContext {
                    winning_beta: true,
                    ..context
                }
            ),
            6
        );
        assert_eq!(
            LmrDispatch::Reckless.adjust_reduction(99, &context),
            policy.adjust_reduction(99, &context)
        );
        assert!(LmrDispatch::Reckless.reduce_noisy_moves());
        assert!(!LmrDispatch::StockLike.reduce_noisy_moves());
        assert_eq!(
            LmrDispatch::Custom(Arc::new(RecklessLmrPolicy)).adjust_reduction(99, &context),
            policy.adjust_reduction(99, &context)
        );
        assert_eq!(
            policy.adjust_reduction(
                99,
                &LmrContext {
                    gives_check: true,
                    ..context
                }
            ),
            4
        );
    }

    #[test]
    fn v60_reckless_lmr_matches_pinned_integer_model() {
        let stable = RecklessLmrPolicy;
        let complete = RecklessFullLmrPolicy;
        let context = LmrContext {
            depth: 8,
            move_count: 8,
            is_quiet: false,
            is_pv: false,
            improving: false,
            improvement: -256,
            alpha_raises: 2,
            is_killer: false,
            gives_check: false,
            is_recapture: true,
            mv_stat_score: 0,
            corr_abs: 0,
            is_cut_node: true,
            winning_beta: false,
            tt_was_pv: false,
            tt_score_above_alpha: false,
            tt_score_below_alpha: true,
            tt_depth_sufficient: false,
            tt_move_missing: true,
            hist_lmr_div: 1,
            lmr_corr_mul: 1,
            lmr_cut_node_bonus: 0,
            child_cutoffs: 0,
        };
        assert_ne!(
            stable.adjust_reduction(0, &context),
            complete.adjust_reduction(0, &context)
        );
        assert_eq!(complete.adjust_reduction(0, &context), 8);
        assert_eq!(
            complete.adjust_reduction(
                0,
                &LmrContext {
                    winning_beta: true,
                    ..context
                }
            ),
            complete.adjust_reduction(0, &context) + 1
        );
    }

    #[test]
    fn reckless_lmp_uses_improvement_adjusted_quadratic_threshold() {
        let policy = RecklessLmpPolicy;
        let context = LmpContext {
            depth: 4,
            move_count: 23,
            improvement: 0,
            improving: false,
            is_root: false,
            is_pv: true,
            in_check: false,
            is_quiet: true,
            best_score: 0,
            stock_depth_limit: 0,
            stock_move_threshold: usize::MAX,
        };
        assert_eq!(
            policy.decision(&context),
            Some(LmpDecision {
                skip_remaining_quiets: true,
                prune_current: false,
            })
        );
        assert_eq!(
            LmpDispatch::Reckless.decision(&context),
            policy.decision(&context)
        );
        assert_eq!(
            LmpDispatch::Custom(Arc::new(RecklessLmpPolicy)).decision(&context),
            policy.decision(&context)
        );
        assert!(
            policy
                .decision(&LmpContext {
                    move_count: 22,
                    ..context
                })
                .is_none()
        );
    }

    #[test]
    fn reckless_futility_uses_history_margin_and_preserves_direct_checks() {
        let policy = RecklessFutilityPolicy;
        let context = FutilityContext {
            depth: 2,
            eval: 0,
            alpha: 100,
            history: 0,
            improving: false,
            is_root: false,
            is_pv: true,
            in_check: false,
            is_quiet: true,
            move_count: 2,
            best_score: 0,
            gives_direct_check: false,
            stock_depth_limit: 0,
            stock_margin: 0,
        };
        assert_eq!(
            policy.decision(&context),
            Some(FutilityDecision {
                skip_remaining_quiets: true,
                score_floor: Some(62),
            })
        );
        assert_eq!(
            FutilityDispatch::Reckless.decision(&context),
            policy.decision(&context)
        );
        assert_eq!(
            FutilityDispatch::Custom(Arc::new(RecklessFutilityPolicy)).decision(&context),
            policy.decision(&context)
        );
        assert!(policy.requires_direct_check());
        assert!(
            policy
                .decision(&FutilityContext {
                    gives_direct_check: true,
                    ..context
                })
                .is_none()
        );
    }

    #[test]
    fn reckless_bad_noisy_futility_matches_source_margin() {
        let policy = RecklessBadNoisyFutilityPolicy;
        let context = BadNoisyFutilityContext {
            depth: 2,
            eval: 0,
            alpha: 200,
            history: 0,
            captured_value: 109,
            is_root: false,
            in_check: false,
            is_bad_noisy: true,
            best_score: 0,
            gives_direct_check: false,
        };
        assert_eq!(policy.score_floor(&context), Some(192));
        assert_eq!(
            BadNoisyFutilityDispatch::Reckless.score_floor(&context),
            policy.score_floor(&context)
        );
        assert_eq!(
            BadNoisyFutilityDispatch::Custom(Arc::new(RecklessBadNoisyFutilityPolicy))
                .score_floor(&context),
            policy.score_floor(&context)
        );
        assert!(policy.requires_direct_check());
        assert!(
            policy
                .score_floor(&BadNoisyFutilityContext {
                    gives_direct_check: true,
                    ..context
                })
                .is_none()
        );
    }

    #[test]
    fn reckless_rfp_uses_quadratic_margin_and_softens_cutoff() {
        let policy = RecklessRfpPolicy;
        let context = RfpContext {
            depth: 2,
            improving: true,
            improvement: 0,
            correction_abs: 0,
            tt_was_pv: false,
            own_pieces_threatened: true,
            stock_margin: 9999,
        };
        assert_eq!(policy.cutoff_score(200, 100, &context), Some(130));
        assert_eq!(
            RfpDispatch::Reckless.cutoff_score(200, 100, &context),
            policy.cutoff_score(200, 100, &context)
        );
        assert_eq!(
            RfpDispatch::Custom(Arc::new(RecklessRfpPolicy)).cutoff_score(200, 100, &context),
            policy.cutoff_score(200, 100, &context)
        );
        assert_eq!(policy.cutoff_score(159, 100, &context), None);
        assert_eq!(
            policy.cutoff_score(
                200,
                100,
                &RfpContext {
                    tt_was_pv: true,
                    ..context
                }
            ),
            None
        );
    }

    #[test]
    fn history_see_threshold_tracks_depth_and_history() {
        assert_eq!(HistorySeePruning::threshold(true, 2, 0), -86);
        assert_eq!(HistorySeePruning::threshold(false, 2, 0), 0);
        assert_eq!(HistorySeePruning::threshold(true, 5, 1024), -380);
        assert_eq!(HistorySeePruning::threshold(false, 5, -1024), 0);
        assert!(
            HistorySeePruning::threshold(false, 8, 0) < HistorySeePruning::threshold(false, 4, 0)
        );
    }
}
