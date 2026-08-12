pub mod adapters;
#[cfg(feature = "book")]
pub mod book;
pub mod engine;
pub mod hce_bench;
pub mod move_picker;
pub mod policy;
pub mod search_params;
pub mod search_stack;
pub mod see;
pub mod tt;

pub use adapters::{AkimboAdapter, EvalSearchAdapter, MujrimHceAdapter, adapter_for_id, install_adapter};
#[cfg(feature = "stockfish-nnue")]
pub use adapters::StockfishAdapter;
#[cfg(feature = "reckless-nnue")]
pub use adapters::RecklessAdapter;
pub use engine::SearchEngine;
pub use hce_bench::{
    CI_HCE_NODE_BUDGET, HceNpsReport, RELEASE_HCE_NPS_TARGET, measure_hce_eval_nodes,
    measure_hce_search_nodes, measure_hce_search_nps,
};
pub use search_params::SearchParams;
pub use search_stack::{EvalMode, SearchExperiment, SearchStack, SearchStackProfile};

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
