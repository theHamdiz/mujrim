//! Coherent evaluator-specific search-stack composition.

use std::sync::Arc;

use eval::nnue::NnueSearchProfile;

use crate::policy::{
    BadNoisyFutilityDispatch, FutilityDispatch, LmpDispatch, LmrDispatch, MoveOrderingProfile,
    RfpDispatch, TimeManagerProfile,
};
use crate::search_params::SearchParams;

/// Evaluator family bound to a search stack (NNUE profile or Mujrim HCE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalMode {
    Nnue(NnueSearchProfile),
    MujrimHce,
}

impl EvalMode {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nnue(profile) => profile.as_str(),
            Self::MujrimHce => "mujrim-hce",
        }
    }

    #[inline]
    pub const fn is_reckless_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Reckless))
    }

    pub const fn is_stockfish_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Stockfish))
    }

    /// In-process Lc0 has no transformer net; search still uses Reckless
    /// check-extension / root-quiet gates.
    pub const fn is_lc0_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Lc0))
    }

    pub const fn is_viridithas_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Viridithas))
    }

    pub const fn is_akimbo_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Akimbo))
    }

    pub const fn defers_threat_maps(self) -> bool {
        self.is_viridithas_nnue() || self.is_akimbo_nnue()
    }

    pub const fn is_ateed_nnue(self) -> bool {
        matches!(self, Self::Nnue(NnueSearchProfile::Ateed))
    }

    #[inline]
    pub const fn nnue_profile(self) -> Option<NnueSearchProfile> {
        match self {
            Self::Nnue(profile) => Some(profile),
            Self::MujrimHce => None,
        }
    }

    /// Contempt is classical-only. NNUE adapters always search with 0.
    #[inline]
    pub const fn allows_contempt(self) -> bool {
        matches!(self, Self::MujrimHce)
    }

    /// Requested UCI contempt, or 0 when the active evaluator is NNUE.
    #[inline]
    pub fn effective_contempt(self, requested: i32) -> i32 {
        if self.allows_contempt() {
            requested.clamp(-100, 100)
        } else {
            0
        }
    }
}

/// Explicit construction-time policy overlays used by the benchmark harness.
/// The evaluator's parameter profile remains intact so an experiment changes
/// exactly one search component unless `RecklessPolicies` is requested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchExperiment {
    #[default]
    None,
    RecklessLmr,
    RecklessLmp,
    RecklessFutility,
    RecklessBadNoisyFutility,
    RecklessRfp,
    RecklessOrdering,
    RecklessPolicies,
}

impl SearchExperiment {
    pub const UCI_NAMES: [&'static str; 8] = [
        "none",
        "reckless-lmr",
        "reckless-lmp",
        "reckless-futility",
        "reckless-bnfp",
        "reckless-rfp",
        "reckless-ordering",
        "reckless-policies",
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "reckless-lmr" => Some(Self::RecklessLmr),
            "reckless-lmp" => Some(Self::RecklessLmp),
            "reckless-futility" => Some(Self::RecklessFutility),
            "reckless-bnfp" => Some(Self::RecklessBadNoisyFutility),
            "reckless-rfp" => Some(Self::RecklessRfp),
            "reckless-ordering" => Some(Self::RecklessOrdering),
            "reckless-policies" => Some(Self::RecklessPolicies),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RecklessLmr => "reckless-lmr",
            Self::RecklessLmp => "reckless-lmp",
            Self::RecklessFutility => "reckless-futility",
            Self::RecklessBadNoisyFutility => "reckless-bnfp",
            Self::RecklessRfp => "reckless-rfp",
            Self::RecklessOrdering => "reckless-ordering",
            Self::RecklessPolicies => "reckless-policies",
        }
    }
}

/// A profile composes every policy that must be changed as one compatible unit.
/// Composition happens when a network is selected; node-level calls still use
/// allocation-free enum dispatch.
pub trait SearchStackProfile {
    fn eval_mode(&self) -> EvalMode;
    fn parameters(&self) -> SearchParams;
    fn policies(&self) -> SearchPolicies;

    fn compose(&self) -> SearchStack {
        SearchStack::new(self.eval_mode(), self.parameters(), self.policies())
    }
}

#[derive(Clone)]
pub struct SearchPolicies {
    pub(crate) lmr: LmrDispatch,
    pub(crate) lmp: LmpDispatch,
    pub(crate) futility: FutilityDispatch,
    pub(crate) bad_noisy_futility: BadNoisyFutilityDispatch,
    pub(crate) rfp: RfpDispatch,
    pub(crate) move_ordering: MoveOrderingProfile,
    pub(crate) time_manager: TimeManagerProfile,
}

impl SearchPolicies {
    pub(crate) fn stock_like() -> Self {
        Self {
            lmr: LmrDispatch::StockLike,
            lmp: LmpDispatch::StockLike,
            futility: FutilityDispatch::StockLike,
            bad_noisy_futility: BadNoisyFutilityDispatch::Disabled,
            rfp: RfpDispatch::StockLike,
            move_ordering: MoveOrderingProfile::StockLike,
            time_manager: TimeManagerProfile::Stockfish,
        }
    }

    /// Native Akimbo reductions and MVV ordering — not the Reckless LMR hybrid.
    pub(crate) fn akimbo() -> Self {
        Self {
            lmr: LmrDispatch::Akimbo,
            lmp: LmpDispatch::Akimbo,
            futility: FutilityDispatch::Akimbo,
            bad_noisy_futility: BadNoisyFutilityDispatch::Disabled,
            rfp: RfpDispatch::Akimbo,
            move_ordering: MoveOrderingProfile::StockLike,
            time_manager: TimeManagerProfile::Default,
        }
    }

    pub(crate) fn reckless() -> Self {
        Self {
            lmr: LmrDispatch::RecklessFull,
            lmp: LmpDispatch::Reckless,
            futility: FutilityDispatch::Reckless,
            bad_noisy_futility: BadNoisyFutilityDispatch::Reckless,
            rfp: RfpDispatch::Reckless,
            move_ordering: MoveOrderingProfile::Reckless,
            time_manager: TimeManagerProfile::Reckless,
        }
    }

    pub(crate) fn viridithas() -> Self {
        Self {
            lmr: LmrDispatch::Viridithas,
            lmp: LmpDispatch::Viridithas,
            futility: FutilityDispatch::Viridithas,
            bad_noisy_futility: BadNoisyFutilityDispatch::Disabled,
            rfp: RfpDispatch::Viridithas,
            move_ordering: MoveOrderingProfile::StockLike,
            time_manager: TimeManagerProfile::Viridithas,
        }
    }

    pub(crate) fn obsidian() -> Self {
        Self {
            lmr: LmrDispatch::Obsidian,
            lmp: LmpDispatch::Obsidian,
            futility: FutilityDispatch::Obsidian,
            bad_noisy_futility: BadNoisyFutilityDispatch::Disabled,
            rfp: RfpDispatch::Obsidian,
            move_ordering: MoveOrderingProfile::StockLike,
            time_manager: TimeManagerProfile::Default,
        }
    }

    pub(crate) fn plentychess() -> Self {
        Self {
            lmr: LmrDispatch::PlentyChess,
            lmp: LmpDispatch::PlentyChess,
            futility: FutilityDispatch::PlentyChess,
            bad_noisy_futility: BadNoisyFutilityDispatch::Disabled,
            rfp: RfpDispatch::PlentyChess,
            move_ordering: MoveOrderingProfile::StockLike,
            time_manager: TimeManagerProfile::Stockfish,
        }
    }

    pub(crate) fn expected_for(eval_mode: EvalMode) -> Self {
        match eval_mode {
            EvalMode::MujrimHce | EvalMode::Nnue(NnueSearchProfile::Stockfish) => {
                Self::stock_like()
            }
            EvalMode::Nnue(NnueSearchProfile::Akimbo) => Self::akimbo(),
            EvalMode::Nnue(NnueSearchProfile::Reckless)
            | EvalMode::Nnue(NnueSearchProfile::Ateed)
            | EvalMode::Nnue(NnueSearchProfile::Lc0) => Self::reckless(),
            EvalMode::Nnue(NnueSearchProfile::Viridithas) => Self::viridithas(),
            EvalMode::Nnue(NnueSearchProfile::Obsidian) => Self::obsidian(),
            EvalMode::Nnue(NnueSearchProfile::PlentyChess) => Self::plentychess(),
        }
    }

    pub(crate) fn uses_dedicated_loop(&self, eval_mode: EvalMode) -> bool {
        !matches!(self.lmr, LmrDispatch::Custom(_))
            && !matches!(self.lmp, LmpDispatch::Custom(_))
            && !matches!(self.futility, FutilityDispatch::Custom(_))
            && !matches!(self.bad_noisy_futility, BadNoisyFutilityDispatch::Custom(_))
            && !matches!(self.rfp, RfpDispatch::Custom(_))
            && self.same_kind(&Self::expected_for(eval_mode))
    }

    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(&self.lmr) == std::mem::discriminant(&other.lmr)
            && std::mem::discriminant(&self.lmp) == std::mem::discriminant(&other.lmp)
            && std::mem::discriminant(&self.futility) == std::mem::discriminant(&other.futility)
            && std::mem::discriminant(&self.bad_noisy_futility)
                == std::mem::discriminant(&other.bad_noisy_futility)
            && std::mem::discriminant(&self.rfp) == std::mem::discriminant(&other.rfp)
            && self.move_ordering == other.move_ordering
            && self.time_manager == other.time_manager
    }

    fn apply_experiment(&mut self, experiment: SearchExperiment) {
        match experiment {
            SearchExperiment::None => {}
            SearchExperiment::RecklessLmr => self.lmr = LmrDispatch::RecklessFull,
            SearchExperiment::RecklessLmp => self.lmp = LmpDispatch::Reckless,
            SearchExperiment::RecklessFutility => self.futility = FutilityDispatch::Reckless,
            SearchExperiment::RecklessBadNoisyFutility => {
                self.bad_noisy_futility = BadNoisyFutilityDispatch::Reckless;
            }
            SearchExperiment::RecklessRfp => self.rfp = RfpDispatch::Reckless,
            SearchExperiment::RecklessOrdering => {
                self.move_ordering = MoveOrderingProfile::Reckless;
            }
            SearchExperiment::RecklessPolicies => *self = Self::reckless(),
        }
    }
}

#[derive(Clone)]
pub struct SearchStack {
    eval_mode: EvalMode,
    pub params: SearchParams,
    pub(crate) lmr_table: Arc<[[i32; 128]; 128]>,
    pub(crate) policies: SearchPolicies,
}

impl SearchStack {
    fn new(eval_mode: EvalMode, params: SearchParams, policies: SearchPolicies) -> Self {
        let lmr_table = params.build_lmr_table();
        Self {
            eval_mode,
            params,
            lmr_table,
            policies,
        }
    }

    #[must_use]
    pub fn for_network(profile: NnueSearchProfile) -> Self {
        match profile {
            NnueSearchProfile::Akimbo => AkimboSearchProfile.compose(),
            NnueSearchProfile::Stockfish => StockfishSearchProfile.compose(),
            NnueSearchProfile::Reckless => RecklessSearchProfile.compose(),
            NnueSearchProfile::Viridithas => ViridithasSearchProfile.compose(),
            NnueSearchProfile::Obsidian => ObsidianSearchProfile.compose(),
            NnueSearchProfile::PlentyChess => PlentyChessSearchProfile.compose(),
            NnueSearchProfile::Ateed => AteedSearchProfile.compose(),
            NnueSearchProfile::Lc0 => Lc0SearchProfile.compose(),
        }
    }

    /// Explicit UCI/benchmark override. Network-derived selection should use
    /// [`Self::for_network`] so incompatible strings cannot enter that path.
    #[must_use]
    pub fn for_preset_name(name: &str) -> Self {
        match name {
            "stockfish" => StockfishSearchProfile.compose(),
            "reckless" => RecklessSearchProfile.compose(),
            "viridithas" => ViridithasSearchProfile.compose(),
            "obsidian" => ObsidianSearchProfile.compose(),
            "plentychess" | "plenty" => PlentyChessSearchProfile.compose(),
            "ateed" => AteedSearchProfile.compose(),
            "lc0" => Lc0SearchProfile.compose(),
            "mujrim-hce" | "hce" => MujrimHceSearchProfile.compose(),
            "reckless-full-lmr" => {
                let mut stack = AkimboSearchProfile.compose();
                stack.policies.lmr = LmrDispatch::RecklessFull;
                stack
            }
            "reckless-lmp" => {
                let mut stack = AkimboSearchProfile.compose();
                stack.policies.lmp = LmpDispatch::Reckless;
                stack
            }
            "reckless-futility" => {
                let mut stack = AkimboSearchProfile.compose();
                stack.policies.futility = FutilityDispatch::Reckless;
                stack
            }
            _ => AkimboSearchProfile.compose(),
        }
    }

    pub fn replace_parameters(&mut self, params: SearchParams) {
        self.lmr_table = params.build_lmr_table();
        self.params = params;
    }

    pub fn apply_experiment(&mut self, experiment: SearchExperiment) {
        self.policies.apply_experiment(experiment);
    }

    #[inline]
    pub const fn eval_mode(&self) -> EvalMode {
        self.eval_mode
    }

    /// NNUE family when active; `None` for Mujrim HCE.
    #[inline]
    pub const fn network_profile(&self) -> Option<NnueSearchProfile> {
        self.eval_mode.nnue_profile()
    }
}

pub struct AkimboSearchProfile;

impl SearchStackProfile for AkimboSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Akimbo)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("akimbo")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::akimbo()
    }
}

pub struct StockfishSearchProfile;

impl SearchStackProfile for StockfishSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Stockfish)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("stockfish")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::stock_like()
    }
}

pub struct RecklessSearchProfile;

impl SearchStackProfile for RecklessSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Reckless)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("reckless")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::reckless()
    }
}

pub struct ViridithasSearchProfile;

impl SearchStackProfile for ViridithasSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Viridithas)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("viridithas")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::viridithas()
    }
}

pub struct ObsidianSearchProfile;

impl SearchStackProfile for ObsidianSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Obsidian)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("obsidian")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::obsidian()
    }
}

pub struct PlentyChessSearchProfile;

impl SearchStackProfile for PlentyChessSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::PlentyChess)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("plentychess")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::plentychess()
    }
}

pub struct AteedSearchProfile;

impl SearchStackProfile for AteedSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Ateed)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("ateed")
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::reckless()
    }
}

pub struct Lc0SearchProfile;

impl SearchStackProfile for Lc0SearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::Nnue(NnueSearchProfile::Lc0)
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::for_preset_with_repo_tuning("lc0")
    }

    fn policies(&self) -> SearchPolicies {
        // Official Lc0 is passthrough. The in-process fallback has no Lc0
        // net, so Reckless reductions/ordering stay as the search half.
        SearchPolicies::reckless()
    }
}

/// Classical (HCE) evaluator with StockLike policies.
///
/// StockLike keeps full-depth root quiets (no LMR) and budgeted check
/// extensions — both are required for pawn breaks and sacrifices. Reckless
/// policies bury those moves under root LMR.
pub struct MujrimHceSearchProfile;

impl SearchStackProfile for MujrimHceSearchProfile {
    fn eval_mode(&self) -> EvalMode {
        EvalMode::MujrimHce
    }

    fn parameters(&self) -> SearchParams {
        SearchParams::mujrim_hce()
    }

    fn policies(&self) -> SearchPolicies {
        SearchPolicies::stock_like()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stockfish_stack_is_composed_as_one_coherent_unit() {
        let stack = SearchStack::for_network(NnueSearchProfile::Stockfish);
        assert_eq!(
            stack.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Stockfish)
        );
        assert!(stack.eval_mode().is_stockfish_nnue());
        assert!(!stack.eval_mode().is_reckless_nnue());
        assert_eq!(stack.params.nmp_base, 5);
        assert_eq!(stack.params.se_double_ext_margin, 18);
        assert_eq!(stack.params.ldse_depth_max, 6);
        assert_eq!(stack.policies.time_manager, TimeManagerProfile::Stockfish);
        assert!(matches!(stack.policies.lmr, LmrDispatch::StockLike));
        assert!(matches!(stack.policies.lmp, LmpDispatch::StockLike));
        assert!(matches!(
            stack.policies.bad_noisy_futility,
            BadNoisyFutilityDispatch::Disabled
        ));
        assert_eq!(stack.policies.move_ordering, MoveOrderingProfile::StockLike);
    }

    #[test]
    fn mujrim_hce_stack_is_classical_stocklike() {
        let stack = SearchStack::for_preset_name("mujrim-hce");
        assert_eq!(stack.eval_mode(), EvalMode::MujrimHce);
        assert!(stack.network_profile().is_none());
        assert_eq!(stack.policies.move_ordering, MoveOrderingProfile::StockLike);
    }

    #[test]
    fn contempt_is_reserved_for_hce_and_zero_on_nnue() {
        assert!(EvalMode::MujrimHce.allows_contempt());
        assert_eq!(EvalMode::MujrimHce.effective_contempt(48), 48);
        assert_eq!(EvalMode::MujrimHce.effective_contempt(200), 100);
        for profile in [
            NnueSearchProfile::Stockfish,
            NnueSearchProfile::Reckless,
            NnueSearchProfile::Akimbo,
            NnueSearchProfile::Viridithas,
            NnueSearchProfile::Obsidian,
            NnueSearchProfile::PlentyChess,
            NnueSearchProfile::Ateed,
            NnueSearchProfile::Lc0,
        ] {
            let mode = EvalMode::Nnue(profile);
            assert!(!mode.allows_contempt(), "{}", profile.as_str());
            assert_eq!(mode.effective_contempt(48), 0, "{}", profile.as_str());
            assert_eq!(mode.effective_contempt(-90), 0, "{}", profile.as_str());
        }
    }

    #[test]
    fn viridithas_and_obsidian_stacks_keep_matching_profiles() {
        let viri = SearchStack::for_network(NnueSearchProfile::Viridithas);
        assert_eq!(
            viri.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Viridithas)
        );
        assert_eq!(viri.params.lmr_cut_node_bonus, 1);
        assert_eq!(viri.params.lmr_base, 0.99);
        assert_eq!(viri.params.probcut_margin, 176);
        assert_eq!(viri.params.se_depth_min, 5);
        assert_eq!(viri.policies.move_ordering, MoveOrderingProfile::StockLike);
        assert_eq!(viri.policies.time_manager, TimeManagerProfile::Viridithas);
        assert!(matches!(viri.policies.lmr, LmrDispatch::Viridithas));
        assert!(matches!(viri.policies.rfp, RfpDispatch::Viridithas));
        assert!(viri.eval_mode().is_viridithas_nnue());

        let obs = SearchStack::for_preset_name("obsidian");
        assert_eq!(obs.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Obsidian));
        assert_eq!(obs.params.se_depth_min, 4);
        assert_eq!(obs.params.probcut_margin, 180);
        assert!(matches!(obs.policies.lmr, LmrDispatch::Obsidian));

        let plenty = SearchStack::for_preset_name("plentychess");
        assert_eq!(
            plenty.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::PlentyChess)
        );
        assert_eq!(plenty.params.probcut_margin, 190);
        assert!(matches!(plenty.policies.lmr, LmrDispatch::PlentyChess));
        let ateed = SearchStack::for_preset_name("ateed");
        assert_eq!(ateed.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Ateed));
        assert!(ateed.eval_mode().is_ateed_nnue());
        assert!(!ateed.eval_mode().is_reckless_nnue());
        assert_eq!(
            ateed.params.aspiration_window,
            SearchParams::reckless().aspiration_window
        );
        assert_eq!(ateed.policies.move_ordering, MoveOrderingProfile::Reckless);
        let lc0 = SearchStack::for_network(NnueSearchProfile::Lc0);
        assert_eq!(lc0.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Lc0));
        assert!(lc0.eval_mode().is_lc0_nnue());
        assert!(!lc0.eval_mode().is_reckless_nnue());
        assert_eq!(
            lc0.params.lmr_cut_node_bonus,
            SearchParams::lc0().lmr_cut_node_bonus
        );
        assert_eq!(lc0.params.lmp_depth_limit, 3);
        assert_eq!(lc0.policies.move_ordering, MoveOrderingProfile::Reckless);
        assert!(matches!(lc0.policies.lmr, LmrDispatch::RecklessFull));
    }

    #[test]
    fn reckless_stack_keeps_reckless_reductions_and_ordering() {
        let stack = SearchStack::for_network(NnueSearchProfile::Reckless);
        assert_eq!(
            stack.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Reckless)
        );
        assert!(matches!(stack.policies.lmr, LmrDispatch::RecklessFull));
        assert!(matches!(stack.policies.lmp, LmpDispatch::Reckless));
        assert!(matches!(
            stack.policies.futility,
            FutilityDispatch::Reckless
        ));
        assert!(matches!(stack.policies.rfp, RfpDispatch::Reckless));
        assert_eq!(stack.policies.move_ordering, MoveOrderingProfile::Reckless);
        assert_eq!(stack.policies.time_manager, TimeManagerProfile::Reckless);
        assert_eq!(stack.policies.time_manager.soft_max(), 1.05);
    }

    #[test]
    fn akimbo_stack_uses_native_akimbo_dispatch() {
        let stack = SearchStack::for_network(NnueSearchProfile::Akimbo);
        assert!(matches!(stack.policies.lmr, LmrDispatch::Akimbo));
        assert!(matches!(stack.policies.lmp, LmpDispatch::Akimbo));
        assert!(matches!(stack.policies.rfp, RfpDispatch::Akimbo));
        assert!(!stack.policies.lmr.reduce_noisy_moves());
        assert!(stack.eval_mode().is_akimbo_nnue());
        assert!(stack.eval_mode().defers_threat_maps());
    }

    #[test]
    fn parameter_replacement_rebuilds_the_lmr_table_without_changing_policies() {
        let mut stack = SearchStack::for_network(NnueSearchProfile::Stockfish);
        let before = stack.lmr_table[24][24];
        let mut params = stack.params.clone();
        params.lmr_divisor = 1.25;
        stack.replace_parameters(params);
        assert_ne!(stack.lmr_table[24][24], before);
        assert!(matches!(stack.policies.lmr, LmrDispatch::StockLike));
    }

    #[test]
    fn experiment_overlays_one_stockfish_component_without_replacing_parameters() {
        let mut stack = SearchStack::for_network(NnueSearchProfile::Akimbo);
        let akimbo_nmp = stack.params.nmp_base;
        stack.apply_experiment(SearchExperiment::RecklessLmp);

        assert_eq!(stack.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Akimbo));
        assert_eq!(stack.params.nmp_base, akimbo_nmp);
        assert!(matches!(stack.policies.lmp, LmpDispatch::Reckless));
        assert!(matches!(stack.policies.lmr, LmrDispatch::Akimbo));
        assert_eq!(stack.policies.move_ordering, MoveOrderingProfile::StockLike);
        assert!(!stack.policies.uses_dedicated_loop(stack.eval_mode()));
    }

    #[test]
    fn hce_and_ateed_use_dedicated_search_loops() {
        let hce = MujrimHceSearchProfile.compose();
        assert!(hce.policies.uses_dedicated_loop(hce.eval_mode()));
        let ateed = AteedSearchProfile.compose();
        assert!(ateed.policies.uses_dedicated_loop(ateed.eval_mode()));
        let stock = StockfishSearchProfile.compose();
        assert!(stock.policies.uses_dedicated_loop(stock.eval_mode()));
    }
}
