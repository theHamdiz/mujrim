//! Puzzle validation and spaced-repetition scheduling.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Puzzle {
    pub id: String,
    pub fen: String,
    pub solution: Vec<String>,
    pub themes: Vec<String>,
    pub rating: u32,
}

impl Puzzle {
    pub fn validate(&self) -> Result<(), String> {
        if self.solution.is_empty() {
            return Err("puzzle solution is empty".to_owned());
        }
        types::init();
        let mut board = types::Board::from_fen(&self.fen)?;
        for (ply, uci) in self.solution.iter().enumerate() {
            let mv = board
                .generate_legal_moves()
                .into_iter()
                .find(|mv| mv.to_uci() == *uci)
                .copied()
                .ok_or_else(|| format!("illegal puzzle move '{uci}' at ply {}", ply + 1))?;
            board.make_move(mv);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewSchedule {
    pub repetitions: u32,
    pub interval_days: u32,
    pub ease_factor: f64,
    pub due_day: u64,
}

impl Default for ReviewSchedule {
    fn default() -> Self {
        Self {
            repetitions: 0,
            interval_days: 0,
            ease_factor: 2.5,
            due_day: 0,
        }
    }
}

impl ReviewSchedule {
    /// Applies an SM-2 grade from 0 (complete failure) through 5 (perfect).
    pub fn review(self, grade: u8, today: u64) -> Self {
        let grade = grade.min(5);
        let mut next = self;
        if grade < 3 {
            next.repetitions = 0;
            next.interval_days = 1;
        } else {
            next.repetitions += 1;
            next.interval_days = match next.repetitions {
                1 => 1,
                2 => 6,
                _ => ((self.interval_days.max(1) as f64 * self.ease_factor).round() as u32).max(1),
            };
        }
        let difficulty = f64::from(5 - grade);
        next.ease_factor =
            (self.ease_factor + 0.1 - difficulty * (0.08 + difficulty * 0.02)).max(1.3);
        next.due_day = today.saturating_add(u64::from(next.interval_days));
        next
    }

    pub const fn is_due(self, today: u64) -> bool {
        self.due_day <= today
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_puzzle_line_is_legal() {
        let puzzle = Puzzle {
            id: "opening".to_owned(),
            fen: crate::opening::START_FEN.to_owned(),
            solution: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            themes: vec!["development".to_owned()],
            rating: 800,
        };
        assert!(puzzle.validate().is_ok());
    }

    #[test]
    fn successful_reviews_expand_interval_and_failures_reset_it() {
        let first = ReviewSchedule::default().review(5, 100);
        let second = first.review(5, 101);
        let third = second.review(5, 107);
        assert_eq!(first.interval_days, 1);
        assert_eq!(second.interval_days, 6);
        assert!(third.interval_days > second.interval_days);
        let failed = third.review(1, 120);
        assert_eq!(failed.repetitions, 0);
        assert_eq!(failed.interval_days, 1);
        assert!(failed.is_due(121));
    }
}
