//! Curated gambit courses for interactive opening preparation.

use crate::board_marks::{
    ArrowRole, BoardArrow, MarkColor, arrows_from_uci_pv, arrows_from_uci_pv_offset,
};
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

/// Runtime gambit line: curated catalog plus book-extended / book-discovered lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGambit {
    pub id: String,
    pub name: String,
    pub eco: String,
    pub summary: String,
    pub moves: Vec<String>,
    pub key_ply: usize,
    pub in_book: bool,
}

impl From<&GambitLesson> for OwnedGambit {
    fn from(lesson: &GambitLesson) -> Self {
        Self {
            id: lesson.id.to_owned(),
            name: lesson.name.to_owned(),
            eco: lesson.eco.to_owned(),
            summary: lesson.summary.to_owned(),
            moves: lesson.moves.iter().map(|mv| (*mv).to_owned()).collect(),
            key_ply: lesson.key_ply,
            in_book: false,
        }
    }
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
        fen_after_uci(self.moves.iter().copied(), plies)
    }
}

impl OwnedGambit {
    pub fn validate(&self) -> Result<(), String> {
        RepertoireLine {
            name: self.name.clone(),
            moves: self.moves.clone(),
        }
        .validate(START_FEN)
    }

    pub fn fen_after_plies(&self, plies: usize) -> Result<String, String> {
        fen_after_uci(self.moves.iter().map(String::as_str), plies)
    }

    /// Remaining lesson arrows from the current ply, numbered with absolute ply indices.
    pub fn coaching_arrows_from(&self, played: usize) -> Result<Vec<BoardArrow>, String> {
        let fen = self.fen_after_plies(played)?;
        let rest: Vec<String> = self.moves.iter().skip(played).cloned().collect();
        if rest.is_empty() {
            return Ok(Vec::new());
        }
        arrows_from_uci_pv_offset(
            &fen,
            &rest,
            MarkColor::Orange,
            ArrowRole::Gambit,
            rest.len().min(8),
            Some(&self.name),
            played as u8,
        )
    }
}

fn fen_after_uci<'a>(
    moves: impl IntoIterator<Item = &'a str>,
    plies: usize,
) -> Result<String, String> {
    types::init();
    let mut board = types::Board::from_fen(START_FEN)?;
    for (index, uci) in moves.into_iter().take(plies).enumerate() {
        let mv = board
            .generate_legal_moves()
            .into_iter()
            .find(|candidate| candidate.to_uci() == uci)
            .copied()
            .ok_or_else(|| format!("illegal gambit move '{uci}' at ply {}", index + 1))?;
        board.make_move(mv);
    }
    Ok(board.to_fen())
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
        id: "kings-gambit-accepted",
        name: "King's Gambit Accepted",
        eco: "C33",
        summary: "Black takes on f4. White hunts the pawn back with Nf3 and Bc4.",
        moves: &["e2e4", "e7e5", "f2f4", "e5f4", "g1f3"],
        key_ply: 3,
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
        id: "goring-gambit",
        name: "Göring Gambit",
        eco: "C44",
        summary: "A Scotch cousin: c3 offers a pawn for a lead in development.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "d2d4", "e5d4", "c2c3"],
        key_ply: 6,
    },
    GambitLesson {
        id: "scotch-gambit",
        name: "Scotch Gambit",
        eco: "C44",
        summary: "White leaves the d4 pawn and develops Bc4 against the Scotch.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "d2d4", "e5d4", "f1c4"],
        key_ply: 6,
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
        id: "wing-gambit",
        name: "Sicilian Wing Gambit",
        eco: "B20",
        summary: "White flings the b-pawn to deflect Black's c-pawn from the center.",
        moves: &["e2e4", "c7c5", "b2b4"],
        key_ply: 2,
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
    GambitLesson {
        id: "blumenfeld",
        name: "Blumenfeld Gambit",
        eco: "E10",
        summary: "Black hits d5 with …b5, seeking a Benko-style queenside initiative.",
        moves: &[
            "d2d4", "g8f6", "c2c4", "e7e6", "g1f3", "c7c5", "d4d5", "b7b5",
        ],
        key_ply: 7,
    },
    GambitLesson {
        id: "queens-gambit",
        name: "Queen's Gambit",
        eco: "D06",
        summary: "The classical c-pawn offer. Declining keeps a solid center; taking concedes it.",
        moves: &["d2d4", "d7d5", "c2c4"],
        key_ply: 2,
    },
    GambitLesson {
        id: "queens-gambit-accepted",
        name: "Queen's Gambit Accepted",
        eco: "D20",
        summary: "Black takes on c4. White recaptures tempo with e3/e4 and development.",
        moves: &["d2d4", "d7d5", "c2c4", "d5c4"],
        key_ply: 3,
    },
    GambitLesson {
        id: "albin",
        name: "Albin Countergambit",
        eco: "D08",
        summary: "Black answers the Queen's Gambit by offering the e-pawn for a wedge on d4.",
        moves: &["d2d4", "d7d5", "c2c4", "e7e5"],
        key_ply: 3,
    },
    GambitLesson {
        id: "blackmar-diemer",
        name: "Blackmar–Diemer Gambit",
        eco: "D00",
        summary: "White offers the e-pawn on move two for open lines and attacking chances.",
        moves: &["d2d4", "d7d5", "e2e4"],
        key_ply: 2,
    },
    GambitLesson {
        id: "englund",
        name: "Englund Gambit",
        eco: "A40",
        summary: "Black meets 1.d4 with …e5, forcing White to accept or concede the center.",
        moves: &["d2d4", "e7e5"],
        key_ply: 1,
    },
    GambitLesson {
        id: "froms",
        name: "From's Gambit",
        eco: "A02",
        summary: "Against Bird's Opening, Black offers the e-pawn to rip open White's kingside.",
        moves: &["f2f4", "e7e5"],
        key_ply: 1,
    },
    GambitLesson {
        id: "staunton",
        name: "Staunton Gambit",
        eco: "A83",
        summary: "White meets the Dutch with e4, sacrificing a pawn for development.",
        moves: &["d2d4", "f7f5", "e2e4"],
        key_ply: 2,
    },
    GambitLesson {
        id: "latvian",
        name: "Latvian Gambit",
        eco: "C40",
        summary: "Black's …f5 after 1.e4 e5 Nf3 — a King's Gambit with colors reversed.",
        moves: &["e2e4", "e7e5", "g1f3", "f7f5"],
        key_ply: 3,
    },
    GambitLesson {
        id: "elephant",
        name: "Elephant Gambit",
        eco: "C40",
        summary: "Black strikes in the center with …d5 instead of defending e5.",
        moves: &["e2e4", "e7e5", "g1f3", "d7d5"],
        key_ply: 3,
    },
    GambitLesson {
        id: "vienna-gambit",
        name: "Vienna Gambit",
        eco: "C25",
        summary: "After Nc3, White offers the f-pawn while the queen still covers d4.",
        moves: &["e2e4", "e7e5", "b1c3", "g8f6", "f2f4"],
        key_ply: 4,
    },
    GambitLesson {
        id: "urusov",
        name: "Urusov Gambit",
        eco: "C24",
        summary: "White offers the e-pawn after Bc4 to drag Black's knight offside.",
        moves: &["e2e4", "e7e5", "f1c4", "g8f6", "d2d4"],
        key_ply: 4,
    },
    GambitLesson {
        id: "traxler",
        name: "Traxler Counterattack",
        eco: "C57",
        summary: "Black ignores Ng5 and counters with …Bc5, offering f7 for a king hunt.",
        moves: &[
            "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "f3g5", "f8c5",
        ],
        key_ply: 7,
    },
    GambitLesson {
        id: "fried-liver",
        name: "Fried Liver Attack",
        eco: "C57",
        summary: "White sacrifices a knight on f7 after the Two Knights …d5.",
        moves: &[
            "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "f3g5", "d7d5", "e4d5", "f6d5", "g5f7",
        ],
        key_ply: 10,
    },
    GambitLesson {
        id: "halloween",
        name: "Halloween Gambit",
        eco: "C47",
        summary: "White gives a knight on e5 in the Four Knights for a massive pawn center.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "b1c3", "g8f6", "f3e5"],
        key_ply: 6,
    },
    GambitLesson {
        id: "cochrane",
        name: "Cochrane Gambit",
        eco: "C42",
        summary: "In the Petroff, White plants a knight on f7 instead of retreating.",
        moves: &["e2e4", "e7e5", "g1f3", "g8f6", "f3e5", "d7d6", "e5f7"],
        key_ply: 6,
    },
    GambitLesson {
        id: "rousseau",
        name: "Rousseau Gambit",
        eco: "C50",
        summary: "Black meets the Italian with …f5, mirroring a King's Gambit.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f7f5"],
        key_ply: 5,
    },
    GambitLesson {
        id: "blackburne-shilling",
        name: "Blackburne Shilling Gambit",
        eco: "C50",
        summary: "Black's …Nd4 invites Nxe5, then …Qg5 starts a famous trap.",
        moves: &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "c6d4"],
        key_ply: 5,
    },
    GambitLesson {
        id: "tennison",
        name: "Tennison Gambit",
        eco: "A06",
        summary: "White answers the Scandinavian with Nf3, offering the e-pawn.",
        moves: &["e2e4", "d7d5", "g1f3"],
        key_ply: 2,
    },
    GambitLesson {
        id: "milner-barry",
        name: "Milner-Barry Gambit",
        eco: "C02",
        summary: "In the French Advance, White leaves d4 to accelerate the attack.",
        moves: &[
            "e2e4", "e7e6", "d2d4", "d7d5", "e4e5", "c7c5", "c2c3", "b8c6", "g1f3", "d8b6", "f1d3",
        ],
        key_ply: 10,
    },
];

pub fn find_gambit(id: &str) -> Option<&'static GambitLesson> {
    GAMBIT_CATALOG.iter().find(|lesson| lesson.id == id)
}

pub fn catalog() -> &'static [GambitLesson] {
    GAMBIT_CATALOG
}

pub fn find_owned<'a>(id: &str, catalog: &'a [OwnedGambit]) -> Option<&'a OwnedGambit> {
    catalog.iter().find(|lesson| lesson.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_line_is_legal() {
        assert!(GAMBIT_CATALOG.len() >= 24);
        for lesson in GAMBIT_CATALOG {
            lesson
                .validate()
                .unwrap_or_else(|error| panic!("{}: {error}", lesson.id));
            assert!(
                !lesson
                    .coaching_arrows(lesson.moves.len())
                    .unwrap()
                    .is_empty()
            );
            let owned = OwnedGambit::from(lesson);
            assert_eq!(owned.id, lesson.id);
            let played = lesson.key_ply.min(lesson.moves.len().saturating_sub(1));
            let from_key = owned.coaching_arrows_from(played).unwrap();
            assert!(
                !from_key.is_empty(),
                "{} should show the remaining offer",
                lesson.id
            );
            assert_eq!(from_key[0].step, Some((played + 1) as u8));
        }
    }

    #[test]
    fn key_ply_fen_differs_from_start() {
        let lesson = find_gambit("kings-gambit").unwrap();
        let fen = lesson.fen_after_plies(lesson.key_ply + 1).unwrap();
        assert_ne!(fen, START_FEN);
    }
}
