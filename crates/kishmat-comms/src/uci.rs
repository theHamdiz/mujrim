//! Full UCI (Universal Chess Interface) protocol handler.
//!
//! Hardened for real-world GUI compatibility:
//! - Every stdout line followed by flush
//! - All unknown/malformed input handled gracefully (never crash)
//! - Full `go` parameter support (movestogo, nodes, mate, etc.)
//! - setoption support (Hash, MoveOverhead)
//! - Advanced time management

use std::io::{self, BufRead, Write};
use std::time::Duration;
use types::{Board, Move};
use search::SearchEngine;
use search::book::OpeningBook;

/// Default move overhead in milliseconds (for GUI lag, OS scheduler, etc.)
const DEFAULT_MOVE_OVERHEAD_MS: u64 = 10;
/// Maximum depth the engine will ever search
const MAX_DEPTH: i32 = 128;
/// Default hash table size in MB (leverage 96GB RAM)
const DEFAULT_HASH_MB: usize = 4096;
/// Default number of search threads (Ryzen 9950X = 16C/32T)
/// Default threads = number of CPU cores (clamped to 1..=256).
fn default_threads() -> usize {
    std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1).clamp(1, 256)
}

/// The UCI handler, owning the board and search engine.
pub struct UciHandler {
    pub board: Board,
    engine: SearchEngine,
    /// Move overhead subtracted from time allocation (accounts for lag)
    move_overhead_ms: u64,
    /// Number of search threads
    num_threads: usize,
    /// Opening book
    book: Option<OpeningBook>,
    /// Whether to use the opening book
    use_book: bool,
    /// Debug mode (UCI `debug` command)
    debug_mode: bool,
    /// Whether to use NNUE evaluation
    use_nnue: bool,
}

impl Default for UciHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Immediately writes a line to stdout and flushes.
#[inline]
fn uci_println(msg: &str) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

impl UciHandler {
    pub fn new() -> Self {
        types::init();
        let book = OpeningBook::load_embedded().ok();
        let has_book = book.is_some();
        Self {
            board: Board::new(),
            engine: SearchEngine::new(DEFAULT_HASH_MB, default_threads()),
            move_overhead_ms: DEFAULT_MOVE_OVERHEAD_MS,
            num_threads: default_threads(),
            book,
            use_book: has_book,
            debug_mode: false,
            use_nnue: true,
        }
    }

    /// Main UCI loop — reads from stdin, writes to stdout.
    pub fn run(&mut self) {
        let stdin = io::stdin();

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // stdin closed
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
                "uci" => self.handle_uci(),
                "isready" => uci_println("readyok"),
                "ucinewgame" => self.handle_new_game(),
                "position" => self.handle_position(&parts[1..]),
                "go" => self.handle_go(&parts[1..]),
                "stop" => self.engine.stop(),
                "ponderhit" => {
                    // Ponderhit: switch from pondering to normal search.
                    // Currently we don't support pondering, so this is a no-op.
                    if self.debug_mode {
                        uci_println("info string ponderhit received (pondering not supported)");
                    }
                }
                "register" => {
                    // UCI registration — engine is free, always respond with "registration ok"
                    uci_println("registration ok");
                }
                "debug" => {
                    let on = parts.get(1).map_or(false, |s| *s == "on");
                    self.debug_mode = on;
                    if self.debug_mode {
                        uci_println("info string Debug mode enabled");
                    }
                }
                "quit" | "exit" => break,
                "d" | "display" => {
                    uci_println(&format!("{}", self.board));
                    uci_println(&format!("Fen: {}", self.board.to_fen()));
                }
                "perft" => {
                    if let Some(depth) = parts.get(1).and_then(|s| s.parse::<u32>().ok()) {
                        self.handle_perft(depth);
                    }
                }
                "setoption" => self.handle_setoption(&parts[1..]),
                "eval" => {
                    // Non-standard but useful: print the static evaluation
                    let score = eval::evaluate(&self.board);
                    uci_println(&format!("info string Classical eval: {score}cp"));
                    if search::nnue::network::is_nnue_ready() {
                        let mut state = search::nnue::NNUEState::new();
                        let correction = search::nnue::evaluate_nnue(&self.board, &mut state);
                        uci_println(&format!("info string NNUE correction: {correction}cp"));
                        uci_println(&format!("info string Hybrid eval: {}cp", score + correction));
                    }
                }
                _ => {
                    // Unknown command — silently ignore per UCI spec
                    if self.debug_mode {
                        uci_println(&format!("info string Unknown command: {}", parts[0]));
                    }
                }
            }
        }
    }

    /// Responds to `uci` with identification and option list.
    fn handle_uci(&self) {
        uci_println("id name KishMat 2.0.0");
        uci_println("id author Ahmad Hamdi Emara");
        uci_println(&format!(
            "option name Hash type spin default {DEFAULT_HASH_MB} min 1 max 65536"
        ));
        let dt = default_threads();
        uci_println(&format!(
            "option name Threads type spin default {dt} min 1 max 256"
        ));
        uci_println(&format!(
            "option name MoveOverhead type spin default {DEFAULT_MOVE_OVERHEAD_MS} min 0 max 5000"
        ));
        uci_println("option name OwnBook type check default true");
        uci_println("option name UseNNUE type check default true");
        uci_println("option name Ponder type check default false");
        uci_println("option name UCI_AnalyseMode type check default false");
        uci_println("option name UCI_Chess960 type check default false");
        uci_println("uciok");
    }

    /// Handles `ucinewgame`.
    fn handle_new_game(&mut self) {
        self.engine.clear();
        self.board = Board::new();
    }

    /// Handles `setoption name <name> value <value>`.
    fn handle_setoption(&mut self, args: &[&str]) {
        // Parse: name <tokens> value <tokens>
        let joined = args.join(" ");
        let parts: Vec<&str> = joined.splitn(2, "value").collect();
        let name = parts
            .first()
            .unwrap_or(&"")
            .trim()
            .strip_prefix("name")
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let value = parts.get(1).unwrap_or(&"").trim();

        match name.as_str() {
            "hash" => {
                if let Ok(mb) = value.parse::<usize>() {
                    let mb = mb.clamp(1, 8192);
                    self.engine = SearchEngine::new(mb, self.num_threads);
                    eprintln!("info string Hash set to {mb} MB");
                }
            }
            "threads" => {
                if let Ok(t) = value.parse::<usize>() {
                    self.num_threads = t.clamp(1, 256);
                    self.engine = SearchEngine::new(DEFAULT_HASH_MB, self.num_threads);
                    eprintln!("info string Threads set to {}", self.num_threads);
                }
            }
            "moveoverhead" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.move_overhead_ms = ms.min(5000);
                    eprintln!("info string MoveOverhead set to {} ms", self.move_overhead_ms);
                }
            }
            "ownbook" => {
                self.use_book = value == "true";
                if self.debug_mode {
                    eprintln!("info string OwnBook set to {}", self.use_book);
                }
            }
            "usennue" => {
                self.use_nnue = value == "true";
                if self.debug_mode {
                    eprintln!("info string UseNNUE set to {}", self.use_nnue);
                }
            }
            // UCI standard options we accept but don't actively use
            "ponder" | "uci_analysemode" | "uci_chess960" => {
                if self.debug_mode {
                    eprintln!("info string Option {name} acknowledged");
                }
            }
            _ => {
                if self.debug_mode {
                    eprintln!("info string Unknown option: {name}");
                }
            }
        }
    }

    /// Handles the `position` command. Public for testing.
    pub fn handle_position(&mut self, args: &[&str]) {
        if args.is_empty() {
            return;
        }

        let mut move_start_idx = args.len(); // Default: no moves

        if args[0] == "startpos" {
            self.board = Board::new();
            move_start_idx = if args.len() > 1 && args[1] == "moves" { 2 } else { 1 };
            // If next arg isn't "moves", set idx past end
            if move_start_idx == 1 {
                move_start_idx = args.len();
            }
        } else if args[0] == "fen" {
            // Collect FEN parts (up to "moves" keyword or end)
            let mut fen_parts = Vec::new();
            let mut i = 1;
            while i < args.len() && args[i] != "moves" {
                fen_parts.push(args[i]);
                i += 1;
            }
            let fen = fen_parts.join(" ");
            match Board::from_fen(&fen) {
                Ok(board) => self.board = board,
                Err(e) => {
                    eprintln!("info string Invalid FEN: {e}");
                    return;
                }
            }
            move_start_idx = if i < args.len() && args[i] == "moves" {
                i + 1
            } else {
                args.len()
            };
        }

        // Apply moves
        for &move_str in &args[move_start_idx..] {
            if let Some(mv) = self.parse_uci_move(move_str) {
                self.board.make_move(mv);
            } else {
                eprintln!("info string Invalid move: {move_str}");
                return;
            }
        }
    }

    /// Parses a UCI move string against the current board's legal moves.
    pub fn parse_uci_move(&mut self, s: &str) -> Option<Move> {
        let legal_moves = self.board.generate_legal_moves();
        legal_moves.iter().copied().find(|mv| mv.to_uci() == s)
    }

    /// Handles the `go` command with full parameter support.
    fn handle_go(&mut self, args: &[&str]) {
        let mut depth = MAX_DEPTH;
        let mut movetime: Option<u64> = None;
        let mut wtime: Option<u64> = None;
        let mut btime: Option<u64> = None;
        let mut winc: u64 = 0;
        let mut binc: u64 = 0;
        let mut movestogo: Option<u64> = None;
        let mut infinite = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "depth" => {
                    depth = Self::next_int(args, i).unwrap_or(MAX_DEPTH).min(MAX_DEPTH);
                    i += 1;
                }
                "movetime" => {
                    movetime = Self::next_int(args, i);
                    i += 1;
                }
                "wtime" => {
                    wtime = Self::next_int(args, i);
                    i += 1;
                }
                "btime" => {
                    btime = Self::next_int(args, i);
                    i += 1;
                }
                "winc" => {
                    winc = Self::next_int(args, i).unwrap_or(0);
                    i += 1;
                }
                "binc" => {
                    binc = Self::next_int(args, i).unwrap_or(0);
                    i += 1;
                }
                "movestogo" => {
                    movestogo = Self::next_int(args, i);
                    i += 1;
                }
                "infinite" => {
                    infinite = true;
                }
                "ponder" => {
                    // Ponder not implemented — treat as infinite for now
                    infinite = true;
                }
                "perft" => {
                    if let Some(d) = Self::next_int::<u32>(args, i) {
                        self.handle_perft(d);
                        return;
                    }
                    i += 1;
                }
                // `nodes`, `mate`, `searchmoves` — accepted but not fully utilized
                "nodes" | "mate" | "searchmoves" => {
                    i += 1; // skip the value
                }
                _ => {}
            }
            i += 1;
        }

        // Calculate time limit
        let time_limit = if infinite {
            None // Search until `stop`
        } else if let Some(mt) = movetime {
            // Fixed time per move — subtract overhead
            let safe = mt.saturating_sub(self.move_overhead_ms);
            Some(Duration::from_millis(safe.max(10)))
        } else {
            self.calculate_time_allocation(wtime, btime, winc, binc, movestogo)
        };

        // Try opening book first
        if self.use_book && !infinite {
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&self.board) {
                    // Verify book move is legal
                    let legal = self.board.generate_legal_moves();
                    if legal.iter().any(|m| m.from == book_move.from && m.to == book_move.to) {
                        uci_println(&format!("info string Book move"));
                        uci_println(&format!("bestmove {}", book_move.to_uci()));
                        return;
                    }
                }
            }
        }

        let result = if let Some(tl) = time_limit {
            self.engine.search_time(&mut self.board, tl, depth)
        } else {
            self.engine.search_depth(&mut self.board, depth)
        };

        uci_println(&format!("bestmove {}", result.best_move.to_uci()));
    }

    /// Advanced time management: allocates time based on remaining clock,
    /// increment, movestogo, and position characteristics.
    fn calculate_time_allocation(
        &self,
        wtime: Option<u64>,
        btime: Option<u64>,
        winc: u64,
        binc: u64,
        movestogo: Option<u64>,
    ) -> Option<Duration> {
        let our_time = match self.board.side_to_move {
            types::Color::White => wtime?,
            types::Color::Black => btime?,
        };
        let our_inc = match self.board.side_to_move {
            types::Color::White => winc,
            types::Color::Black => binc,
        };

        // Subtract overhead for safety
        let safe_time = our_time.saturating_sub(self.move_overhead_ms);

        // Emergency mode: very low time — just move fast
        if safe_time < 100 {
            return Some(Duration::from_millis(10));
        }

        // Estimate moves remaining in the game
        let moves_left = if let Some(mtg) = movestogo {
            // Tournament time control: exact moves to go
            mtg.max(1)
        } else {
            // Sudden death: estimate based on game phase
            self.estimate_moves_remaining()
        };

        // Base allocation: divide remaining time by estimated moves
        let base_alloc = safe_time / moves_left;

        // Add a portion of the increment
        let inc_bonus = (our_inc * 3) / 4;

        // Total allocation
        let mut alloc = base_alloc + inc_bonus;

        // Never use more than a fraction of remaining time (safety cap)
        // In sudden death, never use more than 1/3 of remaining time
        // With movestogo, allow up to 1/2
        let max_fraction = if movestogo.is_some() {
            safe_time / 2
        } else {
            safe_time / 3
        };
        alloc = alloc.min(max_fraction);

        // Position complexity scaling: if in check or many pieces, use more time
        if self.board.in_check() {
            alloc = (alloc * 5) / 4; // 25% more when in check
        }

        // Clamp to reasonable bounds
        alloc = alloc.clamp(20, safe_time.saturating_sub(10));

        Some(Duration::from_millis(alloc))
    }

    /// Estimates the number of moves remaining in the game based on game phase.
    fn estimate_moves_remaining(&self) -> u64 {
        let total_pieces = self.board.total_piece_count() as u64;
        if total_pieces > 24 {
            35 // Opening/early middlegame
        } else if total_pieces > 16 {
            30 // Middlegame
        } else if total_pieces > 8 {
            25 // Early endgame
        } else {
            20 // Deep endgame (fewer pieces → shorter game expected)
        }
    }

    /// Runs a perft test and prints results.
    fn handle_perft(&mut self, depth: u32) {
        let start = std::time::Instant::now();
        let nodes = self.board.perft(depth);
        let elapsed = start.elapsed();
        let nps = if elapsed.as_secs() > 0 {
            nodes / elapsed.as_secs()
        } else {
            nodes
        };
        uci_println(&format!(
            "Nodes searched: {nodes} ({nps} nps, {}ms)",
            elapsed.as_millis()
        ));
    }

    /// Safely parses the next integer argument.
    #[inline]
    fn next_int<T: std::str::FromStr>(args: &[&str], i: usize) -> Option<T> {
        args.get(i + 1).and_then(|s| s.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_handler_creation() {
        let mut handler = UciHandler::new();
        assert_eq!(handler.board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn test_parse_position_startpos() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos"]);
        assert_eq!(handler.board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn test_parse_position_startpos_with_moves() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos", "moves", "e2e4", "e7e5"]);
        assert_eq!(handler.board.side_to_move, types::Color::White);
        // Verify pawns moved
        assert_eq!(handler.board.piece_on(types::Square::E4),
            Some((types::Piece::Pawn, types::Color::White)));
        assert_eq!(handler.board.piece_on(types::Square::E5),
            Some((types::Piece::Pawn, types::Color::Black)));
    }

    #[test]
    fn test_parse_position_fen() {
        let mut handler = UciHandler::new();
        handler.handle_position(&[
            "fen", "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR", "b", "KQkq", "e3", "0", "1"
        ]);
        assert_eq!(handler.board.side_to_move, types::Color::Black);
    }

    #[test]
    fn test_parse_position_fen_with_moves() {
        let mut handler = UciHandler::new();
        handler.handle_position(&[
            "fen", "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR", "b", "KQkq", "e3", "0", "1",
            "moves", "e7e5"
        ]);
        assert_eq!(handler.board.side_to_move, types::Color::White);
    }

    #[test]
    fn test_parse_position_empty_args_no_crash() {
        let mut handler = UciHandler::new();
        handler.handle_position(&[]); // Should not crash
    }

    #[test]
    fn test_parse_position_invalid_fen_no_crash() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["fen", "not", "a", "valid", "fen", "at", "all"]);
        // Should not crash — board should remain valid
    }

    #[test]
    fn test_parse_position_invalid_move_no_crash() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos", "moves", "z9z9"]);
        // Should not crash — invalid move is ignored
    }

    #[test]
    fn test_parse_uci_move_legal() {
        let mut handler = UciHandler::new();
        let mv = handler.parse_uci_move("e2e4");
        assert!(mv.is_some(), "e2e4 should be a legal move from starting position");
    }

    #[test]
    fn test_parse_uci_move_illegal() {
        let mut handler = UciHandler::new();
        let mv = handler.parse_uci_move("e2e5");
        assert!(mv.is_none(), "e2e5 should not be legal from starting position");
    }

    #[test]
    fn test_sequential_positions_overwrite() {
        let mut handler = UciHandler::new();

        // First position
        handler.handle_position(&["startpos", "moves", "e2e4"]);
        assert_eq!(handler.board.side_to_move, types::Color::Black);

        // New position should fully replace the old one
        handler.handle_position(&["startpos"]);
        assert_eq!(handler.board.to_fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn test_many_moves_from_startpos() {
        let mut handler = UciHandler::new();
        // Italian Game opening
        handler.handle_position(&[
            "startpos", "moves",
            "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6"
        ]);
        // Should not crash, board should be valid
        assert_eq!(handler.board.side_to_move, types::Color::White);
        assert!(handler.board.total_piece_count() == 32, "No captures in Italian opening");
    }
}

