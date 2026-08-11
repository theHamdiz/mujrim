//! Curated gambit courses for interactive opening preparation.

use crate::board_marks::{ArrowRole, BoardArrow, MarkColor, arrows_from_uci_pv};
use crate::opening::{RepertoireLine, START_FEN};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GambitLesson {
    pub id: &'static str,
    pub name: &'static str,
    pub eco: &'static str,
    pub summary: &'static str,
    pub moves: &'static [&'static str],
    /// Index into `moves` where the gambit pawn is offered/taken.
    pub key_ply: usize,
}

impl GambitLesson {
    pub fn repertoire(&self) -> RepertoireLine {
        RepertoireLine {
            name: self.name.to_owned(),
            moves: self.moves.iter().map(|mv| (*mv).to_owned()).collect(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.repertoire().validate(START_FEN)
    }

    /// Stepped coaching arrows through the first `visible_plies` of the line.
    pub fn coaching_arrows(&self, visible_plies: usize) -> Result<Vec<BoardArrow>, String> {
        let uci: Vec<String> = self.moves.iter().map(|mv| (*mv).to_owned()).collect();
        arrows_from_uci_pv(
            START_FEN,
            &uci,
            MarkColor::Orange,
            ArrowRole::Gambit,
            visible_plies,
            Some(self.name),
        )
    }

    pub fn fen_after_plies(&self, plies: usize) -> Result<String, String> {
        types::init();
        let mut board = types::Board::from_fen(START_FEN)?;
        for uci in self.moves.iter().take(plies) {
            let mv = board
                .generate_legal_moves()
                .into_iter()
                .find(|candidate| candidate.to_uci() == *uci)
                .copied()
                .ok_or_else(|| format!("illegal gambit move '{uci}'"))?;
            board.make_move(mv);
        }
        Ok(board.to_fen())
    }
}

/// Built-in gambit catalog used by the study hub.
pub const GAMBIT_CATALOG: &[GambitLesson] = &[
    GambitLesson {
        id: "kings-gambit",
        name: "King's Gambit",
        eco: "C30",
        summary: "White offers the f-pawn for rapid development and an open f-file.",
        moves: &["e2e4", "e7e5", "f2f4"],
        key_ply: 2,
    },
    GambitLesson {
        id: "evans-gambit",
        name: "Evans Gambit",
        eco: "C51",
        summary: "White sacrifices a wing pawn to seize the center against the Italian.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5", "b2b4"],
        key_ply: 6,
    },
    GambitLesson {
        id: "danish-gambit",
        name: "Danish Gambit",
        eco: "C21",
        summary: "White doubles down on center tempo with aggressive pawn offers.",
        moves: &["e2e4", "e7e5", "d2d4", "e5d4", "c2c3"],
        key_ply: 4,
    },
    GambitLesson {
        id: "smith-morra",
        name: "Smith–Morra Gambit",
        eco: "B21",
        summary: "Against the Sicilian, White gambits a pawn for open lines and activity.",
        moves: &["e2e4", "c7c5", "d2d4", "c5d4", "c2c3"],
        key_ply: 4,
    },
    GambitLesson {
        id: "budapest",
        name: "Budapest Gambit",
        eco: "A51",
        summary: "Black immediately challenges White's center with an early knight leap.",
        moves: &["d2d4", "g8f6", "c2c4", "e7e5"],
        key_ply: 3,
    },
    GambitLesson {
        id: "benko",
        name: "Benko Gambit",
        eco: "A57",
        summary: "Black offers a queenside pawn for lasting pressure on the a- and b-files.",
        moves: &["d2d4", "g8f6", "c2c4", "c7c5", "d4d5", "b7b5"],
        key_ply: 5,
    },
];

pub fn find_gambit(id: &str) -> Option<&'static GambitLesson> {
    GAMBIT_CATALOG.iter().find(|lesson| lesson.id == id)
}

pub fn catalog() -> &'static [GambitLesson] {
    GAMBIT_CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_line_is_legal() {
        for lesson in GAMBIT_CATALOG {
            lesson
                .validate()
                .unwrap_or_else(|error| panic!("{}: {error}", lesson.id));
            assert!(!lesson.coaching_arrows(lesson.moves.len()).unwrap().is_empty());
        }
    }

    #[test]
    fn key_ply_fen_differs_from_start() {
        let lesson = find_gambit("kings-gambit").unwrap();
        let fen = lesson.fen_after_plies(lesson.key_ply + 1).unwrap();
        assert_ne!(fen, START_FEN);
    }
}
