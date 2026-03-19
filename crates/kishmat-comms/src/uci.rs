//! Full UCI (Universal Chess Interface) protocol handler.
//!
//! Hardened for real-world GUI compatibility:
//! - Every stdout line followed by flush
//! - All unknown/malformed input handled gracefully (never crash)
//! - Full `go` parameter support (movestogo, nodes, mate, etc.)
//! - setoption support (Hash, MoveOverhead)
//! - Advanced time management

use eval::nnue::{enabled_network_formats, load_network, ActiveNetwork, NnueNetworkSource};
#[cfg(feature = "book")]
use search::book::OpeningBook;
use search::engine::{SearchLimits, SearchResult};
use search::SearchEngine;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

/// Default move overhead in milliseconds (for GUI lag, OS scheduler, etc.)
const DEFAULT_MOVE_OVERHEAD_MS: u64 = 10;
/// Maximum depth the engine will ever search
const MAX_DEPTH: i32 = 128;
/// Default hash table size in MB (leverage 96GB RAM)
const DEFAULT_HASH_MB: usize = 4096;
/// Default number of search threads (Ryzen 9950X = 16C/32T)
/// Default threads = number of CPU cores (clamped to 1..=256).
fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1)
        .clamp(1, 256)
}

#[derive(Debug, Clone, Default)]
struct GoCommand {
    depth: Option<i32>,
    movetime: Option<u64>,
    wtime: Option<u64>,
    btime: Option<u64>,
    winc: Option<u64>,
    binc: Option<u64>,
    movestogo: Option<u64>,
    infinite: bool,
    ponder: bool,
    nodes: Option<u64>,
    mate: Option<u64>,
    searchmoves: Vec<String>,
    perft: Option<u32>,
}

struct RunningSearch {
    handle: JoinHandle<Move>,
    stop_token: Arc<AtomicBool>,
    cancel_token: Arc<AtomicBool>,
    emit_bestmove: bool,
}

/// The UCI handler, owning the board and search engine.
pub struct UciHandler {
    pub board: Board,
    engine: SearchEngine,
    hash_mb: usize,
    /// Move overhead subtracted from time allocation (accounts for lag)
    move_overhead_ms: u64,
    /// Number of search threads
    num_threads: usize,
    /// Opening book
    #[cfg(feature = "book")]
    book: Option<OpeningBook>,
    /// Whether to use the opening book
    use_book: bool,
    /// Debug mode (UCI `debug` command)
    debug_mode: bool,
    /// Whether to use NNUE evaluation
    use_nnue: bool,
    /// Multi-PV count (1 = normal, >1 = show N best lines)
    multi_pv: usize,
    /// Contempt value (positive = avoid draws)
    contempt: i32,
    /// Active runtime NNUE source (embedded by default).
    eval_network: Arc<dyn NnueNetworkSource + Send + Sync>,
    /// Eval preset: "auto", "akimbo", or "stockfish".
    eval_preset: String,
    /// Path last used for `EvalFile` (None = embedded).
    eval_file: Option<String>,
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
        #[cfg(feature = "book")]
        let book = OpeningBook::load_embedded().ok();
        #[cfg(feature = "book")]
        let has_book = book.is_some();
        #[cfg(not(feature = "book"))]
        let has_book = false;
        let eval_network: Arc<dyn NnueNetworkSource + Send + Sync> =
            Arc::new(ActiveNetwork::Embedded);
        let mut engine = SearchEngine::new(DEFAULT_HASH_MB, default_threads());
        engine.set_nnue_network_source(Arc::clone(&eval_network));
        engine.set_params_for_preset("akimbo");
        engine.set_use_nnue(true);
        Self {
            board: Board::new(),
            engine,
            hash_mb: DEFAULT_HASH_MB,
            move_overhead_ms: DEFAULT_MOVE_OVERHEAD_MS,
            num_threads: default_threads(),
            #[cfg(feature = "book")]
            book,
            use_book: has_book,
            debug_mode: false,
            use_nnue: true,
            multi_pv: 1,
            contempt: 24,
            eval_network,
            eval_preset: "auto".to_string(),
            eval_file: None,
        }
    }

    /// Main UCI loop — reads from stdin, writes to stdout.
    pub fn run(&mut self) {
        let (tx, rx) = mpsc::channel::<String>();
        let reader = thread::spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut running: Option<RunningSearch> = None;

        loop {
            self.poll_running_search(&mut running);

            let line = match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
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
                "ucinewgame" => {
                    self.abort_running_search(&mut running, false);
                    self.handle_new_game();
                }
                "position" => {
                    self.abort_running_search(&mut running, false);
                    self.handle_position(&parts[1..]);
                }
                "go" => self.handle_go(&parts[1..], &mut running),
                "stop" => self.abort_running_search(&mut running, true),
                "ponderhit" => {
                    if let Some(task) = running.as_mut() {
                        task.emit_bestmove = true;
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
                "quit" | "exit" => {
                    self.abort_running_search(&mut running, false);
                    break;
                }
                "d" | "display" => {
                    uci_println(&format!("{}", self.board));
                    uci_println(&format!("Fen: {}", self.board.to_fen()));
                }
                "perft" => {
                    self.abort_running_search(&mut running, false);
                    if let Some(depth) = parts.get(1).and_then(|s| s.parse::<u32>().ok()) {
                        self.handle_perft(depth);
                    }
                }
                "setoption" => {
                    self.abort_running_search(&mut running, false);
                    self.handle_setoption(&parts[1..]);
                }
                "eval" => {
                    self.abort_running_search(&mut running, false);
                    // Non-standard but useful: print the static evaluation
                    let classical = eval::evaluate(&self.board);
                    uci_println(&format!("info string Classical eval: {classical}cp"));
                    let mut state = eval::NNUEState::with_network(Arc::clone(&self.eval_network));
                    let nnue_score = state.evaluate(&self.board);
                    uci_println(&format!("info string NNUE eval: {nnue_score}cp"));
                }
                _ => {
                    // Unknown command — silently ignore per UCI spec
                    if self.debug_mode {
                        uci_println(&format!("info string Unknown command: {}", parts[0]));
                    }
                }
            }
        }

        self.abort_running_search(&mut running, false);
        let _ = reader.join();
    }

    fn poll_running_search(&self, running: &mut Option<RunningSearch>) {
        let finished = running
            .as_ref()
            .map_or(false, |task| task.handle.is_finished());
        if !finished {
            return;
        }
        if let Some(task) = running.take() {
            let best_move = task.handle.join().unwrap_or(NULL_MOVE);
            if task.emit_bestmove {
                if best_move == NULL_MOVE {
                    uci_println("bestmove 0000");
                } else {
                    uci_println(&format!("bestmove {}", best_move.to_uci()));
                }
            }
        }
    }

    fn abort_running_search(&self, running: &mut Option<RunningSearch>, emit_bestmove: bool) {
        if let Some(mut task) = running.take() {
            task.cancel_token.store(true, Ordering::SeqCst);
            task.stop_token.store(true, Ordering::SeqCst);
            task.emit_bestmove |= emit_bestmove;
            let best_move = task.handle.join().unwrap_or(NULL_MOVE);
            if task.emit_bestmove {
                if best_move == NULL_MOVE {
                    uci_println("bestmove 0000");
                } else {
                    uci_println(&format!("bestmove {}", best_move.to_uci()));
                }
            }
        }
    }

    fn build_search_engine(&self) -> SearchEngine {
        let mut engine = SearchEngine::new(self.hash_mb, self.num_threads);
        engine.set_nnue_network_source(Arc::clone(&self.eval_network));
        engine.set_params_for_preset(self.active_preset_name());
        engine.set_use_nnue(self.use_nnue);
        engine
    }

    fn search_with_limits_on(
        engine: &mut SearchEngine,
        board: &mut Board,
        depth: i32,
        time_limit: Option<Duration>,
        node_limit: Option<u64>,
        cancel_token: &AtomicBool,
    ) -> SearchResult {
        if cancel_token.load(Ordering::SeqCst) {
            return SearchResult {
                best_move: NULL_MOVE,
                score: 0,
                depth: 0,
                nodes: 0,
                elapsed: Duration::ZERO,
                pv: Vec::new(),
            };
        }
        if time_limit.is_none() && node_limit.is_none() {
            return engine.search_depth(board, depth);
        }
        engine.search(
            board,
            SearchLimits {
                max_depth: depth,
                time_limit,
                node_limit,
                stopped: false,
                use_soft_time: time_limit.is_some() && node_limit.is_none(),
            },
        )
    }

    fn search_restricted_root_on(
        engine: &mut SearchEngine,
        board: &mut Board,
        candidates: &[Move],
        depth: i32,
        time_limit: Option<Duration>,
        node_limit: Option<u64>,
        cancel_token: &AtomicBool,
    ) -> Option<Move> {
        if cancel_token.load(Ordering::SeqCst) {
            return None;
        }
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return Some(candidates[0]);
        }

        let per_move_time = time_limit.map(|t| {
            t.div_f64(candidates.len() as f64)
                .max(Duration::from_millis(10))
        });
        let per_move_nodes = node_limit.map(|n| (n / candidates.len() as u64).max(1));
        let child_depth = (depth - 1).max(1);

        let mut best_score = i32::MIN;
        let mut best_move = None;
        for &mv in candidates {
            if cancel_token.load(Ordering::SeqCst) {
                break;
            }
            board.make_move(mv);
            let child = Self::search_with_limits_on(
                engine,
                board,
                child_depth,
                per_move_time,
                per_move_nodes,
                cancel_token,
            );
            board.unmake_move(mv);
            if child.best_move == NULL_MOVE && cancel_token.load(Ordering::SeqCst) {
                break;
            }
            let score = -child.score;
            if best_move.is_none() || score > best_score {
                best_score = score;
                best_move = Some(mv);
            }
        }
        best_move
    }

    fn start_search_task(
        &self,
        depth: i32,
        time_limit: Option<Duration>,
        node_limit: Option<u64>,
        restricted_moves: Vec<Move>,
        emit_bestmove: bool,
    ) -> RunningSearch {
        let mut board = self.board.clone();
        let mut engine = self.build_search_engine();
        let stop_token = engine.stop_token();
        let cancel_token = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel_token);
        let handle = thread::spawn(move || {
            if cancel_clone.load(Ordering::SeqCst) {
                return NULL_MOVE;
            }
            if !restricted_moves.is_empty() {
                Self::search_restricted_root_on(
                    &mut engine,
                    &mut board,
                    &restricted_moves,
                    depth,
                    time_limit,
                    node_limit,
                    cancel_clone.as_ref(),
                )
                .unwrap_or(NULL_MOVE)
            } else {
                Self::search_with_limits_on(
                    &mut engine,
                    &mut board,
                    depth,
                    time_limit,
                    node_limit,
                    cancel_clone.as_ref(),
                )
                .best_move
            }
        });

        RunningSearch {
            handle,
            stop_token,
            cancel_token,
            emit_bestmove,
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
        uci_println("option name EvalFile type string default <empty>");
        uci_println(
            "option name EvalPreset type combo default auto var auto var akimbo var stockfish",
        );
        uci_println("option name Ponder type check default false");
        uci_println("option name MultiPV type spin default 1 min 1 max 500");
        uci_println("option name Contempt type spin default 24 min -100 max 100");
        uci_println("option name SyzygyPath type string default <empty>");
        uci_println("option name UCI_AnalyseMode type check default false");
        uci_println("option name UCI_Chess960 type check default false");
        let enabled = enabled_network_formats();
        if !enabled.is_empty() {
            let names = enabled
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            uci_println(&format!("info string NNUE file formats enabled: {names}"));
        }
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
                    self.hash_mb = mb.clamp(1, 8192);
                    self.reconfigure_engine();
                    eprintln!("info string Hash set to {} MB", self.hash_mb);
                }
            }
            "threads" => {
                if let Ok(t) = value.parse::<usize>() {
                    self.num_threads = t.clamp(1, 256);
                    self.reconfigure_engine();
                    eprintln!("info string Threads set to {}", self.num_threads);
                }
            }
            "moveoverhead" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.move_overhead_ms = ms.min(5000);
                    eprintln!(
                        "info string MoveOverhead set to {} ms",
                        self.move_overhead_ms
                    );
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
                self.engine.set_use_nnue(self.use_nnue);
                if self.debug_mode {
                    eprintln!("info string UseNNUE set to {}", self.use_nnue);
                }
            }
            "evalfile" => match self.set_eval_file(value) {
                Ok(()) => {
                    let info = self.eval_network.info();
                    eprintln!(
                        "info string EvalFile loaded: {} [{}]",
                        info.name, info.format
                    );
                }
                Err(e) => {
                    eprintln!("info string EvalFile error: {e}");
                }
            },
            "evalpreset" => {
                let preset = value.to_lowercase();
                if matches!(preset.as_str(), "auto" | "akimbo" | "stockfish") {
                    self.eval_preset = preset;
                    self.reconfigure_engine();
                    let active = self.active_preset_name();
                    eprintln!(
                        "info string EvalPreset set to {} (active: {active})",
                        self.eval_preset
                    );
                } else {
                    eprintln!("info string EvalPreset error: invalid preset '{value}'");
                }
            }
            // UCI standard options we accept but don't actively use
            "ponder" | "uci_analysemode" | "uci_chess960" => {
                if self.debug_mode {
                    eprintln!("info string Option {name} acknowledged");
                }
            }
            "multipv" => {
                if let Ok(n) = value.parse::<usize>() {
                    self.multi_pv = n.clamp(1, 500);
                    if self.debug_mode {
                        eprintln!("info string MultiPV set to {}", self.multi_pv);
                    }
                }
            }
            "contempt" => {
                if let Ok(c) = value.parse::<i32>() {
                    self.contempt = c.clamp(-100, 100);
                    if self.debug_mode {
                        eprintln!("info string Contempt set to {}", self.contempt);
                    }
                }
            }
            "syzygypath" => {
                // Store the path for future Syzygy tablebase integration
                if self.debug_mode {
                    eprintln!("info string SyzygyPath set to {value} (not yet active)");
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
            move_start_idx = if args.len() > 1 && args[1] == "moves" {
                2
            } else {
                1
            };
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
    fn handle_go(&mut self, args: &[&str], running: &mut Option<RunningSearch>) {
        // Any new `go` supersedes an older search task.
        self.abort_running_search(running, false);

        let go = Self::parse_go_command(args);
        if let Some(perft_depth) = go.perft {
            self.handle_perft(perft_depth);
            return;
        }

        let mut depth = go.depth.unwrap_or(MAX_DEPTH).clamp(1, MAX_DEPTH);
        if let Some(mate_limit) = go.mate {
            let mate_depth = (mate_limit.saturating_mul(2).saturating_add(1)) as i32;
            depth = depth.min(mate_depth.clamp(1, MAX_DEPTH));
        }

        let time_limit = if go.infinite || go.ponder {
            None
        } else if let Some(mt) = go.movetime {
            let safe = mt.saturating_sub(self.move_overhead_ms);
            Some(Duration::from_millis(safe.max(10)))
        } else {
            self.calculate_time_allocation(
                go.wtime,
                go.btime,
                go.winc.unwrap_or(0),
                go.binc.unwrap_or(0),
                go.movestogo,
            )
        };
        let node_limit = go.nodes;

        let mut restricted_moves = Vec::new();
        for move_str in &go.searchmoves {
            if let Some(mv) = self.parse_uci_move(move_str) {
                if !restricted_moves.iter().any(|m: &Move| {
                    m.from == mv.from && m.to == mv.to && m.promotion == mv.promotion
                }) {
                    restricted_moves.push(mv);
                }
            }
        }

        #[cfg(feature = "book")]
        if self.use_book && !(go.infinite || go.ponder) && restricted_moves.is_empty() {
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&self.board) {
                    let legal = self.board.generate_legal_moves();
                    if legal
                        .iter()
                        .any(|m| m.from == book_move.from && m.to == book_move.to)
                    {
                        uci_println("info string Book move");
                        uci_println(&format!("bestmove {}", book_move.to_uci()));
                        return;
                    }
                }
            }
        }

        if !go.searchmoves.is_empty() {
            if restricted_moves.is_empty() {
                uci_println("bestmove 0000");
                return;
            }
        }
        let task =
            self.start_search_task(depth, time_limit, node_limit, restricted_moves, !go.ponder);
        *running = Some(task);
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
    /// Uses a smooth function of piece count for better accuracy.
    fn estimate_moves_remaining(&self) -> u64 {
        let total_pieces = self.board.total_piece_count() as u64;
        // Smoother scaling: 20 + piece_count * 0.5
        // Ranges from ~20 (2 kings) to ~36 (32 pieces)
        let estimate = 20 + total_pieces / 2;
        estimate.clamp(15, 40)
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

    fn is_go_keyword(token: &str) -> bool {
        matches!(
            token,
            "searchmoves"
                | "wtime"
                | "btime"
                | "winc"
                | "binc"
                | "movestogo"
                | "depth"
                | "nodes"
                | "mate"
                | "movetime"
                | "infinite"
                | "ponder"
                | "perft"
        )
    }

    fn parse_go_command(args: &[&str]) -> GoCommand {
        let mut go = GoCommand::default();
        let mut i = 0usize;
        while i < args.len() {
            match args[i] {
                "depth" => {
                    go.depth = Self::next_int(args, i);
                    i += 2;
                }
                "movetime" => {
                    go.movetime = Self::next_int(args, i);
                    i += 2;
                }
                "wtime" => {
                    go.wtime = Self::next_int(args, i);
                    i += 2;
                }
                "btime" => {
                    go.btime = Self::next_int(args, i);
                    i += 2;
                }
                "winc" => {
                    go.winc = Self::next_int(args, i);
                    i += 2;
                }
                "binc" => {
                    go.binc = Self::next_int(args, i);
                    i += 2;
                }
                "movestogo" => {
                    go.movestogo = Self::next_int(args, i);
                    i += 2;
                }
                "nodes" => {
                    go.nodes = Self::next_int(args, i);
                    i += 2;
                }
                "mate" => {
                    go.mate = Self::next_int(args, i);
                    i += 2;
                }
                "infinite" => {
                    go.infinite = true;
                    i += 1;
                }
                "ponder" => {
                    go.ponder = true;
                    i += 1;
                }
                "perft" => {
                    go.perft = Self::next_int(args, i);
                    i += 2;
                }
                "searchmoves" => {
                    i += 1;
                    while i < args.len() && !Self::is_go_keyword(args[i]) {
                        go.searchmoves.push(args[i].to_string());
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        go
    }

    fn active_preset_name(&self) -> &'static str {
        match self.eval_preset.as_str() {
            "akimbo" => "akimbo",
            "stockfish" => "stockfish",
            _ => self.eval_network.preset_hint(),
        }
    }

    fn reconfigure_engine(&mut self) {
        let mut new_engine = SearchEngine::new(self.hash_mb, self.num_threads);
        new_engine.set_nnue_network_source(Arc::clone(&self.eval_network));
        new_engine.set_params_for_preset(self.active_preset_name());
        new_engine.set_use_nnue(self.use_nnue);
        self.engine = new_engine;
    }

    fn set_eval_file(&mut self, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "<empty>" {
            self.eval_file = None;
            self.eval_network = Arc::new(ActiveNetwork::Embedded);
            self.reconfigure_engine();
            return Ok(());
        }

        let loaded = load_network(Path::new(trimmed))?;
        self.eval_file = Some(trimmed.to_string());
        self.eval_network = Arc::new(loaded);
        self.reconfigure_engine();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_handler_creation() {
        let handler = UciHandler::new();
        assert_eq!(
            handler.board.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[test]
    fn test_parse_position_startpos() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos"]);
        assert_eq!(
            handler.board.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[test]
    fn test_parse_position_startpos_with_moves() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos", "moves", "e2e4", "e7e5"]);
        assert_eq!(handler.board.side_to_move, types::Color::White);
        // Verify pawns moved
        assert_eq!(
            handler.board.piece_on(types::Square::E4),
            Some((types::Piece::Pawn, types::Color::White))
        );
        assert_eq!(
            handler.board.piece_on(types::Square::E5),
            Some((types::Piece::Pawn, types::Color::Black))
        );
    }

    #[test]
    fn test_parse_position_fen() {
        let mut handler = UciHandler::new();
        handler.handle_position(&[
            "fen",
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR",
            "b",
            "KQkq",
            "e3",
            "0",
            "1",
        ]);
        assert_eq!(handler.board.side_to_move, types::Color::Black);
    }

    #[test]
    fn test_parse_position_fen_with_moves() {
        let mut handler = UciHandler::new();
        handler.handle_position(&[
            "fen",
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR",
            "b",
            "KQkq",
            "e3",
            "0",
            "1",
            "moves",
            "e7e5",
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
        assert!(
            mv.is_some(),
            "e2e4 should be a legal move from starting position"
        );
    }

    #[test]
    fn test_parse_uci_move_illegal() {
        let mut handler = UciHandler::new();
        let mv = handler.parse_uci_move("e2e5");
        assert!(
            mv.is_none(),
            "e2e5 should not be legal from starting position"
        );
    }

    #[test]
    fn test_sequential_positions_overwrite() {
        let mut handler = UciHandler::new();

        // First position
        handler.handle_position(&["startpos", "moves", "e2e4"]);
        assert_eq!(handler.board.side_to_move, types::Color::Black);

        // New position should fully replace the old one
        handler.handle_position(&["startpos"]);
        assert_eq!(
            handler.board.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[test]
    fn test_many_moves_from_startpos() {
        let mut handler = UciHandler::new();
        // Italian Game opening
        handler.handle_position(&[
            "startpos", "moves", "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6",
        ]);
        // Should not crash, board should be valid
        assert_eq!(handler.board.side_to_move, types::Color::White);
        assert!(
            handler.board.total_piece_count() == 32,
            "No captures in Italian opening"
        );
    }

    #[test]
    fn test_parse_go_command_supports_nodes_mate_searchmoves() {
        let go = UciHandler::parse_go_command(&[
            "depth",
            "20",
            "nodes",
            "500000",
            "mate",
            "4",
            "searchmoves",
            "e2e4",
            "d2d4",
            "movetime",
            "3000",
        ]);
        assert_eq!(go.depth, Some(20));
        assert_eq!(go.nodes, Some(500000));
        assert_eq!(go.mate, Some(4));
        assert_eq!(go.searchmoves, vec!["e2e4".to_string(), "d2d4".to_string()]);
        assert_eq!(go.movetime, Some(3000));
    }

    #[test]
    fn test_parse_go_command_perft() {
        let go = UciHandler::parse_go_command(&["perft", "5"]);
        assert_eq!(go.perft, Some(5));
    }

    #[test]
    fn test_setoption_evalpreset_switches_search_params() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalPreset", "value", "stockfish"]);
        assert_eq!(handler.eval_preset, "stockfish");
        assert_eq!(handler.engine.params.nmp_base, 7);
    }

    #[test]
    fn test_setoption_evalfile_invalid_keeps_current_network() {
        let mut handler = UciHandler::new();
        let before = handler.eval_network.info().name;
        handler.handle_setoption(&["name", "EvalFile", "value", "/nonexistent/kishmat/net.bin"]);
        assert_eq!(handler.eval_network.info().name, before);
        assert!(handler.eval_file.is_none());
    }

    #[test]
    fn test_setoption_evalfile_loads_embedded_compatible_bin_when_available() {
        let mut handler = UciHandler::new();
        if !enabled_network_formats().contains(&eval::nnue::NetworkFormat::Akimbo) {
            return;
        }
        let net_path = format!(
            "{}/../kishmat-eval/resources/net.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        let args = vec!["name", "EvalFile", "value", net_path.as_str()];
        handler.handle_setoption(&args);
        assert!(handler.eval_file.is_some());
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Akimbo
        );
    }

    #[test]
    fn test_reconfigure_keeps_evalpreset_on_hash_and_threads_change() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalPreset", "value", "stockfish"]);
        handler.handle_setoption(&["name", "Hash", "value", "256"]);
        handler.handle_setoption(&["name", "Threads", "value", "2"]);
        assert_eq!(handler.eval_preset, "stockfish");
        assert_eq!(handler.engine.params.nmp_base, 7);
    }

    #[test]
    fn test_setoption_usennue_toggles_engine_eval_mode() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "UseNNUE", "value", "false"]);
        assert!(!handler.engine.use_nnue());
        handler.handle_setoption(&["name", "UseNNUE", "value", "true"]);
        assert!(handler.engine.use_nnue());
    }

    #[test]
    fn test_handle_go_starts_and_aborts_async_search() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let mut running = None;
        handler.handle_go(&["depth", "4"], &mut running);
        assert!(running.is_some());
        handler.abort_running_search(&mut running, false);
        assert!(running.is_none());
    }

    #[test]
    fn test_handle_go_with_illegal_searchmoves_does_not_start_task() {
        let mut handler = UciHandler::new();
        let mut running = None;
        handler.handle_go(&["searchmoves", "a1a1", "h1h1"], &mut running);
        assert!(running.is_none());
    }

    #[test]
    fn test_poll_running_search_reaps_finished_task() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let mut running = Some(handler.start_search_task(
            1,
            Some(Duration::from_millis(20)),
            None,
            Vec::new(),
            false,
        ));

        for _ in 0..40 {
            let done = running
                .as_ref()
                .is_some_and(|task| task.handle.is_finished());
            if done {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        handler.poll_running_search(&mut running);
        assert!(running.is_none());
    }

    #[test]
    fn test_abort_running_search_with_emit_flag_clears_task() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let mut running = Some(handler.start_search_task(32, None, None, Vec::new(), false));
        handler.abort_running_search(&mut running, true);
        assert!(running.is_none());
    }
}
