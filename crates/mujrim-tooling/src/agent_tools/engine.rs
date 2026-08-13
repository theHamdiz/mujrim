use serde_json::{Value, json};
use types::Board;

use crate::agent_tools::tool::{AgentTool, AgentToolSpec};

pub fn tools() -> Vec<Box<dyn AgentTool>> {
    vec![Box::new(AnalyzeTool), Box::new(PerftTool)]
}

struct AnalyzeTool;

impl AgentTool for AnalyzeTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "engine.analyze",
            domain: "engine",
            description: "Analyze a position and return best move, score, depth, nodes, and PV.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "fen": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 1 },
                    "hash_mb": { "type": "integer", "minimum": 1 },
                    "threads": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, input: &Value) -> Result<Value, String> {
        let fen = optional_string(input, "fen")?;
        let depth = positive_i32(input, "depth", 10)?;
        let hash_mb = positive_i32(input, "hash_mb", 64)?;
        let threads = positive_i32(input, "threads", 1)?;

        types::init();
        let mut board = board_from_optional_fen(fen)?;
        let mut engine = search::SearchEngine::new(hash_mb as usize, threads as usize);
        let result = engine.search_depth(&mut board, depth);

        let pv: Vec<String> = result.pv.iter().map(|mv| mv.to_uci()).collect();
        Ok(json!({
            "best_move": result.best_move.to_uci(),
            "score_cp": result.score,
            "depth": result.depth,
            "nodes": result.nodes,
            "elapsed_ms": result.elapsed.as_millis(),
            "pv": pv
        }))
    }
}

struct PerftTool;

impl AgentTool for PerftTool {
    fn spec(&self) -> AgentToolSpec {
        AgentToolSpec {
            name: "engine.perft",
            domain: "engine",
            description: "Run perft for a position and return the total node count.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "fen": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 1 }
                },
                "required": ["depth"],
                "additionalProperties": false
            }),
        }
    }

    fn call(&self, input: &Value) -> Result<Value, String> {
        let fen = optional_string(input, "fen")?;
        let depth = positive_u32(input, "depth", 1)?;

        types::init();
        let mut board = board_from_optional_fen(fen)?;
        let nodes = board.perft(depth);
        Ok(json!({ "depth": depth, "nodes": nodes }))
    }
}

fn optional_string<'a>(input: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match input.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a string")),
    }
}

fn positive_i32(input: &Value, key: &str, default: i32) -> Result<i32, String> {
    let Some(raw) = input.get(key) else {
        return Ok(default);
    };
    let value = raw
        .as_i64()
        .ok_or_else(|| format!("'{key}' must be an integer"))?;
    if !(1..=i32::MAX as i64).contains(&value) {
        return Err(format!("'{key}' must be between 1 and {}", i32::MAX));
    }
    Ok(value as i32)
}

fn positive_u32(input: &Value, key: &str, default: u32) -> Result<u32, String> {
    let Some(raw) = input.get(key) else {
        return Ok(default);
    };
    let value = raw
        .as_u64()
        .ok_or_else(|| format!("'{key}' must be an integer"))?;
    if value == 0 || value > u32::MAX as u64 {
        return Err(format!("'{key}' must be between 1 and {}", u32::MAX));
    }
    Ok(value as u32)
}

fn board_from_optional_fen(fen: Option<&str>) -> Result<Board, String> {
    match fen {
        Some(value) => Board::from_fen(value).map_err(|e| format!("invalid FEN: {e}")),
        None => Ok(Board::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perft_depth_one_matches_startpos() {
        let tool = PerftTool;
        let out = tool.call(&json!({ "depth": 1 })).unwrap();
        assert_eq!(out["nodes"], 20);
    }

    #[test]
    fn analyze_returns_uci_move() {
        let tool = AnalyzeTool;
        let out = tool
            .call(&json!({ "depth": 1, "threads": 1, "hash_mb": 1 }))
            .unwrap();
        let best = out["best_move"].as_str().unwrap();
        assert!(best.len() >= 4);
    }

    #[test]
    fn depth_validation_rejects_zero() {
        let tool = PerftTool;
        let err = tool.call(&json!({ "depth": 0 })).unwrap_err();
        assert!(err.contains("between 1"));
    }
}
