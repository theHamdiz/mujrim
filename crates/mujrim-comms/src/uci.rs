//! Full UCI (Universal Chess Interface) protocol handler.
//!
//! Hardened for real-world GUI compatibility:
//! - Every stdout line followed by flush
//! - All unknown/malformed input handled gracefully (never crash)
//! - Full `go` parameter support (movestogo, nodes, mate, etc.)
//! - setoption support (Hash, MoveOverhead)
//! - Advanced time management

use crate::aesthetic::{AestheticConfig, MAX_AESTHETIC_DELTA_CP, RootCandidate, select_root_move};
use eval::nnue::{ActiveNetwork, NnueNetworkSource, enabled_network_formats, load_network};
#[cfg(feature = "book")]
use search::book::OpeningBook;
use search::engine::{SearchLimits, SearchResult};
use search::{SearchEngine, SearchExperiment};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use types::chess_move::NULL_MOVE;
use types::{Board, Move};

/// Default move overhead in milliseconds (for GUI lag, OS scheduler, etc.)
const DEFAULT_MOVE_OVERHEAD_MS: u64 = 10;
/// Maximum depth the engine will ever search
const MAX_DEPTH: i32 = 128;
/// Conservative startup hash; GUIs and match runners can resize it through UCI.
const DEFAULT_HASH_MB: usize = 64;
/// Maximum accepted hash size; also advertised through UCI.
const MAX_HASH_MB: usize = 1024;
/// Conservative startup thread count; callers opt into parallel search via UCI.
const DEFAULT_THREADS: usize = 1;
#[cfg(feature = "book")]
const BOOK_VALIDATION_DEPTH: i32 = 8;
#[cfg(feature = "book")]
const BOOK_VALIDATION_NODES: u64 = 5_000;

#[inline(always)]
fn clamp_hash_mb(size_mb: usize) -> usize {
    size_mb.clamp(1, MAX_HASH_MB)
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

impl GoCommand {
    #[cfg(feature = "book")]
    fn has_explicit_search_rules(&self) -> bool {
        self.depth.is_some()
            || self.movetime.is_some()
            || self.wtime.is_some()
            || self.btime.is_some()
            || self.winc.is_some()
            || self.binc.is_some()
            || self.movestogo.is_some()
            || self.infinite
            || self.ponder
            || self.nodes.is_some()
            || self.mate.is_some()
            || !self.searchmoves.is_empty()
    }
}

struct RootCandidateSearch<'a> {
    candidates: &'a [Move],
    multi_pv: usize,
    aesthetic: AestheticConfig,
    depth: i32,
    time_limit: Option<Duration>,
    node_limit: Option<u64>,
    cancel_token: &'a AtomicBool,
}

struct RunningSearch {
    handle: JoinHandle<(SearchEngine, Move, Option<Move>)>,
    stop_token: Arc<AtomicBool>,
    cancel_token: Arc<AtomicBool>,
    emit_bestmove: bool,
    root_board: Board,
    fallback_move: Move,
}

/// The UCI handler, owning the board and search engine.
pub struct UciHandler {
    pub board: Board,
    engine: Option<SearchEngine>,
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
    /// Standard UCI analysis mode; embedded book moves are never used here.
    analyse_mode: bool,
    /// Whether to use NNUE evaluation
    use_nnue: bool,
    /// Multi-PV count (1 = normal, >1 = show N best lines)
    multi_pv: usize,
    /// Enables root-only style selection among scored Multi-PV candidates.
    aesthetic_bias: bool,
    /// Maximum centipawn loss accepted by root-only style selection.
    aesthetic_delta_cp: i32,
    /// Contempt value (positive = avoid draws)
    contempt: i32,
    /// Active runtime NNUE source (embedded by default).
    eval_network: Arc<dyn NnueNetworkSource + Send + Sync>,
    /// Eval preset: "auto", "akimbo", "stockfish", or "reckless".
    eval_preset: String,
    /// Optional single-component search-policy overlay for controlled matches.
    search_experiment: SearchExperiment,
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

fn format_bestmove(best_move: Move, ponder_move: Option<Move>) -> String {
    if best_move == NULL_MOVE {
        return "bestmove 0000".to_string();
    }
    ponder_move
        .filter(|ponder| *ponder != NULL_MOVE)
        .map_or_else(
            || format!("bestmove {}", best_move.to_uci()),
            |ponder| format!("bestmove {} ponder {}", best_move.to_uci(), ponder.to_uci()),
        )
}

/// Validate worker output against the immutable root snapshot before exposing
/// it to a tournament controller. This is deliberately outside the search hot
/// path and also validates a predicted reply after making the selected move.
fn sanitize_search_output(
    root_board: &Board,
    best_move: Move,
    ponder_move: Option<Move>,
    fallback_move: Move,
) -> (Move, Option<Move>) {
    let mut after_best = root_board.clone();
    let legal_moves = after_best.generate_legal_moves();
    let selected = legal_moves
        .iter()
        .copied()
        .find(|candidate| *candidate == best_move)
        .or_else(|| {
            legal_moves
                .iter()
                .copied()
                .find(|candidate| *candidate == fallback_move)
        })
        .or_else(|| legal_moves.as_slice().first().copied())
        .unwrap_or(NULL_MOVE);

    if selected == NULL_MOVE {
        return (NULL_MOVE, None);
    }

    after_best.make_move(selected);
    let legal_ponder = ponder_move.filter(|ponder| {
        after_best
            .generate_legal_moves()
            .iter()
            .any(|candidate| candidate == ponder)
    });
    (selected, legal_ponder)
}

fn format_final_search_info(result: &SearchResult, score: &str) -> String {
    let elapsed_ms = result.elapsed.as_millis().max(1);
    let elapsed_ns = result.elapsed.as_nanos().max(1);
    let nps = (u128::from(result.nodes).saturating_mul(1_000_000_000) / elapsed_ns)
        .min(u128::from(u64::MAX)) as u64;
    let mut line = format!(
        "info depth {} seldepth {} score {score} nodes {} nps {nps} time {elapsed_ms}",
        result.depth, result.seldepth, result.nodes
    );
    if !result.pv.is_empty() {
        line.push_str(" pv");
        for mv in &result.pv {
            line.push(' ');
            line.push_str(&mv.to_uci());
        }
    }
    line
}

fn normalize_command(line: &str) -> &str {
    line.trim().trim_start_matches('\u{feff}')
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
            Arc::new(eval::nnue::default_embedded_network());

        let preset = eval_network.search_profile().as_str();
        let mut engine = SearchEngine::new(DEFAULT_HASH_MB, DEFAULT_THREADS);
        engine.set_nnue_network_source(Arc::clone(&eval_network));
        engine.set_params_for_preset(preset);
        engine.set_use_nnue(true);
        Self {
            board: Board::new(),
            engine: Some(engine),
            hash_mb: DEFAULT_HASH_MB,
            move_overhead_ms: DEFAULT_MOVE_OVERHEAD_MS,
            num_threads: DEFAULT_THREADS,
            #[cfg(feature = "book")]
            book,
            use_book: has_book,
            debug_mode: false,
            analyse_mode: false,
            use_nnue: true,
            multi_pv: 1,
            aesthetic_bias: false,
            aesthetic_delta_cp: MAX_AESTHETIC_DELTA_CP,
            contempt: 24,
            eval_network,
            eval_preset: "auto".to_string(),
            search_experiment: SearchExperiment::None,
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

            let line = normalize_command(&line).to_string();
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
                        task.stop_token.store(true, Ordering::Relaxed);
                    }
                }
                "register" => {
                    // UCI registration — engine is free, always respond with "registration ok"
                    uci_println("registration ok");
                }
                "debug" => {
                    let on = parts.get(1).is_some_and(|s| *s == "on");
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

    fn poll_running_search(&mut self, running: &mut Option<RunningSearch>) {
        let finished = running
            .as_ref()
            .is_some_and(|task| task.handle.is_finished());
        if !finished || running.as_ref().is_some_and(|task| !task.emit_bestmove) {
            return;
        }
        if let Some(task) = running.take() {
            let emit_bestmove = task.emit_bestmove;
            let (best_move, ponder_move) = self.finish_search_task(task);
            if emit_bestmove {
                uci_println(&format_bestmove(best_move, ponder_move));
            }
        }
    }

    fn abort_running_search(&mut self, running: &mut Option<RunningSearch>, emit_bestmove: bool) {
        if let Some(mut task) = running.take() {
            task.cancel_token.store(true, Ordering::SeqCst);
            task.stop_token.store(true, Ordering::SeqCst);
            task.emit_bestmove |= emit_bestmove;
            let emit_bestmove = task.emit_bestmove;
            let (best_move, ponder_move) = self.finish_search_task(task);
            if emit_bestmove {
                uci_println(&format_bestmove(best_move, ponder_move));
            }
        }
    }

    fn finish_search_task(&mut self, task: RunningSearch) -> (Move, Option<Move>) {
        let root_board = task.root_board;
        let fallback_move = task.fallback_move;
        match task.handle.join() {
            Ok((engine, best_move, ponder_move)) => {
                self.engine = Some(engine);
                sanitize_search_output(&root_board, best_move, ponder_move, fallback_move)
            }
            Err(_) => {
                uci_println("info string Search worker failed; returning a legal fallback move");
                self.engine = Some(self.build_search_engine());
                sanitize_search_output(&root_board, NULL_MOVE, None, fallback_move)
            }
        }
    }

    fn build_search_engine(&self) -> SearchEngine {
        let mut engine = SearchEngine::new(self.hash_mb, self.num_threads);
        engine.set_nnue_network_source(Arc::clone(&self.eval_network));
        engine.set_params_for_preset(self.active_preset_name());
        engine.set_search_experiment(self.search_experiment);
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
                seldepth: 0,
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

    fn search_root_candidates_on(
        engine: &mut SearchEngine,
        board: &mut Board,
        request: RootCandidateSearch<'_>,
    ) -> Option<Move> {
        if request.cancel_token.load(Ordering::SeqCst) {
            return None;
        }
        if request.candidates.is_empty() {
            return None;
        }

        let per_move_time = request.time_limit.map(|t| {
            t.div_f64(request.candidates.len() as f64)
                .max(Duration::from_millis(10))
        });
        let per_move_nodes = request
            .node_limit
            .map(|n| (n / request.candidates.len() as u64).max(1));
        let child_depth = (request.depth - 1).max(1);

        let mut scored = Vec::with_capacity(request.candidates.len());
        let mut total_nodes = 0_u64;
        let mut total_elapsed = Duration::ZERO;
        let mut completed_depth = 0;
        let mut max_seldepth = 0;
        for &mv in request.candidates {
            if request.cancel_token.load(Ordering::SeqCst) {
                break;
            }
            board.make_move(mv);
            let child = Self::search_with_limits_on(
                engine,
                board,
                child_depth,
                per_move_time,
                per_move_nodes,
                request.cancel_token,
            );
            board.unmake_move(mv);
            if child.best_move == NULL_MOVE && request.cancel_token.load(Ordering::SeqCst) {
                break;
            }
            let score = -child.score;
            total_nodes = total_nodes.saturating_add(child.nodes);
            total_elapsed = total_elapsed.saturating_add(child.elapsed);
            completed_depth = completed_depth.max(child.depth.saturating_add(1));
            max_seldepth = max_seldepth.max(child.seldepth.saturating_add(1));
            scored.push(RootCandidate { mv, eval: score });
        }
        scored.sort_unstable_by_key(|candidate| std::cmp::Reverse(candidate.eval));
        scored.truncate(request.multi_pv.clamp(1, scored.len().max(1)));
        let selected = select_root_move(board, &scored, request.aesthetic);
        if let Some(selected_move) = selected {
            let selected_score = scored
                .iter()
                .find(|candidate| candidate.mv == selected_move)
                .map_or(0, |candidate| candidate.eval);
            let elapsed_ms = total_elapsed.as_millis().max(1);
            let nps = u128::from(total_nodes).saturating_mul(1_000) / elapsed_ms;
            uci_println(&format!(
                "info depth {completed_depth} seldepth {max_seldepth} score cp {selected_score} nodes {total_nodes} nps {nps} time {elapsed_ms} pv {}",
                selected_move.to_uci()
            ));
        }
        selected
    }

    fn start_search_task(
        &mut self,
        depth: i32,
        time_limit: Option<Duration>,
        node_limit: Option<u64>,
        restricted_moves: Vec<Move>,
        emit_bestmove: bool,
    ) -> RunningSearch {
        let fallback_move = self
            .board
            .generate_legal_moves()
            .as_slice()
            .first()
            .copied()
            .unwrap_or(NULL_MOVE);
        let root_board = self.board.clone();
        let mut board = root_board.clone();
        let mut engine = self
            .engine
            .take()
            .unwrap_or_else(|| self.build_search_engine());
        let stop_token = engine.stop_token();
        let cancel_token = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel_token);
        let multi_pv = self.multi_pv;
        let aesthetic = AestheticConfig {
            enabled: self.aesthetic_bias,
            max_delta_cp: self.aesthetic_delta_cp,
        };
        let worker_fallback = fallback_move;
        let handle = thread::spawn(move || {
            let (best_move, ponder_move) = if cancel_clone.load(Ordering::SeqCst) {
                (worker_fallback, None)
            } else if !restricted_moves.is_empty() {
                (
                    Self::search_root_candidates_on(
                        &mut engine,
                        &mut board,
                        RootCandidateSearch {
                            candidates: &restricted_moves,
                            multi_pv,
                            aesthetic,
                            depth,
                            time_limit,
                            node_limit,
                            cancel_token: cancel_clone.as_ref(),
                        },
                    )
                    .unwrap_or(worker_fallback),
                    None,
                )
            } else if multi_pv > 1 {
                let legal_moves = board.generate_legal_moves();
                (
                    Self::search_root_candidates_on(
                        &mut engine,
                        &mut board,
                        RootCandidateSearch {
                            candidates: legal_moves.as_slice(),
                            multi_pv,
                            aesthetic,
                            depth,
                            time_limit,
                            node_limit,
                            cancel_token: cancel_clone.as_ref(),
                        },
                    )
                    .unwrap_or(worker_fallback),
                    None,
                )
            } else {
                let result = Self::search_with_limits_on(
                    &mut engine,
                    &mut board,
                    depth,
                    time_limit,
                    node_limit,
                    cancel_clone.as_ref(),
                );
                let score = engine.format_uci_score(&board, result.score);
                uci_println(&format_final_search_info(&result, &score));
                (result.best_move, result.pv.get(1).copied())
            };
            (engine, best_move, ponder_move)
        });

        RunningSearch {
            handle,
            stop_token,
            cancel_token,
            emit_bestmove,
            root_board,
            fallback_move,
        }
    }

    /// Responds to `uci` with identification and option list.
    fn handle_uci(&self) {
        uci_println("id name Mujrim 2.0.0");
        uci_println("id author Ahmad Hamdi Emara (Egypt)");
        uci_println(&format!(
            "option name Hash type spin default {DEFAULT_HASH_MB} min 1 max {MAX_HASH_MB}"
        ));
        uci_println(&format!(
            "option name Threads type spin default {DEFAULT_THREADS} min 1 max 256"
        ));
        uci_println(&format!(
            "option name MoveOverhead type spin default {DEFAULT_MOVE_OVERHEAD_MS} min 0 max 5000"
        ));
        uci_println("option name OwnBook type check default true");
        uci_println("option name UseNNUE type check default true");
        uci_println(&format!(
            "option name EvalFile type string default {}",
            self.advertised_eval_file()
        ));
        uci_println(
            "option name EvalPreset type combo default auto var auto var akimbo var stockfish var reckless",
        );
        uci_println(&format!(
            "option name SearchExperiment type combo default none{}",
            SearchExperiment::UCI_NAMES
                .iter()
                .map(|name| format!(" var {name}"))
                .collect::<String>()
        ));
        uci_println("option name Ponder type check default false");
        uci_println("option name MultiPV type spin default 1 min 1 max 500");
        uci_println("option name AestheticBias type check default false");
        uci_println(&format!(
            "option name AestheticDeltaCP type spin default {MAX_AESTHETIC_DELTA_CP} min 0 max {MAX_AESTHETIC_DELTA_CP}"
        ));
        uci_println("option name Contempt type spin default 24 min -100 max 100");
        uci_println("option name SyzygyPath type string default <empty>");
        uci_println("option name UCI_AnalyseMode type check default false");
        uci_println("option name UCI_Chess960 type check default false");
        uci_println("option name Country type string default Egypt");
        uci_println(
            "option name UCI_EngineAbout type string default Mujrim Chess Engine, Cairo, Egypt",
        );
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
        if let Some(engine) = self.engine.as_mut() {
            engine.clear();
        }
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
                    self.hash_mb = clamp_hash_mb(mb);
                    self.reconfigure_engine(true);
                    eprintln!("info string Hash set to {} MB", self.hash_mb);
                }
            }
            "threads" => {
                if let Ok(t) = value.parse::<usize>() {
                    self.num_threads = t.clamp(1, 256);
                    self.reconfigure_engine(false);
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
                if let Some(engine) = self.engine.as_mut() {
                    engine.set_use_nnue(self.use_nnue);
                }
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
                if matches!(
                    preset.as_str(),
                    "auto" | "akimbo" | "stockfish" | "reckless"
                ) {
                    if preset != "auto"
                        && let Err(error) = self.set_eval_file(&format!("embedded:{preset}"))
                    {
                        eprintln!("info string EvalPreset error: {error}");
                        return;
                    }
                    self.eval_preset = preset;
                    self.reconfigure_engine(false);
                    let active = self.active_preset_name();
                    eprintln!(
                        "info string EvalPreset set to {} (active: {active})",
                        self.eval_preset
                    );
                } else {
                    eprintln!("info string EvalPreset error: invalid preset '{value}'");
                }
            }
            "searchexperiment" => {
                let name = value.to_lowercase();
                if let Some(experiment) = SearchExperiment::from_name(&name) {
                    self.search_experiment = experiment;
                    self.reconfigure_engine(false);
                    eprintln!(
                        "info string SearchExperiment set to {}",
                        self.search_experiment.as_str()
                    );
                } else {
                    eprintln!("info string SearchExperiment error: invalid experiment '{value}'");
                }
            }
            "uci_analysemode" => {
                self.analyse_mode = value.eq_ignore_ascii_case("true");
                if self.debug_mode {
                    eprintln!("info string UCI_AnalyseMode set to {}", self.analyse_mode);
                }
            }
            // UCI standard options we accept but don't actively use
            "ponder" | "uci_chess960" | "country" | "uci_engineabout" => {
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
            "aestheticbias" => {
                self.aesthetic_bias = value.eq_ignore_ascii_case("true");
                if self.debug_mode {
                    eprintln!("info string AestheticBias set to {}", self.aesthetic_bias);
                }
            }
            "aestheticdeltacp" => {
                if let Ok(delta) = value.parse::<i32>() {
                    self.aesthetic_delta_cp = delta.clamp(0, MAX_AESTHETIC_DELTA_CP);
                    if self.debug_mode {
                        eprintln!(
                            "info string AestheticDeltaCP set to {}",
                            self.aesthetic_delta_cp
                        );
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

        let (mut next_board, move_start_idx) = if args[0] == "startpos" {
            let move_start_idx = if args.get(1) == Some(&"moves") {
                2
            } else {
                args.len()
            };
            (Board::new(), move_start_idx)
        } else if args[0] == "fen" {
            let mut fen_parts = Vec::new();
            let mut i = 1;
            while i < args.len() && args[i] != "moves" {
                fen_parts.push(args[i]);
                i += 1;
            }
            let fen = fen_parts.join(" ");
            let board = match Board::from_fen(&fen) {
                Ok(board) => board,
                Err(e) => {
                    eprintln!("info string Invalid FEN: {e}");
                    return;
                }
            };
            let move_start_idx = if i < args.len() && args[i] == "moves" {
                i + 1
            } else {
                args.len()
            };
            (board, move_start_idx)
        } else {
            eprintln!("info string Invalid position command");
            return;
        };

        for &move_str in &args[move_start_idx..] {
            let legal_moves = next_board.generate_legal_moves();
            let Some(mv) = legal_moves
                .iter()
                .copied()
                .find(|mv| mv.to_uci() == move_str)
            else {
                eprintln!("info string Invalid move: {move_str}");
                return;
            };
            next_board.make_move(mv);
        }

        self.board = next_board;
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
        #[cfg(feature = "book")]
        let mut node_limit = go.nodes;
        #[cfg(not(feature = "book"))]
        let node_limit = go.nodes;

        let mut restricted_moves = Vec::new();
        for move_str in &go.searchmoves {
            if let Some(mv) = self.parse_uci_move(move_str)
                && !restricted_moves.iter().any(|m: &Move| {
                    m.from == mv.from && m.to == mv.to && m.promotion == mv.promotion
                })
            {
                restricted_moves.push(mv);
            }
        }

        #[cfg(feature = "book")]
        if self.use_book
            && !self.analyse_mode
            && !go.has_explicit_search_rules()
            && restricted_moves.is_empty()
            && let Some(book_move) = self.book.as_ref().and_then(|book| book.probe(&self.board))
            && let Some(legal_book_move) = self.parse_uci_move(&book_move.to_uci())
        {
            uci_println(&format!(
                "info string Embedded book candidate {}; validating with search",
                legal_book_move.to_uci()
            ));
            restricted_moves.push(legal_book_move);
            depth = depth.min(BOOK_VALIDATION_DEPTH);
            node_limit = Some(BOOK_VALIDATION_NODES);
        }

        if !go.searchmoves.is_empty() && restricted_moves.is_empty() {
            uci_println("bestmove 0000");
            return;
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
            "reckless" => "reckless",
            _ => self.eval_network.search_profile().as_str(),
        }
    }

    fn advertised_eval_file(&self) -> String {
        format!("embedded:{}", self.eval_network.search_profile().as_str())
    }

    fn reconfigure_engine(&mut self, resize_hash: bool) {
        let preset = self.active_preset_name();
        if let Some(engine) = self.engine.as_mut() {
            if resize_hash {
                engine.resize_tt(self.hash_mb);
            }
            engine.num_threads = self.num_threads;
            engine.set_nnue_network_source(Arc::clone(&self.eval_network));
            engine.set_params_for_preset(preset);
            engine.set_search_experiment(self.search_experiment);
            engine.set_use_nnue(self.use_nnue);
            return;
        }

        let mut engine = SearchEngine::new(self.hash_mb, self.num_threads);
        engine.set_nnue_network_source(Arc::clone(&self.eval_network));
        engine.set_params_for_preset(preset);
        engine.set_search_experiment(self.search_experiment);
        engine.set_use_nnue(self.use_nnue);
        self.engine = Some(engine);
    }

    fn set_eval_file(&mut self, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "<empty>" {
            self.install_eval_network(eval::nnue::default_embedded_network(), None);
            return Ok(());
        }

        if trimmed.eq_ignore_ascii_case("embedded:akimbo") {
            self.install_eval_network(ActiveNetwork::Embedded, Some("embedded:akimbo".to_string()));
            return Ok(());
        }

        if trimmed.eq_ignore_ascii_case("embedded:reckless") {
            #[cfg(feature = "reckless-nnue")]
            {
                self.install_eval_network(
                    ActiveNetwork::EmbeddedReckless,
                    Some("embedded:reckless".to_string()),
                );
                return Ok(());
            }
            #[cfg(not(feature = "reckless-nnue"))]
            return Err("Reckless NNUE support is not compiled into this binary".to_string());
        }

        #[cfg(feature = "stockfish-nnue")]
        if trimmed.eq_ignore_ascii_case("embedded:stockfish") {
            self.install_eval_network(
                ActiveNetwork::EmbeddedStockfish,
                Some("embedded:stockfish".to_string()),
            );
            return Ok(());
        }

        let loaded = load_network(Path::new(trimmed))?;
        self.install_eval_network(loaded, Some(trimmed.to_string()));
        Ok(())
    }

    fn install_eval_network(&mut self, network: ActiveNetwork, source: Option<String>) {
        self.eval_file = source;
        self.eval_network = Arc::new(network);
        self.eval_preset = "auto".to_string();
        self.reconfigure_engine(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "stockfish-nnue")]
    #[test]
    fn uci_starts_with_one_search_thread() {
        let handler = UciHandler::new();
        assert_eq!(handler.num_threads, DEFAULT_THREADS);
        assert_eq!(handler.engine.as_ref().unwrap().num_threads, 1);
    }

    #[test]
    fn hash_limit_matches_the_advertised_safe_range() {
        assert_eq!(clamp_hash_mb(0), 1);
        assert_eq!(clamp_hash_mb(DEFAULT_HASH_MB), DEFAULT_HASH_MB);
        assert_eq!(clamp_hash_mb(usize::MAX), MAX_HASH_MB);
    }

    #[test]
    fn test_uci_handler_creation() {
        let handler = UciHandler::new();
        assert_eq!(handler.hash_mb, DEFAULT_HASH_MB);
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::default_embedded_network().info().format
        );
        assert_eq!(
            handler.board.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[cfg(feature = "reckless-nnue")]
    #[test]
    fn uci_advertises_the_actual_default_network() {
        let handler = UciHandler::new();
        assert_eq!(handler.advertised_eval_file(), "embedded:reckless");
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
        handler.handle_position(&["startpos", "moves", "e2e4"]);
        let before = handler.board.to_fen();
        handler.handle_position(&["fen", "not", "a", "valid", "fen", "at", "all"]);
        assert_eq!(handler.board.to_fen(), before);
        // Should not crash — board should remain valid
    }

    #[test]
    fn invalid_position_move_rolls_back_the_whole_command() {
        let mut handler = UciHandler::new();
        handler.handle_position(&["startpos", "moves", "d2d4"]);
        let before = handler.board.to_fen();
        handler.handle_position(&["startpos", "moves", "e2e4", "e7e5", "z9z9"]);
        assert_eq!(handler.board.to_fen(), before);
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

    #[cfg(feature = "book")]
    #[test]
    fn embedded_book_is_only_eligible_without_controller_search_rules() {
        assert!(!GoCommand::default().has_explicit_search_rules());
        assert!(
            GoCommand {
                nodes: Some(10_000),
                ..GoCommand::default()
            }
            .has_explicit_search_rules()
        );
        assert!(
            GoCommand {
                searchmoves: vec!["e2e4".to_string()],
                ..GoCommand::default()
            }
            .has_explicit_search_rules()
        );
        assert!(
            GoCommand {
                wtime: Some(60_000),
                ..GoCommand::default()
            }
            .has_explicit_search_rules()
        );
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
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Stockfish
        );
        assert_eq!(handler.engine.as_ref().unwrap().params().nmp_base, 7);
    }

    #[test]
    fn evalpreset_selects_a_compatible_eval_and_search_stack() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalPreset", "value", "reckless"]);
        assert_eq!(handler.eval_file.as_deref(), Some("embedded:reckless"));
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Reckless
        );
        assert_eq!(handler.active_preset_name(), "reckless");

        handler.handle_setoption(&["name", "EvalFile", "value", "embedded:stockfish"]);
        assert_eq!(handler.eval_preset, "auto");
        assert_eq!(handler.active_preset_name(), "stockfish");
    }

    #[test]
    fn search_experiment_preserves_the_active_network_parameters() {
        let mut handler = UciHandler::new();
        let nmp_base = handler.engine.as_ref().unwrap().params().nmp_base;

        handler.handle_setoption(&["name", "SearchExperiment", "value", "reckless-lmp"]);

        assert_eq!(handler.search_experiment, SearchExperiment::RecklessLmp);
        assert_eq!(handler.engine.as_ref().unwrap().params().nmp_base, nmp_base);
        assert_eq!(handler.active_preset_name(), "reckless");
    }

    #[test]
    fn uci_analyse_mode_disables_embedded_book_eligibility() {
        let mut handler = UciHandler::new();
        assert!(!handler.analyse_mode);

        handler.handle_setoption(&["name", "UCI_AnalyseMode", "value", "true"]);
        assert!(handler.analyse_mode);

        handler.handle_setoption(&["name", "UCI_AnalyseMode", "value", "false"]);
        assert!(!handler.analyse_mode);
    }

    #[test]
    fn test_setoption_evalfile_invalid_keeps_current_network() {
        let mut handler = UciHandler::new();
        let before = handler.eval_network.info().name;
        handler.handle_setoption(&[
            "name",
            "EvalFile",
            "value",
            "/nonexistent/mujrim/ak_default.bin",
        ]);
        assert_eq!(handler.eval_network.info().name, before);
        assert!(handler.eval_file.is_none());
    }

    #[test]
    fn test_setoption_selects_embedded_threat_network_without_disk_io() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalFile", "value", "embedded:reckless"]);
        assert_eq!(handler.eval_file.as_deref(), Some("embedded:reckless"));
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Reckless
        );
        assert_eq!(handler.active_preset_name(), "reckless");
    }

    #[test]
    fn test_setoption_selects_native_embedded_network_without_disk_io() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalFile", "value", "embedded:akimbo"]);
        assert_eq!(handler.eval_file.as_deref(), Some("embedded:akimbo"));
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Embedded
        );
        assert_eq!(handler.active_preset_name(), "akimbo");
    }

    #[test]
    fn eval_file_selects_embedded_current_stockfish_adapter() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "EvalFile", "value", "embedded:stockfish"]);
        assert_eq!(handler.eval_file.as_deref(), Some("embedded:stockfish"));
        assert_eq!(
            handler.eval_network.info().format,
            eval::nnue::NetworkFormat::Stockfish
        );
        assert_eq!(handler.active_preset_name(), "stockfish");
    }

    #[test]
    fn test_setoption_evalfile_loads_embedded_compatible_bin_when_available() {
        let mut handler = UciHandler::new();
        if !enabled_network_formats().contains(&eval::nnue::NetworkFormat::Akimbo) {
            return;
        }
        let net_path = format!(
            "{}/../mujrim-eval/resources/ak_default.bin",
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
        let previous_tt = Arc::downgrade(&handler.engine.as_ref().unwrap().tt);
        handler.handle_setoption(&["name", "EvalPreset", "value", "stockfish"]);
        assert!(previous_tt.upgrade().is_some());

        let previous_tt = Arc::downgrade(&handler.engine.as_ref().unwrap().tt);
        handler.handle_setoption(&["name", "Hash", "value", "256"]);
        assert!(previous_tt.upgrade().is_none());

        let current_tt = Arc::clone(&handler.engine.as_ref().unwrap().tt);
        handler.handle_setoption(&["name", "Threads", "value", "2"]);
        assert!(Arc::ptr_eq(
            &current_tt,
            &handler.engine.as_ref().unwrap().tt
        ));
        assert_eq!(handler.eval_preset, "stockfish");
        assert_eq!(handler.engine.as_ref().unwrap().params().nmp_base, 7);
    }

    #[test]
    fn test_setoption_usennue_toggles_engine_eval_mode() {
        let mut handler = UciHandler::new();
        handler.handle_setoption(&["name", "UseNNUE", "value", "false"]);
        assert!(!handler.engine.as_ref().unwrap().use_nnue());
        handler.handle_setoption(&["name", "UseNNUE", "value", "true"]);
        assert!(handler.engine.as_ref().unwrap().use_nnue());
    }

    #[test]
    fn aesthetic_options_are_strength_safe_by_default_and_clamped() {
        let mut handler = UciHandler::new();
        assert!(!handler.aesthetic_bias);
        assert_eq!(handler.aesthetic_delta_cp, MAX_AESTHETIC_DELTA_CP);

        handler.handle_setoption(&["name", "AestheticBias", "value", "true"]);
        handler.handle_setoption(&["name", "AestheticDeltaCP", "value", "300"]);
        assert!(handler.aesthetic_bias);
        assert_eq!(handler.aesthetic_delta_cp, MAX_AESTHETIC_DELTA_CP);

        handler.handle_setoption(&["name", "AestheticDeltaCP", "value", "-4"]);
        assert_eq!(handler.aesthetic_delta_cp, 0);
    }

    #[test]
    fn root_multi_pv_pipeline_returns_a_legal_candidate() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        handler.multi_pv = 3;
        let legal = handler.board.generate_legal_moves();
        let mut engine = handler.engine.take().unwrap();
        let cancel = AtomicBool::new(false);
        let selected = UciHandler::search_root_candidates_on(
            &mut engine,
            &mut handler.board,
            RootCandidateSearch {
                candidates: legal.as_slice(),
                multi_pv: handler.multi_pv,
                aesthetic: AestheticConfig::default(),
                depth: 1,
                time_limit: None,
                node_limit: Some(2_000),
                cancel_token: &cancel,
            },
        )
        .expect("a legal root candidate");
        assert!(legal.iter().any(|candidate| *candidate == selected));
    }

    #[test]
    fn one_restricted_root_move_is_still_evaluated() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let legal = handler.board.generate_legal_moves();
        let candidate = legal[0];
        let mut engine = handler.engine.take().unwrap();
        let cancel = AtomicBool::new(false);

        let selected = UciHandler::search_root_candidates_on(
            &mut engine,
            &mut handler.board,
            RootCandidateSearch {
                candidates: std::slice::from_ref(&candidate),
                multi_pv: 1,
                aesthetic: AestheticConfig::default(),
                depth: 4,
                time_limit: None,
                node_limit: Some(2_000),
                cancel_token: &cancel,
            },
        );

        assert_eq!(selected, Some(candidate));
        handler.board.make_move(candidate);
        assert!(
            engine.tt.probe(handler.board.tt_hash()).is_some(),
            "restricted move was not searched"
        );
        handler.board.unmake_move(candidate);
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
    fn completed_search_restores_the_same_engine() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let tt = Arc::as_ptr(&handler.engine.as_ref().unwrap().tt);
        let mut running = Some(handler.start_search_task(1, None, None, Vec::new(), false));

        handler.abort_running_search(&mut running, false);

        assert!(running.is_none());
        assert_eq!(Arc::as_ptr(&handler.engine.as_ref().unwrap().tt), tt);
    }

    #[test]
    fn failed_search_worker_returns_a_legal_root_fallback() {
        let mut handler = UciHandler::new();
        let fallback_move = handler.board.generate_legal_moves()[0];
        let handle: JoinHandle<(SearchEngine, Move, Option<Move>)> =
            thread::spawn(|| panic!("deliberate worker failure"));
        let task = RunningSearch {
            handle,
            stop_token: Arc::new(AtomicBool::new(false)),
            cancel_token: Arc::new(AtomicBool::new(false)),
            emit_bestmove: true,
            root_board: handler.board.clone(),
            fallback_move,
        };

        let (best_move, ponder_move) = handler.finish_search_task(task);

        assert_eq!(best_move, fallback_move);
        assert_eq!(ponder_move, None);
    }

    #[test]
    fn final_output_rejects_illegal_worker_moves_and_ponder() {
        let mut board = Board::new();
        let fallback = board.generate_legal_moves()[0];
        let illegal = Move::quiet(types::Square::A1, types::Square::A1);

        let (best_move, ponder_move) =
            sanitize_search_output(&board, illegal, Some(illegal), fallback);

        assert_eq!(best_move, fallback);
        assert_eq!(ponder_move, None);
    }

    #[test]
    fn test_handle_go_with_illegal_searchmoves_does_not_start_task() {
        let mut handler = UciHandler::new();
        let mut running = None;
        handler.handle_go(&["searchmoves", "a1a1", "h1h1"], &mut running);
        assert!(running.is_none());
    }

    #[test]
    fn completed_ponder_search_waits_for_ponderhit() {
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
        assert!(running.is_some());
        let task = running.as_mut().unwrap();
        task.emit_bestmove = true;
        task.stop_token.store(true, Ordering::Relaxed);
        handler.poll_running_search(&mut running);
        assert!(running.is_none());
    }

    #[test]
    fn bestmove_reports_predicted_reply_for_arena_pondering() {
        let best = Move::quiet(types::Square::E2, types::Square::E4);
        let ponder = Move::quiet(types::Square::E7, types::Square::E5);
        assert_eq!(
            format_bestmove(best, Some(ponder)),
            "bestmove e2e4 ponder e7e5"
        );
        assert_eq!(format_bestmove(best, None), "bestmove e2e4");
        assert_eq!(format_bestmove(NULL_MOVE, Some(ponder)), "bestmove 0000");
    }

    #[test]
    fn final_search_info_reports_exact_telemetry() {
        let result = SearchResult {
            best_move: Move::quiet(types::Square::E2, types::Square::E4),
            score: 0,
            depth: 14,
            seldepth: 23,
            nodes: 250_000,
            elapsed: Duration::from_millis(500),
            pv: vec![
                Move::quiet(types::Square::E2, types::Square::E4),
                Move::quiet(types::Square::E7, types::Square::E5),
            ],
        };
        assert_eq!(
            format_final_search_info(&result, "cp 31"),
            "info depth 14 seldepth 23 score cp 31 nodes 250000 nps 500000 time 500 pv e2e4 e7e5"
        );
    }

    #[test]
    fn test_abort_running_search_with_emit_flag_clears_task() {
        let mut handler = UciHandler::new();
        handler.use_book = false;
        let mut running = Some(handler.start_search_task(32, None, None, Vec::new(), false));
        handler.abort_running_search(&mut running, true);
        assert!(running.is_none());
    }

    #[test]
    fn test_normalize_command_accepts_utf8_bom() {
        assert_eq!(normalize_command("\u{feff}uci\r\n"), "uci");
        assert_eq!(normalize_command("  isready  "), "isready");
    }
}
