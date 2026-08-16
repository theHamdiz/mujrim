use crate::engine::{SearchContext, SearchNode, ThreadState, dedicated_akimbo};
use types::Board;

/// Official jw1912/akimbo `pvs`.
pub(crate) fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    dedicated_akimbo::pvs(board, state, context, node)
}
