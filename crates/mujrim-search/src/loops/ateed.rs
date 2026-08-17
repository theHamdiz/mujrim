use crate::engine::{SearchContext, SearchNode, ThreadState, dedicated_ateed};
use types::Board;

/// Ateed MoE PVS: snapshot make, deferred FT ensure, expert/WDL hooks.
pub(crate) fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    dedicated_ateed::pvs(board, state, context, node)
}
