pub mod adapters;
#[cfg(feature = "book")]
pub mod book;
pub mod conversion;
pub mod engine;
pub mod hce_bench;
pub(crate) mod loops;
pub mod move_picker;
pub mod policy;
pub(crate) mod search_family;
pub mod search_params;
pub mod search_stack;
pub mod see;
pub mod syzygy;
pub mod tt;

#[cfg(feature = "reckless-nnue")]
pub use adapters::RecklessAdapter;
#[cfg(feature = "stockfish-nnue")]
pub use adapters::StockfishAdapter;
pub use adapters::{
    AkimboAdapter, AteedAdapter, EvalSearchAdapter, Lc0Adapter, MujrimHceAdapter, ObsidianAdapter,
    PlentyChessAdapter, ViridithasAdapter, adapter_for_id, install_adapter,
};
pub use conversion::{DEFAULT_CONTEMPT, WIN_CONVERSION_CP};
pub use engine::SearchEngine;
pub use hce_bench::{
    CI_HCE_NODE_BUDGET, HceNpsReport, RELEASE_HCE_NPS_TARGET, measure_hce_eval_nodes,
    measure_hce_search_nodes, measure_hce_search_nps,
};
pub use search_params::SearchParams;
pub use search_stack::{EvalMode, SearchExperiment, SearchStack, SearchStackProfile};
pub use syzygy::{DEFAULT_PROBE_DEPTH, DEFAULT_PROBE_LIMIT, SyzygyTables};

#[cfg(test)]
mod feature_contract_tests {
    #[test]
    fn search_crate_does_not_force_embedded_networks_on_eval() {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        // Dependency line must stay free of forced network embeds (feature unification).
        let eval_dep_line = manifest
            .lines()
            .find(|line| line.contains("path = \"../mujrim-eval\""))
            .expect("eval dependency line");
        assert!(
            !eval_dep_line.contains("embedded-networks"),
            "mujrim-search must not force eval/embedded-networks on its dependency"
        );
        assert!(
            manifest.contains("embedded-networks = [\"eval/embedded-networks\"]"),
            "embedded-networks must remain an opt-in search feature"
        );
        assert!(
            !manifest.contains("default = [\"book\", \"nnue\", \"simd\", \"akimbo-nnue\", \"stockfish-nnue\", \"reckless-nnue\", \"embedded-networks\"]"),
            "embedded-networks must not be part of search defaults"
        );
    }
}
