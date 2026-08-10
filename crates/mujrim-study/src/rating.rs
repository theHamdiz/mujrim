//! Elo estimates for players and engines from observed game results.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatedResult {
    pub opponent_elo: f64,
    /// `1.0` for a win, `0.5` for a draw, and `0.0` for a loss.
    pub score: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EloEstimate {
    pub elo: f64,
    pub lower_95: f64,
    pub upper_95: f64,
    pub games: usize,
}

pub fn expected_score(player_elo: f64, opponent_elo: f64) -> f64 {
    1.0 / (1.0 + 10.0_f64.powf((opponent_elo - player_elo) / 400.0))
}

/// Maximum-likelihood performance rating with half-game regularization so an
/// undefeated sample remains finite. Returns `None` for an empty sample.
pub fn estimate_performance(results: &[RatedResult]) -> Option<EloEstimate> {
    if results.is_empty() {
        return None;
    }
    let observed = results
        .iter()
        .map(|result| result.score.clamp(0.0, 1.0))
        .sum::<f64>();
    let target = (observed + 0.5) / (results.len() as f64 + 1.0);
    let opponent_mean = results
        .iter()
        .map(|result| result.opponent_elo)
        .sum::<f64>()
        / results.len() as f64;
    let mut low = opponent_mean - 1_600.0;
    let mut high = opponent_mean + 1_600.0;
    for _ in 0..80 {
        let midpoint = (low + high) * 0.5;
        let expected = results
            .iter()
            .map(|result| expected_score(midpoint, result.opponent_elo))
            .sum::<f64>()
            / results.len() as f64;
        if expected < target {
            low = midpoint;
        } else {
            high = midpoint;
        }
    }
    let elo = (low + high) * 0.5;
    let information = results
        .iter()
        .map(|result| {
            let probability = expected_score(elo, result.opponent_elo);
            probability * (1.0 - probability)
        })
        .sum::<f64>()
        .max(0.01);
    let standard_error = (400.0 / std::f64::consts::LN_10) / information.sqrt();
    Some(EloEstimate {
        elo,
        lower_95: elo - 1.96 * standard_error,
        upper_95: elo + 1.96 * standard_error,
        games: results.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_samples_have_no_rating() {
        assert_eq!(estimate_performance(&[]), None);
    }

    #[test]
    fn balanced_results_track_opponent_strength() {
        let estimate = estimate_performance(&[
            RatedResult {
                opponent_elo: 2400.0,
                score: 1.0,
            },
            RatedResult {
                opponent_elo: 2400.0,
                score: 0.0,
            },
        ])
        .unwrap();
        assert!((estimate.elo - 2400.0).abs() < 0.01);
        assert!(estimate.lower_95 < estimate.elo);
        assert!(estimate.upper_95 > estimate.elo);
    }

    #[test]
    fn stronger_scores_produce_higher_performance_ratings() {
        let results = [
            RatedResult {
                opponent_elo: 2400.0,
                score: 1.0,
            },
            RatedResult {
                opponent_elo: 2400.0,
                score: 1.0,
            },
            RatedResult {
                opponent_elo: 2400.0,
                score: 1.0,
            },
            RatedResult {
                opponent_elo: 2400.0,
                score: 0.0,
            },
        ];
        let estimate = estimate_performance(&results).unwrap();
        assert!(estimate.elo > 2500.0);
        assert!(estimate.elo < 2700.0);
    }
}
