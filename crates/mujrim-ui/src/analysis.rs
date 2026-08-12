//! Multi-engine analysis sessions for the analysis board.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mujrim_protocols::{EngineOptions, EngineSession, ProtocolKind, SearchRequest};
use mujrim_study::board_marks::BoardArrow;
use mujrim_study::engine_opinion::{
    EngineLine, EngineOpinion, MultiEngineAnalysis, color_for_engine_slot,
};

use crate::uci_process::{ExternalEngineProtocol, ExternalSearchConfig, query_best_move};

#[derive(Debug, Clone)]
pub struct AnalysisEngineSpec {
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub protocol: ExternalEngineProtocol,
    pub builtin: bool,
}

#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    pub fen: String,
    pub depth: i32,
    pub movetime: Duration,
    pub hash_mb: usize,
    pub threads: usize,
    pub multipv: u32,
    pub engines: Vec<AnalysisEngineSpec>,
    pub max_pv_plies: usize,
}

#[derive(Debug, Clone)]
pub struct AnalysisSnapshot {
    pub analysis: MultiEngineAnalysis,
    pub arrows: Vec<BoardArrow>,
    pub consensus: Option<String>,
    pub status: String,
}

/// Run sequential multi-engine analysis (any UCI/XBoard engine + optional builtin).
pub fn run_multi_engine_analysis(
    request: AnalysisRequest,
    builtin_search: impl Fn(&str, i32) -> Result<(String, i32, Vec<String>), String>,
) -> AnalysisSnapshot {
    let mut analysis = MultiEngineAnalysis::new(request.fen.clone());
    let search = ExternalSearchConfig {
        ponder: false,
        use_nnue: true,
        own_book: false,
        eval_file: None,
    };
    let multipv = request.multipv.max(1);

    for (slot, engine) in request.engines.iter().enumerate() {
        let opinion = if engine.builtin {
            match builtin_search(&request.fen, request.depth) {
                Ok((best, score, pv)) => Some(EngineOpinion {
                    engine_id: engine.id.clone(),
                    engine_name: engine.name.clone(),
                    color: color_for_engine_slot(slot),
                    lines: vec![EngineLine {
                        multipv: 1,
                        score_cp: score,
                        depth: request.depth,
                        pv: if pv.is_empty() { vec![best] } else { pv },
                        nodes: 0,
                        nps: 0,
                    }],
                }),
                Err(_) => None,
            }
        } else if let Some(path) = &engine.path {
            match query_analysis_lines(
                path,
                engine.protocol,
                &request.fen,
                request.depth,
                request.movetime,
                request.hash_mb,
                request.threads,
                multipv,
                &search,
            ) {
                Ok(lines) if !lines.is_empty() => Some(EngineOpinion {
                    engine_id: engine.id.clone(),
                    engine_name: engine.name.clone(),
                    color: color_for_engine_slot(slot),
                    lines,
                }),
                _ => None,
            }
        } else {
            None
        };

        if let Some(opinion) = opinion {
            analysis.push_opinion(opinion);
        }
    }

    let arrows = analysis.all_arrows(request.max_pv_plies, multipv as usize);
    let consensus = analysis.consensus_best_move();
    let status = if analysis.opinions.is_empty() {
        "No engine returned an analysis line.".to_owned()
    } else {
        format!(
            "{} engines · consensus {}",
            analysis.opinions.len(),
            consensus.as_deref().unwrap_or("—")
        )
    };
    AnalysisSnapshot {
        analysis,
        arrows,
        consensus,
        status,
    }
}

#[allow(clippy::too_many_arguments)]
fn query_analysis_lines(
    engine_path: &Path,
    protocol: ExternalEngineProtocol,
    fen: &str,
    depth: i32,
    movetime: Duration,
    hash_mb: usize,
    threads: usize,
    multipv: u32,
    search: &ExternalSearchConfig,
) -> Result<Vec<EngineLine>, String> {
    if protocol == ExternalEngineProtocol::Uci
        && multipv > 1
        && let Ok(lines) =
            query_multipv_bundle(engine_path, fen, depth, movetime, hash_mb, threads, multipv)
        && !lines.is_empty()
    {
        return Ok(lines);
    }
    let result = query_best_move(
        engine_path.to_string_lossy().as_ref(),
        protocol,
        fen,
        depth,
        movetime,
        hash_mb,
        threads,
        search,
    )?;
    if result.pv.is_empty() && result.best_move.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![EngineLine {
        multipv: 1,
        score_cp: result.score,
        depth: result.depth,
        pv: if result.pv.is_empty() {
            vec![result.best_move]
        } else {
            result.pv
        },
        nodes: result.nodes,
        nps: result.nps,
    }])
}

fn query_multipv_bundle(
    engine_path: &Path,
    fen: &str,
    depth: i32,
    movetime: Duration,
    hash_mb: usize,
    threads: usize,
    multipv: u32,
) -> Result<Vec<EngineLine>, String> {
    let memory_limit = ((hash_mb + 192).min(4096) as u64).saturating_mul(1024 * 1024);
    let mut session = EngineSession::spawn_with_args_and_memory_limit(
        engine_path,
        &[],
        ProtocolKind::Uci,
        Some(memory_limit),
    )?;
    session.configure(&EngineOptions {
        hash_mb: Some(hash_mb),
        threads: Some(threads),
        own_book: Some(false),
        custom: vec![("MultiPV".to_owned(), multipv.to_string())],
    })?;
    session.new_game()?;
    let info = session.search(&SearchRequest {
        fen: fen.to_owned(),
        moves: Vec::new(),
        depth,
        movetime: Some(movetime),
        node_limit: None,
        clock: None,
    })?;
    if !info.multipv_lines.is_empty() {
        return Ok(info
            .multipv_lines
            .into_iter()
            .map(|line| EngineLine {
                multipv: line.multipv,
                score_cp: line.score,
                depth: line.depth,
                pv: line.pv,
                nodes: info.nodes,
                nps: info.nps,
            })
            .collect());
    }
    if info.pv.is_empty() && info.best_move.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![EngineLine {
        multipv: 1,
        score_cp: info.score,
        depth: info.depth,
        pv: if info.pv.is_empty() {
            vec![info.best_move]
        } else {
            info.pv
        },
        nodes: info.nodes,
        nps: info.nps,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_only_analysis_produces_stepped_arrows() {
        let snapshot = run_multi_engine_analysis(
            AnalysisRequest {
                fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".into(),
                depth: 6,
                movetime: Duration::from_millis(50),
                hash_mb: 16,
                threads: 1,
                multipv: 1,
                engines: vec![AnalysisEngineSpec {
                    id: "builtin".into(),
                    name: "Mujrim".into(),
                    path: None,
                    protocol: ExternalEngineProtocol::Uci,
                    builtin: true,
                }],
                max_pv_plies: 3,
            },
            |_fen, _depth| {
                Ok((
                    "e2e4".into(),
                    25,
                    vec!["e2e4".into(), "e7e5".into(), "g1f3".into()],
                ))
            },
        );
        assert_eq!(snapshot.analysis.opinions.len(), 1);
        assert_eq!(snapshot.arrows.len(), 3);
        assert_eq!(snapshot.consensus.as_deref(), Some("e2e4"));
        assert_eq!(snapshot.arrows[0].step, Some(1));
    }
}
