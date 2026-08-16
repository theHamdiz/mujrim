use crate::engine::{SearchContext, SearchNode, ThreadState};
use crate::search_family::Lc0Family;
use types::Board;

pub(crate) fn search_ab(
    board: &mut Board,
    state: &mut ThreadState,
    context: &SearchContext<'_>,
    node: SearchNode,
) -> i32 {
    super::run::<Lc0Family>(board, state, context, node)
}
