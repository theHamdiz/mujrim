//! External engine process helpers for GUI play.

use std::path::Path;
use std::time::Duration;

use kishmat_protocols::{EngineOptions, ProtocolKind, SearchRequest, analyze_once};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalEngineProtocol {
    Uci,
    Xboard,
}

impl ExternalEngineProtocol {
    pub fn as_protocol_kind(self) -> ProtocolKind {
        match self {
            Self::Uci => ProtocolKind::Uci,
            Self::Xboard => ProtocolKind::Xboard,
        }
    }
}

impl std::fmt::Display for ExternalEngineProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uci => write!(f, "UCI"),
            Self::Xboard => write!(f, "XBoard"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalMoveResult {
    pub best_move: String,
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub nps: u64,
}

pub fn query_best_move(
    engine_path: &str,
    protocol: ExternalEngineProtocol,
    fen: &str,
    depth: i32,
    movetime: Duration,
    hash_mb: usize,
    threads: usize,
) -> Result<ExternalMoveResult, String> {
    let info = analyze_once(
        Path::new(engine_path),
        protocol.as_protocol_kind(),
        &EngineOptions {
            hash_mb: Some(hash_mb),
            threads: Some(threads),
        },
        &SearchRequest {
            fen: fen.to_string(),
            depth,
            movetime: Some(movetime),
            node_limit: None,
        },
    )?;

    Ok(ExternalMoveResult {
        best_move: info.best_move,
        depth: info.depth,
        score: info.score,
        nodes: info.nodes,
        nps: info.nps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_display() {
        assert_eq!(ExternalEngineProtocol::Uci.to_string(), "UCI");
        assert_eq!(ExternalEngineProtocol::Xboard.to_string(), "XBoard");
    }
}
