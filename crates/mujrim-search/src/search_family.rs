//! Per-adapter search families.
//!
//! Each adapter compiles its own PVS/qsearch via monomorphization. Policy
//! methods are static for dedicated families; only the experiment/custom
//! fallback still matches `SearchPolicies` enums.

use crate::policy::{
    AkimboFutilityPolicy, AkimboLmpPolicy, AkimboLmrPolicy, AkimboRfpPolicy,
    BadNoisyFutilityContext, BadNoisyFutilityPolicy, DisabledBadNoisyFutilityPolicy,
    FutilityContext, FutilityDecision, FutilityPolicy, LmpContext, LmpDecision, LmpPolicy,
    LmrContext, LmrPolicy, ObsidianFutilityPolicy, ObsidianLmpPolicy, ObsidianLmrPolicy,
    ObsidianRfpPolicy, PlentyChessFutilityPolicy, PlentyChessLmpPolicy, PlentyChessLmrPolicy,
    PlentyChessRfpPolicy, RecklessBadNoisyFutilityPolicy, RecklessFullLmrPolicy,
    RecklessFutilityPolicy, RecklessLmpPolicy, RecklessRfpPolicy, RfpContext, RfpPolicy,
    StockLikeFutilityPolicy, StockLikeLmpPolicy, StockLikeLmrPolicy, StockLikeRfpPolicy,
    ViridithasFutilityPolicy, ViridithasLmpPolicy, ViridithasLmrPolicy, ViridithasRfpPolicy,
};
use crate::search_stack::SearchPolicies;

pub trait SearchFamily: Copy + 'static {
    type Lmr: LmrPolicy + Default;
    type Lmp: LmpPolicy + Default;
    type Futility: FutilityPolicy + Default;
    type BadNoisy: BadNoisyFutilityPolicy + Default;
    type Rfp: RfpPolicy + Default;

    #[inline(always)]
    fn reduce_noisy_moves(_policies: &SearchPolicies) -> bool {
        Self::Lmr::default().reduce_noisy_moves()
    }

    #[inline(always)]
    fn adjust_lmr(base: i32, ctx: &LmrContext, _policies: &SearchPolicies) -> i32 {
        Self::Lmr::default().adjust_reduction(base, ctx)
    }

    #[inline(always)]
    fn lmp_decision(ctx: &LmpContext, _policies: &SearchPolicies) -> Option<LmpDecision> {
        Self::Lmp::default().decision(ctx)
    }

    #[inline(always)]
    fn futility_requires_direct_check(_policies: &SearchPolicies) -> bool {
        Self::Futility::default().requires_direct_check()
    }

    #[inline(always)]
    fn futility_decision(
        ctx: &FutilityContext,
        _policies: &SearchPolicies,
    ) -> Option<FutilityDecision> {
        Self::Futility::default().decision(ctx)
    }

    #[inline(always)]
    fn bad_noisy_requires_direct_check(_policies: &SearchPolicies) -> bool {
        Self::BadNoisy::default().requires_direct_check()
    }

    #[inline(always)]
    fn bad_noisy_score_floor(
        ctx: &BadNoisyFutilityContext,
        _policies: &SearchPolicies,
    ) -> Option<i32> {
        Self::BadNoisy::default().score_floor(ctx)
    }

    #[inline(always)]
    fn rfp_cutoff(
        eval: i32,
        beta: i32,
        ctx: &RfpContext,
        _policies: &SearchPolicies,
    ) -> Option<i32> {
        Self::Rfp::default().cutoff_score(eval, beta, ctx)
    }
}

macro_rules! dedicated_family {
    ($name:ident, $lmr:ty, $lmp:ty, $fut:ty, $bn:ty, $rfp:ty) => {
        #[derive(Clone, Copy)]
        pub struct $name;

        impl SearchFamily for $name {
            type Lmr = $lmr;
            type Lmp = $lmp;
            type Futility = $fut;
            type BadNoisy = $bn;
            type Rfp = $rfp;
        }
    };
}

dedicated_family!(
    HceFamily,
    StockLikeLmrPolicy,
    StockLikeLmpPolicy,
    StockLikeFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    StockLikeRfpPolicy
);
dedicated_family!(
    StockfishFamily,
    StockLikeLmrPolicy,
    StockLikeLmpPolicy,
    StockLikeFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    StockLikeRfpPolicy
);
dedicated_family!(
    AkimboFamily,
    AkimboLmrPolicy,
    AkimboLmpPolicy,
    AkimboFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    AkimboRfpPolicy
);
const _: AkimboFamily = AkimboFamily;
dedicated_family!(
    RecklessFamily,
    RecklessFullLmrPolicy,
    RecklessLmpPolicy,
    RecklessFutilityPolicy,
    RecklessBadNoisyFutilityPolicy,
    RecklessRfpPolicy
);
dedicated_family!(
    ViridithasFamily,
    ViridithasLmrPolicy,
    ViridithasLmpPolicy,
    ViridithasFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    ViridithasRfpPolicy
);
const _: ViridithasFamily = ViridithasFamily;
dedicated_family!(
    ObsidianFamily,
    ObsidianLmrPolicy,
    ObsidianLmpPolicy,
    ObsidianFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    ObsidianRfpPolicy
);
dedicated_family!(
    PlentyChessFamily,
    PlentyChessLmrPolicy,
    PlentyChessLmpPolicy,
    PlentyChessFutilityPolicy,
    DisabledBadNoisyFutilityPolicy,
    PlentyChessRfpPolicy
);
dedicated_family!(
    AteedFamily,
    RecklessFullLmrPolicy,
    RecklessLmpPolicy,
    RecklessFutilityPolicy,
    RecklessBadNoisyFutilityPolicy,
    RecklessRfpPolicy
);
dedicated_family!(
    Lc0Family,
    RecklessFullLmrPolicy,
    RecklessLmpPolicy,
    RecklessFutilityPolicy,
    RecklessBadNoisyFutilityPolicy,
    RecklessRfpPolicy
);

/// Experiment / custom-policy fallback. Not used by production adapters.
#[derive(Clone, Copy)]
pub struct DynamicFamily;

impl SearchFamily for DynamicFamily {
    type Lmr = StockLikeLmrPolicy;
    type Lmp = StockLikeLmpPolicy;
    type Futility = StockLikeFutilityPolicy;
    type BadNoisy = DisabledBadNoisyFutilityPolicy;
    type Rfp = StockLikeRfpPolicy;

    #[inline(always)]
    fn reduce_noisy_moves(policies: &SearchPolicies) -> bool {
        policies.lmr.reduce_noisy_moves()
    }

    #[inline(always)]
    fn adjust_lmr(base: i32, ctx: &LmrContext, policies: &SearchPolicies) -> i32 {
        policies.lmr.adjust_reduction(base, ctx)
    }

    #[inline(always)]
    fn lmp_decision(ctx: &LmpContext, policies: &SearchPolicies) -> Option<LmpDecision> {
        policies.lmp.decision(ctx)
    }

    #[inline(always)]
    fn futility_requires_direct_check(policies: &SearchPolicies) -> bool {
        policies.futility.requires_direct_check()
    }

    #[inline(always)]
    fn futility_decision(
        ctx: &FutilityContext,
        policies: &SearchPolicies,
    ) -> Option<FutilityDecision> {
        policies.futility.decision(ctx)
    }

    #[inline(always)]
    fn bad_noisy_requires_direct_check(policies: &SearchPolicies) -> bool {
        policies.bad_noisy_futility.requires_direct_check()
    }

    #[inline(always)]
    fn bad_noisy_score_floor(
        ctx: &BadNoisyFutilityContext,
        policies: &SearchPolicies,
    ) -> Option<i32> {
        policies.bad_noisy_futility.score_floor(ctx)
    }

    #[inline(always)]
    fn rfp_cutoff(
        eval: i32,
        beta: i32,
        ctx: &RfpContext,
        policies: &SearchPolicies,
    ) -> Option<i32> {
        policies.rfp.cutoff_score(eval, beta, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search_stack::{EvalMode, SearchPolicies};
    use eval::nnue::NnueSearchProfile;

    #[test]
    fn dedicated_families_skip_enum_dispatch() {
        let stock = SearchPolicies::stock_like();
        let reckless = SearchPolicies::reckless();
        assert!(!StockfishFamily::reduce_noisy_moves(&stock));
        assert!(!HceFamily::reduce_noisy_moves(&stock));
        assert!(RecklessFamily::reduce_noisy_moves(&reckless));
        assert!(AteedFamily::reduce_noisy_moves(&reckless));
        assert!(ViridithasFamily::reduce_noisy_moves(
            &SearchPolicies::viridithas()
        ));
        assert!(ObsidianFamily::reduce_noisy_moves(
            &SearchPolicies::obsidian()
        ));
        assert!(PlentyChessFamily::reduce_noisy_moves(
            &SearchPolicies::plentychess()
        ));
        assert!(!AkimboFamily::reduce_noisy_moves(&SearchPolicies::akimbo()));
    }

    #[test]
    fn every_adapter_has_a_dedicated_family() {
        let modes = [
            EvalMode::MujrimHce,
            EvalMode::Nnue(NnueSearchProfile::Stockfish),
            EvalMode::Nnue(NnueSearchProfile::Akimbo),
            EvalMode::Nnue(NnueSearchProfile::Reckless),
            EvalMode::Nnue(NnueSearchProfile::Viridithas),
            EvalMode::Nnue(NnueSearchProfile::Obsidian),
            EvalMode::Nnue(NnueSearchProfile::PlentyChess),
            EvalMode::Nnue(NnueSearchProfile::Ateed),
            EvalMode::Nnue(NnueSearchProfile::Lc0),
        ];
        for mode in modes {
            assert!(
                SearchPolicies::expected_for(mode).uses_dedicated_loop(mode),
                "{mode:?} must map to a dedicated search family"
            );
        }
    }
}
