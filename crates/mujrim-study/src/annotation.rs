//! Deterministic move-quality annotations for coaching and game review.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveAnnotation {
    Aura,
    Brilliant,
    Great,
    Best,
    Excellent,
    Good,
    Ok,
    Book,
    Novelty,
    Inaccuracy,
    Mistake,
    Blunder,
}

impl MoveAnnotation {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Aura => "!!!",
            Self::Brilliant => "!!",
            Self::Great => "!",
            Self::Best => "★",
            Self::Excellent => "✓✓",
            Self::Good => "✓",
            Self::Ok => "",
            Self::Book => "B",
            Self::Novelty => "N",
            Self::Inaccuracy => "?!",
            Self::Mistake => "?",
            Self::Blunder => "??",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Aura => "Aura Move",
            Self::Brilliant => "Brilliant",
            Self::Great => "Great Move",
            Self::Best => "Best Move",
            Self::Excellent => "Excellent",
            Self::Good => "Good",
            Self::Ok => "OK",
            Self::Book => "Book",
            Self::Novelty => "Novelty",
            Self::Inaccuracy => "Inaccuracy",
            Self::Mistake => "Mistake",
            Self::Blunder => "Blunder",
        }
    }

    /// Chess.com Game Review badge fill colors (RGB 0–255).
    ///
    /// Sourced from chess.com's classification palette used on destination-square
    /// markers (Brilliant teal, Great blue, Best/Excellent green, Book brown,
    /// Inaccuracy yellow, Mistake orange, Blunder red).
    pub const fn chess_com_rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Aura => (27, 172, 166),       // Brilliant teal, brighter family
            Self::Brilliant => (27, 172, 166),  // #1BACA6
            Self::Great => (92, 139, 176),      // #5C8BB0
            Self::Best => (150, 188, 75),       // #96BC4B
            Self::Excellent => (129, 182, 76),  // #81B64C
            Self::Good => (149, 183, 118),      // #95B776
            Self::Ok => (160, 160, 160),        // muted neutral
            Self::Book => (210, 166, 121),      // #D2A679
            Self::Novelty => (210, 166, 121),   // book-family brown
            Self::Inaccuracy => (247, 198, 49), // #F7C631
            Self::Mistake => (230, 143, 50),    // #E68F32
            Self::Blunder => (224, 40, 40),     // #E02828
        }
    }

    pub const fn shows_board_badge(self) -> bool {
        !matches!(self, Self::Ok)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnnotationContext {
    /// Best engine score before the move, from the mover's perspective.
    pub best_score_cp: i32,
    /// Score after the played move, from the same perspective.
    pub played_score_cp: i32,
    /// Score of the second-best legal move, when multi-PV analysis is available.
    pub second_best_score_cp: Option<i32>,
    pub is_sacrifice: bool,
    pub is_best_move: bool,
    pub is_only_move: bool,
    pub position_in_opening_database: bool,
    pub move_in_opening_database: bool,
}

impl AnnotationContext {
    pub fn classify(self) -> MoveAnnotation {
        if self.move_in_opening_database {
            return MoveAnnotation::Book;
        }

        let cp_loss = self
            .best_score_cp
            .saturating_sub(self.played_score_cp)
            .max(0);
        let probability_loss =
            win_probability(self.best_score_cp) - win_probability(self.played_score_cp);
        let uniqueness = self
            .second_best_score_cp
            .map_or(0, |score| self.best_score_cp.saturating_sub(score));

        if self.position_in_opening_database && cp_loss <= 35 && probability_loss <= 0.03 {
            return MoveAnnotation::Novelty;
        }
        if cp_loss <= 10
            && probability_loss <= 0.005
            && self.is_sacrifice
            && self.is_best_move
            && (self.is_only_move || uniqueness >= 150 || self.played_score_cp >= 300)
            && self.played_score_cp >= 200
        {
            return MoveAnnotation::Aura;
        }
        if cp_loss <= 20
            && probability_loss <= 0.01
            && self.is_sacrifice
            && (self.is_best_move || self.is_only_move || uniqueness >= 80)
        {
            return MoveAnnotation::Brilliant;
        }
        if cp_loss <= 30 && probability_loss <= 0.015 && (self.is_only_move || uniqueness >= 100) {
            return MoveAnnotation::Great;
        }
        if cp_loss >= 300 || probability_loss >= 0.25 {
            return MoveAnnotation::Blunder;
        }
        if cp_loss >= 150 || probability_loss >= 0.12 {
            return MoveAnnotation::Mistake;
        }
        if cp_loss >= 75 || probability_loss >= 0.05 {
            return MoveAnnotation::Inaccuracy;
        }
        if cp_loss <= 10 || probability_loss <= 0.005 {
            return MoveAnnotation::Best;
        }
        if cp_loss <= 25 || probability_loss <= 0.015 {
            return MoveAnnotation::Excellent;
        }
        if cp_loss <= 60 || probability_loss <= 0.035 {
            return MoveAnnotation::Good;
        }
        MoveAnnotation::Ok
    }
}

/// Smooth score conversion used to compare errors consistently in winning,
/// equal, and losing positions.
pub fn win_probability(score_cp: i32) -> f64 {
    let bounded = score_cp.clamp(-4_000, 4_000) as f64;
    1.0 / (1.0 + (-bounded / 400.0).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_annotation_symbols_are_stable() {
        assert_eq!(MoveAnnotation::Aura.symbol(), "!!!");
        assert_eq!(MoveAnnotation::Brilliant.symbol(), "!!");
        assert_eq!(MoveAnnotation::Great.symbol(), "!");
        assert_eq!(MoveAnnotation::Blunder.symbol(), "??");
        assert_eq!(MoveAnnotation::Mistake.symbol(), "?");
        assert_eq!(MoveAnnotation::Inaccuracy.symbol(), "?!");
        assert_eq!(MoveAnnotation::Novelty.symbol(), "N");
    }

    #[test]
    fn chess_com_badge_colors_match_review_palette() {
        assert_eq!(MoveAnnotation::Brilliant.chess_com_rgb(), (27, 172, 166));
        assert_eq!(MoveAnnotation::Great.chess_com_rgb(), (92, 139, 176));
        assert_eq!(MoveAnnotation::Best.chess_com_rgb(), (150, 188, 75));
        assert_eq!(MoveAnnotation::Excellent.chess_com_rgb(), (129, 182, 76));
        assert_eq!(MoveAnnotation::Good.chess_com_rgb(), (149, 183, 118));
        assert_eq!(MoveAnnotation::Book.chess_com_rgb(), (210, 166, 121));
        assert_eq!(MoveAnnotation::Inaccuracy.chess_com_rgb(), (247, 198, 49));
        assert_eq!(MoveAnnotation::Mistake.chess_com_rgb(), (230, 143, 50));
        assert_eq!(MoveAnnotation::Blunder.chess_com_rgb(), (224, 40, 40));
        assert!(!MoveAnnotation::Ok.shows_board_badge());
        assert!(MoveAnnotation::Brilliant.shows_board_badge());
    }

    #[test]
    fn aura_requires_a_winning_unique_sacrifice() {
        let context = AnnotationContext {
            best_score_cp: 350,
            played_score_cp: 348,
            second_best_score_cp: Some(100),
            is_sacrifice: true,
            is_best_move: true,
            ..Default::default()
        };
        assert_eq!(context.classify(), MoveAnnotation::Aura);
    }

    #[test]
    fn large_eval_losses_receive_error_annotations() {
        let classify = |loss: i32| {
            AnnotationContext {
                best_score_cp: 0,
                played_score_cp: -loss,
                ..Default::default()
            }
            .classify()
        };
        assert_eq!(classify(90), MoveAnnotation::Inaccuracy);
        assert_eq!(classify(180), MoveAnnotation::Mistake);
        assert_eq!(classify(500), MoveAnnotation::Blunder);
    }

    #[test]
    fn known_opening_and_sound_novelty_are_distinct() {
        let book = AnnotationContext {
            move_in_opening_database: true,
            ..Default::default()
        };
        assert_eq!(book.classify(), MoveAnnotation::Book);

        let novelty = AnnotationContext {
            best_score_cp: 20,
            played_score_cp: 10,
            position_in_opening_database: true,
            ..Default::default()
        };
        assert_eq!(novelty.classify(), MoveAnnotation::Novelty);
    }
}
