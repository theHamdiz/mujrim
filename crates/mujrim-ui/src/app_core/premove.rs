//! Chess.com-style premove queue with multi-premove projection.

use types::{Board, Color, Move, Piece, Square};

/// Maximum queued premoves (chess.com allows a long chain; keep a firm cap).
pub const MAX_PREMOVES: usize = 10;

/// One queued premove (from → to, optional promotion).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Premove {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<Piece>,
}

impl Premove {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn new(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
        }
    }

    #[allow(dead_code)]
    pub const fn with_promotion(from: Square, to: Square, promotion: Piece) -> Self {
        Self {
            from,
            to,
            promotion: Some(promotion),
        }
    }
}

/// Apply a premove chain as chess.com does: only the human side moves, opponent
/// pieces stay until the real game advances. Each step is applied with the human
/// as side-to-move using pseudo-legal matching (checks ignored).
pub fn projected_board(board: &Board, queue: &[Premove], human: Color) -> Board {
    let mut projected = board.clone();
    for premove in queue {
        projected.side_to_move = human;
        if let Some(mv) = resolve_premove(&projected, *premove, human) {
            projected.make_move(mv);
        }
    }
    projected.side_to_move = board.side_to_move;
    projected
}

/// Pseudo-legal destination squares for a piece on the projected board.
pub fn premove_destinations(
    board: &Board,
    queue: &[Premove],
    human: Color,
    from: Square,
) -> Vec<Square> {
    let mut projected = projected_board(board, queue, human);
    projected.side_to_move = human;
    if projected
        .piece_on(from)
        .is_none_or(|(_, color)| color != human)
    {
        return Vec::new();
    }
    let mut targets = Vec::new();
    for mv in projected.generate_pseudo_legal_moves(human).iter() {
        if mv.from == from && !targets.contains(&mv.to) {
            targets.push(mv.to);
        }
    }
    targets
}

/// Build a premove from `from` → `to` on the projected board (queen promo default).
pub fn make_premove(
    board: &Board,
    queue: &[Premove],
    human: Color,
    from: Square,
    to: Square,
) -> Option<Premove> {
    let mut projected = projected_board(board, queue, human);
    projected.side_to_move = human;
    let candidates: Vec<Move> = projected
        .generate_pseudo_legal_moves(human)
        .iter()
        .copied()
        .filter(|mv| mv.from == from && mv.to == to)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mv = candidates
        .iter()
        .find(|mv| mv.promotion == Some(Piece::Queen))
        .or_else(|| candidates.first())
        .copied()?;
    Some(Premove {
        from: mv.from,
        to: mv.to,
        promotion: mv.promotion,
    })
}

/// Resolve a queued premove against the live board for execution (legal moves only).
pub fn resolve_legal(board: &mut Board, premove: Premove) -> Option<Move> {
    board.generate_legal_moves().iter().copied().find(|mv| {
        mv.from == premove.from
            && mv.to == premove.to
            && match premove.promotion {
                Some(piece) => mv.promotion == Some(piece),
                None => true,
            }
    })
}

fn resolve_premove(board: &Board, premove: Premove, human: Color) -> Option<Move> {
    board
        .generate_pseudo_legal_moves(human)
        .iter()
        .copied()
        .find(|mv| {
            mv.from == premove.from
                && mv.to == premove.to
                && match premove.promotion {
                    Some(piece) => mv.promotion == Some(piece),
                    None => mv.promotion.is_none() || mv.promotion == Some(Piece::Queen),
                }
        })
}

/// Whether a square holds a human piece on the projected board.
pub fn can_select_for_premove(
    board: &Board,
    queue: &[Premove],
    human: Color,
    square: Square,
) -> bool {
    let projected = projected_board(board, queue, human);
    projected
        .piece_on(square)
        .is_some_and(|(_, color)| color == human)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> Board {
        types::init();
        Board::new()
    }

    #[test]
    fn single_premove_projects_piece_forward() {
        let board = start();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        let queue = [Premove::new(e2, e4)];
        let projected = projected_board(&board, &queue, Color::White);
        assert!(projected.piece_on(e2).is_none());
        assert_eq!(projected.piece_on(e4), Some((Piece::Pawn, Color::White)));
    }

    #[test]
    fn multi_premove_chains_on_projected_board() {
        let board = start();
        let e2 = Square::from_index(12);
        let e4 = Square::from_index(28);
        let d2 = Square::from_index(11);
        let d4 = Square::from_index(27);
        let first = make_premove(&board, &[], Color::White, e2, e4).unwrap();
        let second = make_premove(&board, &[first], Color::White, d2, d4).unwrap();
        let projected = projected_board(&board, &[first, second], Color::White);
        assert!(projected.piece_on(e2).is_none());
        assert!(projected.piece_on(d2).is_none());
        assert!(projected.piece_on(e4).is_some());
        assert!(projected.piece_on(d4).is_some());
    }

    #[test]
    fn destinations_follow_prior_premoves() {
        let board = start();
        let g1 = Square::from_index(6);
        let f3 = Square::from_index(21);
        let first = make_premove(&board, &[], Color::White, g1, f3).unwrap();
        // After Nf3, the knight can continue to e5 / g5 / h4 / d4 / d2 / etc.
        let targets = premove_destinations(&board, &[first], Color::White, f3);
        assert!(targets.contains(&Square::from_index(36))); // e5
    }

    #[test]
    fn illegal_from_empty_square_is_rejected() {
        let board = start();
        let e4 = Square::from_index(28);
        let e5 = Square::from_index(36);
        assert!(make_premove(&board, &[], Color::White, e4, e5).is_none());
    }
}
