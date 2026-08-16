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
    pub tt_capture: bool,
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

/// Official Viridithas LMR: reduce noisies, keep PV/check lines, cut-node tax.
#[derive(Default)]
pub struct ViridithasLmrPolicy;

impl LmrPolicy for ViridithasLmrPolicy {
    fn reduce_noisy_moves(&self) -> bool {
        true
    }

    fn adjust_reduction(&self, _base_reduction: i32, ctx: &LmrContext) -> i32 {
        // Official viridithas/src/search.rs LMTable + Config multipliers, in 1024ths.
        let depth = f64::from(ctx.depth.clamp(1, 63));
        let played = f64::from(ctx.move_count.clamp(1, 63) as u32);
        let base = 99.0 / 100.0 * 1024.0;
        let division = 260.0 / 100.0 / 1024.0;
        let mut reduction = (base + depth.ln() * played.ln() / division) as i32;
        reduction += 226;
        reduction += i32::from(!ctx.is_pv) * 987;
        reduction -= i32::from(ctx.tt_was_pv) * 1289;
        reduction += i32::from(ctx.tt_was_pv && ctx.tt_score_below_alpha) * 1136;
        reduction += i32::from(ctx.is_cut_node) * 1601;
        reduction -= ctx.mv_stat_score.saturating_mul(1024) / 17_017;
        reduction -= i32::from(ctx.is_killer) * 775;
        reduction += i32::from(!ctx.improving) * 613;
        reduction += i32::from(ctx.tt_capture) * 999;
        reduction -= i32::from(ctx.gives_check) * 1361;
        reduction -= ctx.corr_abs.saturating_mul(448) / 16_384;
        reduction += ctx.alpha_raises.saturating_mul(384);
        reduction / 1024
    }
}

/// Obsidian keeps SF-shaped whole-ply LMR but still reduces later captures.
#[derive(Default)]
pub struct ObsidianLmrPolicy;

impl LmrPolicy for ObsidianLmrPolicy {
    fn reduce_noisy_moves(&self) -> bool {
        true
    }

    fn adjust_reduction(&self, base_reduction: i32, ctx: &LmrContext) -> i32 {
        let mut reduction = StockLikeLmrPolicy.adjust_reduction(base_reduction, ctx);
        if ctx.is_cut_node {
            reduction += 1;
        }
        reduction
    }
}

/// PlentyChess is Stockfish-family LMR with noisy reductions enabled.
#[derive(Default)]
pub struct PlentyChessLmrPolicy;

impl LmrPolicy for PlentyChessLmrPolicy {
    fn reduce_noisy_moves(&self) -> bool {
        true
    }

    fn adjust_reduction(&self, base_reduction: i32, ctx: &LmrContext) -> i32 {
        StockLikeLmrPolicy.adjust_reduction(base_reduction, ctx)
    }
}

/// Native Akimbo LMR: whole-ply StockLike curve, no Reckless noisy model.
#[derive(Default)]
pub struct AkimboLmrPolicy;

impl LmrPolicy for AkimboLmrPolicy {
    fn adjust_reduction(&self, _base_reduction: i32, ctx: &LmrContext) -> i32 {
        // Official jw1912/akimbo search.rs: 48/248 plus PV/check/cutoff/history.
        let depth = f64::from(ctx.depth.max(1));
        let played = f64::from(ctx.move_count.max(1) as u32);
        let mut reduction = (0.48 + depth.ln() / 2.48 * played.ln()) as i32;
        reduction -= i32::from(ctx.is_pv);
        reduction -= i32::from(ctx.gives_check);
        reduction -= i32::from(ctx.child_cutoffs < 4);
        reduction -= ctx.mv_stat_score / 8192;
        reduction.max(0)
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
    Viridithas,
    Obsidian,
    PlentyChess,
    Akimbo,
    Custom(Arc<dyn LmrPolicy + Send + Sync>),
}

impl LmrDispatch {
    #[inline(always)]
    pub fn reduce_noisy_moves(&self) -> bool {
        match self {
            Self::StockLike => StockLikeLmrPolicy.reduce_noisy_moves(),
            Self::Reckless => RecklessLmrPolicy.reduce_noisy_moves(),
            Self::RecklessFull => RecklessFullLmrPolicy.reduce_noisy_moves(),
            Self::Viridithas => ViridithasLmrPolicy.reduce_noisy_moves(),
            Self::Obsidian => ObsidianLmrPolicy.reduce_noisy_moves(),
            Self::PlentyChess => PlentyChessLmrPolicy.reduce_noisy_moves(),
            Self::Akimbo => AkimboLmrPolicy.reduce_noisy_moves(),
            Self::Custom(policy) => policy.reduce_noisy_moves(),
        }
    }

    #[inline(always)]
    pub fn adjust_reduction(&self, base_reduction: i32, context: &LmrContext) -> i32 {
        match self {
            Self::StockLike => StockLikeLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Reckless => RecklessLmrPolicy.adjust_reduction(base_reduction, context),
            Self::RecklessFull => RecklessFullLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Viridithas => ViridithasLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Obsidian => ObsidianLmrPolicy.adjust_reduction(base_reduction, context),
            Self::PlentyChess => PlentyChessLmrPolicy.adjust_reduction(base_reduction, context),
            Self::Akimbo => AkimboLmrPolicy.adjust_reduction(base_reduction, context),
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

#[derive(Default)]
pub struct ViridithasLmpPolicy;

impl LmpPolicy for ViridithasLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        let depth = context.depth.clamp(1, 11) as f64;
        let threshold = if context.improving {
            (4.0 + 4.0 * depth * depth / 4.5) as usize
        } else {
            (2.5 + 2.0 * depth * depth / 4.5) as usize
        };
        (!context.is_pv
            && !context.in_check
            && !context.is_root
            && context.is_quiet
            && context.best_score > -29_000 + 100
            && context.move_count > threshold)
            .then_some(LmpDecision {
                skip_remaining_quiets: true,
                prune_current: true,
            })
    }
}

#[derive(Default)]
pub struct ObsidianLmpPolicy;

impl LmpPolicy for ObsidianLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        StockLikeLmpPolicy.decision(context)
    }
}

#[derive(Default)]
pub struct PlentyChessLmpPolicy;

impl LmpPolicy for PlentyChessLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        StockLikeLmpPolicy.decision(context)
    }
}

#[derive(Default)]
pub struct AkimboLmpPolicy;

impl LmpPolicy for AkimboLmpPolicy {
    fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        let threshold = (context.depth as usize).saturating_mul(context.depth as usize) + 3;
        (!context.is_root
            && !context.in_check
            && context.depth <= context.stock_depth_limit
            && context.move_count > threshold
            && context.is_quiet
            && context.best_score > -29_000 + 100)
            .then_some(LmpDecision {
                skip_remaining_quiets: true,
                prune_current: true,
            })
    }
}

#[derive(Clone, Default)]
pub enum LmpDispatch {
    #[default]
    StockLike,
    Reckless,
    Viridithas,
    Obsidian,
    PlentyChess,
    Akimbo,
    Custom(Arc<dyn LmpPolicy + Send + Sync>),
}

impl LmpDispatch {
    #[inline(always)]
    pub fn decision(&self, context: &LmpContext) -> Option<LmpDecision> {
        match self {
            Self::StockLike => StockLikeLmpPolicy.decision(context),
            Self::Reckless => RecklessLmpPolicy.decision(context),
            Self::Viridithas => ViridithasLmpPolicy.decision(context),
            Self::Obsidian => ObsidianLmpPolicy.decision(context),
            Self::PlentyChess => PlentyChessLmpPolicy.decision(context),
            Self::Akimbo => AkimboLmpPolicy.decision(context),
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
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        (!context.is_pv
            && !context.in_check
            && !context.gives_direct_check
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

#[derive(Default)]
pub struct ViridithasFutilityPolicy;

impl FutilityPolicy for ViridithasFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        // Official uses LMR-preview depth and skips the rest of the quiets.
        let margin = 86 + 70 * context.depth + context.history / 128;
        (context.depth < 6
            && !context.is_pv
            && !context.in_check
            && !context.gives_direct_check
            && context.is_quiet
            && context.move_count > 1
            && context.eval + margin <= context.alpha
            && context.best_score > -29_000 + 100)
            .then_some(FutilityDecision {
                skip_remaining_quiets: true,
                score_floor: None,
            })
    }
}

#[derive(Default)]
pub struct ObsidianFutilityPolicy;

impl FutilityPolicy for ObsidianFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        StockLikeFutilityPolicy.decision(context)
    }
}

#[derive(Default)]
pub struct PlentyChessFutilityPolicy;

impl FutilityPolicy for PlentyChessFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        StockLikeFutilityPolicy.decision(context)
    }
}

#[derive(Default)]
pub struct AkimboFutilityPolicy;

impl FutilityPolicy for AkimboFutilityPolicy {
    fn requires_direct_check(&self) -> bool {
        true
    }

    fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        StockLikeFutilityPolicy.decision(context)
    }
}

#[derive(Clone, Default)]
pub enum FutilityDispatch {
    #[default]
    StockLike,
    Reckless,
    Viridithas,
    Obsidian,
    PlentyChess,
    Akimbo,
    Custom(Arc<dyn FutilityPolicy + Send + Sync>),
}

impl FutilityDispatch {
    #[inline(always)]
    pub fn requires_direct_check(&self) -> bool {
        match self {
            Self::StockLike => StockLikeFutilityPolicy.requires_direct_check(),
            Self::Reckless => RecklessFutilityPolicy.requires_direct_check(),
            Self::Viridithas => ViridithasFutilityPolicy.requires_direct_check(),
            Self::Obsidian => ObsidianFutilityPolicy.requires_direct_check(),
            Self::PlentyChess => PlentyChessFutilityPolicy.requires_direct_check(),
            Self::Akimbo => AkimboFutilityPolicy.requires_direct_check(),
            Self::Custom(policy) => policy.requires_direct_check(),
        }
    }

    #[inline(always)]
    pub fn decision(&self, context: &FutilityContext) -> Option<FutilityDecision> {
        match self {
            Self::StockLike => StockLikeFutilityPolicy.decision(context),
            Self::Reckless => RecklessFutilityPolicy.decision(context),
            Self::Viridithas => ViridithasFutilityPolicy.decision(context),
            Self::Obsidian => ObsidianFutilityPolicy.decision(context),
            Self::PlentyChess => PlentyChessFutilityPolicy.decision(context),
            Self::Akimbo => AkimboFutilityPolicy.decision(context),
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

#[derive(Default)]
pub struct ViridithasRfpPolicy;

impl RfpPolicy for ViridithasRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        let margin = 65 * context.depth - i32::from(context.improving) * 76;
        (context.depth <= 8 && eval - margin >= beta).then_some(eval)
    }
}

#[derive(Default)]
pub struct ObsidianRfpPolicy;

impl RfpPolicy for ObsidianRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        StockLikeRfpPolicy.cutoff_score(eval, beta, context)
    }
}

#[derive(Default)]
pub struct PlentyChessRfpPolicy;

impl RfpPolicy for PlentyChessRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        StockLikeRfpPolicy.cutoff_score(eval, beta, context)
    }
}

#[derive(Default)]
pub struct AkimboRfpPolicy;

impl RfpPolicy for AkimboRfpPolicy {
    fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        // Official jw1912/akimbo: margin = 94 * depth / (improving ? 2 : 1).
        let margin = 94 * context.depth / if context.improving { 2 } else { 1 };
        (context.depth <= 8 && eval >= beta + margin).then_some(eval)
    }
}

#[derive(Clone, Default)]
pub enum RfpDispatch {
    #[default]
    StockLike,
    Reckless,
    Viridithas,
    Obsidian,
    PlentyChess,
    Akimbo,
    Custom(Arc<dyn RfpPolicy + Send + Sync>),
}

impl RfpDispatch {
    #[inline(always)]
    pub fn cutoff_score(&self, eval: i32, beta: i32, context: &RfpContext) -> Option<i32> {
        match self {
            Self::StockLike => StockLikeRfpPolicy.cutoff_score(eval, beta, context),
            Self::Reckless => RecklessRfpPolicy.cutoff_score(eval, beta, context),
            Self::Viridithas => ViridithasRfpPolicy.cutoff_score(eval, beta, context),
            Self::Obsidian => ObsidianRfpPolicy.cutoff_score(eval, beta, context),
            Self::PlentyChess => PlentyChessRfpPolicy.cutoff_score(eval, beta, context),
            Self::Akimbo => AkimboRfpPolicy.cutoff_score(eval, beta, context),
            Self::Custom(policy) => policy.cutoff_score(eval, beta, context),
        }
    }
}

/// Soft/hard clock profile. Reckless/v60 flag when the default 1.5× soft cap
/// spends the increment; Viridithas official TM is more conservative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TimeManagerProfile {
    #[default]
    Default,
    Reckless,
    Viridithas,
    Stockfish,
}

/// Official-style soft/hard think budget for one move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockBudget {
    pub soft: std::time::Duration,
    pub hard: std::time::Duration,
}

impl TimeManagerProfile {
    pub const fn soft_base(self) -> f64 {
        match self {
            Self::Default => 0.50,
            Self::Reckless => 0.55,
            Self::Viridithas => 0.45,
            Self::Stockfish => 0.50,
        }
    }

    pub const fn soft_min(self) -> f64 {
        match self {
            Self::Default => 0.25,
            Self::Reckless | Self::Viridithas => 0.20,
            Self::Stockfish => 0.22,
        }
    }

    pub const fn soft_max(self) -> f64 {
        match self {
            Self::Default => 1.35,
            Self::Reckless => 1.05,
            Self::Viridithas => 1.00,
            Self::Stockfish => 1.15,
        }
    }

    /// Fischer / cyclic allocation from the matching official engine.
    pub fn allocate(
        self,
        remaining_ms: u64,
        increment_ms: u64,
        movestogo: Option<u64>,
        overhead_ms: u64,
        fullmove_number: u32,
        ply: u32,
    ) -> ClockBudget {
        if remaining_ms.saturating_sub(overhead_ms) < 100 {
            return ClockBudget {
                soft: std::time::Duration::from_millis(10),
                hard: std::time::Duration::from_millis(20),
            };
        }
        let (soft, hard) = match self {
            Self::Reckless => reckless_budget(
                remaining_ms,
                increment_ms,
                movestogo,
                overhead_ms,
                fullmove_number,
            ),
            Self::Viridithas => {
                viridithas_budget(remaining_ms, increment_ms, movestogo, overhead_ms)
            }
            Self::Stockfish => {
                stockfish_budget(remaining_ms, increment_ms, movestogo, overhead_ms, ply)
            }
            Self::Default => default_budget(remaining_ms, increment_ms, movestogo, overhead_ms),
        };
        let hard = hard.max(soft).max(20);
        ClockBudget {
            soft: std::time::Duration::from_millis(soft.max(10)),
            hard: std::time::Duration::from_millis(hard),
        }
    }
}

fn spendable_clock(main: u64, overhead: u64) -> u64 {
    let reserve = (main / 100).clamp(50, 250);
    main.saturating_sub(overhead.saturating_add(reserve)).max(1)
}

fn reckless_budget(
    remaining: u64,
    increment: u64,
    movestogo: Option<u64>,
    overhead: u64,
    fullmove_number: u32,
) -> (u64, u64) {
    let available = spendable_clock(remaining, overhead);
    let fullmove = fullmove_number.max(1) as f64;
    if let Some(moves) = movestogo {
        let base = (available as f64 / moves.max(1) as f64) + 0.75 * increment as f64;
        (
            (base as u64).min(available),
            ((5.0 * base) as u64).min(available),
        )
    } else {
        let soft_scale = 0.0594 - 0.0492 * (-0.0386 * fullmove).exp();
        let soft = (soft_scale * available as f64 + 0.75 * increment as f64) as u64;
        let hard = (0.7281 * available as f64 + 0.75 * increment as f64) as u64;
        (soft.min(available), hard.min(available))
    }
}

fn viridithas_budget(
    remaining: u64,
    increment: u64,
    movestogo: Option<u64>,
    overhead: u64,
) -> (u64, u64) {
    let max_time = (remaining * 600 / 1000).saturating_sub(overhead.max(30));
    let hard = (remaining * 46 / 100).min(max_time);
    if let Some(moves) = movestogo {
        let divisor = moves.clamp(2, 24);
        let opt = (remaining / divisor).min(max_time) * 73 / 100;
        (opt.min(hard), hard)
    } else {
        let computed = remaining / 24 + increment * 94 / 100;
        let computed = computed.saturating_sub(overhead.max(30));
        let opt = (computed.min(max_time) * 73 / 100).min(hard);
        (opt, hard)
    }
}

fn stockfish_budget(
    remaining: u64,
    increment: u64,
    movestogo: Option<u64>,
    overhead: u64,
    ply: u32,
) -> (u64, u64) {
    let mut mtg = movestogo.unwrap_or(50).min(50);
    if remaining < 1_000 {
        mtg = ((remaining as f64) * 0.05) as u64;
        mtg = mtg.max(1);
    }
    let time_left = remaining
        .saturating_add(increment.saturating_mul(mtg.saturating_sub(1)))
        .saturating_sub(overhead.saturating_mul(2 + mtg))
        .max(1);
    let (opt_scale, max_scale) = if movestogo.is_none() {
        let adjust = (0.3272 * (time_left as f64).log10() - 0.4141).max(0.05);
        let log_time = ((remaining.max(1) as f64) / 1000.0).log10();
        let opt_constant = (0.0029869 + 0.00033554 * log_time).min(0.004905);
        let max_constant = (3.3744 + 3.0608 * log_time).max(3.1441);
        let ply_term = (f64::from(ply) + 3.22713).max(1.0).powf(0.46866);
        let opt = (0.012112 + ply_term * opt_constant)
            .min(0.19404 * remaining as f64 / time_left as f64)
            * adjust;
        (opt, 6.873_f64.min(max_constant + f64::from(ply) / 12.352))
    } else {
        (
            ((0.88 + f64::from(ply) / 116.4) / mtg.max(1) as f64)
                .min(0.88 * remaining as f64 / time_left as f64),
            1.3 + 0.11 * mtg as f64,
        )
    };
    let optimum = (opt_scale * time_left as f64).max(1.0) as u64;
    let maximum = optimum.max(
        ((0.8097 * remaining as f64) as u64)
            .saturating_sub(overhead)
            .min((max_scale * optimum as f64) as u64),
    );
    (optimum.min(remaining), maximum.min(remaining))
}

fn default_budget(
    remaining: u64,
    increment: u64,
    movestogo: Option<u64>,
    overhead: u64,
) -> (u64, u64) {
    let available = spendable_clock(remaining, overhead);
    let moves = movestogo.unwrap_or(30).clamp(15, 40);
    let soft = available / moves + increment * 3 / 4;
    let hard = (available / 3).max(soft.saturating_mul(2));
    (soft.min(available), hard.min(available))
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
            tt_capture: false,
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
            tt_capture: false,
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
            tt_capture: false,
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
            tt_capture: false,
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
    fn stocklike_futility_does_not_prune_direct_checks() {
        let policy = StockLikeFutilityPolicy;
        let context = FutilityContext {
            depth: 2,
            eval: 0,
            alpha: 400,
            history: 0,
            improving: false,
            is_root: false,
            is_pv: false,
            in_check: false,
            is_quiet: true,
            move_count: 3,
            best_score: 0,
            gives_direct_check: false,
            stock_depth_limit: 8,
            stock_margin: 80,
        };
        assert!(policy.requires_direct_check());
        assert!(policy.decision(&context).is_some());
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

    fn sample_lmr_context() -> LmrContext {
        LmrContext {
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
            tt_capture: false,
            hist_lmr_div: 8192,
            lmr_corr_mul: 448,
            lmr_cut_node_bonus: 1,
            child_cutoffs: 10,
        }
    }

    #[test]
    fn viridithas_lmp_uses_official_quadratic_table() {
        let quiet = LmpContext {
            depth: 4,
            move_count: 10,
            improvement: 0,
            improving: false,
            is_root: false,
            is_pv: false,
            in_check: false,
            is_quiet: true,
            best_score: 0,
            stock_depth_limit: 8,
            stock_move_threshold: 99,
        };
        assert!(ViridithasLmpPolicy.decision(&quiet).is_some());
        assert!(
            ViridithasLmpPolicy
                .decision(&LmpContext {
                    move_count: 9,
                    ..quiet
                })
                .is_none()
        );
        assert!(
            ViridithasLmpPolicy
                .decision(&LmpContext {
                    improving: true,
                    move_count: 19,
                    ..quiet
                })
                .is_some()
        );
    }

    #[test]
    fn viridithas_policies_lock_official_rfp_lmr_and_futility() {
        let rfp = ViridithasRfpPolicy;
        let context = RfpContext {
            depth: 4,
            improving: true,
            improvement: 0,
            correction_abs: 0,
            tt_was_pv: false,
            own_pieces_threatened: false,
            stock_margin: 0,
        };
        // Official: margin = 65*depth - improving*76 = 260-76 = 184
        assert_eq!(rfp.cutoff_score(300, 100, &context), Some(300));
        assert_eq!(rfp.cutoff_score(283, 100, &context), None);
        assert_eq!(
            RfpDispatch::Viridithas.cutoff_score(300, 100, &context),
            Some(300)
        );
        assert_eq!(
            ViridithasRfpPolicy.cutoff_score(
                300,
                100,
                &RfpContext {
                    own_pieces_threatened: true,
                    ..context
                }
            ),
            Some(300),
            "official Viridithas RFP ignores threat maps, so they can be deferred"
        );

        assert!(ViridithasLmrPolicy.reduce_noisy_moves());
        let mut lmr = sample_lmr_context();
        let stock = StockLikeLmrPolicy.adjust_reduction(3, &lmr);
        assert_eq!(
            ViridithasLmrPolicy.adjust_reduction(3, &lmr),
            stock + 1,
            "cut-node tax"
        );
        lmr.child_cutoffs = 0;
        let few_cutoffs = ViridithasLmrPolicy.adjust_reduction(3, &lmr);
        lmr.child_cutoffs = 10;
        assert_eq!(
            few_cutoffs,
            ViridithasLmrPolicy.adjust_reduction(3, &lmr),
            "official Viridithas LMR has no child-cutoff relief"
        );
        let no_raises = ViridithasLmrPolicy.adjust_reduction(3, &lmr);
        lmr.alpha_raises = 2;
        assert!(
            ViridithasLmrPolicy.adjust_reduction(3, &lmr) > no_raises,
            "official LMR taxes later alpha raises"
        );
        lmr.alpha_raises = 0;
        lmr.tt_capture = true;
        assert!(
            ViridithasLmrPolicy.adjust_reduction(3, &lmr) > no_raises,
            "official LMR taxes a tactical TT move"
        );
        lmr.tt_capture = false;
        lmr.gives_check = true;
        assert_eq!(
            ViridithasLmrPolicy.adjust_reduction(99, &lmr),
            4,
            "official 1024ths LMR with check: (5530-1361)/1024"
        );
        lmr.gives_check = false;
        assert_eq!(
            ViridithasLmrPolicy.adjust_reduction(0, &lmr),
            ViridithasLmrPolicy.adjust_reduction(99, &lmr),
            "official LMR rebuilds from depth/moves, not the whole-ply table"
        );
        assert_eq!(
            ViridithasLmrPolicy.adjust_reduction(0, &lmr),
            5,
            "official non-PV cut-node LMR at depth 8 / move 8"
        );

        let futility = FutilityContext {
            depth: 2,
            eval: 0,
            alpha: 300,
            history: 0,
            improving: false,
            is_root: false,
            is_pv: false,
            in_check: false,
            is_quiet: true,
            move_count: 2,
            best_score: 0,
            gives_direct_check: false,
            stock_depth_limit: 8,
            stock_margin: 0,
        };
        // Official: lmr_depth < 6 and 86 + 70*depth + hist/128; 0+226 <= 300
        assert!(ViridithasFutilityPolicy.decision(&futility).is_some());
        assert!(FutilityDispatch::Viridithas.decision(&futility).is_some());
        assert!(
            ViridithasFutilityPolicy
                .decision(&FutilityContext {
                    depth: 6,
                    ..futility
                })
                .is_none(),
            "official FP only applies when preview LMR depth is below 6"
        );
    }

    #[test]
    fn adapter_lmr_dispatch_reduces_noisies_except_akimbo_and_stocklike() {
        assert!(LmrDispatch::Viridithas.reduce_noisy_moves());
        assert!(LmrDispatch::Obsidian.reduce_noisy_moves());
        assert!(LmrDispatch::PlentyChess.reduce_noisy_moves());
        assert!(!LmrDispatch::Akimbo.reduce_noisy_moves());
        assert!(!LmrDispatch::StockLike.reduce_noisy_moves());
        let context = sample_lmr_context();
        let stock = StockLikeLmrPolicy.adjust_reduction(4, &context);
        assert_eq!(
            LmrDispatch::Obsidian.adjust_reduction(4, &context),
            stock + 1
        );
        assert_eq!(
            LmrDispatch::PlentyChess.adjust_reduction(4, &context),
            stock
        );
        assert_eq!(
            LmrDispatch::Akimbo.adjust_reduction(4, &context),
            2,
            "official Akimbo LMR at depth 8 / move 8"
        );
        let mut few_cutoffs = context;
        few_cutoffs.child_cutoffs = 3;
        assert_eq!(
            LmrDispatch::Akimbo.adjust_reduction(4, &few_cutoffs),
            1,
            "official Akimbo relieves LMR when child cutoffs < 4"
        );
    }

    #[test]
    fn akimbo_rfp_uses_official_margin_and_ignores_threats() {
        let context = RfpContext {
            depth: 4,
            improving: true,
            improvement: 0,
            correction_abs: 0,
            tt_was_pv: false,
            own_pieces_threatened: true,
            stock_margin: 0,
        };
        // Official: margin = 94 * depth / (improving ? 2 : 1) = 188
        assert_eq!(AkimboRfpPolicy.cutoff_score(288, 100, &context), Some(288));
        assert_eq!(AkimboRfpPolicy.cutoff_score(287, 100, &context), None);
        assert_eq!(
            AkimboRfpPolicy.cutoff_score(
                288,
                100,
                &RfpContext {
                    own_pieces_threatened: false,
                    ..context
                }
            ),
            Some(288),
            "official Akimbo RFP ignores threat maps, so they can be deferred"
        );
        assert_eq!(
            RfpDispatch::Akimbo.cutoff_score(288, 100, &context),
            Some(288)
        );
        assert_eq!(
            AkimboRfpPolicy.cutoff_score(
                500,
                100,
                &RfpContext {
                    depth: 9,
                    ..context
                }
            ),
            None
        );
        let not_improving = RfpContext {
            improving: false,
            ..context
        };
        // margin = 94 * 4 = 376; 476 >= 100 + 376
        assert_eq!(
            AkimboRfpPolicy.cutoff_score(476, 100, &not_improving),
            Some(476)
        );
        assert_eq!(AkimboRfpPolicy.cutoff_score(475, 100, &not_improving), None);
    }

    #[test]
    fn akimbo_lmp_uses_quadratic_depth_threshold() {
        let context = LmpContext {
            depth: 3,
            move_count: 13,
            improvement: 0,
            improving: false,
            is_root: false,
            is_pv: false,
            in_check: false,
            is_quiet: true,
            best_score: 0,
            stock_depth_limit: 8,
            stock_move_threshold: 99,
        };
        // threshold = 3*3+3 = 12; move_count 13 prunes
        assert!(AkimboLmpPolicy.decision(&context).is_some());
        assert!(
            AkimboLmpPolicy
                .decision(&LmpContext {
                    move_count: 12,
                    ..context
                })
                .is_none()
        );
        assert_eq!(
            LmpDispatch::Akimbo.decision(&context),
            AkimboLmpPolicy.decision(&context)
        );
    }

    #[test]
    fn time_manager_profiles_cap_soft_time() {
        assert_eq!(TimeManagerProfile::Reckless.soft_max(), 1.05);
        assert_eq!(TimeManagerProfile::Viridithas.soft_max(), 1.00);
        assert_eq!(TimeManagerProfile::Stockfish.soft_max(), 1.15);
        assert_eq!(TimeManagerProfile::Default.soft_max(), 1.35);
        assert!(TimeManagerProfile::Reckless.soft_max() < TimeManagerProfile::Default.soft_max());
        assert!(
            TimeManagerProfile::Viridithas.soft_base() < TimeManagerProfile::Reckless.soft_base()
        );
    }

    #[test]
    fn reckless_fischer_matches_official_soft_hard() {
        let budget = TimeManagerProfile::Reckless.allocate(60_000, 2_000, None, 10, 1, 0);
        let available = 60_000u64 - 10 - 250;
        let soft_scale = 0.0594 - 0.0492 * (-0.0386_f64).exp();
        let expected_soft = (soft_scale * available as f64 + 1_500.0) as u64;
        let expected_hard = (0.7281 * available as f64 + 1_500.0) as u64;
        assert_eq!(budget.soft, std::time::Duration::from_millis(expected_soft));
        assert_eq!(budget.hard, std::time::Duration::from_millis(expected_hard));
        assert!(budget.soft < budget.hard);
        assert!(budget.hard < std::time::Duration::from_millis(60_000));
    }

    #[test]
    fn reckless_cyclic_uses_five_times_base() {
        let budget = TimeManagerProfile::Reckless.allocate(60_000, 2_000, Some(40), 10, 1, 0);
        let available = 60_000u64 - 10 - 250;
        let base = (available as f64 / 40.0) + 1_500.0;
        assert_eq!(
            budget.soft,
            std::time::Duration::from_millis((base as u64).min(available))
        );
        assert_eq!(
            budget.hard,
            std::time::Duration::from_millis(((5.0 * base) as u64).min(available))
        );
    }

    #[test]
    fn viridithas_fischer_uses_official_windows() {
        let budget = TimeManagerProfile::Viridithas.allocate(60_000, 2_000, None, 10, 1, 0);
        let max_time = (60_000u64 * 600 / 1000).saturating_sub(30);
        let hard = (60_000u64 * 46 / 100).min(max_time);
        let computed = (60_000u64 / 24 + 2_000 * 94 / 100).saturating_sub(30);
        let soft = (computed.min(max_time) * 73 / 100).min(hard);
        assert_eq!(budget.soft, std::time::Duration::from_millis(soft));
        assert_eq!(budget.hard, std::time::Duration::from_millis(hard));
    }

    #[test]
    fn stockfish_optimum_stays_under_maximum() {
        let budget = TimeManagerProfile::Stockfish.allocate(60_000, 2_000, None, 10, 1, 0);
        assert!(budget.soft >= std::time::Duration::from_millis(10));
        assert!(budget.soft <= budget.hard);
        assert!(budget.hard <= std::time::Duration::from_millis(60_000));
        let sudden = TimeManagerProfile::Default.allocate(60_000, 0, None, 10, 1, 0);
        assert!(sudden.hard <= std::time::Duration::from_millis(60_000 / 3));
    }

    #[test]
    fn emergency_clock_returns_minimum_think() {
        let budget = TimeManagerProfile::Reckless.allocate(80, 0, None, 10, 1, 0);
        assert_eq!(budget.soft, std::time::Duration::from_millis(10));
        assert_eq!(budget.hard, std::time::Duration::from_millis(20));
    }
}
