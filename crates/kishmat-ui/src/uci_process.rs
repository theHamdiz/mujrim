//! UCI process management — spawn and communicate with external UCI engines.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

/// A running UCI engine process.
#[allow(dead_code)]
pub struct UciProcess {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

#[allow(dead_code)]
impl UciProcess {
    /// Spawns a new UCI engine process.
    pub fn new(engine_path: &str) -> Result<Self, String> {
        let mut child = Command::new(engine_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start engine: {e}"))?;

        let stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let reader = BufReader::new(stdout);

        let mut proc = Self { child, stdin, reader };

        // Initialize UCI handshake
        proc.send("uci")?;
        proc.wait_for("uciok")?;
        proc.send("isready")?;
        proc.wait_for("readyok")?;

        Ok(proc)
    }

    /// Sends a command to the engine.
    pub fn send(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stdin, "{cmd}")
            .map_err(|e| format!("Failed to write to engine: {e}"))?;
        self.stdin.flush()
            .map_err(|e| format!("Failed to flush: {e}"))?;
        Ok(())
    }

    /// Reads lines until one starts with the expected prefix.
    pub fn wait_for(&mut self, prefix: &str) -> Result<String, String> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return Err("Engine closed stdout".to_string()),
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.starts_with(prefix) {
                        return Ok(trimmed.to_string());
                    }
                }
                Err(e) => return Err(format!("Read error: {e}")),
            }
        }
    }

    /// Sends a position and go command, waits for bestmove.
    pub fn go_position(&mut self, fen: &str, depth: i32) -> Result<String, String> {
        self.send(&format!("position fen {fen}"))?;
        self.send(&format!("go depth {depth}"))?;

        let bestmove_line = self.wait_for("bestmove")?;

        // Parse "bestmove e2e4 ponder e7e5" -> "e2e4"
        let parts: Vec<&str> = bestmove_line.split_whitespace().collect();
        if parts.len() >= 2 {
            Ok(parts[1].to_string())
        } else {
            Err("Invalid bestmove response".to_string())
        }
    }

    /// Sends the quit command and kills the process.
    pub fn quit(&mut self) {
        let _ = self.send("quit");
        let _ = self.child.wait();
    }
}

impl Drop for UciProcess {
    fn drop(&mut self) {
        self.quit();
    }
}
