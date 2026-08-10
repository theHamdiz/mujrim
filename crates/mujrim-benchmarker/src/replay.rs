//! Resource-bounded comparison of two engines at an exact game position.

use std::path::PathBuf;

use mujrim_protocols::SearchInfo;

use crate::external::{ExternalBenchConfig, run_external_search};

#[derive(Clone, Debug)]
pub struct EngineProbe {
    pub path: PathBuf,
    pub info: SearchInfo,
}

#[derive(Clone, Debug)]
pub struct ReplayComparison {
    pub candidate: EngineProbe,
    pub reference: EngineProbe,
}

impl ReplayComparison {
    pub fn same_best_move(&self) -> bool {
        self.candidate.info.best_move == self.reference.info.best_move
    }

    pub fn score_delta(&self) -> i32 {
        self.candidate.info.score - self.reference.info.score
    }

    pub fn to_json_value(&self, fen: &str, moves: &[String], nodes: u64) -> serde_json::Value {
        serde_json::json!({
            "fen": fen,
            "moves": moves,
            "nodes": nodes,
            "same_best_move": self.same_best_move(),
            "score_delta_cp": self.score_delta(),
            "candidate": probe_json(&self.candidate),
            "reference": probe_json(&self.reference),
        })
    }
}

fn probe_json(probe: &EngineProbe) -> serde_json::Value {
    let info = &probe.info;
    serde_json::json!({
        "path": probe.path,
        "best_move": info.best_move,
        "ponder_move": info.ponder_move,
        "score_cp": info.score,
        "depth": info.depth,
        "seldepth": info.seldepth,
        "nodes": info.nodes,
        "nps": info.nps,
        "time_ms": info.time_ms,
        "hashfull": info.hashfull,
        "tablebase_hits": info.tablebase_hits,
        "pv": info.pv,
    })
}

pub fn compare_position(
    candidate: &ExternalBenchConfig,
    reference: &ExternalBenchConfig,
    fen: &str,
    moves: &[String],
) -> Result<ReplayComparison, String> {
    let candidate_info = run_external_search(fen, moves, candidate)?;
    let reference_info = run_external_search(fen, moves, reference)?;

    Ok(ReplayComparison {
        candidate: EngineProbe {
            path: candidate.engine_path.clone(),
            info: candidate_info,
        },
        reference: EngineProbe {
            path: reference.engine_path.clone(),
            info: reference_info,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(path: &str, best_move: &str, score: i32) -> EngineProbe {
        EngineProbe {
            path: path.into(),
            info: SearchInfo {
                best_move: best_move.to_owned(),
                score,
                nodes: 100,
                pv: vec![best_move.to_owned()],
                ..SearchInfo::default()
            },
        }
    }

    #[test]
    fn comparison_reports_move_and_score_differences() {
        let comparison = ReplayComparison {
            candidate: probe("candidate", "e2e4", 35),
            reference: probe("reference", "d2d4", 20),
        };

        assert!(!comparison.same_best_move());
        assert_eq!(comparison.score_delta(), 15);
        let json = comparison.to_json_value("fixture", &["g1f3".to_owned()], 100);
        assert_eq!(json["candidate"]["best_move"], "e2e4");
        assert_eq!(json["reference"]["best_move"], "d2d4");
        assert_eq!(json["moves"][0], "g1f3");
    }
}
