//! Elo estimates for players and engines from observed game results.
//!
//! Absolute Elo is identified only up to a scale. Known engines (Stockfish on the
//! CCRL 40/15 scale used elsewhere in this repo) pin that scale. Unknown engines
//! get a weak prior at 2000 — not 3000 — so club-strength programs can land near
//! 1500–2200 instead of being forced into super-GM numbers.

/// CCRL 40/15 Stockfish reference used as a scale anchor when Stockfish plays.
pub const STOCKFISH_REFERENCE_ELO: f64 = 3_612.0;
/// Uninformative prior for engines with no published or seeded rating.
pub const UNANCHORED_PRIOR_ELO: f64 = 2_000.0;
const RATING_FLOOR: f64 = 800.0;
const RATING_CEILING: f64 = 4_500.0;
const STOCKFISH_VIRTUAL_DRAWS: f64 = 48.0;
const SEEDED_VIRTUAL_DRAWS: f64 = 32.0;
const UNKNOWN_VIRTUAL_DRAWS: f64 = 1.0;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatingPrior {
    pub elo: f64,
    pub virtual_draws: f64,
}

impl RatingPrior {
    pub fn for_engine(name: &str, seed_elo: Option<f64>) -> Self {
        if let Some(elo) = seed_elo {
            return Self {
                elo: elo.clamp(RATING_FLOOR, RATING_CEILING),
                virtual_draws: SEEDED_VIRTUAL_DRAWS,
            };
        }
        if let Some(elo) = published_reference_elo(name) {
            return Self {
                elo,
                virtual_draws: STOCKFISH_VIRTUAL_DRAWS,
            };
        }
        if let Some(elo) = published_seed_elo(name) {
            return Self {
                elo,
                virtual_draws: SEEDED_VIRTUAL_DRAWS,
            };
        }
        Self {
            elo: UNANCHORED_PRIOR_ELO,
            virtual_draws: UNKNOWN_VIRTUAL_DRAWS,
        }
    }
}

pub fn expected_score(player_elo: f64, opponent_elo: f64) -> f64 {
    1.0 / (1.0 + 10.0_f64.powf((opponent_elo - player_elo) / 400.0))
}

/// Published scale pin. Only Stockfish is treated as a CCRL anchor.
/// Unknown names are not assumed to be 3000+.
pub fn published_reference_elo(name: &str) -> Option<f64> {
    let key = normalize_engine_key(name);
    if is_stockfish_reference(&key) {
        Some(STOCKFISH_REFERENCE_ELO)
    } else {
        None
    }
}

/// Soft seeds for well-known engines. These are not CCRL 40/15 ratings.
pub fn published_seed_elo(name: &str) -> Option<f64> {
    let key = normalize_engine_key(name);
    if is_stockfish_reference(&key) {
        return Some(STOCKFISH_REFERENCE_ELO);
    }
    let tokens: Vec<&str> = key.split(' ').collect();
    if tokens.iter().any(|token| token.contains("mujrim"))
        && tokens
            .iter()
            .any(|token| token.contains("lc0") || token.contains("leela"))
    {
        return None;
    }
    for token in &tokens {
        if *token == "lc0" || token.starts_with("leela") {
            return Some(3_550.0);
        }
        if *token == "v60" || token.contains("v60") || *token == "reckless" {
            return Some(3_560.0);
        }
        if *token == "akimbo" || *token == "ak" {
            return Some(3_480.0);
        }
        if *token == "viridithas" || *token == "viri" {
            return Some(3_520.0);
        }
        if *token == "obsidian" || *token == "obs" {
            return Some(3_540.0);
        }
        if *token == "plentychess" || *token == "plenty" {
            return Some(3_500.0);
        }
    }
    None
}

pub fn seed_elo_for_engine(name: &str) -> Option<f64> {
    published_reference_elo(name).or_else(|| published_seed_elo(name))
}

/// Live table caption: these numbers are event ratings, not CCRL 40/15.
pub const EVENT_ELO_CAPTION: &str = "event Elo from this field (not CCRL 40/15)";

fn normalize_engine_key(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_stockfish_reference(key: &str) -> bool {
    if key.contains("fairy") {
        return false;
    }
    key.split(' ').any(|token| token.starts_with("stockfish"))
}

/// Maximum-likelihood performance rating with half-game regularization so an
/// undefeated sample remains finite. Returns `None` for an empty sample.
pub fn estimate_performance(results: &[RatedResult]) -> Option<EloEstimate> {
    solve_rating(results, true)
}

/// Bradley-Terry field ratings from the complete result list, updated as games
/// finish. Seeded / Stockfish engines pin the absolute scale; everyone else is
/// estimated from the games plus a weak 2000 prior.
pub fn estimate_field_ratings(
    names: &[String],
    seeds: &[Option<f64>],
    games: &[(usize, usize, f64)],
) -> Vec<Option<EloEstimate>> {
    let n = names.len();
    if n == 0 {
        return Vec::new();
    }
    let priors: Vec<RatingPrior> = (0..n)
        .map(|index| {
            RatingPrior::for_engine(
                names.get(index).map(String::as_str).unwrap_or(""),
                seeds.get(index).copied().flatten(),
            )
        })
        .collect();
    let mut ratings: Vec<f64> = priors.iter().map(|prior| prior.elo).collect();
    let mut played = vec![false; n];
    for &(white, black, _) in games {
        if white < n {
            played[white] = true;
        }
        if black < n {
            played[black] = true;
        }
    }
    for _ in 0..80 {
        let previous = ratings.clone();
        for index in 0..n {
            if !played[index] {
                continue;
            }
            let samples = samples_for(index, &ratings, games, priors[index]);
            if let Some(estimate) = solve_rating(&samples, false) {
                ratings[index] = estimate.elo;
            }
        }
        let drift: f64 = ratings
            .iter()
            .zip(&previous)
            .map(|(next, prev)| (next - prev).abs())
            .sum();
        if drift < 0.05 {
            break;
        }
    }
    (0..n)
        .map(|index| {
            if !played[index] {
                return None;
            }
            let samples = samples_for(index, &ratings, games, priors[index]);
            solve_rating(&samples, false).map(|mut estimate| {
                estimate.games = games
                    .iter()
                    .filter(|(white, black, _)| *white == index || *black == index)
                    .count();
                estimate.elo = shrink_toward_prior(estimate.elo, priors[index].elo, estimate.games);
                estimate
            })
        })
        .collect()
}

/// Force displayed ratings to be non-increasing with standings order so a
/// table leader cannot show a lower Elo than engines ranked below it.
pub fn apply_isotonic_ratings(ratings: &mut [Option<EloEstimate>]) {
    let mut values: Vec<(usize, f64)> = ratings
        .iter()
        .enumerate()
        .filter_map(|(index, estimate)| estimate.as_ref().map(|value| (index, value.elo)))
        .collect();
    if values.len() < 2 {
        return;
    }
    let mut start = 0;
    while start < values.len() {
        let mut end = start + 1;
        let mut sum = values[start].1;
        while end < values.len() && values[end].1 > sum / (end - start) as f64 {
            sum += values[end].1;
            end += 1;
        }
        let mean = sum / (end - start) as f64;
        for slot in &mut values[start..end] {
            slot.1 = mean;
        }
        start = end;
    }
    for i in 1..values.len() {
        if values[i].1 > values[i - 1].1 {
            values[i].1 = values[i - 1].1;
        }
    }
    for (index, elo) in values {
        if let Some(estimate) = ratings.get_mut(index).and_then(|slot| slot.as_mut()) {
            estimate.elo = elo;
            if estimate.lower_95 > elo {
                estimate.lower_95 = elo;
            }
            if estimate.upper_95 < elo {
                estimate.upper_95 = elo;
            }
        }
    }
}

fn shrink_toward_prior(mle: f64, prior: f64, games: usize) -> f64 {
    if games >= 8 {
        return mle.clamp(RATING_FLOOR, RATING_CEILING);
    }
    let weight = games as f64 / (games as f64 + 3.0);
    (prior + (mle - prior) * weight).clamp(RATING_FLOOR, RATING_CEILING)
}

fn samples_for(
    index: usize,
    ratings: &[f64],
    games: &[(usize, usize, f64)],
    prior: RatingPrior,
) -> Vec<RatedResult> {
    let mut samples = Vec::new();
    let played = games
        .iter()
        .filter(|(white, black, _)| *white == index || *black == index)
        .count() as f64;
    let virtual_draws = (prior.virtual_draws / (1.0 + played * 0.35))
        .max(0.0)
        .round() as usize;
    for _ in 0..virtual_draws {
        samples.push(RatedResult {
            opponent_elo: prior.elo,
            score: 0.5,
        });
    }
    for &(white, black, white_score) in games {
        let score = white_score.clamp(0.0, 1.0);
        if white == index && black < ratings.len() {
            samples.push(RatedResult {
                opponent_elo: ratings[black],
                score,
            });
        } else if black == index && white < ratings.len() {
            samples.push(RatedResult {
                opponent_elo: ratings[white],
                score: 1.0 - score,
            });
        }
    }
    samples
}

fn solve_rating(results: &[RatedResult], half_game: bool) -> Option<EloEstimate> {
    if results.is_empty() {
        return None;
    }
    let observed = results
        .iter()
        .map(|result| result.score.clamp(0.0, 1.0))
        .sum::<f64>();
    let n = results.len() as f64;
    let target = if half_game {
        (observed + 0.5) / (n + 1.0)
    } else {
        observed / n
    };
    let opponent_mean = results
        .iter()
        .map(|result| result.opponent_elo)
        .sum::<f64>()
        / n;
    let mut low = (opponent_mean - 1_600.0).max(RATING_FLOOR);
    let mut high = (opponent_mean + 1_600.0).min(RATING_CEILING);
    if low >= high {
        low = RATING_FLOOR;
        high = RATING_CEILING;
    }
    for _ in 0..80 {
        let midpoint = (low + high) * 0.5;
        let expected = results
            .iter()
            .map(|result| expected_score(midpoint, result.opponent_elo))
            .sum::<f64>()
            / n;
        if expected < target {
            low = midpoint;
        } else {
            high = midpoint;
        }
    }
    let elo = ((low + high) * 0.5).clamp(RATING_FLOOR, RATING_CEILING);
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
        lower_95: (elo - 1.96 * standard_error).max(RATING_FLOOR),
        upper_95: (elo + 1.96 * standard_error).min(RATING_CEILING),
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

    #[test]
    fn stockfish_is_the_only_published_anchor() {
        assert_eq!(
            published_reference_elo("Stockfish 17"),
            Some(STOCKFISH_REFERENCE_ELO)
        );
        assert_eq!(
            published_reference_elo("stockfish-dev"),
            Some(STOCKFISH_REFERENCE_ELO)
        );
        assert_eq!(published_reference_elo("Fairy-Stockfish 14"), None);
        assert_eq!(published_reference_elo("Koivisto"), None);
        assert_eq!(published_reference_elo("MyClubEngine"), None);
        assert_eq!(published_seed_elo("Mujrim v60"), Some(3_560.0));
        assert_eq!(published_seed_elo("Mujrim Akimbo"), Some(3_480.0));
        assert_eq!(published_seed_elo("Mujrim Lc0"), None);
        assert_eq!(published_seed_elo("Lc0"), Some(3_550.0));
        assert_eq!(published_seed_elo("Weak"), None);
        assert_eq!(published_seed_elo("ClubEngine"), None);
        assert_eq!(
            seed_elo_for_engine("Stockfish 17"),
            Some(STOCKFISH_REFERENCE_ELO)
        );
    }

    #[test]
    fn early_win_against_stockfish_does_not_jump_to_the_ceiling() {
        let names = ["Stockfish".to_owned(), "Contender".to_owned()];
        let ratings = estimate_field_ratings(&names, &[None, None], &[(1, 0, 1.0)]);
        let contender = ratings[1].unwrap().elo;
        assert!(
            contender < 4_000.0,
            "one game must not print a 4400 Elo: {contender}"
        );
        assert!(contender > 2_100.0, "{contender}");
    }

    #[test]
    fn unanchored_field_stays_near_club_prior_not_super_gm() {
        let names = ["ClubA".to_owned(), "ClubB".to_owned()];
        let ratings = estimate_field_ratings(
            &names,
            &[None, None],
            &[(0, 1, 0.5), (1, 0, 0.5), (0, 1, 0.5)],
        );
        let left = ratings[0].unwrap().elo;
        let right = ratings[1].unwrap().elo;
        assert!((left - UNANCHORED_PRIOR_ELO).abs() < 80.0, "{left}");
        assert!((right - UNANCHORED_PRIOR_ELO).abs() < 80.0, "{right}");
    }

    #[test]
    fn winner_is_rated_above_loser_without_seeds() {
        let names = ["Strong".to_owned(), "Weak".to_owned()];
        let games: Vec<_> = (0..10).map(|_| (0, 1, 1.0)).collect();
        let ratings = estimate_field_ratings(&names, &[None, None], &games);
        let strong = ratings[0].unwrap().elo;
        let weak = ratings[1].unwrap().elo;
        assert!(strong > weak + 200.0, "{strong} vs {weak}");
        assert!(
            weak < 2_400.0,
            "weak engine must not be assumed 3000+: {weak}"
        );
        assert!(weak > 1_400.0, "{weak}");
    }

    #[test]
    fn stockfish_anchor_keeps_absolute_scale() {
        let names = ["Stockfish 17".to_owned(), "ClubEngine".to_owned()];
        let games: Vec<_> = (0..8).map(|_| (0, 1, 1.0)).collect();
        let ratings = estimate_field_ratings(&names, &[None, None], &games);
        let stockfish = ratings[0].unwrap().elo;
        let club = ratings[1].unwrap().elo;
        assert!(
            (stockfish - STOCKFISH_REFERENCE_ELO).abs() < 80.0,
            "{stockfish}"
        );
        assert!(
            club < 3_000.0,
            "a shutout victim is not a 3000 engine: {club}"
        );
        assert!(club > 1_500.0, "{club}");
    }

    #[test]
    fn seeded_1500_engine_is_not_inflated() {
        let names = ["Club".to_owned(), "Peer".to_owned()];
        let ratings = estimate_field_ratings(
            &names,
            &[Some(1_520.0), Some(1_480.0)],
            &[(0, 1, 0.5), (1, 0, 0.5), (0, 1, 1.0), (1, 0, 0.0)],
        );
        let club = ratings[0].unwrap().elo;
        let peer = ratings[1].unwrap().elo;
        assert!(club > 1_400.0 && club < 1_800.0, "{club}");
        assert!(peer > 1_300.0 && peer < 1_700.0, "{peer}");
        assert!(club > peer);
    }

    #[test]
    fn drawing_stockfish_lands_in_super_gm_band() {
        let names = ["Stockfish".to_owned(), "Contender".to_owned()];
        let games: Vec<_> = (0..20)
            .map(|i| (0, 1, if i % 2 == 0 { 1.0 } else { 0.0 }))
            .collect();
        let ratings = estimate_field_ratings(&names, &[None, None], &games);
        let contender = ratings[1].unwrap().elo;
        assert!(contender > 3_200.0, "{contender}");
        assert!(contender < 4_000.0, "{contender}");
    }

    #[test]
    fn isotonic_keeps_the_leader_at_or_above_lower_rows() {
        let mut ratings = vec![
            Some(EloEstimate {
                elo: 2_000.0,
                lower_95: 1_800.0,
                upper_95: 2_200.0,
                games: 2,
            }),
            Some(EloEstimate {
                elo: 3_400.0,
                lower_95: 3_200.0,
                upper_95: 3_600.0,
                games: 2,
            }),
        ];
        apply_isotonic_ratings(&mut ratings);
        let leader = ratings[0].unwrap().elo;
        let second = ratings[1].unwrap().elo;
        assert!(leader >= second, "{leader} vs {second}");
    }
}
