//! External Benchmark Runner — benchmarks any UCI-compatible engine binary.
//!
//! Spawns the engine as a subprocess, communicates via stdin/stdout pipes,
//! and collects best moves + node counts from UCI `info` and `bestmove` output.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::suite::{BenchSummary, PositionResult, TestPosition, format_nps};

/// Configuration for an external benchmark run.
#[derive(Clone, Debug)]
pub struct ExternalBenchConfig {
    pub engine_path: PathBuf,
    pub depth: i32,
    pub hash_mb: usize,
    pub threads: usize,
    pub time_per_position: Duration,
}

impl Default for ExternalBenchConfig {
    fn default() -> Self {
        Self {
            engine_path: PathBuf::from("./kishmat"),
            depth: 16,
            hash_mb: 128,
            threads: 1,
            time_per_position: Duration::from_secs(120),
        }
    }
}

/// Parsed search info from UCI `info` lines.
#[derive(Default, Clone, Debug)]
struct SearchInfo {
    best_move: String,
    depth: i32,
    score: i32,
    nodes: u64,
    nps: u64,
}

impl SearchInfo {
    fn parse_info_line(&mut self, line: &str) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for i in 0..tokens.len() {
            match tokens[i] {
                "depth" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        self.depth = v;
                    }
                }
                "cp" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        self.score = v;
                    }
                }
                "mate" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse::<i32>().ok()) {
                        self.score = if v > 0 { 29000 - v } else { -29000 - v };
                    }
                }
                "nodes" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        self.nodes = v;
                    }
                }
                "nps" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        self.nps = v;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Run an external benchmark against the given positions.
pub fn run_external_bench(
    positions: &[TestPosition],
    config: &ExternalBenchConfig,
) -> Result<BenchSummary, String> {
    if !config.engine_path.exists() {
        return Err(format!(
            "Engine binary not found: {}",
            config.engine_path.display()
        ));
    }

    let mut child = Command::new(&config.engine_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn engine: {e}"))?;

    let stdin = child.stdin.take().ok_or("Failed to open engine stdin")?;
    let stdout = child.stdout.take().ok_or("Failed to open engine stdout")?;

    let mut writer = std::io::BufWriter::new(stdin);
    let mut reader = BufReader::new(stdout);
    let mut engine_name = config
        .engine_path
        .file_name()
        .map_or("unknown".to_string(), |n| n.to_string_lossy().to_string());

    // UCI handshake
    send(&mut writer, "uci")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("Timeout waiting for 'uciok'".into());
        }
        let line = read_line(&mut reader)?;
        if line.starts_with("id name ") {
            engine_name = line["id name ".len()..].trim().to_string();
        }
        if line.trim() == "uciok" {
            break;
        }
    }

    // Configure
    send(
        &mut writer,
        &format!("setoption name Hash value {}", config.hash_mb),
    )?;
    send(
        &mut writer,
        &format!("setoption name Threads value {}", config.threads),
    )?;
    send(&mut writer, "isready")?;
    wait_for(&mut reader, "readyok", Duration::from_secs(10))?;

    println!("Engine: {engine_name}");
    println!();

    let mut results = Vec::with_capacity(positions.len());

    for (i, pos) in positions.iter().enumerate() {
        // Set position
        send(&mut writer, &format!("position fen {}", pos.fen))?;
        send(&mut writer, "isready")?;
        wait_for(&mut reader, "readyok", Duration::from_secs(5))?;

        // Search
        let movetime_ms = config.time_per_position.as_millis();
        send(
            &mut writer,
            &format!("go depth {} movetime {movetime_ms}", config.depth),
        )?;

        let start = Instant::now();
        let mut info = SearchInfo::default();
        let hard_deadline = Instant::now() + config.time_per_position + Duration::from_secs(30);

        loop {
            if Instant::now() > hard_deadline {
                send(&mut writer, "stop")?;
                return Err(format!("Hard timeout on position {}", i + 1));
            }
            let line = read_line(&mut reader)?;
            if line.starts_with("info ") {
                info.parse_info_line(&line);
            } else if line.starts_with("bestmove ") {
                info.best_move = line.split_whitespace().nth(1).unwrap_or("").to_string();
                break;
            }
        }

        let elapsed = start.elapsed();
        let correct = !pos.expected_move.is_empty() && info.best_move == pos.expected_move;
        let nps = if elapsed.as_millis() > 0 {
            info.nodes * 1000 / elapsed.as_millis() as u64
        } else {
            info.nodes
        };

        let pos_result = PositionResult {
            index: i,
            fen: pos.fen.clone(),
            expected_move: pos.expected_move.clone(),
            found_move: info.best_move.clone(),
            correct,
            score: info.score,
            depth: info.depth,
            nodes: info.nodes,
            nps,
            elapsed,
        };

        let status = if pos.expected_move.is_empty() {
            "  "
        } else if correct {
            "OK"
        } else {
            "--"
        };
        println!(
            "[{:>2}] {} found={:<8} expected={:<8} score={:>5}cp  {} NPS ({}ms)",
            i + 1,
            status,
            info.best_move,
            if pos.expected_move.is_empty() {
                "N/A"
            } else {
                &pos.expected_move
            },
            info.score,
            format_nps(nps),
            elapsed.as_millis(),
        );

        results.push(pos_result);
    }

    // Quit
    let _ = send(&mut writer, "quit");
    let _ = child.wait();

    Ok(BenchSummary::from_results(&engine_name, results))
}

/// Send a line to the engine's stdin.
fn send<W: Write>(writer: &mut W, cmd: &str) -> Result<(), String> {
    writeln!(writer, "{cmd}").map_err(|e| format!("Write error: {e}"))?;
    writer.flush().map_err(|e| format!("Flush error: {e}"))?;
    Ok(())
}

/// Read one line from the engine's stdout.
fn read_line<R: BufRead>(reader: &mut R) -> Result<String, String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err("Engine closed stdout (crashed?)".into()),
        Ok(_) => Ok(line.trim_end().to_string()),
        Err(e) => Err(format!("Read error: {e}")),
    }
}

/// Read lines until a specific response is found.
fn wait_for<R: BufRead>(reader: &mut R, expected: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err(format!("Timeout waiting for '{expected}'"));
        }
        let line = read_line(reader)?;
        if line.trim() == expected {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_info_line() {
        let mut info = SearchInfo::default();
        info.parse_info_line("info depth 12 score cp 45 nodes 123456 nps 1000000");
        assert_eq!(info.depth, 12);
        assert_eq!(info.score, 45);
        assert_eq!(info.nodes, 123456);
        assert_eq!(info.nps, 1000000);
    }

    #[test]
    fn test_parse_mate_score() {
        let mut info = SearchInfo::default();
        info.parse_info_line("info depth 8 score mate 3 nodes 5000");
        assert_eq!(info.depth, 8);
        assert_eq!(info.score, 29000 - 3);
        assert_eq!(info.nodes, 5000);
    }
}
