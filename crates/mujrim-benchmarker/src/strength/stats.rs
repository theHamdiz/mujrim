//! Statistical summaries and sequential stopping for paired matches.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameOutcome {
    Win,
    Draw,
    Loss,
}

impl GameOutcome {
    #[inline]
    pub const fn score(self) -> f64 {
        match self {
            Self::Win => 1.0,
            Self::Draw => 0.5,
            Self::Loss => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreCount {
    pub wins: u64,
    pub draws: u64,
    pub losses: u64,
}

/// Counts color-swapped opening pairs by their combined candidate score:
/// 0, 0.5, 1.0, 1.5, or 2.0 game points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairCount {
    pub bins: [u64; 5],
}

impl PairCount {
    pub fn push(&mut self, first: GameOutcome, second: GameOutcome) {
        let half_points = (2.0 * (first.score() + second.score())) as usize;
        self.bins[half_points] += 1;
    }

    pub fn pairs(self) -> u64 {
        self.bins.iter().sum()
    }

    pub fn score_rate(self) -> f64 {
        let pairs = self.pairs();
        if pairs == 0 {
            return 0.5;
        }
        const PAIR_SCORES: [f64; 5] = [0.0, 0.5, 1.0, 1.5, 2.0];
        let points = self
            .bins
            .iter()
            .zip(PAIR_SCORES)
            .map(|(count, score)| *count as f64 * score)
            .sum::<f64>();
        points / (2.0 * pairs as f64)
    }
}

impl ScoreCount {
    pub fn push(&mut self, outcome: GameOutcome) {
        match outcome {
            GameOutcome::Win => self.wins += 1,
            GameOutcome::Draw => self.draws += 1,
            GameOutcome::Loss => self.losses += 1,
        }
    }

    pub const fn games(self) -> u64 {
        self.wins + self.draws + self.losses
    }

    pub fn score_rate(self) -> f64 {
        let games = self.games();
        if games == 0 {
            0.5
        } else {
            (self.wins as f64 + 0.5 * self.draws as f64) / games as f64
        }
    }

    pub fn elo(self) -> f64 {
        score_to_elo(self.score_rate())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SprtDecision {
    Continue,
    AcceptH0,
    AcceptH1,
}

#[derive(Clone, Copy, Debug)]
pub struct Sprt {
    pub elo0: f64,
    pub elo1: f64,
    pub alpha: f64,
    pub beta: f64,
}

impl Default for Sprt {
    fn default() -> Self {
        Self {
            elo0: -3.0,
            elo1: 3.0,
            alpha: 0.05,
            beta: 0.05,
        }
    }
}

impl Sprt {
    pub fn bounds(self) -> (f64, f64) {
        (
            (self.beta / (1.0 - self.alpha)).ln(),
            ((1.0 - self.beta) / self.alpha).ln(),
        )
    }

    /// Log-likelihood ratio with the observed draw rate as a nuisance parameter.
    pub fn llr(self, scores: ScoreCount) -> f64 {
        if scores.games() == 0 {
            return 0.0;
        }

        let q0 = elo_to_score(self.elo0);
        let q1 = elo_to_score(self.elo1);
        let max_draw = 2.0 * q0.min(q1).min(1.0 - q0).min(1.0 - q1) - f64::EPSILON;
        let draw_rate = ((scores.draws as f64 + 1.0) / (scores.games() as f64 + 2.0))
            .clamp(0.0, max_draw.max(0.0));

        let probabilities = |score: f64| {
            let win = (score - draw_rate / 2.0).max(f64::MIN_POSITIVE);
            let loss = (1.0 - score - draw_rate / 2.0).max(f64::MIN_POSITIVE);
            (win, draw_rate.max(f64::MIN_POSITIVE), loss)
        };
        let (w0, d0, l0) = probabilities(q0);
        let (w1, d1, l1) = probabilities(q1);

        scores.wins as f64 * (w1 / w0).ln()
            + scores.draws as f64 * (d1 / d0).ln()
            + scores.losses as f64 * (l1 / l0).ln()
    }

    pub fn decision(self, scores: ScoreCount) -> SprtDecision {
        let (lower, upper) = self.bounds();
        let llr = self.llr(scores);
        if llr <= lower {
            SprtDecision::AcceptH0
        } else if llr >= upper {
            SprtDecision::AcceptH1
        } else {
            SprtDecision::Continue
        }
    }

    /// Generalized paired SPRT over the five possible two-game pair scores.
    /// The observed pentanomial shape is treated as a nuisance distribution,
    /// then exponentially tilted to each Elo hypothesis.
    pub fn paired_llr(self, pairs: PairCount) -> f64 {
        if pairs.pairs() == 0 {
            return 0.0;
        }

        let hypothesis = |elo| tilted_pair_probabilities(pairs, 2.0 * elo_to_score(elo));
        let h0 = hypothesis(self.elo0);
        let h1 = hypothesis(self.elo1);
        pairs
            .bins
            .iter()
            .zip(h0.iter().zip(h1.iter()))
            .map(|(count, (p0, p1))| *count as f64 * (p1 / p0).ln())
            .sum()
    }

    pub fn paired_decision(self, pairs: PairCount) -> SprtDecision {
        let (lower, upper) = self.bounds();
        let llr = self.paired_llr(pairs);
        if llr <= lower {
            SprtDecision::AcceptH0
        } else if llr >= upper {
            SprtDecision::AcceptH1
        } else {
            SprtDecision::Continue
        }
    }
}

fn tilted_pair_probabilities(pairs: PairCount, target_mean: f64) -> [f64; 5] {
    const SCORES: [f64; 5] = [0.0, 0.5, 1.0, 1.5, 2.0];
    const PRIOR: f64 = 0.5;
    let denominator = pairs.pairs() as f64 + PRIOR * 5.0;
    let base = pairs.bins.map(|count| (count as f64 + PRIOR) / denominator);

    let distribution = |lambda: f64| {
        let exponents = SCORES.map(|score| lambda * score);
        let max_exponent = exponents.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut weights = [0.0; 5];
        for index in 0..5 {
            weights[index] = base[index] * (exponents[index] - max_exponent).exp();
        }
        let total = weights.iter().sum::<f64>();
        weights.map(|weight| weight / total)
    };

    let target = target_mean.clamp(f64::EPSILON, 2.0 - f64::EPSILON);
    let mut low = -64.0;
    let mut high = 64.0;
    for _ in 0..80 {
        let middle = (low + high) / 2.0;
        let probabilities = distribution(middle);
        let mean = probabilities
            .iter()
            .zip(SCORES)
            .map(|(probability, score)| probability * score)
            .sum::<f64>();
        if mean < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    distribution((low + high) / 2.0)
}

pub fn elo_to_score(elo: f64) -> f64 {
    1.0 / (1.0 + 10.0_f64.powf(-elo / 400.0))
}

pub fn score_to_elo(score: f64) -> f64 {
    let bounded = score.clamp(1e-9, 1.0 - 1e-9);
    400.0 * (bounded / (1.0 - bounded)).log10()
}

/// A Wilson interval over pair scores, with each opening/color pair as one sample.
/// Using pairs as the sampling unit retains their covariance, while Wilson's
/// bounded-score variance prevents a zero-width interval after identical results.
pub fn paired_elo_interval(pairs: &[(GameOutcome, GameOutcome)]) -> (f64, f64) {
    if pairs.is_empty() {
        return (f64::NEG_INFINITY, f64::INFINITY);
    }

    let mean = pairs
        .iter()
        .map(|(a, b)| (a.score() + b.score()) / 2.0)
        .sum::<f64>()
        / pairs.len() as f64;
    let n = pairs.len() as f64;
    let z = 1.959_963_984_540_054;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (mean + z2 / (2.0 * n)) / denominator;
    let margin = z * (mean * (1.0 - mean) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    (
        score_to_elo((center - margin).clamp(1e-9, 1.0 - 1e-9)),
        score_to_elo((center + margin).clamp(1e-9, 1.0 - 1e-9)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elo_conversion_round_trips() {
        for elo in [-400.0, -25.0, 0.0, 80.0, 400.0] {
            assert!((score_to_elo(elo_to_score(elo)) - elo).abs() < 1e-9);
        }
    }

    #[test]
    fn balanced_score_is_zero_elo() {
        let scores = ScoreCount {
            wins: 10,
            draws: 20,
            losses: 10,
        };
        assert!(scores.elo().abs() < 1e-9);
    }

    #[test]
    fn sprt_reaches_each_boundary() {
        let sprt = Sprt::default();
        assert_eq!(
            sprt.decision(ScoreCount {
                wins: 10_000,
                draws: 0,
                losses: 0
            }),
            SprtDecision::AcceptH1
        );
        assert_eq!(
            sprt.decision(ScoreCount {
                wins: 0,
                draws: 0,
                losses: 10_000
            }),
            SprtDecision::AcceptH0
        );
    }

    #[test]
    fn paired_sprt_reaches_each_boundary() {
        let sprt = Sprt::default();
        let mut winning = PairCount::default();
        let mut losing = PairCount::default();
        for _ in 0..10_000 {
            winning.push(GameOutcome::Win, GameOutcome::Win);
            losing.push(GameOutcome::Loss, GameOutcome::Loss);
        }
        assert_eq!(sprt.paired_decision(winning), SprtDecision::AcceptH1);
        assert_eq!(sprt.paired_decision(losing), SprtDecision::AcceptH0);
    }

    #[test]
    fn wide_screen_rejects_a_clear_regression_before_strict_sprt() {
        let strict = Sprt::default();
        let screen = Sprt {
            elo0: -30.0,
            elo1: 10.0,
            ..strict
        };
        let pairs_to_reject = |sprt: Sprt| {
            let mut pairs = PairCount::default();
            for count in 1_usize..=1_000 {
                pairs.push(GameOutcome::Loss, GameOutcome::Loss);
                if sprt.paired_decision(pairs) == SprtDecision::AcceptH0 {
                    return count;
                }
            }
            panic!("clear regression did not reach the lower SPRT boundary");
        };

        let screen_pairs = pairs_to_reject(screen);
        let strict_pairs = pairs_to_reject(strict);

        assert!(screen_pairs.saturating_mul(4) < strict_pairs);
    }

    #[test]
    fn paired_sprt_is_neutral_for_one_point_pairs() {
        let sprt = Sprt::default();
        let mut pairs = PairCount::default();
        for _ in 0..100 {
            pairs.push(GameOutcome::Win, GameOutcome::Loss);
            pairs.push(GameOutcome::Draw, GameOutcome::Draw);
        }
        assert!(sprt.paired_llr(pairs).abs() < 1e-9);
        assert_eq!(pairs.score_rate(), 0.5);
    }

    #[test]
    fn paired_interval_contains_equal_score() {
        let pairs = vec![
            (GameOutcome::Win, GameOutcome::Loss),
            (GameOutcome::Loss, GameOutcome::Win),
            (GameOutcome::Draw, GameOutcome::Draw),
        ];
        let (low, high) = paired_elo_interval(&pairs);
        assert!(low <= 0.0 && high >= 0.0);
    }

    #[test]
    fn identical_draw_pairs_still_have_uncertainty() {
        let pairs = vec![(GameOutcome::Draw, GameOutcome::Draw); 8];
        let (low, high) = paired_elo_interval(&pairs);
        assert!(low < 0.0 && high > 0.0);
        assert!(low.is_finite() && high.is_finite());
    }
}
