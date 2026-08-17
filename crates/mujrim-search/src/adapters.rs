//! Trait-bound eval + search adapters.
//!
//! Each adapter installs a matching evaluator and search stack as one unit so
//! Stockfish / Reckless / Akimbo / Mujrim HCE cannot drift apart at runtime.

use eval::nnue::{ActiveNetwork, NnueNetworkSource, NnueSearchProfile};

use crate::engine::SearchEngine;
use crate::search_stack::{
    AteedSearchProfile, Lc0SearchProfile, MujrimHceSearchProfile, ObsidianSearchProfile,
    PlentyChessSearchProfile, SearchStack, SearchStackProfile, ViridithasSearchProfile,
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
pub struct AteedAdapter;
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

fn bind_matching_disk_network(
    engine: &mut SearchEngine,
    preset: &str,
    expected: NnueSearchProfile,
) {
    match eval::nnue::load_network_for_preset(preset) {
        Ok(network) if network.search_profile() == expected => {
            engine.set_nnue_network(network);
            engine.set_use_nnue(true);
        }
        Ok(_) | Err(_) => engine.set_use_nnue(false),
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
        #[cfg(all(feature = "viridithas-nnue", feature = "embedded-networks"))]
        {
            engine.set_nnue_network(ActiveNetwork::EmbeddedViridithas);
            engine.set_use_nnue(true);
        }
        #[cfg(not(all(feature = "viridithas-nnue", feature = "embedded-networks")))]
        {
            bind_matching_disk_network(engine, "viridithas", NnueSearchProfile::Viridithas);
        }
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
        engine.set_search_stack(ObsidianSearchProfile.compose());
        #[cfg(all(feature = "obsidian-nnue", feature = "embedded-networks"))]
        {
            engine.set_nnue_network(ActiveNetwork::EmbeddedObsidian);
            engine.set_use_nnue(true);
        }
        #[cfg(not(all(feature = "obsidian-nnue", feature = "embedded-networks")))]
        {
            bind_matching_disk_network(engine, "obsidian", NnueSearchProfile::Obsidian);
        }
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
        #[cfg(all(feature = "plentychess-nnue", feature = "embedded-networks"))]
        {
            engine.set_nnue_network(ActiveNetwork::EmbeddedPlentyChess);
            engine.set_use_nnue(true);
        }
        #[cfg(not(all(feature = "plentychess-nnue", feature = "embedded-networks")))]
        {
            bind_matching_disk_network(engine, "plentychess", NnueSearchProfile::PlentyChess);
        }
    }
}

impl EvalSearchAdapter for AteedAdapter {
    fn id(&self) -> &'static str {
        "ateed"
    }

    fn display_name(&self) -> &'static str {
        "Ateed"
    }

    fn install(&self, engine: &mut SearchEngine) {
        engine.set_search_stack(AteedSearchProfile.compose());
        bind_matching_disk_network(engine, "ateed", NnueSearchProfile::Ateed);
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
        // Official lc0 evaluates the sidecar BT4 `.pb.gz`; in-process search
        // must not fall back to Reckless/Stockfish/Akimbo NNUE.
        engine.set_use_nnue(false);
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
        "ateed" => Some(&AteedAdapter),
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
        assert!(engine.eval_mode().is_stockfish_nnue());
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

    fn assert_adapter_owns_its_network(id: &str, expected: NnueSearchProfile, net_present: bool) {
        let mut engine = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut engine, id));
        assert_eq!(engine.eval_mode(), EvalMode::Nnue(expected));
        if net_present {
            assert!(engine.use_nnue(), "{id} must evaluate its own net");
            assert_eq!(engine.nnue_preset_hint(), expected.as_str());
        } else {
            assert!(
                !engine.use_nnue(),
                "{id} must not evaluate a foreign leftover net"
            );
        }
    }

    #[test]
    fn viridithas_and_obsidian_adapters_install_matching_stacks() {
        let viri_net = eval::nnue::discover_named_network("sandhi-s2-b200.nnue.zst").is_some()
            || eval::nnue::discover_named_network("viri_default.nnue.zst").is_some();
        assert_adapter_owns_its_network("viridithas", NnueSearchProfile::Viridithas, viri_net);
        let mut viri = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut viri, "viridithas"));
        viri.set_contempt(48);
        assert_eq!(viri.contempt(), 0);

        let obs_net = eval::nnue::discover_named_network("obs_default.bin").is_some()
            || eval::nnue::discover_named_network("net89perm.bin").is_some();
        assert_adapter_owns_its_network("obsidian", NnueSearchProfile::Obsidian, obs_net);
        let mut obs = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut obs, "obsidian"));
        assert_ne!(
            obs.nnue_info().format.to_string(),
            "Stockfish",
            "obsidian adapter must not bind a Stockfish net"
        );
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
        let plenty_net = cfg!(feature = "plentychess-nnue")
            && (eval::nnue::discover_named_network("plenty_default.bin").is_some()
                || eval::nnue::discover_named_network("0179r.bin").is_some());
        assert_adapter_owns_its_network("plentychess", NnueSearchProfile::PlentyChess, plenty_net);
        let mut plenty = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut plenty, "plentychess"));
        plenty.set_contempt(48);
        assert_eq!(plenty.contempt(), 0);

        let ateed_net = cfg!(feature = "ateed-nnue")
            && eval::nnue::discover_named_network("ateed_default.bin").is_some();
        assert_adapter_owns_its_network("ateed", NnueSearchProfile::Ateed, ateed_net);
        let mut ateed = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut ateed, "ateed"));
        ateed.set_contempt(48);
        assert_eq!(ateed.contempt(), 0);

        let mut lc0 = SearchEngine::new(1, 1);
        assert!(install_adapter(&mut lc0, "lc0"));
        assert_eq!(lc0.eval_mode(), EvalMode::Nnue(NnueSearchProfile::Lc0));
        assert!(lc0.eval_mode().is_lc0_nnue());
        assert!(
            !lc0.use_nnue(),
            "in-process Lc0 has no transformer net and must not evaluate Reckless/Stockfish"
        );
        if let Some(weights) = eval::nnue::discover_lc0_weights() {
            let name = weights
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            assert!(
                eval::nnue::LC0_WEIGHT_FILENAMES.contains(&name),
                "Lc0 adapter must discover its own transformer weights, got {name}"
            );
        }
        assert_eq!(
            lc0.search_stack.policies.move_ordering,
            MoveOrderingProfile::Reckless
        );
        lc0.set_contempt(48);
        assert_eq!(lc0.contempt(), 0);
    }

    #[test]
    fn adapters_never_evaluate_a_foreign_network() {
        let cases = [
            ("akimbo", "akimbo"),
            ("viridithas", "viridithas"),
            ("obsidian", "obsidian"),
            ("plentychess", "plentychess"),
            ("ateed", "ateed"),
            ("lc0", "lc0"),
            ("mujrim-hce", "mujrim-hce"),
        ];
        for (id, expected) in cases {
            let mut engine = SearchEngine::new(1, 1);
            assert!(install_adapter(&mut engine, id));
            if engine.use_nnue() {
                assert_eq!(
                    engine.nnue_preset_hint(),
                    expected,
                    "{id} evaluated a foreign net"
                );
            }
        }
        #[cfg(feature = "stockfish-nnue")]
        {
            let mut engine = SearchEngine::new(1, 1);
            assert!(install_adapter(&mut engine, "stockfish"));
            assert_eq!(engine.nnue_preset_hint(), "stockfish");
        }
        #[cfg(feature = "reckless-nnue")]
        {
            let mut engine = SearchEngine::new(1, 1);
            assert!(install_adapter(&mut engine, "reckless"));
            assert_eq!(engine.nnue_preset_hint(), "reckless");
        }
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
        assert_eq!(AteedAdapter.id(), "ateed");
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
