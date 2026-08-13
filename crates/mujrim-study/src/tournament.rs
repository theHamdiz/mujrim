//! Deterministic tournament scheduling and standings.

use crate::rating::{EloEstimate, estimate_field_ratings};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TournamentFormat {
    #[default]
    RoundRobin,
    DoubleRoundRobin,
    Swiss,
    Knockout,
}

impl TournamentFormat {
    pub const ALL: [Self; 4] = [
        Self::RoundRobin,
        Self::DoubleRoundRobin,
        Self::Swiss,
        Self::Knockout,
    ];
}

impl std::fmt::Display for TournamentFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RoundRobin => "Round robin",
            Self::DoubleRoundRobin => "Double round robin",
            Self::Swiss => "Swiss",
            Self::Knockout => "Knockout",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entrant {
    pub id: String,
    pub name: String,
    pub seed_elo: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub round: usize,
    pub white: usize,
    pub black: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TournamentResult {
    pub pairing: Pairing,
    /// White's score: `1.0`, `0.5`, or `0.0`.
    pub white_score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Standing {
    pub entrant: usize,
    pub played: usize,
    pub wins: usize,
    pub draws: usize,
    pub losses: usize,
    pub points: f64,
    pub performance: Option<EloEstimate>,
}

pub fn schedule(entrant_count: usize, format: TournamentFormat) -> Vec<Pairing> {
    if entrant_count < 2 {
        return Vec::new();
    }
    if format == TournamentFormat::Swiss {
        return swiss_round(entrant_count, &[], 1);
    }
    if format == TournamentFormat::Knockout {
        return knockout_round(&(0..entrant_count).collect::<Vec<_>>(), 1);
    }
    let mut slots: Vec<Option<usize>> = (0..entrant_count).map(Some).collect();
    if !slots.len().is_multiple_of(2) {
        slots.push(None);
    }
    let rounds = slots.len() - 1;
    let games_per_round = slots.len() / 2;
    let mut pairings = Vec::with_capacity(rounds * games_per_round);
    for round in 0..rounds {
        for board in 0..games_per_round {
            let left = slots[board];
            let right = slots[slots.len() - 1 - board];
            if let (Some(left), Some(right)) = (left, right) {
                let (white, black) = if (round + board) % 2 == 0 {
                    (left, right)
                } else {
                    (right, left)
                };
                pairings.push(Pairing {
                    round: round + 1,
                    white,
                    black,
                });
            }
        }
        let last = slots.pop().expect("round-robin ring is non-empty");
        slots.insert(1, last);
    }

    if format == TournamentFormat::DoubleRoundRobin {
        let return_leg = pairings
            .iter()
            .map(|pairing| Pairing {
                round: pairing.round + rounds,
                white: pairing.black,
                black: pairing.white,
            })
            .collect::<Vec<_>>();
        pairings.extend(return_leg);
    }
    pairings
}

/// Produce one deterministic Swiss round, grouping equal scores first and
/// avoiding repeat opponents whenever an alternative remains.
pub fn swiss_round(
    entrant_count: usize,
    results: &[TournamentResult],
    round: usize,
) -> Vec<Pairing> {
    if entrant_count < 2 {
        return Vec::new();
    }
    let mut points = vec![0.0_f64; entrant_count];
    let mut opponents = vec![Vec::new(); entrant_count];
    for result in results {
        let Pairing { white, black, .. } = result.pairing;
        if white >= entrant_count || black >= entrant_count || white == black {
            continue;
        }
        let white_score = normalized_score(result.white_score);
        points[white] += white_score;
        points[black] += 1.0 - white_score;
        opponents[white].push(black);
        opponents[black].push(white);
    }
    let mut pool = (0..entrant_count).collect::<Vec<_>>();
    pool.sort_by(|&left, &right| {
        points[right]
            .total_cmp(&points[left])
            .then_with(|| left.cmp(&right))
    });
    let mut pairings = Vec::with_capacity(entrant_count / 2);
    let mut board = 0;
    while pool.len() >= 2 {
        let first = pool.remove(0);
        let opponent_index = pool
            .iter()
            .position(|opponent| !opponents[first].contains(opponent))
            .unwrap_or(0);
        let second = pool.remove(opponent_index);
        let (white, black) = if (round + board).is_multiple_of(2) {
            (first, second)
        } else {
            (second, first)
        };
        pairings.push(Pairing {
            round: round.max(1),
            white,
            black,
        });
        board += 1;
    }
    pairings
}

/// Pair a seeded knockout field. Top seeds receive byes until the remaining
/// field fits the next lower power-of-two bracket.
pub fn knockout_round(participants: &[usize], round: usize) -> Vec<Pairing> {
    if participants.len() < 2 {
        return Vec::new();
    }
    let bye_count = participants.len().next_power_of_two() - participants.len();
    let playing = &participants[bye_count..];
    (0..playing.len() / 2)
        .map(|board| Pairing {
            round: round.max(1),
            white: playing[board],
            black: playing[playing.len() - 1 - board],
        })
        .collect()
}

/// Resolve a knockout round. Drawn matches require a tie-break game and are
/// rejected instead of silently advancing an arbitrary engine.
pub fn knockout_advancers(
    participants: &[usize],
    pairings: &[Pairing],
    results: &[TournamentResult],
) -> Result<Vec<usize>, String> {
    let mut advancers = participants
        .iter()
        .copied()
        .filter(|entrant| {
            !pairings
                .iter()
                .any(|pairing| pairing.white == *entrant || pairing.black == *entrant)
        })
        .collect::<Vec<_>>();
    for pairing in pairings {
        let result = results
            .iter()
            .find(|result| result.pairing == *pairing)
            .ok_or_else(|| {
                format!(
                    "missing knockout result for {} vs {}",
                    pairing.white, pairing.black
                )
            })?;
        let score = normalized_score(result.white_score);
        if score == 0.5 {
            return Err(format!(
                "knockout tie between {} and {} requires a tie-break",
                pairing.white, pairing.black
            ));
        }
        advancers.push(if score > 0.5 {
            pairing.white
        } else {
            pairing.black
        });
    }
    advancers.sort_by_key(|entrant| {
        participants
            .iter()
            .position(|participant| participant == entrant)
            .unwrap_or(usize::MAX)
    });
    Ok(advancers)
}

pub fn standings(entrants: &[Entrant], results: &[TournamentResult]) -> Vec<Standing> {
    let mut table = (0..entrants.len())
        .map(|entrant| Standing {
            entrant,
            played: 0,
            wins: 0,
            draws: 0,
            losses: 0,
            points: 0.0,
            performance: None,
        })
        .collect::<Vec<_>>();
    let mut games = Vec::new();

    for result in results {
        if result.pairing.white >= entrants.len() || result.pairing.black >= entrants.len() {
            continue;
        }
        let white_score = normalized_score(result.white_score);
        let black_score = 1.0 - white_score;
        update_standing(&mut table[result.pairing.white], white_score);
        update_standing(&mut table[result.pairing.black], black_score);
        games.push((result.pairing.white, result.pairing.black, white_score));
    }
    let names: Vec<String> = entrants
        .iter()
        .map(|entrant| entrant.name.clone())
        .collect();
    let seeds: Vec<Option<f64>> = entrants.iter().map(|entrant| entrant.seed_elo).collect();
    let ratings = estimate_field_ratings(&names, &seeds, &games);
    for (entrant, estimate) in ratings.into_iter().enumerate() {
        table[entrant].performance = estimate;
    }
    table.sort_by(|left, right| {
        right
            .points
            .total_cmp(&left.points)
            .then_with(|| right.wins.cmp(&left.wins))
            .then_with(|| left.entrant.cmp(&right.entrant))
    });
    table
}

fn normalized_score(score: f64) -> f64 {
    if score >= 0.75 {
        1.0
    } else if score <= 0.25 {
        0.0
    } else {
        0.5
    }
}

fn update_standing(standing: &mut Standing, score: f64) {
    standing.played += 1;
    standing.points += score;
    if score >= 0.75 {
        standing.wins += 1;
    } else if score <= 0.25 {
        standing.losses += 1;
    } else {
        standing.draws += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn round_robin_schedules_every_pair_once_with_odd_byes() {
        let pairings = schedule(5, TournamentFormat::RoundRobin);
        assert_eq!(pairings.len(), 10);
        let pairs = pairings
            .iter()
            .map(|pairing| {
                let low = pairing.white.min(pairing.black);
                let high = pairing.white.max(pairing.black);
                (low, high)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(pairs.len(), 10);
        assert!(pairings.iter().all(|pairing| pairing.round <= 5));
    }

    #[test]
    fn double_round_robin_reverses_every_color_pairing() {
        let pairings = schedule(4, TournamentFormat::DoubleRoundRobin);
        assert_eq!(pairings.len(), 12);
        for first_leg in &pairings[..6] {
            assert!(pairings[6..].iter().any(|return_leg| {
                return_leg.white == first_leg.black && return_leg.black == first_leg.white
            }));
        }
    }

    #[test]
    fn swiss_pairs_score_groups_without_repeating_when_possible() {
        let previous = vec![
            TournamentResult {
                pairing: Pairing {
                    round: 1,
                    white: 0,
                    black: 1,
                },
                white_score: 1.0,
            },
            TournamentResult {
                pairing: Pairing {
                    round: 1,
                    white: 2,
                    black: 3,
                },
                white_score: 1.0,
            },
        ];
        let second = swiss_round(4, &previous, 2);
        assert_eq!(second.len(), 2);
        assert!(!second.iter().any(|pairing| {
            matches!(
                (
                    pairing.white.min(pairing.black),
                    pairing.white.max(pairing.black)
                ),
                (0, 1) | (2, 3)
            )
        }));
    }

    #[test]
    fn knockout_gives_top_seeds_byes_and_requires_decisive_results() {
        let participants = vec![0, 1, 2, 3, 4];
        let pairings = knockout_round(&participants, 1);
        assert_eq!(pairings.len(), 1);
        assert_eq!((pairings[0].white, pairings[0].black), (3, 4));
        let advancers = knockout_advancers(
            &participants,
            &pairings,
            &[TournamentResult {
                pairing: pairings[0],
                white_score: 0.0,
            }],
        )
        .unwrap();
        assert_eq!(advancers, vec![0, 1, 2, 4]);
        assert!(
            knockout_advancers(
                &participants,
                &pairings,
                &[TournamentResult {
                    pairing: pairings[0],
                    white_score: 0.5,
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn standings_include_scores_and_anchored_performance() {
        let entrants = vec![
            Entrant {
                id: "a".to_owned(),
                name: "A".to_owned(),
                seed_elo: Some(2500.0),
            },
            Entrant {
                id: "b".to_owned(),
                name: "B".to_owned(),
                seed_elo: Some(2400.0),
            },
        ];
        let results = vec![TournamentResult {
            pairing: Pairing {
                round: 1,
                white: 0,
                black: 1,
            },
            white_score: 1.0,
        }];
        let table = standings(&entrants, &results);
        assert_eq!(table[0].entrant, 0);
        assert_eq!(table[0].points, 1.0);
        assert!(table[0].performance.is_some());
        assert_eq!(table[1].losses, 1);
    }

    #[test]
    fn standings_rate_unseeded_engines_from_the_games() {
        let entrants = vec![
            Entrant {
                id: "sf".to_owned(),
                name: "Stockfish 17".to_owned(),
                seed_elo: None,
            },
            Entrant {
                id: "club".to_owned(),
                name: "ClubEngine".to_owned(),
                seed_elo: None,
            },
        ];
        let results = (0..6)
            .map(|round| TournamentResult {
                pairing: Pairing {
                    round: round + 1,
                    white: 0,
                    black: 1,
                },
                white_score: 1.0,
            })
            .collect::<Vec<_>>();
        let table = standings(&entrants, &results);
        let stockfish = table[0].performance.expect("stockfish elo");
        let club = table[1].performance.expect("club elo");
        assert!((stockfish.elo - crate::rating::STOCKFISH_REFERENCE_ELO).abs() < 80.0);
        assert!(club.elo < 3_000.0);
        assert!(table[0].points > table[1].points);
    }
}
