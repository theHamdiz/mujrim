//! XBoard/CECP (Chess Engine Communication Protocol) handler.
//!
//! Provides compatibility with XBoard, WinBoard, and other CECP-based GUIs.
//! Implements the core protocol commands needed for tournament play.

use std::io::{self, BufRead, Write};
use std::time::Duration;
use types::{Board, Move, Color};
use search::SearchEngine;
#[cfg(feature = "book")]
use search::book::OpeningBook;

/// Immediately writes a line to stdout and flushes.
#[inline]
fn xboard_println(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

/// XBoard protocol handler.
pub struct XBoardHandler {
    pub board: Board,
    engine: SearchEngine,
    #[cfg(feature = "book")]
    book: Option<OpeningBook>,
    use_book: bool,
    /// Remaining time in centiseconds
    time_remaining_cs: u64,
    /// Opponent's remaining time in centiseconds
    opp_time_cs: u64,
    /// Search depth limit (0 = unlimited)
    max_depth: i32,
    /// Are we in force mode (not thinking)?
    force_mode: bool,
    /// Are we playing? (false during setup/edit)
    playing: bool,
    /// Engine color
    engine_color: Color,
    /// Post thinking output
    post: bool,
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
        Self {
            board: Board::new(),
            engine: SearchEngine::new(256, 4), // Conservative defaults for XBoard
            #[cfg(feature = "book")]
            book,
            use_book: has_book,
            time_remaining_cs: 30000, // 5 minutes default
            opp_time_cs: 30000,
            max_depth: 128,
            force_mode: false,
            playing: false,
            engine_color: Color::Black, // Default: engine plays black
            post: true,
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
                "xboard" => {
                    // Acknowledge XBoard mode
                    xboard_println("");
                }
                "protover" => {
                    // Protocol version — send features
                    xboard_println("feature myname=\"KishMat 2.0.0\"");
                    xboard_println("feature setboard=1");
                    xboard_println("feature ping=1");
                    xboard_println("feature usermove=1");
                    xboard_println("feature sigint=0");
                    xboard_println("feature sigterm=0");
                    xboard_println("feature colors=0");
                    xboard_println("feature done=1");
                }
                "new" => {
                    self.board = Board::new();
                    self.engine.clear();
                    self.force_mode = false;
                    self.playing = true;
                    self.engine_color = Color::Black;
                    self.max_depth = 128;
                }
                "quit" => break,
                "force" => {
                    self.force_mode = true;
                }
                "go" => {
                    self.force_mode = false;
                    self.engine_color = self.board.side_to_move;
                    self.playing = true;
                    self.think_and_move();
                }
                "ping" => {
                    if let Some(n) = parts.get(1) {
                        xboard_println(&format!("pong {n}"));
                    }
                }
                "setboard" => {
                    let fen = parts[1..].join(" ");
                    if let Ok(board) = Board::from_fen(&fen) {
                        self.board = board;
                    }
                }
                "level" => {
                    // level MPS BASE INC — time control
                    // We just store the base time
                    if let Some(base) = parts.get(2).and_then(|s| s.parse::<u64>().ok()) {
                        self.time_remaining_cs = base * 6000; // minutes to centiseconds
                    }
                }
                "time" => {
                    // time N — remaining time in centiseconds
                    if let Some(t) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) {
                        self.time_remaining_cs = t;
                    }
                }
                "otim" => {
                    // otim N — opponent's remaining time in centiseconds
                    if let Some(t) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) {
                        self.opp_time_cs = t;
                    }
                }
                "sd" => {
                    // sd N — set max depth
                    if let Some(d) = parts.get(1).and_then(|s| s.parse::<i32>().ok()) {
                        self.max_depth = d.clamp(1, 128);
                    }
                }
                "post" => {
                    self.post = true;
                }
                "nopost" => {
                    self.post = false;
                }
                "usermove" => {
                    // usermove MOVE — user makes a move
                    if let Some(move_str) = parts.get(1) {
                        if let Some(mv) = self.parse_move(move_str) {
                            self.board.make_move(mv);
                            if !self.force_mode && self.playing {
                                self.think_and_move();
                            }
                        } else {
                            xboard_println(&format!("Illegal move: {move_str}"));
                        }
                    }
                }
                "result" => {
                    // Game over
                    self.playing = false;
                }
                "undo" => {
                    // Undo last move — would need move history
                    // For now, just acknowledge
                }
                "remove" => {
                    // Remove last two moves — would need move history
                }
                "random" | "computer" | "name" | "rating" | "accepted" | "rejected" => {
                    // Ignore these informational commands
                }
                cmd => {
                    // Try to parse as a move (for GUIs that don't use "usermove" prefix)
                    if cmd.len() >= 4 {
                        if let Some(mv) = self.parse_move(cmd) {
                            self.board.make_move(mv);
                            if !self.force_mode && self.playing {
                                self.think_and_move();
                            }
                        }
                    }
                }
            }
        }
    }

    /// Think and send the best move.
    fn think_and_move(&mut self) {
        // Try opening book
        #[cfg(feature = "book")]
        if self.use_book {
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&self.board) {
                    let legal = self.board.generate_legal_moves();
                    if legal.iter().any(|m| m.from == book_move.from && m.to == book_move.to) {
                        xboard_println(&format!("move {}", book_move.to_uci()));
                        self.board.make_move(book_move);
                        return;
                    }
                }
            }
        }

        // Calculate time
        let time_ms = (self.time_remaining_cs * 10) / 30; // Use ~1/30th of remaining
        let time_limit = Duration::from_millis(time_ms.max(100));

        let result = self.engine.search_time(&mut self.board, time_limit, self.max_depth);
        let mv = result.best_move;

        xboard_println(&format!("move {}", mv.to_uci()));
        self.board.make_move(mv);
    }

    /// Parse a move string against the current board.
    fn parse_move(&mut self, s: &str) -> Option<Move> {
        let legal_moves = self.board.generate_legal_moves();
        legal_moves.iter().copied().find(|mv| mv.to_uci() == s)
    }
}
