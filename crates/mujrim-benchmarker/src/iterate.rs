//! Iterative BK benchmark loop: run `run_internal_bench` until `approx_ccrl_40_15 >= target`
//! or limits hit. Designed for agent/human-in-the-loop improvement cycles (`--between`).

use std::process::Command;

use serde_json::{Value, json};

use crate::internal::{self, InternalBenchConfig};
use crate::suite::{BenchSummary, TestPosition};

/// Controls the outer improvement loop.
#[derive(Clone, Debug)]
pub struct EloIterateConfig {
    pub target_elo: i32,
    /// Stop successfully when at least this many BK positions match (e.g. 20 on the 24-pos suite).
    pub min_bk_correct: Option<usize>,
    pub max_rounds: u32,
    /// Exit with `stagnation_exceeded` after this many rounds with no CCRL-proxy gain.
    pub stagnation_limit: u32,
    /// Optional shell command between rounds (rebuild, swap NNUE, tune params).
    pub between_shell: Option<String>,
    /// Print one JSON object per round on stdout (even when `quiet`).
    pub json_progress: bool,
    /// Suppress decorative CLI output in the benchmark driver; still honors `json_progress`.
    pub quiet: bool,
    pub bench: InternalBenchConfig,
    pub measure_nps: bool,
}

impl Default for EloIterateConfig {
    fn default() -> Self {
        Self {
            target_elo: 3500,
            min_bk_correct: None,
            max_rounds: 100,
            stagnation_limit: 20,
            between_shell: None,
            json_progress: false,
            quiet: true,
            bench: InternalBenchConfig::default(),
            measure_nps: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EloIterateRound {
    pub round: u32,
    pub summary: BenchSummary,
    pub nps_startpos: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct EloIterateOutcome {
    pub success: bool,
    pub reason: &'static str,
    pub rounds_run: u32,
    pub best_elo: i32,
    pub best_round: u32,
    pub final_round: EloIterateRound,
    pub history: Vec<Value>,
}

/// Run benchmark rounds until target **CCRL 40/15 proxy** from BK is reached or limits stop the loop.
pub fn run_elo_iterate(positions: &[TestPosition], config: &EloIterateConfig) -> EloIterateOutcome {
    let mut best_elo = i32::MIN;
    let mut best_round = 0u32;
    let mut stagnant = 0u32;
    let mut history: Vec<Value> = Vec::new();

    if !config.quiet && !config.json_progress {
        eprintln!(
            "═══ CCRL proxy iterate: target={}  max_rounds={}  stagnation_limit={} ═══",
            config.target_elo, config.max_rounds, config.stagnation_limit
        );
    }

    let mut last_round = EloIterateRound {
        round: 0,
        summary: BenchSummary::from_results("Mujrim", Vec::new()),
        nps_startpos: None,
    };

    for round in 1..=config.max_rounds {
        if round > 1
            && let Some(cmd) = &config.between_shell
        {
            if !config.quiet {
                eprintln!("── between: {cmd}");
            }
            let st = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .status()
                .map_err(|e| e.to_string());
            match st {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    let code = s.code().unwrap_or(-1);
                    if config.json_progress {
                        println!(
                            "{}",
                            json!({
                                "event": "between_failed",
                                "round": round,
                                "exit_code": code,
                            })
                        );
                    }
                    return EloIterateOutcome {
                        success: false,
                        reason: "between_command_failed",
                        rounds_run: round - 1,
                        best_elo,
                        best_round,
                        final_round: last_round,
                        history,
                    };
                }
                Err(e) => {
                    if config.json_progress {
                        println!(
                            "{}",
                            json!({
                                "event": "between_spawn_error",
                                "round": round,
                                "error": e,
                            })
                        );
                    }
                    return EloIterateOutcome {
                        success: false,
                        reason: "between_command_error",
                        rounds_run: round - 1,
                        best_elo,
                        best_round,
                        final_round: last_round,
                        history,
                    };
                }
            }
        }

        let mut bench_cfg = config.bench.clone();
        bench_cfg.quiet = true;

        let summary = internal::run_internal_bench(positions, &bench_cfg, None);

        let nps = if config.measure_nps {
            Some(internal::measure_startpos_nps(
                bench_cfg.threads,
                bench_cfg.hash_mb,
                &bench_cfg.eval_preset,
                bench_cfg.eval_file.as_deref(),
                config.quiet || config.json_progress,
            ))
        } else {
            None
        };

        last_round = EloIterateRound {
            round,
            summary: summary.clone(),
            nps_startpos: nps,
        };

        let elo = summary.approx_ccrl_40_15;
        let row = json!({
            "event": "round_complete",
            "round": round,
            "approx_ccrl_40_15": elo,
            "approx_lichess_blitz": summary.approx_lichess_blitz,
            "accuracy": summary.accuracy,
            "correct": summary.correct,
            "total": summary.total,
            "nps_aggregate": summary.nps,
            "nps_startpos_5s": nps,
        });
        history.push(row.clone());
        if config.json_progress {
            println!("{row}");
        } else if !config.quiet {
            println!("{summary}");
            if let Some(n) = nps {
                println!("  NPS (5s startpos): {}", crate::suite::format_nps(n));
            }
            println!(
                "  Round {round}  approx. CCRL 40/15 ~{elo}  (best {best_elo}, target {})\n",
                config.target_elo
            );
        }

        if elo > best_elo {
            best_elo = elo;
            best_round = round;
            stagnant = 0;
        } else {
            stagnant += 1;
        }

        if let Some(min_ok) = config.min_bk_correct
            && summary.correct >= min_ok
        {
            return EloIterateOutcome {
                success: true,
                reason: "bk_minimum_reached",
                rounds_run: round,
                best_elo,
                best_round,
                final_round: last_round,
                history,
            };
        }

        if elo >= config.target_elo {
            return EloIterateOutcome {
                success: true,
                reason: "target_reached",
                rounds_run: round,
                best_elo,
                best_round,
                final_round: last_round,
                history,
            };
        }

        if stagnant >= config.stagnation_limit {
            if config.json_progress {
                println!(
                    "{}",
                    json!({
                        "event": "stagnation_exceeded",
                        "round": round,
                        "stagnant_rounds": stagnant,
                        "best_elo": best_elo,
                    })
                );
            }
            return EloIterateOutcome {
                success: false,
                reason: "stagnation_exceeded",
                rounds_run: round,
                best_elo,
                best_round,
                final_round: last_round,
                history,
            };
        }
    }

    EloIterateOutcome {
        success: false,
        reason: "max_rounds_exceeded",
        rounds_run: config.max_rounds,
        best_elo,
        best_round,
        final_round: last_round,
        history,
    }
}

impl EloIterateOutcome {
    pub fn to_json_value(&self) -> Value {
        json!({
            "success": self.success,
            "reason": self.reason,
            "rounds_run": self.rounds_run,
            "best_elo": self.best_elo,
            "best_round": self.best_round,
            "final_approx_ccrl_40_15": self.final_round.summary.approx_ccrl_40_15,
            "final_approx_lichess_blitz": self.final_round.summary.approx_lichess_blitz,
            "final_accuracy": self.final_round.summary.accuracy,
            "final_bench": self.final_round.summary.to_json_value(),
            "nps_startpos_5s": self.final_round.nps_startpos,
            "history": self.history,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::bk_suite;

    #[test]
    fn iterate_one_round_low_target_finishes() {
        let positions = bk_suite();
        let cfg = EloIterateConfig {
            target_elo: 400,
            min_bk_correct: None,
            max_rounds: 1,
            stagnation_limit: 50,
            between_shell: None,
            json_progress: false,
            quiet: true,
            bench: InternalBenchConfig {
                depth: 3,
                threads: 1,
                hash_mb: 8,
                time_per_position: std::time::Duration::from_secs(2),
                suite_name: "BK".into(),
                eval_preset: "auto".into(),
                eval_file: None,
                quiet: true,
            },
            measure_nps: false,
        };
        let out = run_elo_iterate(&positions, &cfg);
        assert_eq!(out.rounds_run, 1);
        assert!(out.final_round.summary.total > 0);
        let j = out.to_json_value();
        assert!(j.get("final_bench").is_some());
    }
}
