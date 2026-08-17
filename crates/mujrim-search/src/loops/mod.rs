//! Dedicated search entry points, one per adapter (including HCE and Ateed).
//!
//! `SearchEngine::search` matches the active evaluator once, then stays inside
//! that adapter's monomorphized PVS for the rest of the iteration.

pub(crate) mod akimbo;
pub(crate) mod ateed;
pub(crate) mod hce;
pub(crate) mod lc0;
pub(crate) mod obsidian;
pub(crate) mod plentychess;
pub(crate) mod reckless;
pub(crate) mod stockfish;
pub(crate) mod viridithas;

use crate::engine::{SearchContext, SearchNode, ThreadState, search_ab_for};
use crate::search_family::{DynamicFamily, SearchFamily};
use crate::search_stack::EvalMode;
use eval::nnue::NnueSearchProfile;
use types::Board;

pub(crate) fn enter_search(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    if !context.policies.uses_dedicated_loop(context.eval_mode) {
        return search_ab_for::<DynamicFamily>(board, state, context, node);
    }
    match context.eval_mode {
        EvalMode::MujrimHce => hce::search_ab(board, state, context, node),
        EvalMode::Nnue(NnueSearchProfile::Stockfish) => {
            stockfish::search_ab(board, state, context, node)
        }
        EvalMode::Nnue(NnueSearchProfile::Akimbo) => akimbo::search_ab(board, state, context, node),
        EvalMode::Nnue(NnueSearchProfile::Reckless) => {
            reckless::search_ab(board, state, context, node)
        }
        EvalMode::Nnue(NnueSearchProfile::Viridithas) => {
            viridithas::search_ab(board, state, context, node)
        }
        EvalMode::Nnue(NnueSearchProfile::Obsidian) => {
            obsidian::search_ab(board, state, context, node)
        }
        EvalMode::Nnue(NnueSearchProfile::PlentyChess) => {
            plentychess::search_ab(board, state, context, node)
        }
        EvalMode::Nnue(NnueSearchProfile::Ateed) => ateed::search_ab(board, state, context, node),
        EvalMode::Nnue(NnueSearchProfile::Lc0) => lc0::search_ab(board, state, context, node),
    }
}

pub(crate) fn run<F: SearchFamily>(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    search_ab_for::<F>(board, state, context, node)
}

#[cfg(test)]
mod tests {
    use crate::adapters::install_adapter;
    use crate::engine::SearchEngine;
    use types::Board;
    use types::chess_move::NULL_MOVE;

    #[test]
    fn dedicated_loops_return_legal_root_moves() {
        types::init();
        for adapter in ["viridithas", "akimbo", "ateed"] {
            let mut engine = SearchEngine::new(8, 1);
            assert!(
                install_adapter(&mut engine, adapter),
                "{adapter} adapter must install"
            );
            let mut board = Board::new();
            let result = engine.search_nodes(&mut board, 800, 4);
            assert!(result.nodes > 0, "{adapter} must visit nodes");
            assert_ne!(result.best_move, NULL_MOVE, "{adapter} must pick a move");
            let legal = board.generate_legal_moves();
            assert!(
                legal.iter().any(|mv| mv.from == result.best_move.from
                    && mv.to == result.best_move.to
                    && mv.flag == result.best_move.flag),
                "{adapter} best move must be legal"
            );
        }
    }

    #[test]
    fn akimbo_depth_7_startpos_is_not_a_nonsense_wing_pawn() {
        types::init();
        let mut engine = SearchEngine::new(16, 1);
        assert!(install_adapter(&mut engine, "akimbo"));
        let mut board = Board::new();
        let start_hash = board.hash;
        let result = engine.search_depth(&mut board, 7);
        let uci = result.best_move.to_uci();
        assert_ne!(uci, "a2a3", "Akimbo depth 7 must not collapse to a2a3");
        assert_ne!(uci, "b1c3", "Akimbo depth 7 must not collapse to b1c3");
        assert_eq!(uci, "e2e4", "Akimbo depth 7 must play e2e4");
        assert_ne!(result.best_move, NULL_MOVE);
        assert_eq!(
            board.hash, start_hash,
            "Akimbo snapshot restore must leave startpos"
        );
    }

    #[test]
    fn viridithas_depth_2_startpos_plays_nf3() {
        types::init();
        if eval::nnue::discover_named_network("sandhi-s2-b200.nnue.zst").is_none()
            && eval::nnue::discover_named_network("viri_default.nnue.zst").is_none()
        {
            return;
        }
        let mut engine = SearchEngine::new(16, 1);
        assert!(install_adapter(&mut engine, "viridithas"));
        let mut board = Board::new();
        let result = engine.search_depth(&mut board, 2);
        assert_eq!(
            result.best_move.to_uci(),
            "g1f3",
            "official Viridithas 20 depth 2 is Nf3, got {} score {}",
            result.best_move.to_uci(),
            result.score
        );
    }

    #[test]
    fn viridithas_snapshot_restore_leaves_startpos() {
        types::init();
        let mut engine = SearchEngine::new(16, 1);
        assert!(install_adapter(&mut engine, "viridithas"));
        let mut board = Board::new();
        let start_hash = board.hash;
        let result = engine.search_nodes(&mut board, 2000, 6);
        assert_ne!(result.best_move, NULL_MOVE);
        assert_eq!(
            board.hash, start_hash,
            "Viridithas snapshot restore must leave startpos"
        );
    }

    #[test]
    fn viridithas_and_akimbo_loops_are_not_the_generic_pvs() {
        let viri = include_str!("viridithas.rs");
        let akimbo = include_str!("akimbo.rs");
        let ateed = include_str!("ateed.rs");
        assert!(
            !viri.contains("search_ab_for<") && !viri.contains("super::run"),
            "Viridithas must run official alpha_beta, not the generic PVS"
        );
        assert!(
            !akimbo.contains("search_ab_for<") && !akimbo.contains("super::run"),
            "Akimbo must run official pvs, not the generic PVS"
        );
        assert!(
            !ateed.contains("search_ab_for<") && !ateed.contains("super::run"),
            "Ateed must run the MoE PVS, not the generic Reckless loop"
        );
        assert!(
            viri.contains("dedicated_viridithas"),
            "Viridithas loop must call the official alpha_beta"
        );
        assert!(
            akimbo.contains("dedicated_akimbo"),
            "Akimbo loop must call the official pvs"
        );
        assert!(
            ateed.contains("dedicated_ateed"),
            "Ateed loop must call the MoE PVS"
        );
    }

    #[test]
    fn dedicated_loops_keep_official_probcut_and_fractional_lmr() {
        let viri = include_str!("../engine_loops/viridithas.rs");
        let akimbo = include_str!("../engine_loops/akimbo.rs");
        assert!(
            viri.contains("reduction_1024ths")
                && viri.contains("viridithas_probcut_beta")
                && viri.contains("viridithas_later_move_reduction")
                && viri.contains("viridithas_singular_verdict")
                && viri.contains("skip_quiets")
                && viri.contains("raw_eval")
                && viri.contains("hint_common_access")
                && viri.contains("hint_common_access_once")
                && viri.contains("resolve_threats")
                && viri.contains("prefetch")
                && viri.contains("restore_snapshot")
                && viri.contains("make_search_move_no_undo"),
            "Viridithas must keep 1024ths LMR, official later-move r=1, skip-quiet LMP, ProbCut, deferred sandhi ensure, and snapshot restore"
        );
        assert!(
            akimbo.contains("akimbo_probcut_beta")
                && akimbo.contains("make_search_move_no_undo")
                && akimbo.contains("prefetch")
                && akimbo.contains("quiescence_snapshot"),
            "Akimbo must run official ProbCut on snapshot make, prefetch the child TT, and use snapshot QS"
        );
        let ateed_loop = include_str!("../engine_loops/ateed.rs");
        assert!(
            ateed_loop.contains("ateed_moe_lmr_delta")
                && ateed_loop.contains("evaluate_ateed_search_signal")
                && ateed_loop.contains("hint_common_access_once")
                && ateed_loop.contains("resolve_threats")
                && ateed_loop.contains("prefetch")
                && ateed_loop.contains("restore_snapshot")
                && ateed_loop.contains("make_search_move_no_undo")
                && ateed_loop.contains("quiescence_snapshot")
                && ateed_loop.contains("AteedLmrPolicy")
                && ateed_loop.contains("AteedRfpPolicy"),
            "Ateed must keep MoE LMR, deferred ensure, lazy threats, prefetch, and snapshot QS"
        );
        let qs = include_str!("../engine_loops/qs.rs");
        assert_eq!(
            qs.matches("tt.probe").count(),
            1,
            "dedicated QS must probe the TT once per node"
        );
        let probe_at = qs.find("tt.probe").expect("QS probes the TT");
        let reuse_at = qs
            .find("if let Some(raw) = raw_eval")
            .expect("QS reuses a TT raw_eval for stand-pat");
        assert!(
            probe_at < reuse_at,
            "official QS reuses a TT eval and must probe before stand-pat"
        );
        assert!(
            qs.contains("entry.raw_eval") && qs.contains("hint_common_access"),
            "QS must reuse TT raw_eval and ensure sandhi only when a child is made"
        );
        assert!(
            include_str!("../engine_loops/viridithas.rs").contains("quiescence_snapshot"),
            "Viridithas QS must restore snapshots like the official copy-make path"
        );
    }
}
