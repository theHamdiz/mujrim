//! Trait-bound eval + search adapters.
//!
//! Each adapter installs a matching evaluator and search stack as one unit so
//! Stockfish / Reckless / Akimbo / Mujrim HCE cannot drift apart at runtime.

use eval::nnue::{ActiveNetwork, NnueSearchProfile};

use crate::engine::SearchEngine;
use crate::search_stack::{MujrimHceSearchProfile, SearchStack, SearchStackProfile};

/// Binds one evaluator to its compatible search composition.
pub trait EvalSearchAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn install(&self, engine: &mut SearchEngine);
}

pub struct StockfishAdapter;
pub struct RecklessAdapter;
pub struct AkimboAdapter;
pub struct MujrimHceAdapter;

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
        "stockfish" => Some(&StockfishAdapter),
        "reckless" => Some(&RecklessAdapter),
        "akimbo" => Some(&AkimboAdapter),
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
    }

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
    }

    #[test]
    fn adapter_ids_are_stable() {
        assert_eq!(StockfishAdapter.id(), "stockfish");
        assert_eq!(RecklessAdapter.id(), "reckless");
        assert_eq!(AkimboAdapter.id(), "akimbo");
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
