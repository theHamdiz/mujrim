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
