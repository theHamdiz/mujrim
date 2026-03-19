use std::fmt::{Display, Formatter};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolKind {
    Uci,
    Xboard,
}

impl Display for ProtocolKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uci => write!(f, "uci"),
            Self::Xboard => write!(f, "xboard"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineOptions {
    pub hash_mb: Option<usize>,
    pub threads: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub fen: String,
    pub depth: i32,
    pub movetime: Option<Duration>,
    pub node_limit: Option<u64>,
}

impl SearchRequest {
    pub fn depth_only(fen: String, depth: i32) -> Self {
        Self {
            fen,
            depth,
            movetime: None,
            node_limit: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchInfo {
    pub best_move: String,
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub nps: u64,
}

pub trait ProtocolDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String>;
    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String>;
    fn set_position(&mut self, io: &mut EngineIo, fen: &str) -> Result<(), String>;
    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String>;
    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String>;

    fn quit(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("quit")
    }
}

pub struct EngineIo {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
    read_timeout: Duration,
}

impl EngineIo {
    fn spawn(path: &Path, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn engine '{}': {e}", path.display()))?;

        let stdin = child.stdin.take().ok_or("failed to open engine stdin")?;
        let stdout = child.stdout.take().ok_or("failed to open engine stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || pump_stdout(stdout, tx));

        Ok(Self {
            child,
            stdin,
            stdout_rx: rx,
            read_timeout: DEFAULT_READ_TIMEOUT,
        })
    }

    pub fn send(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stdin, "{cmd}").map_err(|e| format!("write error: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush error: {e}"))?;
        Ok(())
    }

    pub fn read_line(&mut self) -> Result<String, String> {
        match self.stdout_rx.recv_timeout(self.read_timeout) {
            Ok(line) => Ok(line),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "read timeout after {} ms",
                self.read_timeout.as_millis()
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("engine closed stdout".to_string()),
        }
    }

    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }
}

fn pump_stdout(stdout: ChildStdout, tx: mpsc::Sender<String>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if tx.send(line.trim_end().to_string()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

impl Drop for EngineIo {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

pub struct EngineSession {
    io: EngineIo,
    driver: Box<dyn ProtocolDriver + Send>,
}

impl EngineSession {
    pub fn spawn(path: &Path, protocol: ProtocolKind) -> Result<Self, String> {
        Self::spawn_with_args(path, &[], protocol)
    }

    pub fn spawn_with_args(
        path: &Path,
        args: &[String],
        protocol: ProtocolKind,
    ) -> Result<Self, String> {
        let mut io = EngineIo::spawn(path, args)?;
        let mut driver: Box<dyn ProtocolDriver + Send> = match protocol {
            ProtocolKind::Uci => Box::new(UciDriver::default()),
            ProtocolKind::Xboard => Box::new(XboardDriver::default()),
        };
        driver.initialize(&mut io)?;
        Ok(Self { io, driver })
    }

    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.io.set_read_timeout(timeout);
    }

    pub fn configure(&mut self, options: &EngineOptions) -> Result<(), String> {
        self.driver.configure(&mut self.io, options)
    }

    pub fn search(&mut self, req: &SearchRequest) -> Result<SearchInfo, String> {
        self.driver.set_position(&mut self.io, &req.fen)?;
        self.driver.start_search(&mut self.io, req)?;

        let mut info = SearchInfo::default();
        loop {
            let line = self.io.read_line()?;
            if let Some(best) = self.driver.parse_output_line(&line, &mut info) {
                info.best_move = best;
                return Ok(info);
            }
        }
    }
}

pub fn analyze_once(
    path: &Path,
    protocol: ProtocolKind,
    options: &EngineOptions,
    req: &SearchRequest,
) -> Result<SearchInfo, String> {
    let mut session = EngineSession::spawn(path, protocol)?;
    session.configure(options)?;
    session.search(req)
}

#[derive(Default)]
struct UciDriver;

impl UciDriver {
    fn parse_info_line(&self, line: &str, info: &mut SearchInfo) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for i in 0..tokens.len() {
            match tokens[i] {
                "depth" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        info.depth = v;
                    }
                }
                "cp" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        info.score = v;
                    }
                }
                "mate" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse::<i32>().ok()) {
                        info.score = if v > 0 { 29_000 - v } else { -29_000 - v };
                    }
                }
                "nodes" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        info.nodes = v;
                    }
                }
                "nps" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|s| s.parse().ok()) {
                        info.nps = v;
                    }
                }
                _ => {}
            }
        }
    }
}

impl ProtocolDriver for UciDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("uci")?;
        loop {
            let line = io.read_line()?;
            if line == "uciok" {
                break;
            }
        }
        io.send("isready")?;
        loop {
            if io.read_line()? == "readyok" {
                break;
            }
        }
        Ok(())
    }

    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String> {
        if let Some(hash_mb) = options.hash_mb {
            io.send(&format!("setoption name Hash value {hash_mb}"))?;
        }
        if let Some(threads) = options.threads {
            io.send(&format!("setoption name Threads value {threads}"))?;
        }
        io.send("isready")?;
        loop {
            if io.read_line()? == "readyok" {
                break;
            }
        }
        Ok(())
    }

    fn set_position(&mut self, io: &mut EngineIo, fen: &str) -> Result<(), String> {
        io.send(&format!("position fen {fen}"))?;
        Ok(())
    }

    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String> {
        let mut cmd = format!("go depth {}", req.depth.max(1));
        if let Some(movetime) = req.movetime {
            cmd.push_str(&format!(" movetime {}", movetime.as_millis().max(1)));
        }
        if let Some(nodes) = req.node_limit {
            cmd.push_str(&format!(" nodes {}", nodes.max(1)));
        }
        io.send(&cmd)
    }

    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String> {
        if line.starts_with("info ") {
            self.parse_info_line(line, info);
            return None;
        }
        if line.starts_with("bestmove ") {
            return line.split_whitespace().nth(1).map(ToString::to_string);
        }
        None
    }
}

#[derive(Default)]
struct XboardDriver;

impl XboardDriver {
    fn parse_post_line(&self, line: &str, info: &mut SearchInfo) {
        // Typical post format: "<depth> <score> <time> <nodes> <pv...>"
        let mut it = line.split_whitespace();
        let depth = it.next().and_then(|s| s.parse::<i32>().ok());
        let score = it.next().and_then(|s| s.parse::<i32>().ok());
        let _time = it.next().and_then(|s| s.parse::<u64>().ok());
        let nodes = it.next().and_then(|s| s.parse::<u64>().ok());
        if let Some(d) = depth {
            info.depth = d;
        }
        if let Some(s) = score {
            info.score = s;
        }
        if let Some(n) = nodes {
            info.nodes = n;
        }
    }
}

impl ProtocolDriver for XboardDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("xboard")?;
        io.send("protover 2")?;
        // Read feature lines until done=1 (or until engine starts normal output).
        for _ in 0..128 {
            let line = io.read_line()?;
            if line.starts_with("feature ") && line.contains("done=1") {
                break;
            }
            if line.is_empty() {
                continue;
            }
            if !line.starts_with("feature ") {
                break;
            }
        }
        io.send("new")?;
        Ok(())
    }

    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String> {
        if let Some(hash_mb) = options.hash_mb {
            io.send(&format!("memory {hash_mb}"))?;
        }
        if let Some(threads) = options.threads {
            io.send(&format!("cores {threads}"))?;
        }
        Ok(())
    }

    fn set_position(&mut self, io: &mut EngineIo, fen: &str) -> Result<(), String> {
        io.send(&format!("setboard {fen}"))
    }

    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String> {
        io.send(&format!("sd {}", req.depth.max(1)))?;
        if let Some(movetime) = req.movetime {
            let secs = (movetime.as_millis() as f64 / 1000.0).ceil() as u64;
            io.send(&format!("st {}", secs.max(1)))?;
        }
        io.send("go")
    }

    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String> {
        if let Some(rest) = line.strip_prefix("move ") {
            return rest.split_whitespace().next().map(ToString::to_string);
        }

        if line
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-')
        {
            self.parse_post_line(line, info);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_info_line_parse_cp() {
        let drv = UciDriver;
        let mut info = SearchInfo::default();
        drv.parse_info_line(
            "info depth 12 score cp 45 nodes 12345 nps 900000",
            &mut info,
        );
        assert_eq!(info.depth, 12);
        assert_eq!(info.score, 45);
        assert_eq!(info.nodes, 12345);
        assert_eq!(info.nps, 900000);
    }

    #[test]
    fn test_uci_info_line_parse_mate() {
        let drv = UciDriver;
        let mut info = SearchInfo::default();
        drv.parse_info_line("info depth 10 score mate 3 nodes 1000", &mut info);
        assert_eq!(info.depth, 10);
        assert_eq!(info.score, 28_997);
        assert_eq!(info.nodes, 1000);
    }

    #[test]
    fn test_xboard_parse_post_line() {
        let drv = XboardDriver;
        let mut info = SearchInfo::default();
        drv.parse_post_line("14 36 1234 987654 e2e4 e7e5", &mut info);
        assert_eq!(info.depth, 14);
        assert_eq!(info.score, 36);
        assert_eq!(info.nodes, 987654);
    }

    #[test]
    fn test_protocol_display() {
        assert_eq!(ProtocolKind::Uci.to_string(), "uci");
        assert_eq!(ProtocolKind::Xboard.to_string(), "xboard");
    }
}
