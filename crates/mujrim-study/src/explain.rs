//! Rule-based natural-language commentary (Decode Chess style, no LLM).

use types::chess_move::MoveFlag;
use types::{Board, Color, Move, Piece, Square};

use crate::annotation::MoveAnnotation;
use crate::threats::{self, ThreatMark};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    pub threats: Vec<String>,
    pub idea: Vec<String>,
    pub this_move: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplainLine {
    pub text: String,
    pub squares: Vec<Square>,
}

impl Explanation {
    pub fn is_empty(&self) -> bool {
        self.threats.is_empty() && self.idea.is_empty() && self.this_move.is_empty()
    }

    pub fn lines(&self) -> Vec<ExplainLine> {
        self.threats
            .iter()
            .chain(self.idea.iter())
            .chain(self.this_move.iter())
            .map(|text| ExplainLine {
                squares: squares_in(text),
                text: text.clone(),
            })
            .collect()
    }

    pub fn panel_text(&self) -> String {
        let mut sections = Vec::new();
        if !self.threats.is_empty() {
            sections.push(format!("Threats\n{}", self.threats.join("\n")));
        }
        if !self.idea.is_empty() {
            sections.push(format!("Strategic idea\n{}", self.idea.join("\n")));
        }
        if !self.this_move.is_empty() {
            sections.push(format!("This move\n{}", self.this_move.join("\n")));
        }
        if sections.is_empty() {
            "Quiet position — no immediate tactical alarms.".to_owned()
        } else {
            sections.join("\n\n")
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MoveContext {
    pub annotation: Option<MoveAnnotation>,
    pub score_cp: Option<i32>,
    pub mv: Option<Move>,
    pub san: Option<String>,
}

pub fn explain_position(board: &Board, ply: usize, last: MoveContext) -> Explanation {
    types::init();
    let seed = fen_seed(&board.to_fen()) ^ ply as u64;
    let stm = board.side_to_move;
    let marks = threats::threatened_pieces(board);
    let threats = threat_sentences(board, &marks, seed);
    let idea = strategic_sentences(board, stm, seed.wrapping_add(17));
    let this_move = move_sentences(board, last, seed.wrapping_add(41));
    Explanation {
        threats,
        idea,
        this_move,
    }
}

fn threat_sentences(board: &Board, marks: &[ThreatMark], seed: u64) -> Vec<String> {
    let stm = board.side_to_move;
    let mut lines = Vec::new();
    if board.is_in_check(stm) {
        lines.push(pick(
            seed,
            &[
                "The king is in check and must be answered immediately.",
                "Check! The king needs a flight square, a block, or a capture.",
                "The side to move is in check — every other plan waits.",
            ],
        ));
    }
    for mark in marks.iter().take(4) {
        let Some((piece, _)) = board.piece_on(mark.square) else {
            continue;
        };
        let Some((attacker, _)) = board.piece_on(mark.attacker) else {
            continue;
        };
        let target = piece_phrase(piece, mark.square);
        let source = piece_phrase(attacker, mark.attacker);
        if mark.hanging {
            let options = [
                format!("{target} is hanging, hit by {source}."),
                format!("{source} is attacking the undefended {target}."),
                format!("Tactical alarm: {target} can be taken by {source}."),
            ];
            lines.push(pick_owned(seed ^ mark.square.index() as u64, &options));
        } else {
            let options = [
                format!("{source} pressures {target}."),
                format!("{target} is under fire from {source}."),
                format!("{source} eyes {target} — the piece is defended, but tense."),
            ];
            lines.push(pick_owned(
                seed ^ (mark.square.index() as u64).wrapping_mul(3),
                &options,
            ));
        }
    }
    lines
}

fn strategic_sentences(board: &Board, stm: Color, seed: u64) -> Vec<String> {
    let mut lines = Vec::new();
    let us = stm;
    let them = stm.opponent();
    let material = material_cp(board, us) - material_cp(board, them);
    if material.abs() >= 200 {
        if material > 0 {
            lines.push(pick(
                seed,
                &[
                    "Convert the extra material by trading into a clean ending.",
                    "With a material plus, simplify — exchanges favor the richer side.",
                    "Keep pieces coordinated; the extra wood should decide if the king stays safe.",
                ],
            ));
        } else {
            lines.push(pick(
                seed,
                &[
                    "Hunt counterplay — material is down, so complications are a friend.",
                    "Avoid a dry ending; the deficit grows when pieces come off.",
                    "Look for activity and attacks to offset the missing material.",
                ],
            ));
        }
    }

    if can_castle(board, us) && center_is_open(board) {
        lines.push(pick(
            seed.wrapping_add(3),
            &[
                "Castle before the center fully opens — the king is still in the middle.",
                "King safety first: tuck the king away while files can still close.",
                "The center is loosening; finishing development and castling is the plan.",
            ],
        ));
    }

    if let Some(sq) = passed_pawn(board, us) {
        let phrase = format!("the passed pawn on {}", sq);
        let options = [
            format!("Escort {phrase} — it is a long-term winning theme."),
            format!("{phrase} wants a path; pieces should restrain enemy blockers."),
            format!("Use {phrase} to freeze enemy pieces on defence."),
        ];
        lines.push(pick_owned(seed.wrapping_add(9), &options));
    }

    if board.is_in_check(them) {
        lines.push(pick(
            seed.wrapping_add(11),
            &[
                "Keep the enemy king uncomfortable; one more check may force a collapse.",
                "The opponent is on the run — look for a follow-up that does not let them breathe.",
            ],
        ));
    }

    if let Some(line) = pin_sentence(board, seed.wrapping_add(19)) {
        lines.push(line);
    }
    if let Some(line) = fork_sentence(board, seed.wrapping_add(23)) {
        lines.push(line);
    }

    if lines.is_empty() {
        lines.push(pick(
            seed.wrapping_add(13),
            &[
                "Improve the worst-placed piece and claim more of the center.",
                "Play for a pawn break that opens a file for the rooks.",
                "Control key squares, then decide whether to expand on the kingside or queenside.",
            ],
        ));
    }
    lines
}

fn move_sentences(board: &Board, last: MoveContext, seed: u64) -> Vec<String> {
    let Some(mv) = last.mv else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mover = board.side_to_move.opponent();
    match last.annotation {
        Some(MoveAnnotation::Blunder) => lines.push(pick(
            seed,
            &[
                "This was a blunder — the evaluation collapses.",
                "A serious error: the previous advantage (or equality) is gone.",
                "This move hands the opponent a winning tactical chance.",
            ],
        )),
        Some(MoveAnnotation::Mistake) => lines.push(pick(
            seed,
            &[
                "A mistake: better was available, and the cost is real.",
                "This is a clear inaccuracy of the expensive kind.",
            ],
        )),
        Some(MoveAnnotation::Inaccuracy) => lines.push(pick(
            seed,
            &[
                "Slightly imprecise — the idea is fine, the timing is not perfect.",
                "An inaccuracy: the position is still playable, just less pleasant.",
            ],
        )),
        Some(MoveAnnotation::Brilliant | MoveAnnotation::Aura | MoveAnnotation::Great) => lines
            .push(pick(
                seed,
                &[
                    "A high-class shot — this move changes the story of the game.",
                    "Tactical vision: the piece is given or the king is hunted with purpose.",
                ],
            )),
        Some(MoveAnnotation::Best | MoveAnnotation::Excellent) => lines.push(pick(
            seed,
            &[
                "Engine-approved: this keeps the evaluation on track.",
                "The strongest continuation — it answers the position's demand.",
            ],
        )),
        Some(MoveAnnotation::Book) => lines.push(pick(
            seed,
            &[
                "Still in book — this is a known theoretical path.",
                "Opening theory: the move follows a mapped highway.",
            ],
        )),
        _ => {}
    }

    if mv.is_castling() {
        lines.push(pick(
            seed.wrapping_add(2),
            &[
                "Castling tucks the king and connects the rooks.",
                "The king steps into safety and the rook joins the game.",
            ],
        ));
    } else if mv.is_promotion() {
        lines.push(pick(
            seed.wrapping_add(2),
            &[
                "A new queen (or piece) arrives — the pawn's journey is complete.",
                "Promotion changes the material count in a single ply.",
            ],
        ));
    } else if mv.flag == MoveFlag::EnPassant {
        lines.push(pick(
            seed.wrapping_add(2),
            &[
                "En passant: the pawn is captured in passing, a one-move window.",
                "The special pawn capture removes the passer that just leaped.",
            ],
        ));
    } else if mv.is_capture() {
        lines.push(pick(
            seed.wrapping_add(2),
            &[
                "A capture — pieces come off and the pawn structure may shift.",
                "Taking on that square forces a recapture decision.",
            ],
        ));
    }

    if board.is_in_check(board.side_to_move) {
        lines.push(format!(
            "{} leaves the opponent in check.",
            color_name(mover)
        ));
    }
    if let Some(score) = last.score_cp {
        let pawns = score as f32 / 100.0;
        lines.push(format!(
            "Engine snapshot: {:+.1} pawns from {}'s view after the move.",
            pawns,
            color_name(mover)
        ));
    }
    if let Some(san) = last.san {
        lines.insert(0, format!("The last move was {san}."));
    }
    lines
}

fn piece_phrase(piece: Piece, square: Square) -> String {
    format!("{} on {square}", piece_name(piece))
}

fn piece_name(piece: Piece) -> &'static str {
    match piece {
        Piece::Pawn => "pawn",
        Piece::Knight => "knight",
        Piece::Bishop => "bishop",
        Piece::Rook => "rook",
        Piece::Queen => "queen",
        Piece::King => "king",
    }
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

fn material_cp(board: &Board, color: Color) -> i32 {
    Square::ALL
        .iter()
        .filter_map(|&sq| board.piece_on(sq))
        .filter(|(_, c)| *c == color)
        .map(|(piece, _)| match piece {
            Piece::Pawn => 100,
            Piece::Knight | Piece::Bishop => 300,
            Piece::Rook => 500,
            Piece::Queen => 900,
            Piece::King => 0,
        })
        .sum()
}

fn can_castle(board: &Board, color: Color) -> bool {
    match color {
        Color::White => board.castling_rights & 0b0011 != 0,
        Color::Black => board.castling_rights & 0b1100 != 0,
    }
}

fn pin_sentence(board: &Board, seed: u64) -> Option<String> {
    let stm = board.side_to_move;
    let king = board.king_square(stm);
    for square in Square::ALL {
        let Some((piece, color)) = board.piece_on(square) else {
            continue;
        };
        if color != stm || piece == Piece::King || piece == Piece::Pawn {
            continue;
        }
        if !aligned(square, king) {
            continue;
        }
        let Some(attacker) = slider_on_ray(board, square, king, stm.opponent()) else {
            continue;
        };
        return Some(pick_owned(
            seed,
            &[
                format!(
                    "{} on {square} is pinned to the king by the {attacker}.",
                    piece_name(piece)
                ),
                format!(
                    "A pin: the {attacker} stares through {} toward the king.",
                    piece_name(piece)
                ),
            ],
        ));
    }
    None
}

fn fork_sentence(board: &Board, seed: u64) -> Option<String> {
    use types::board::attack_tables::{knight_attacks, pawn_attacks};
    for square in Square::ALL {
        let Some((Piece::Knight, color)) = board.piece_on(square) else {
            continue;
        };
        if color != board.side_to_move {
            continue;
        }
        let hits = knight_attacks(square.index());
        let mut royal = 0u32;
        let mut heavy = 0u32;
        for target in Square::ALL {
            if hits & (1u64 << target.index()) == 0 {
                continue;
            }
            let Some((piece, other)) = board.piece_on(target) else {
                continue;
            };
            if other == color {
                continue;
            }
            match piece {
                Piece::King => royal += 1,
                Piece::Queen | Piece::Rook => heavy += 1,
                _ => {}
            }
        }
        if royal + heavy >= 2 {
            return Some(pick_owned(
                seed,
                &[
                    format!("The knight on {square} forks two valuable units."),
                    format!(
                        "Look at the knight fork from {square} — two pieces are hanging on the same hop."
                    ),
                ],
            ));
        }
        let _ = pawn_attacks;
    }
    None
}

fn aligned(a: Square, b: Square) -> bool {
    a.file() == b.file()
        || a.rank() == b.rank()
        || a.file().abs_diff(b.file()) == a.rank().abs_diff(b.rank())
}

fn slider_on_ray(
    board: &Board,
    through: Square,
    king: Square,
    attacker: Color,
) -> Option<&'static str> {
    let df = (king.file() as i32 - through.file() as i32).signum();
    let dr = (king.rank() as i32 - through.rank() as i32).signum();
    let back_df = -df;
    let back_dr = -dr;
    let mut file = through.file() as i32 + back_df;
    let mut rank = through.rank() as i32 + back_dr;
    while (0..8).contains(&file) && (0..8).contains(&rank) {
        let square = Square::from_file_rank(file as u8, rank as u8);
        if let Some((piece, color)) = board.piece_on(square) {
            if color != attacker {
                return None;
            }
            let diagonal = back_df != 0 && back_dr != 0;
            return match (piece, diagonal) {
                (Piece::Bishop | Piece::Queen, true) => Some(piece_name(piece)),
                (Piece::Rook | Piece::Queen, false) => Some(piece_name(piece)),
                _ => None,
            };
        }
        file += back_df;
        rank += back_dr;
    }
    None
}

fn center_is_open(board: &Board) -> bool {
    [Square::D4, Square::E4, Square::D5, Square::E5]
        .iter()
        .filter(|sq| board.piece_on(**sq).is_none())
        .count()
        >= 2
}

fn passed_pawn(board: &Board, color: Color) -> Option<Square> {
    for square in Square::ALL {
        let Some((Piece::Pawn, pawn_color)) = board.piece_on(square) else {
            continue;
        };
        if pawn_color != color {
            continue;
        }
        if is_passed(board, square, color) {
            return Some(square);
        }
    }
    None
}

fn is_passed(board: &Board, square: Square, color: Color) -> bool {
    let file = square.file();
    let rank = square.rank();
    let them = color.opponent();
    for other in Square::ALL {
        let Some((Piece::Pawn, pawn_color)) = board.piece_on(other) else {
            continue;
        };
        if pawn_color != them {
            continue;
        }
        let df = other.file().abs_diff(file);
        if df > 1 {
            continue;
        }
        let ahead = match color {
            Color::White => other.rank() > rank,
            Color::Black => other.rank() < rank,
        };
        if ahead {
            return false;
        }
    }
    true
}

fn fen_seed(fen: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for byte in fen.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn pick(seed: u64, options: &[&str]) -> String {
    options[(seed as usize) % options.len()].to_owned()
}

fn pick_owned(seed: u64, options: &[String]) -> String {
    options[(seed as usize) % options.len()].clone()
}

pub fn squares_in(text: &str) -> Vec<Square> {
    let bytes = text.as_bytes();
    let mut squares = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let file = bytes[i];
        let rank = bytes[i + 1];
        if (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank) {
            let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let next_ok = i + 2 >= bytes.len() || !bytes[i + 2].is_ascii_alphanumeric();
            if prev_ok && next_ok {
                squares.push(Square::from_file_rank(file - b'a', rank - b'1'));
            }
        }
        i += 1;
    }
    squares
}

pub fn comments_for_line(initial_fen: &str, moves: &[String]) -> Vec<(usize, String)> {
    types::init();
    let mut board = Board::from_fen(initial_fen).unwrap_or_else(|_| Board::new());
    let mut comments = Vec::new();
    for (index, notation) in moves.iter().enumerate() {
        let ply = index + 1;
        let legal = board.generate_legal_moves();
        let Some(mv) = legal.iter().copied().find(|candidate| {
            let uci = candidate.to_uci();
            *notation == uci || notation.starts_with(&uci)
        }) else {
            break;
        };
        let last = MoveContext {
            mv: Some(mv),
            ..MoveContext::default()
        };
        board.make_move(mv);
        let explanation = explain_position(&board, ply, last);
        if !explanation.is_empty() {
            comments.push((ply, explanation.panel_text()));
        }
    }
    comments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::AnnotationContext;

    #[test]
    fn squares_in_extracts_algebraic() {
        assert_eq!(
            squares_in("The queen on a8 is hanging, hit by Ra1."),
            vec![Square::A8]
        );
        assert!(squares_in("Quiet position.").is_empty());
    }

    #[test]
    fn pin_and_fork_sentences_are_non_empty() {
        types::init();
        let pinned = Board::from_fen("4k3/8/8/8/7b/8/5N2/4K3 w - - 0 1").expect("fen");
        let pin_text = explain_position(&pinned, 4, MoveContext::default()).panel_text();
        assert!(
            pin_text.to_lowercase().contains("pin") || pin_text.contains("N"),
            "{pin_text}"
        );
        let forked = Board::from_fen("4k3/8/8/3n4/8/8/2K1R3/8 b - - 0 1").expect("fen");
        let fork_text = explain_position(&forked, 5, MoveContext::default()).panel_text();
        assert!(!fork_text.is_empty(), "{fork_text}");
    }

    #[test]
    fn hanging_queen_is_called_out() {
        types::init();
        let board = Board::from_fen("4k3/8/8/8/8/8/4K3/R6q b - - 0 1").expect("fen");
        let text = explain_position(&board, 12, MoveContext::default()).panel_text();
        assert!(text.to_lowercase().contains("queen"), "{text}");
        let lower = text.to_lowercase();
        assert!(
            lower.contains("hanging")
                || lower.contains("undefended")
                || lower.contains("taken")
                || lower.contains("attacking"),
            "{text}"
        );
    }

    #[test]
    fn scholars_f7_is_a_threat() {
        types::init();
        let board =
            Board::from_fen("rnbqkbnr/pppp1ppp/8/4p3/2B1P3/8/PPPP1PPP/RNBQK1NR b KQkq - 1 2")
                .expect("fen");
        let text = explain_position(&board, 3, MoveContext::default()).panel_text();
        assert!(
            text.contains("f7") || text.to_lowercase().contains("pawn"),
            "{text}"
        );
    }

    #[test]
    fn castling_gets_a_move_sentence() {
        types::init();
        let board =
            Board::from_fen("rnbqk2r/pppp1ppp/5n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQ1RK1 b kq - 4 4")
                .expect("fen");
        let last = MoveContext {
            mv: Some(Move::king_castle(Square::E1, Square::G1)),
            annotation: Some(MoveAnnotation::Book),
            ..MoveContext::default()
        };
        let text = explain_position(&board, 8, last).panel_text();
        assert!(text.to_lowercase().contains("castl"), "{text}");
    }

    #[test]
    fn blunder_annotation_is_phrased() {
        types::init();
        let board = Board::new();
        let last = MoveContext {
            mv: Some(Move::quiet(Square::E2, Square::E4)),
            annotation: Some(MoveAnnotation::Blunder),
            score_cp: Some(-320),
            ..MoveContext::default()
        };
        let text = explain_position(&board, 1, last).panel_text();
        let lower = text.to_lowercase();
        assert!(
            lower.contains("chance")
                || lower.contains("blunder")
                || lower.contains("error")
                || lower.contains("collapses"),
            "{text}"
        );
    }

    #[test]
    fn phrase_pick_is_stable_for_a_seed() {
        let a = pick(42, &["one", "two", "three"]);
        let b = pick(42, &["one", "two", "three"]);
        assert_eq!(a, b);
        assert_ne!(
            pick(42, &["one", "two", "three"]),
            pick(43, &["one", "two", "three"])
        );
    }

    #[test]
    fn annotation_blunder_threshold_still_feeds_comments() {
        let annotation = AnnotationContext {
            best_score_cp: 80,
            played_score_cp: -250,
            ..AnnotationContext::default()
        }
        .classify();
        assert_eq!(annotation, MoveAnnotation::Blunder);
    }

    #[test]
    fn comments_for_line_covers_castling() {
        types::init();
        let comments = comments_for_line(
            "rnbqk2r/pppp1ppp/5n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            &["e1g1".to_owned()],
        );
        assert!(
            comments
                .iter()
                .any(|(_, text)| text.to_lowercase().contains("castl")),
            "{comments:?}"
        );
    }
}
