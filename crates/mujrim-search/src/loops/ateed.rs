use crate::engine::{SearchContext, SearchNode, ThreadState};
use crate::search_family::AteedFamily;
use types::Board;

pub(crate) fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    super::run::<AteedFamily>(board, state, context, node)
}
