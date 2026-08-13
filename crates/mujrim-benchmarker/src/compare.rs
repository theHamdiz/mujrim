//! Deterministic fixed-node comparison gates for candidate engine binaries.

use std::time::Duration;

use crate::suite::{BenchSummary, rate_per_second};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Accept,
    Reject,
    PlayRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionComparison {
    pub index: usize,
    pub fen: String,
    pub expected_move: String,
    pub candidate_moves: Vec<String>,
    pub reference_moves: Vec<String>,
    pub candidate_scores: Vec<i32>,
    pub reference_scores: Vec<i32>,
    pub candidate_correct: usize,
    pub reference_correct: usize,
}

impl PositionComparison {
    pub const fn outcome(&self) -> &'static str {
        if self.candidate_correct > self.reference_correct {
            "candidate_gain"
        } else if self.candidate_correct < self.reference_correct {
            "candidate_loss"
        } else {
            "move_change"
        }
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "index": self.index,
            "fen": self.fen,
            "expected_move": self.expected_move,
            "outcome": self.outcome(),
            "candidate": {
                "moves": self.candidate_moves,
                "scores_cp": self.candidate_scores,
                "correct": self.candidate_correct,
            },
            "reference": {
                "moves": self.reference_moves,
                "scores_cp": self.reference_scores,
                "correct": self.reference_correct,
            },
        })
    }
}

impl GateDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::PlayRequired => "play_required",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ComparisonGate {
    pub decision: GateDecision,
    pub reason: &'static str,
    pub candidate_correct: usize,
    pub reference_correct: usize,
    pub candidate_failures: usize,
    pub reference_failures: usize,
    pub candidate_nps: u64,
    pub reference_nps: u64,
    pub speedup_percent: f64,
    pub same_moves: bool,
    pub candidate_stable: bool,
    pub reference_stable: bool,
    pub rounds: usize,
    pub position_deltas: Vec<PositionComparison>,
}

impl ComparisonGate {
    pub fn to_json_value(&self) -> serde_json::Value {
        let position_deltas = self
            .position_deltas
            .iter()
            .map(PositionComparison::to_json_value)
            .collect::<Vec<_>>();
        serde_json::json!({
            "decision": self.decision.as_str(),
            "reason": self.reason,
            "rounds": self.rounds,
            "candidate": {
                "correct": self.candidate_correct,
                "failures": self.candidate_failures,
                "nps": self.candidate_nps,
                "stable": self.candidate_stable,
            },
            "reference": {
                "correct": self.reference_correct,
                "failures": self.reference_failures,
                "nps": self.reference_nps,
                "stable": self.reference_stable,
            },
            "speedup_percent": self.speedup_percent,
            "same_moves": self.same_moves,
            "position_deltas": position_deltas,
        })
    }
}

fn aggregate_nps(runs: &[BenchSummary]) -> u64 {
    let nodes = runs.iter().map(|run| run.total_nodes).sum::<u64>();
    let elapsed = runs.iter().map(|run| run.total_time).sum::<Duration>();
    rate_per_second(nodes, elapsed)
}

fn move_signature(run: &BenchSummary) -> Vec<(usize, &str)> {
    run.results
        .iter()
        .map(|result| (result.index, result.found_move.as_str()))
        .collect()
}

fn stable_moves(runs: &[BenchSummary]) -> bool {
    let Some(first) = runs.first().map(move_signature) else {
        return false;
    };
    runs.iter().skip(1).all(|run| move_signature(run) == first)
}

fn position_deltas(
    candidate: &[BenchSummary],
    reference: &[BenchSummary],
) -> Vec<PositionComparison> {
    let mut indices = candidate
        .iter()
        .chain(reference)
        .flat_map(|run| run.results.iter().map(|result| result.index))
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices.dedup();

    indices
        .into_iter()
        .filter_map(|index| {
            let candidate_results = candidate
                .iter()
                .filter_map(|run| run.results.iter().find(|result| result.index == index))
                .collect::<Vec<_>>();
            let reference_results = reference
                .iter()
                .filter_map(|run| run.results.iter().find(|result| result.index == index))
                .collect::<Vec<_>>();
            let metadata = candidate_results
                .first()
                .copied()
                .or_else(|| reference_results.first().copied())?;
            let candidate_moves = candidate_results
                .iter()
                .map(|result| result.found_move.clone())
                .collect::<Vec<_>>();
            let reference_moves = reference_results
                .iter()
                .map(|result| result.found_move.clone())
                .collect::<Vec<_>>();
            let candidate_correct = candidate_results
                .iter()
                .filter(|result| result.correct)
                .count();
            let reference_correct = reference_results
                .iter()
                .filter(|result| result.correct)
                .count();
            (candidate_moves != reference_moves || candidate_correct != reference_correct).then(
                || PositionComparison {
                    index,
                    fen: metadata.fen.clone(),
                    expected_move: metadata.expected_move.clone(),
                    candidate_scores: candidate_results
                        .iter()
                        .map(|result| result.score)
                        .collect(),
                    reference_scores: reference_results
                        .iter()
                        .map(|result| result.score)
                        .collect(),
                    candidate_moves,
                    reference_moves,
                    candidate_correct,
                    reference_correct,
                },
            )
        })
        .collect()
}

pub fn compare_runs(
    candidate: &[BenchSummary],
    reference: &[BenchSummary],
    minimum_speedup_percent: f64,
) -> Result<ComparisonGate, String> {
    if candidate.is_empty() || candidate.len() != reference.len() {
        return Err("candidate and reference require the same non-zero run count".to_owned());
    }
    if !minimum_speedup_percent.is_finite() || minimum_speedup_percent < 0.0 {
        return Err("minimum speedup must be a finite non-negative percentage".to_owned());
    }

    let candidate_correct = candidate.iter().map(|run| run.correct).sum();
    let reference_correct = reference.iter().map(|run| run.correct).sum();
    let candidate_failures = candidate.iter().map(|run| run.failures.len()).sum();
    let reference_failures = reference.iter().map(|run| run.failures.len()).sum();
    let candidate_nps = aggregate_nps(candidate);
    let reference_nps = aggregate_nps(reference);
    let speedup_percent = if reference_nps == 0 {
        0.0
    } else {
        (candidate_nps as f64 / reference_nps as f64 - 1.0) * 100.0
    };
    let candidate_stable = stable_moves(candidate);
    let reference_stable = stable_moves(reference);
    let has_balanced_speed_sample = candidate.len() >= 2;
    let same_moves = candidate
        .iter()
        .zip(reference)
        .all(|(candidate_run, reference_run)| {
            move_signature(candidate_run) == move_signature(reference_run)
        });
    let position_deltas = position_deltas(candidate, reference);

    let (decision, reason) = if candidate_failures > 0 || reference_failures > 0 {
        (GateDecision::Reject, "engine_failure")
    } else if !candidate_stable || !reference_stable {
        (GateDecision::PlayRequired, "non_deterministic_moves")
    } else if candidate_correct < reference_correct {
        (GateDecision::Reject, "tactical_regression")
    } else if candidate_correct > reference_correct {
        (GateDecision::Accept, "tactical_improvement")
    } else if same_moves && has_balanced_speed_sample && speedup_percent >= minimum_speedup_percent
    {
        (GateDecision::Accept, "equivalent_and_faster")
    } else if same_moves && has_balanced_speed_sample && speedup_percent <= -minimum_speedup_percent
    {
        (GateDecision::Reject, "equivalent_and_slower")
    } else if same_moves && !has_balanced_speed_sample {
        (GateDecision::PlayRequired, "insufficient_rounds_for_speed")
    } else {
        (
            GateDecision::PlayRequired,
            "strength_changed_or_speed_inconclusive",
        )
    };

    Ok(ComparisonGate {
        decision,
        reason,
        candidate_correct,
        reference_correct,
        candidate_failures,
        reference_failures,
        candidate_nps,
        reference_nps,
        speedup_percent,
        same_moves,
        candidate_stable,
        reference_stable,
        rounds: candidate.len(),
        position_deltas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::PositionResult;

    fn summary(found: &str, correct: bool, elapsed_ms: u64) -> BenchSummary {
        BenchSummary::from_results(
            "fixture",
            vec![PositionResult {
                index: 0,
                fen: "fixture".to_owned(),
                expected_move: "e2e4".to_owned(),
                found_move: found.to_owned(),
                correct,
                score: 0,
                depth: 1,
                nodes: 100_000,
                nps: 100_000_000 / elapsed_ms,
                elapsed: Duration::from_millis(elapsed_ms),
            }],
        )
    }

    fn two_position_summary(second_found: &str, second_correct: bool) -> BenchSummary {
        BenchSummary::from_results(
            "fixture",
            vec![
                PositionResult {
                    index: 0,
                    fen: "first".to_owned(),
                    expected_move: "e2e4".to_owned(),
                    found_move: "e2e4".to_owned(),
                    correct: true,
                    score: 20,
                    depth: 1,
                    nodes: 100_000,
                    nps: 1_000_000,
                    elapsed: Duration::from_millis(100),
                },
                PositionResult {
                    index: 1,
                    fen: "second".to_owned(),
                    expected_move: "g1f3".to_owned(),
                    found_move: second_found.to_owned(),
                    correct: second_correct,
                    score: -15,
                    depth: 1,
                    nodes: 100_000,
                    nps: 1_000_000,
                    elapsed: Duration::from_millis(100),
                },
            ],
        )
    }

    #[test]
    fn equivalent_faster_candidate_is_accepted() {
        let candidate = [summary("e2e4", true, 90), summary("e2e4", true, 90)];
        let reference = [summary("e2e4", true, 100), summary("e2e4", true, 100)];
        let gate = compare_runs(&candidate, &reference, 0.5).unwrap();
        assert_eq!(gate.decision, GateDecision::Accept);
        assert_eq!(gate.reason, "equivalent_and_faster");
        assert!(gate.speedup_percent > 10.0);
    }

    #[test]
    fn one_round_cannot_make_a_speed_only_decision() {
        let candidate = [summary("e2e4", true, 50)];
        let reference = [summary("e2e4", true, 100)];
        let gate = compare_runs(&candidate, &reference, 0.5).unwrap();
        assert_eq!(gate.decision, GateDecision::PlayRequired);
        assert_eq!(gate.reason, "insufficient_rounds_for_speed");
    }

    #[test]
    fn tactical_regression_is_rejected_even_when_faster() {
        let candidate = [summary("a2a3", false, 50)];
        let reference = [summary("e2e4", true, 100)];
        let gate = compare_runs(&candidate, &reference, 0.5).unwrap();
        assert_eq!(gate.decision, GateDecision::Reject);
        assert_eq!(gate.reason, "tactical_regression");
    }

    #[test]
    fn changed_equal_accuracy_requires_play() {
        let candidate = [summary("d2d4", false, 100)];
        let reference = [summary("a2a3", false, 100)];
        let gate = compare_runs(&candidate, &reference, 0.5).unwrap();
        assert_eq!(gate.decision, GateDecision::PlayRequired);
        assert!(!gate.same_moves);
    }

    #[test]
    fn mismatched_run_counts_are_rejected() {
        assert!(compare_runs(&[summary("e2e4", true, 100)], &[], 0.5).is_err());
    }

    #[test]
    fn comparison_reports_only_changed_positions() {
        let candidate = [
            two_position_summary("d2d4", false),
            two_position_summary("d2d4", false),
        ];
        let reference = [
            two_position_summary("g1f3", true),
            two_position_summary("g1f3", true),
        ];
        let gate = compare_runs(&candidate, &reference, 0.5).unwrap();

        assert_eq!(gate.position_deltas.len(), 1);
        let delta = &gate.position_deltas[0];
        assert_eq!(delta.index, 1);
        assert_eq!(delta.outcome(), "candidate_loss");
        assert_eq!(delta.candidate_moves, ["d2d4", "d2d4"]);
        assert_eq!(delta.reference_moves, ["g1f3", "g1f3"]);
        assert_eq!(gate.to_json_value()["position_deltas"][0]["index"], 1);
    }
}
