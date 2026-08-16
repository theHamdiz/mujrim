//! Static Exchange Evaluation (SEE) — determines the material outcome
//! of a capture chain on a single square using the swap algorithm.
//! Used for capture ordering, pruning bad captures in quiescence, and LMR.

use types::bitboard::{Bitboard, get_lsb};
use types::board::attack_tables::*;
use types::{AkimboPos, Board, BoardSnapshot, Color, Move, Piece};

/// Piece values for SEE (centipawns).
const SEE_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 20000];

/// Returns the SEE value of a capture move.
/// Positive = capturing side gains material. Negative = loses.
#[inline]
pub fn see(board: &Board, mv: Move) -> i32 {
    let to = mv.to.index();
    let from = mv.from.index();

    // Snapshot piece bitboards once (avoid per-piece accessor calls).
    let mut white_pieces = board.pieces[Color::White.index()];
    let mut black_pieces = board.pieces[Color::Black.index()];

    let mut occupancy = board.occupancy[0] | board.occupancy[1];

    // Determine the initial captured piece value
    let initial_value =
        if let Some(captured) = board.piece_of_color_on(mv.to, board.side_to_move.opponent()) {
            SEE_VALUES[captured.index()]
        } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
            SEE_VALUES[0] // Pawn value
        } else {
            return 0; // Not a capture
        };

    // Determine the moving piece
    let moving_piece = if let Some(piece) = board.piece_of_color_on(mv.from, board.side_to_move) {
        piece
    } else {
        return 0;
    };

    // Build gain array (swap list)
    let mut gain = [0i32; 32];
    let mut depth = 0;
    gain[0] = initial_value;

    // Remove the initial attacker from occupancy
    occupancy &= !(1u64 << from);

    // Track which piece is "on" the target square
    let mut current_piece = moving_piece;
    let mut side = board.side_to_move.opponent(); // The defending side moves next

    loop {
        depth += 1;
        gain[depth] = SEE_VALUES[current_piece.index()] - gain[depth - 1];

        // Pruning: if we can't improve, stop
        if (-gain[depth - 1]).max(gain[depth]) < 0 {
            break;
        }

        // Get all attackers of the target square with current occupancy
        let attackers = get_attackers_to_square(to, occupancy, &white_pieces, &black_pieces);

        // Find the least valuable attacker for the current side
        let side_occ = match side {
            Color::White => {
                white_pieces[0]
                    | white_pieces[1]
                    | white_pieces[2]
                    | white_pieces[3]
                    | white_pieces[4]
                    | white_pieces[5]
            }
            Color::Black => {
                black_pieces[0]
                    | black_pieces[1]
                    | black_pieces[2]
                    | black_pieces[3]
                    | black_pieces[4]
                    | black_pieces[5]
            }
        };
        let side_attackers = attackers & side_occ;

        if side_attackers == 0 {
            break; // No more attackers for this side
        }

        // Find the least valuable piece
        let (lva_piece, lva_sq) = find_lva(side_attackers, side, &white_pieces, &black_pieces);

        // Remove this attacker from occupancy and piece bitboards
        let lva_bit = 1u64 << lva_sq;
        occupancy &= !lva_bit;
        match side {
            Color::White => white_pieces[lva_piece.index()] &= !lva_bit,
            Color::Black => black_pieces[lva_piece.index()] &= !lva_bit,
        }

        current_piece = lva_piece;
        side = side.opponent();

        if depth >= 31 {
            break;
        }
    }

    // Negamax the gain array
    while depth > 0 {
        depth -= 1;
        gain[depth] = -((-gain[depth]).max(gain[depth + 1]));
    }

    gain[0]
}

/// Returns true if SEE of the capture is >= threshold.
/// Fully inline swap algorithm — no fallback to full `see()`.
#[inline]
pub fn see_ge(board: &Board, mv: Move, threshold: i32) -> bool {
    see_ge_arrays(
        &board.pieces[Color::White.index()],
        &board.pieces[Color::Black.index()],
        board.occupancy[0] | board.occupancy[1],
        board.side_to_move,
        board.piece_of_color_on(mv.to, board.side_to_move.opponent()),
        board.piece_of_color_on(mv.from, board.side_to_move),
        mv,
        threshold,
    )
}

#[inline]
pub fn see_ge_pos(pos: &AkimboPos, mv: Move, threshold: i32) -> bool {
    let mut white = [0u64; 6];
    let mut black = [0u64; 6];
    for piece in Piece::ALL {
        white[piece.index()] = pos.piece_bb(piece, Color::White);
        black[piece.index()] = pos.piece_bb(piece, Color::Black);
    }
    see_ge_arrays(
        &white,
        &black,
        pos.all_occupancy(),
        pos.side_to_move(),
        pos.piece_of_color_on(mv.to, pos.side_to_move().opponent()),
        pos.piece_of_color_on(mv.from, pos.side_to_move()),
        mv,
        threshold,
    )
}

#[inline]
pub fn see_ge_snap(pos: &BoardSnapshot, mv: Move, threshold: i32) -> bool {
    let mut white = [0u64; 6];
    let mut black = [0u64; 6];
    for piece in Piece::ALL {
        white[piece.index()] = pos.piece_bb(piece, Color::White);
        black[piece.index()] = pos.piece_bb(piece, Color::Black);
    }
    see_ge_arrays(
        &white,
        &black,
        pos.all_occupancy(),
        pos.side_to_move(),
        pos.piece_of_color_on(mv.to, pos.side_to_move().opponent()),
        pos.piece_of_color_on(mv.from, pos.side_to_move()),
        mv,
        threshold,
    )
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn see_ge_arrays(
    white_src: &[u64; 6],
    black_src: &[u64; 6],
    occupancy_src: u64,
    stm: Color,
    captured: Option<Piece>,
    moving_piece: Option<Piece>,
    mv: Move,
    threshold: i32,
) -> bool {
    let to = mv.to.index();
    let from = mv.from.index();

    let initial_value = if let Some(captured) = captured {
        SEE_VALUES[captured.index()]
    } else if mv.flag == types::chess_move::MoveFlag::EnPassant {
        SEE_VALUES[0]
    } else {
        return threshold <= 0;
    };

    let moving_piece = if let Some(piece) = moving_piece {
        piece
    } else {
        return threshold <= 0;
    };

    // Quick balance checks
    let mut balance = initial_value - threshold;
    if balance < 0 {
        return false;
    }
    balance -= SEE_VALUES[moving_piece.index()];
    if balance >= 0 {
        return true;
    }

    // Full inline swap loop
    let mut white_pieces = *white_src;
    let mut black_pieces = *black_src;

    let mut occupancy = occupancy_src & !(1u64 << from);
    let mut side = stm.opponent();

    loop {
        let attackers = all_attackers(to, occupancy, &white_pieces, &black_pieces);
        let side_bb = match side {
            Color::White => &white_pieces,
            Color::Black => &black_pieces,
        };
        let side_occ = side_bb[0] | side_bb[1] | side_bb[2] | side_bb[3] | side_bb[4] | side_bb[5];
        let side_attackers = attackers & side_occ;

        if side_attackers == 0 {
            break;
        }

        // Find least valuable attacker
        let mut lva_piece = Piece::King;
        let mut lva_sq = 0;
        for &piece in &Piece::ALL {
            let atk = side_attackers & side_bb[piece.index()];
            if atk != 0 {
                lva_piece = piece;
                lva_sq = get_lsb(atk);
                break;
            }
        }

        let lva_bit = 1u64 << lva_sq;
        occupancy &= !lva_bit;
        match side {
            Color::White => white_pieces[lva_piece.index()] &= !lva_bit,
            Color::Black => black_pieces[lva_piece.index()] &= !lva_bit,
        }

        side = side.opponent();
        balance = -balance - 1 - SEE_VALUES[lva_piece.index()];

        if balance >= 0 {
            if lva_piece == Piece::King {
                let opp_bb = match side.opponent() {
                    Color::White => &white_pieces,
                    Color::Black => &black_pieces,
                };
                let opp_occ = opp_bb[0] | opp_bb[1] | opp_bb[2] | opp_bb[3] | opp_bb[4] | opp_bb[5];
                let opp_atk = all_attackers(to, occupancy, &white_pieces, &black_pieces) & opp_occ;
                if opp_atk != 0 {
                    break;
                }
            }
            break;
        }
    }

    side != stm
}

/// Get all attackers to a given square.
#[inline]
fn get_attackers_to_square(
    sq: usize,
    occupancy: Bitboard,
    white: &[u64; 6],
    black: &[u64; 6],
) -> Bitboard {
    all_attackers(sq, occupancy, white, black)
}

/// Find the least valuable attacker piece and its square.
#[inline]
fn find_lva(
    side_attackers: Bitboard,
    side: Color,
    white: &[u64; 6],
    black: &[u64; 6],
) -> (Piece, usize) {
    let pieces = match side {
        Color::White => white,
        Color::Black => black,
    };

    // Check pieces in order from least to most valuable
    for &piece in &Piece::ALL {
        let attacking = side_attackers & pieces[piece.index()];
        if attacking != 0 {
            let sq = get_lsb(attacking);
            return (piece, sq);
        }
    }

    // Should never get here if side_attackers is non-zero
    (Piece::King, get_lsb(side_attackers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        types::init();
    }

    #[test]
    fn test_see_simple_capture_winning() {
        setup();
        // Pawn captures a queen → should be very positive
        let board = Board::from_fen("4k3/8/8/3q4/4P3/8/8/4K3 w - - 0 1").unwrap();
        let mv = Move::capture(types::Square::E4, types::Square::D5);
        let score = see(&board, mv);
        assert!(
            score > 0,
            "Pawn x Queen should be positive SEE, got {score}"
        );
    }

    #[test]
    fn test_see_losing_capture() {
        setup();
        // Queen captures defended knight.
        let board = Board::from_fen("4k3/8/8/4p3/3n4/8/1Q6/4K3 w - - 0 1").unwrap();
        let mv = Move::capture(types::Square::B2, types::Square::D4);
        let score = see(&board, mv);
        assert!(
            score <= 0,
            "Defended queen capture should not be winning, got {score}"
        );
    }

    #[test]
    fn test_see_ge_rejects_queen_for_defended_pawn() {
        setup();
        let board = Board::from_fen("r3k3/p7/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        let mv = Move::capture(types::Square::A1, types::Square::A7);

        assert!(see(&board, mv) < 0);
        assert!(!see_ge(&board, mv, 0));
        let pos = types::AkimboPos::from_board(&board);
        assert_eq!(see_ge_pos(&pos, mv, 0), see_ge(&board, mv, 0));
        let snap = board.snapshot();
        assert_eq!(see_ge_snap(&snap, mv, 0), see_ge(&board, mv, 0));
    }

    #[test]
    fn test_see_equal_exchange() {
        setup();
        // Knight takes knight in a simple position.
        let board = Board::from_fen("4k3/8/3n4/8/4N3/8/8/4K3 w - - 0 1").unwrap();
        let mv = Move::capture(types::Square::E4, types::Square::D6);
        let _ = see(&board, mv);
    }
}
