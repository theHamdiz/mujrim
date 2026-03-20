//! XBoard/CECP protocol handler.

use search::SearchEngine;
#[cfg(feature = "book")]
use search::book::OpeningBook;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use types::{Board, Color, Move};

/// Immediately writes a line to stdout and flushes.
#[inline]
fn xboard_println(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

/// A handle to an in-progress background search (used for async pondering).
#[allow(dead_code)]
struct RunningSearch {
    handle: JoinHandle<search::engine::SearchResult>,
    stop_token: Arc<AtomicBool>,
    emit_move: bool,
}

/// XBoard protocol handler.
pub struct XBoardHandler {
    pub board: Board,
    engine: SearchEngine,
    #[cfg(feature = "book")]
    book: Option<OpeningBook>,
    use_book: bool,
    /// Remaining time in centiseconds.
    time_remaining_cs: u64,
    /// Opponent's remaining time in centiseconds.
    opp_time_cs: u64,
    /// Increment in centiseconds.
    increment_cs: u64,
    /// Fixed time per move in centiseconds (`st` command).
    fixed_time_per_move_cs: Option<u64>,
    /// Search depth limit.
    max_depth: i32,
    /// Force mode (engine does not move).
    force_mode: bool,
    /// Analyze mode.
    analyze_mode: bool,
    /// Paused mode.
    paused: bool,
    /// Are we currently in an active game.
    playing: bool,
    /// Color played by the engine.
    engine_color: Color,
    /// Post output enabled.
    post: bool,
    /// Pondering mode.
    ponder: bool,
    /// Current hash size for dynamic reconfiguration.
    hash_mb: usize,
    /// Current thread count for dynamic reconfiguration.
    threads: usize,
    /// Move history for `undo` / `remove`.
    move_history: Vec<Move>,
}

impl Default for XBoardHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl XBoardHandler {
    pub fn new() -> Self {
        types::init();
        #[cfg(feature = "book")]
        let book = OpeningBook::load_embedded().ok();
        #[cfg(feature = "book")]
        let has_book = book.is_some();
        #[cfg(not(feature = "book"))]
        let has_book = false;
        let hash_mb = 256usize;
        let threads = 4usize;
        Self {
            board: Board::new(),
            engine: SearchEngine::new(hash_mb, threads),
            #[cfg(feature = "book")]
            book,
            use_book: has_book,
            time_remaining_cs: 30_000,
            opp_time_cs: 30_000,
            increment_cs: 0,
            fixed_time_per_move_cs: None,
            max_depth: 128,
            force_mode: false,
            analyze_mode: false,
            paused: false,
            playing: false,
            engine_color: Color::Black,
            post: true,
            ponder: false,
            hash_mb,
            threads,
            move_history: Vec::new(),
        }
    }

    /// Main XBoard loop.
    pub fn run(&mut self) {
        let stdin = io::stdin();

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "xboard" => {}
                "protover" => self.send_features(),
                "new" => self.new_game(),
                "quit" => break,
                "force" => self.force_mode = true,
                "go" => {
                    self.force_mode = false;
                    self.analyze_mode = false;
                    self.playing = true;
                    self.engine_color = self.board.side_to_move;
                    self.think_and_move();
                }
                "playother" => {
                    self.force_mode = false;
                    self.playing = true;
                    self.engine_color = opposite(self.board.side_to_move);
                }
                "ping" => {
                    if let Some(n) = parts.get(1) {
                        xboard_println(&format!("pong {n}"));
                    }
                }
                "setboard" => {
                    let fen = parts[1..].join(" ");
                    if let Err(err) = self.apply_setboard(&fen) {
                        xboard_println(&format!("Error (invalid FEN): {err}"));
                    }
                }
                "white" => {
                    self.handle_white_command();
                }
                "black" => {
                    self.handle_black_command();
                }
                "level" => self.handle_level(&parts),
                "st" => {
                    if let Some(secs) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) {
                        self.fixed_time_per_move_cs = Some(secs.max(1) * 100);
                    }
                }
                "time" => {
                    if let Some(t) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) {
                        self.time_remaining_cs = t;
                    }
                }
                "otim" => {
                    if let Some(t) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) {
                        self.opp_time_cs = t;
                    }
                }
                "sd" => {
                    if let Some(d) = parts.get(1).and_then(|s| s.parse::<i32>().ok()) {
                        self.max_depth = d.clamp(1, 128);
                    }
                }
                "memory" => {
                    if let Some(mb) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                        self.hash_mb = mb.clamp(1, 8192);
                        self.reconfigure_engine();
                    }
                }
                "cores" => {
                    if let Some(n) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                        self.threads = n.clamp(1, 256);
                        self.reconfigure_engine();
                    }
                }
                "hard" => self.ponder = true,
                "easy" => self.ponder = false,
                "post" => self.post = true,
                "nopost" => self.post = false,
                "analyze" => {
                    self.analyze_mode = true;
                    self.force_mode = true;
                    self.playing = true;
                }
                "exit" => {
                    if self.analyze_mode {
                        self.analyze_mode = false;
                        self.force_mode = false;
                    }
                }
                "pause" => self.paused = true,
                "resume" => {
                    self.paused = false;
                    if self.should_engine_move() {
                        self.think_and_move();
                    }
                }
                "?" => {
                    // Move now.
                    if self.should_engine_move() {
                        self.think_and_move();
                    }
                }
                "result" => {
                    self.playing = false;
                    self.force_mode = true;
                }
                "undo" => self.undo_last_move(),
                "remove" => {
                    self.undo_last_move();
                    self.undo_last_move();
                }
                "usermove" => {
                    if let Some(move_str) = parts.get(1) {
                        self.handle_user_move(move_str);
                    }
                }
                "variant" => {
                    if let Some(v) = parts.get(1) {
                        if *v != "normal" {
                            xboard_println(&format!("Error (unsupported variant): {v}"));
                        }
                    }
                }
                // Informational/optional CECP commands.
                "random" | "computer" | "name" | "rating" | "ics" | "accepted" | "rejected"
                | "option" | "hint" => {}
                cmd => {
                    // Some GUIs send bare coordinate moves without `usermove`.
                    if cmd.len() >= 4 {
                        self.handle_user_move(cmd);
                    }
                }
            }
        }
    }

    fn send_features(&self) {
        xboard_println("feature myname=\"KishMat 2.0.0\"");
        xboard_println("feature setboard=1");
        xboard_println("feature ping=1");
        xboard_println("feature usermove=1");
        xboard_println("feature time=1");
        xboard_println("feature analyze=1");
        xboard_println("feature memory=1");
        xboard_println("feature smp=1");
        xboard_println("feature sigint=0");
        xboard_println("feature sigterm=0");
        xboard_println("feature colors=0");
        xboard_println("feature done=1");
    }

    fn new_game(&mut self) {
        self.board = Board::new();
        self.engine.clear();
        self.move_history.clear();
        self.force_mode = false;
        self.analyze_mode = false;
        self.paused = false;
        self.playing = true;
        self.engine_color = Color::Black;
        self.max_depth = 128;
        self.increment_cs = 0;
        self.fixed_time_per_move_cs = None;
    }

    fn apply_setboard(&mut self, fen: &str) -> Result<(), String> {
        let board = Board::from_fen(fen).map_err(|e| e.to_string())?;
        self.board = board;
        self.move_history.clear();
        Ok(())
    }

    fn handle_white_command(&mut self) {
        self.force_mode = false;
        self.playing = true;
        self.engine_color = Color::Black;
        if self.should_engine_move() {
            self.think_and_move();
        }
    }

    fn handle_black_command(&mut self) {
        self.force_mode = false;
        self.playing = true;
        self.engine_color = Color::White;
        if self.should_engine_move() {
            self.think_and_move();
        }
    }

    fn reconfigure_engine(&mut self) {
        self.engine = SearchEngine::new(self.hash_mb, self.threads);
    }

    fn should_engine_move(&self) -> bool {
        !self.paused
            && !self.force_mode
            && self.playing
            && (self.analyze_mode || self.board.side_to_move == self.engine_color)
    }

    fn handle_user_move(&mut self, move_str: &str) {
        if let Some(mv) = self.parse_move(move_str) {
            self.board.make_move(mv);
            self.move_history.push(mv);
            if self.should_engine_move() {
                self.think_and_move();
            }
        } else {
            xboard_println(&format!("Illegal move: {move_str}"));
        }
    }

    fn undo_last_move(&mut self) {
        if let Some(mv) = self.move_history.pop() {
            self.board.unmake_move(mv);
        }
    }

    fn think_and_move(&mut self) {
        if !self.should_engine_move() {
            return;
        }

        #[cfg(feature = "book")]
        if self.use_book {
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&self.board) {
                    let legal = self.board.generate_legal_moves();
                    if legal
                        .iter()
                        .any(|m| m.from == book_move.from && m.to == book_move.to)
                    {
                        xboard_println(&format!("move {}", book_move.to_uci()));
                        self.board.make_move(book_move);
                        self.move_history.push(book_move);
                        return;
                    }
                }
            }
        }

        let move_time_cs = self
            .fixed_time_per_move_cs
            .unwrap_or_else(|| ((self.time_remaining_cs / 30) + (self.increment_cs / 2)).max(10));
        let time_limit = Duration::from_millis(move_time_cs.saturating_mul(10));

        let start = Instant::now();
        let result = self
            .engine
            .search_time_hard(&mut self.board, time_limit, self.max_depth);
        let elapsed_cs = (start.elapsed().as_millis() / 10) as u64;

        let mv = result.best_move;
        xboard_println(&format!("move {}", mv.to_uci()));
        self.board.make_move(mv);
        self.move_history.push(mv);

        if self.fixed_time_per_move_cs.is_none() {
            self.time_remaining_cs = self
                .time_remaining_cs
                .saturating_sub(elapsed_cs)
                .saturating_add(self.increment_cs);
        }
    }

    fn parse_move(&mut self, s: &str) -> Option<Move> {
        let legal_moves = self.board.generate_legal_moves();
        legal_moves.iter().copied().find(|mv| mv.to_uci() == s)
    }

    fn handle_level(&mut self, parts: &[&str]) {
        // CECP: level MPS BASE INC
        // BASE is minutes or "M:SS". INC is seconds (may be fractional).
        if parts.len() < 4 {
            return;
        }

        let _mps = parts[1].parse::<u64>().ok();
        if let Some(base_cs) = parse_base_time_cs(parts[2]) {
            self.time_remaining_cs = base_cs;
            self.opp_time_cs = base_cs;
        }
        if let Ok(inc) = parts[3].parse::<f64>() {
            self.increment_cs = (inc * 100.0).round().max(0.0) as u64;
        }
    }
}

#[inline]
fn opposite(color: Color) -> Color {
    match color {
        Color::White => Color::Black,
        Color::Black => Color::White,
    }
}

fn parse_base_time_cs(base: &str) -> Option<u64> {
    if let Some((mins, secs)) = base.split_once(':') {
        let m = mins.parse::<u64>().ok()?;
        let s = secs.parse::<u64>().ok()?;
        Some((m.saturating_mul(60).saturating_add(s)).saturating_mul(100))
    } else {
        let mins = base.parse::<u64>().ok()?;
        Some(mins.saturating_mul(60).saturating_mul(100))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_base_time_cs() {
        assert_eq!(parse_base_time_cs("5"), Some(30_000));
        assert_eq!(parse_base_time_cs("2:30"), Some(15_000));
        assert_eq!(parse_base_time_cs("0:05"), Some(500));
    }

    #[test]
    fn test_handle_level_sets_increment() {
        let mut h = XBoardHandler::new();
        h.handle_level(&["level", "40", "5:00", "2"]);
        assert_eq!(h.time_remaining_cs, 30_000);
        assert_eq!(h.increment_cs, 200);
    }

    #[test]
    fn test_undo_last_move_restores_position() {
        let mut h = XBoardHandler::new();
        let start_fen = h.board.to_fen();
        let mv = h.parse_move("e2e4").expect("e2e4 legal");
        h.board.make_move(mv);
        h.move_history.push(mv);
        h.undo_last_move();
        assert_eq!(h.board.to_fen(), start_fen);
    }

    #[test]
    fn test_apply_setboard_rejects_invalid_fen() {
        let mut h = XBoardHandler::new();
        let before = h.board.to_fen();
        assert!(h.apply_setboard("this is not fen").is_err());
        assert_eq!(h.board.to_fen(), before);
    }

    #[test]
    fn test_white_and_black_set_engine_color() {
        let mut h = XBoardHandler::new();
        h.handle_white_command();
        assert_eq!(h.engine_color, Color::Black);

        h.handle_black_command();
        assert_eq!(h.engine_color, Color::White);
    }
}
