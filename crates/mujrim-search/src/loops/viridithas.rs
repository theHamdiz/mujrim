use crate::engine::{SearchContext, SearchNode, ThreadState, dedicated_viridithas};
use types::Board;

/// Official cosmobobak/viridithas `alpha_beta`.
pub(crate) fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    dedicated_viridithas::alpha_beta(board, state, context, node)
}
