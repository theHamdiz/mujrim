//! Trait-bound eval + search adapters.
//!
//! Each adapter installs a matching evaluator and search stack as one unit so
//! Stockfish / Reckless / Akimbo / Mujrim HCE cannot drift apart at runtime.

use eval::nnue::{ActiveNetwork, NnueSearchProfile};

use crate::engine::SearchEngine;
use crate::search_stack::{
    Lc0SearchProfile, MujrimHceSearchProfile, ObsidianSearchProfile, PlentyChessSearchProfile,
    SearchStack, SearchStackProfile, ViridithasSearchProfile,
};

/// Binds one evaluator to its compatible search composition.
pub trait EvalSearchAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn install(&self, engine: &mut SearchEngine);
}

#[cfg(feature = "stockfish-nnue")]
pub struct StockfishAdapter;
#[cfg(feature = "reckless-nnue")]
pub struct RecklessAdapter;
pub struct AkimboAdapter;
pub struct ViridithasAdapter;
pub struct ObsidianAdapter;
pub struct PlentyChessAdapter;
pub struct Lc0Adapter;
pub struct MujrimHceAdapter;

#[cfg(feature = "stockfish-nnue")]
impl EvalSearchAdapter for StockfishAdapter {
    fn id(&self) -> &'static str {
        "stockfish"
    }

    fn display_name(&self) -> &'static str {
        "Stockfish"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_nnue_network(ActiveNetwork::EmbeddedStockfish);
        engine.set_use_nnue(true);
        debug_assert_eq!(
            engine.eval_mode(),
            crate::search_stack::EvalMode::Nnue(NnueSearchProfile::Stockfish)
        );
    }
}

#[cfg(feature = "reckless-nnue")]
impl EvalSearchAdapter for RecklessAdapter {
    fn id(&self) -> &'static str {
        "reckless"
    }

    fn display_name(&self) -> &'static str {
        "Reckless"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_nnue_network(ActiveNetwork::EmbeddedReckless);
        engine.set_use_nnue(true);
        debug_assert_eq!(
            engine.eval_mode(),
            crate::search_stack::EvalMode::Nnue(NnueSearchProfile::Reckless)
        );
    }
}

impl EvalSearchAdapter for AkimboAdapter {
    fn id(&self) -> &'static str {
        "akimbo"
    }

    fn display_name(&self) -> &'static str {
        "Akimbo"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_nnue_network(ActiveNetwork::Embedded);
        engine.set_use_nnue(true);
        debug_assert_eq!(
            engine.eval_mode(),
            crate::search_stack::EvalMode::Nnue(NnueSearchProfile::Akimbo)
        );
    }
}

impl EvalSearchAdapter for ViridithasAdapter {
    fn id(&self) -> &'static str {
        "viridithas"
    }

    fn display_name(&self) -> &'static str {
        "Viridithas"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_search_stack(ViridithasSearchProfile.compose());
        if let Ok(network) = eval::nnue::load_network_for_preset("viridithas") {
            engine.set_nnue_network(network);
        }
        engine.set_use_nnue(true);
    }
}

impl EvalSearchAdapter for ObsidianAdapter {
    fn id(&self) -> &'static str {
        "obsidian"
    }

    fn display_name(&self) -> &'static str {
        "Obsidian"
    }

    fn install(&self, engine: &mut SearchEngine) {
        match eval::nnue::load_network_for_preset("obsidian") {
            Ok(network) => engine.set_nnue_network(network),
            Err(_) => engine.set_search_stack(ObsidianSearchProfile.compose()),
        }
        engine.set_use_nnue(true);
    }
}

impl EvalSearchAdapter for PlentyChessAdapter {
    fn id(&self) -> &'static str {
        "plentychess"
    }

    fn display_name(&self) -> &'static str {
        "PlentyChess"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_search_stack(PlentyChessSearchProfile.compose());
        if let Ok(network) = eval::nnue::load_network_for_preset("plentychess") {
            engine.set_nnue_network(network);
        }
        engine.set_use_nnue(true);
    }
}

impl EvalSearchAdapter for Lc0Adapter {
    fn id(&self) -> &'static str {
        "lc0"
    }

    fn display_name(&self) -> &'static str {
        "Lc0"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_search_stack(Lc0SearchProfile.compose());
        engine.set_use_nnue(true);
    }
}

impl EvalSearchAdapter for MujrimHceAdapter {
    fn id(&self) -> &'static str {
        "mujrim-hce"
    }

    fn display_name(&self) -> &'static str {
        "Mujrim HCE"
    }

    fn install(&self, engine: &mut SearchEngine) {
        // Classical eval does not require an NNUE payload; keep any loaded net
        // inert while the HCE search stack is active.
        engine.set_search_stack(MujrimHceSearchProfile.compose());
        engine.set_use_nnue(false);
    }
}

/// Resolve a stable adapter id (`stockfish`, `reckless`, `akimbo`, `mujrim-hce` / `hce`).
pub fn adapter_for_id(id: &str) -> Option<&'static dyn EvalSearchAdapter> {
    match id {
        #[cfg(feature = "stockfish-nnue")]
        "stockfish" => Some(&StockfishAdapter),
        #[cfg(feature = "reckless-nnue")]
        "reckless" => Some(&RecklessAdapter),
        "akimbo" => Some(&AkimboAdapter),
        "viridithas" => Some(&ViridithasAdapter),
        "obsidian" => Some(&ObsidianAdapter),
        "plentychess" | "plenty" => Some(&PlentyChessAdapter),
        "lc0" => Some(&Lc0Adapter),
        "mujrim-hce" | "hce" => Some(&MujrimHceAdapter),
        _ => None,
    }
}

/// Install by id; returns false when the id is unknown.
pub fn install_adapter(engine: &mut SearchEngine, id: &str) -> bool {
    let Some(adapter) = adapter_for_id(id) else {
        return false;
    };
    adapter.install(engine);
    true
}

/// Re-compose the search stack for an already-selected NNUE profile without
/// reloading weights (used when only the experiment overlay changes).
pub fn stack_for_nnue_profile(profile: NnueSearchProfile) -> SearchStack {
    SearchStack::for_network(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::MoveOrderingProfile;
    use crate::search_stack::EvalMode;

    #[cfg(feature = "stockfish-nnue")]
    #[test]
    fn stockfish_adapter_pairs_stockfish_net_with_stocklike_stack() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "stockfish"));
        assert!(engine.use_nnue());
        assert_eq!(
            engine.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Stockfish)
        );
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::StockLike
        );
        engine.set_contempt(48);
        assert_eq!(engine.contempt(), 0);
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn reckless_adapter_pairs_reckless_net_with_reckless_stack() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "reckless"));
        assert!(engine.use_nnue());
        assert_eq!(
            engine.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Reckless)
        );
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::Reckless
        );
        engine.set_contempt(48);
        assert_eq!(engine.contempt(), 0);
    }

    #[test]
    fn akimbo_adapter_pairs_akimbo_net_with_stocklike_stack() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "akimbo"));
        assert!(engine.use_nnue());
        assert_eq!(
            engine.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Akimbo)
        );
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::StockLike
        );
        engine.set_contempt(48);
        assert_eq!(engine.contempt(), 0);
    }

    #[test]
    fn mujrim_hce_adapter_disables_nnue_and_uses_hce_stack() {
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "mujrim-hce"));
        assert!(!engine.use_nnue());
        assert_eq!(engine.eval_mode(), EvalMode::MujrimHce);
        assert_eq!(
            engine.search_stack.policies.move_ordering,
            MoveOrderingProfile::StockLike
        );
        engine.set_contempt(48);
        assert_eq!(engine.contempt(), 48);
    }

    #[test]
    fn viridithas_and_obsidian_adapters_install_matching_stacks() {
        let mut viri = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut viri, "viridithas"));
        assert!(viri.use_nnue());
        assert_eq!(
            viri.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Viridithas)
        );
        viri.set_contempt(48);
        assert_eq!(viri.contempt(), 0);

        let mut obs = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut obs, "obsidian"));
        assert_eq!(obs.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Obsidian));
        obs.set_contempt(48);
        assert_eq!(obs.contempt(), 0);
    }

    #[cfg(feature = "viridithas-nnue")]
    #[test]
    fn viridithas_adapter_binds_sandhi_eval_to_viri_search() {
        if eval::nnue::discover_named_network("sandhi-s2-b200.nnue.zst").is_none()
            && eval::nnue::discover_named_network("viri_default.nnue.zst").is_none()
        {
            return;
        }
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "viridithas"));
        assert_eq!(
            engine.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::Viridithas)
        );
        let info = engine.nnue_info();
        assert!(
            info.architecture.contains("sandhi"),
            "viridithas adapter must bind sandhi, got {}",
            info.architecture
        );
        assert_eq!(info.format.to_string(), "Viridithas");
    }

    #[test]
    fn plentychess_and_lc0_adapters_install_matching_stacks() {
        let mut plenty = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut plenty, "plentychess"));
        assert_eq!(
            plenty.eval_mode(),
            EvalMode::Nnue(NnueSearchProfile::PlentyChess)
        );
        if eval::nnue::discover_named_network("plenty_default.bin").is_some()
            || eval::nnue::discover_named_network("0179r.bin").is_some()
        {
            assert_eq!(plenty.nnue_info().format.to_string(), "PlentyChess");
        }
        plenty.set_contempt(48);
        assert_eq!(plenty.contempt(), 0);

        let mut lc0 = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut lc0, "lc0"));
        assert_eq!(lc0.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Lc0));
        assert!(lc0.eval_mode().is_lc0_nnue());
        assert_eq!(
            lc0.search_stack.policies.move_ordering,
            MoveOrderingProfile::Reckless
        );
        lc0.set_contempt(48);
        assert_eq!(lc0.contempt(), 0);
    }

    #[test]
    fn adapter_ids_are_stable() {
        #[cfg(feature = "stockfish-nnue")]
        assert_eq!(StockfishAdapter.id(), "stockfish");
        #[cfg(feature = "reckless-nnue")]
        assert_eq!(RecklessAdapter.id(), "reckless");
        assert_eq!(AkimboAdapter.id(), "akimbo");
        assert_eq!(ViridithasAdapter.id(), "viridithas");
        assert_eq!(ObsidianAdapter.id(), "obsidian");
        assert_eq!(PlentyChessAdapter.id(), "plentychess");
        assert_eq!(Lc0Adapter.id(), "lc0");
        assert_eq!(MujrimHceAdapter.id(), "mujrim-hce");
        assert_eq!(MujrimHceAdapter.display_name(), "Mujrim HCE");
        assert!(adapter_for_id("native").is_none());
    }

    #[test]
    fn hce_alias_resolves() {
        assert!(adapter_for_id("hce").is_some());
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, "hce"));
        assert_eq!(engine.eval_mode(), EvalMode::MujrimHce);
    }

    #[test]
    fn mujrim_hce_smoke_returns_legal_move_without_nnue() {
        types::init();
        let mut engine = SearchEngine::new(4, 1);
        assert!(install_adapter(&mut engine, "mujrim-hce"));
        let mut board = types::Board::new();
        let result = engine.search_nodes(&mut board, 800, 4);
        assert!(result.nodes > 0);
        assert_ne!(result.best_move, types::chess_move::NULL_MOVE);
        let legal = board.generate_legal_moves();
        assert!(legal.iter().any(|mv| *mv == result.best_move));
    }
}
