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
        for adapter in ["viridithas", "akimbo"] {
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
    fn viridithas_and_akimbo_loops_are_not_the_generic_pvs() {
        let viri = include_str!("viridithas.rs");
        let akimbo = include_str!("akimbo.rs");
        assert!(
            !viri.contains("search_ab_for<") && !viri.contains("super::run"),
            "Viridithas must run official alpha_beta, not the generic PVS"
        );
        assert!(
            !akimbo.contains("search_ab_for<") && !akimbo.contains("super::run"),
            "Akimbo must run official pvs, not the generic PVS"
        );
        assert!(
            viri.contains("dedicated_viridithas"),
            "Viridithas loop must call the official alpha_beta"
        );
        assert!(
            akimbo.contains("dedicated_akimbo"),
            "Akimbo loop must call the official pvs"
        );
    }
}
