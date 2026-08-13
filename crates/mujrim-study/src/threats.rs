//! Lichess-style hanging and attacked-piece highlights for study/learn.

use types::bitboard::iter_bits;
use types::board::attack_tables::all_attackers;
use types::{Board, Color, Piece, Square};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreatMark {
    pub square: Square,
    pub attacker: Square,
    pub hanging: bool,
}

const fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Knight | Piece::Bishop => 300,
        Piece::Rook => 500,
        Piece::Queen => 900,
        Piece::King => 10_000,
    }
}

pub fn threatened_pieces(board: &Board) -> Vec<ThreatMark> {
    types::init();
    let stm = board.side_to_move;
    let them = stm.opponent();
    let occupancy = board.all_occupancy();
    let mut marks = Vec::new();
    for square in Square::ALL {
        let Some((piece, color)) = board.piece_on(square) else {
            continue;
        };
        if color != stm {
            continue;
        }
        if !board.is_square_attacked(square, them) {
            continue;
        }
        let attackers = opponent_attackers(board, square, them, occupancy);
        let Some(attacker) = cheapest_attacker(board, &attackers) else {
            continue;
        };
        let defended = board.is_square_attacked(square, stm);
        let hanging = !defended || piece_value(attacker.1) < piece_value(piece);
        marks.push(ThreatMark {
            square,
            attacker: attacker.0,
            hanging,
        });
    }
    marks
}

fn opponent_attackers(
    board: &Board,
    square: Square,
    them: Color,
    occupancy: types::Bitboard,
) -> Vec<Square> {
    let raw = all_attackers(
        square.index(),
        occupancy,
        &board.pieces[Color::White.index()],
        &board.pieces[Color::Black.index()],
    ) & board.color_occupancy(them);
    iter_bits(raw).map(Square::from_index).collect()
}

fn cheapest_attacker(board: &Board, attackers: &[Square]) -> Option<(Square, Piece)> {
    attackers
        .iter()
        .copied()
        .filter_map(|square| board.piece_on(square).map(|(piece, _)| (square, piece)))
        .min_by_key(|(_, piece)| piece_value(*piece))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undefended_queen_is_hanging() {
        types::init();
        let board = Board::from_fen("4k3/8/8/8/8/8/4K3/R6q b - - 0 1").expect("fen");
        let marks = threatened_pieces(&board);
        assert!(
            marks
                .iter()
                .any(|mark| mark.square == Square::H1 && mark.hanging),
            "black queen on h1 is attacked by the rook on a1: {marks:?}"
        );
    }

    #[test]
    fn starting_position_has_no_hanging_pieces() {
        types::init();
        let board = Board::new();
        assert!(threatened_pieces(&board).is_empty());
    }

    #[test]
    fn scholar_style_f7_is_marked_for_black() {
        types::init();
        let board =
            Board::from_fen("rnbqkbnr/pppp1ppp/8/4p3/2B1P3/8/PPPP1PPP/RNBQK1NR b KQkq - 1 2")
                .expect("fen");
        let marks = threatened_pieces(&board);
        assert!(
            marks.iter().any(|mark| mark.square == Square::F7),
            "bishop on c4 attacks f7: {marks:?}"
        );
    }
}
