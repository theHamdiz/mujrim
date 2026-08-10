//! External engine process helpers for GUI play.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mujrim_protocols::{
    EngineOptions, EngineSearchState, EngineSession, ProtocolKind, SearchInfo, SearchRequest,
};

const EXTERNAL_ENGINE_MEMORY_OVERHEAD_MB: usize = 192;
/// Cap is intentionally high: large NNUE nets + hash must not OOM-kill engines
/// mid-game the way a 768 MiB ceiling did for mujrim / Stockfish stacks.
const MAX_EXTERNAL_ENGINE_MEMORY_MB: usize = 4096;
const MAX_CACHED_ENGINE_SESSIONS: usize = 2;

static ENGINE_POOL: OnceLock<Mutex<Vec<CachedExternalEngine>>> = OnceLock::new();
static CANCEL_EPOCH: AtomicU64 = AtomicU64::new(0);

fn external_memory_limit_mb(hash_mb: usize) -> usize {
    hash_mb
        .saturating_add(EXTERNAL_ENGINE_MEMORY_OVERHEAD_MB)
        .min(MAX_EXTERNAL_ENGINE_MEMORY_MB)
}

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
    pub ponder_move: Option<String>,
    pub depth: i32,
    pub seldepth: i32,
    pub score: i32,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub hashfull: u16,
    pub tablebase_hits: u64,
    pub current_move: Option<String>,
    pub pv: Vec<String>,
    pub ponder_hit: bool,
}

impl ExternalMoveResult {
    pub fn telemetry(&self) -> String {
        let ponder = self
            .ponder_move
            .as_deref()
            .map_or_else(String::new, |mv| format!(" | ponder {mv}"));
        let current = self
            .current_move
            .as_deref()
            .map_or_else(String::new, |mv| format!(" | current {mv}"));
        let pv = if self.pv.is_empty() {
            String::new()
        } else {
            format!(" | pv {}", self.pv.join(" "))
        };
        let ponder_hit = if self.ponder_hit { " | ponderhit" } else { "" };
        format!(
            "depth {}/{} | score {} cp | {} nodes | {} nps | {} ms | hashfull {}/1000 | tbhits {}{ponder_hit}{ponder}{current}{pv}",
            self.depth,
            self.seldepth,
            self.score,
            self.nodes,
            self.nps,
            self.time_ms,
            self.hashfull,
            self.tablebase_hits,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExternalSearchConfig {
    pub ponder: bool,
    pub use_nnue: bool,
    pub own_book: bool,
    pub eval_file: Option<String>,
}

pub fn query_best_move(
    engine_path: &str,
    protocol: ExternalEngineProtocol,
    fen: &str,
    depth: i32,
    movetime: Duration,
    hash_mb: usize,
    threads: usize,
    search: &ExternalSearchConfig,
) -> Result<ExternalMoveResult, String> {
    let cancel_epoch = CANCEL_EPOCH.load(Ordering::Acquire);
    let key = EngineSessionKey {
        path: engine_path.to_owned(),
        protocol,
        hash_mb,
        threads,
        use_nnue: search.use_nnue,
        own_book: search.own_book,
        eval_file: search.eval_file.clone(),
    };
    let mut pool = engine_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let index = match pool.iter().position(|entry| entry.key == key) {
        Some(index) => index,
        None => {
            if pool.len() == MAX_CACHED_ENGINE_SESSIONS {
                pool.remove(0);
            }
            pool.push(CachedExternalEngine::spawn(key)?);
            pool.len() - 1
        }
    };
    let request = SearchRequest {
        fen: fen.to_string(),
        moves: Vec::new(),
        depth,
        movetime: Some(movetime),
        node_limit: None,
    };
    let result = run_cached_search(&mut pool[index], &request, search.ponder, cancel_epoch);
    if result.is_err() {
        pool.remove(index);
    }
    let (info, ponder_hit) = result?;

    Ok(ExternalMoveResult {
        best_move: info.best_move,
        ponder_move: info.ponder_move,
        depth: info.depth,
        seldepth: info.seldepth,
        score: info.score,
        nodes: info.nodes,
        nps: info.nps,
        time_ms: info.time_ms,
        hashfull: info.hashfull,
        tablebase_hits: info.tablebase_hits,
        current_move: info.current_move,
        pv: info.pv,
        ponder_hit,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EngineSessionKey {
    path: String,
    protocol: ExternalEngineProtocol,
    hash_mb: usize,
    threads: usize,
    use_nnue: bool,
    own_book: bool,
    eval_file: Option<String>,
}

struct CachedExternalEngine {
    key: EngineSessionKey,
    session: EngineSession,
    predicted_fen: Option<String>,
    ponder_enabled: bool,
}

impl CachedExternalEngine {
    fn spawn(key: EngineSessionKey) -> Result<Self, String> {
        let memory_limit_mb = external_memory_limit_mb(key.hash_mb);
        let session = EngineSession::spawn_with_args_and_memory_limit(
            Path::new(&key.path),
            &[],
            key.protocol.as_protocol_kind(),
            Some((memory_limit_mb as u64).saturating_mul(1024 * 1024)),
        )?;
        let mut engine = Self {
            key,
            session,
            predicted_fen: None,
            ponder_enabled: false,
        };
        engine.configure(false)?;
        engine.session.new_game()?;
        Ok(engine)
    }

    fn configure(&mut self, ponder: bool) -> Result<(), String> {
        let mut custom = if self.key.protocol == ExternalEngineProtocol::Uci {
            uci_resource_options(
                Path::new(&self.key.path),
                ponder,
                self.key.use_nnue,
                self.key.eval_file.as_deref(),
            )
        } else {
            Vec::new()
        };
        // Prefer explicit GUI EvalFile over any auto-detected network.
        if self.key.protocol == ExternalEngineProtocol::Uci
            && let Some(eval_file) = self.key.eval_file.as_ref()
        {
            custom.retain(|(name, _)| !name.eq_ignore_ascii_case("EvalFile"));
            custom.push(("EvalFile".to_owned(), eval_file.clone()));
        }
        self.session.configure(&EngineOptions {
            hash_mb: Some(self.key.hash_mb),
            threads: Some(self.key.threads),
            own_book: Some(self.key.own_book),
            custom,
        })?;
        self.ponder_enabled = ponder;
        Ok(())
    }

    fn cancel_active_search(&mut self) -> Result<(), String> {
        if self.session.search_state() != EngineSearchState::Idle {
            let _ = self.session.stop_search()?;
        }
        self.predicted_fen = None;
        Ok(())
    }
}

pub fn uci_resource_options(
    engine: &Path,
    ponder: bool,
    use_nnue: bool,
    eval_file: Option<&str>,
) -> Vec<(String, String)> {
    const STOCKFISH_SHA256: &str =
        "ab28990d4ea3d5c97f7d3918bc5dd5061609330369fe00c2d93a34d4777b5552";

    let mut options = vec![
        ("Ponder".to_owned(), ponder.to_string()),
        ("UseNNUE".to_owned(), use_nnue.to_string()),
    ];
    let file_name = engine
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let overhead_name = if file_name.contains("v60") || file_name.contains("reckless") {
        "MoveOverhead"
    } else if file_name.contains("stockfish") {
        "Move Overhead"
    } else {
        // Mujrim and most UCI engines use the compact spelling.
        "MoveOverhead"
    };
    options.push((overhead_name.to_owned(), "150".to_owned()));

    if let Some(directory) = resource_directories(engine, "syzygy")
        .into_iter()
        .find(|directory| updater::syzygy::check_installed(directory).0 > 0)
    {
        options.push((
            "SyzygyPath".to_owned(),
            directory.to_string_lossy().into_owned(),
        ));
    }

    // Only auto-inject Stockfish EvalFile for actual Stockfish binaries.
    // Forcing it onto mujrim previously overrode the embedded Reckless net and
    // made GUI play diverge from CuteChess (which leaves EvalFile alone).
    let is_stockfish = file_name.contains("stockfish");
    static STOCKFISH_NETWORK: OnceLock<Option<PathBuf>> = OnceLock::new();
    if eval_file.is_none()
        && is_stockfish
        && let Some(network) = STOCKFISH_NETWORK
            .get_or_init(|| {
                resource_directories(engine, "nnue")
                    .into_iter()
                    .find_map(|directory| {
                        updater::nnue::find_by_fingerprint(&directory, STOCKFISH_SHA256)
                    })
            })
            .as_ref()
    {
        options.push((
            "EvalFile".to_owned(),
            network.to_string_lossy().into_owned(),
        ));
    } else if let Some(path) = eval_file {
        options.push(("EvalFile".to_owned(), path.to_owned()));
    }
    options
}

fn resource_directories(engine: &Path, name: &str) -> Vec<std::path::PathBuf> {
    let mut directories = Vec::new();
    if let Some(parent) = engine.parent() {
        directories.push(parent.join(name));
        for ancestor in parent.ancestors().take(7) {
            directories.push(ancestor.join(name));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        directories.push(current.join(name));
        directories.push(current.join("dist").join(name));
    }
    directories.sort();
    directories.dedup();
    directories
}

fn engine_pool() -> &'static Mutex<Vec<CachedExternalEngine>> {
    ENGINE_POOL.get_or_init(|| Mutex::new(Vec::with_capacity(MAX_CACHED_ENGINE_SESSIONS)))
}

fn run_cached_search(
    engine: &mut CachedExternalEngine,
    request: &SearchRequest,
    ponder: bool,
    cancel_epoch: u64,
) -> Result<(SearchInfo, bool), String> {
    if engine.ponder_enabled != ponder {
        engine.cancel_active_search()?;
        engine.configure(ponder)?;
    }

    let ponder_hit = engine.session.search_state() == EngineSearchState::Pondering
        && engine.predicted_fen.as_deref() == Some(request.fen.as_str())
        && ponder;
    let info = if ponder_hit {
        engine.session.ponder_hit()?;
        engine.session.wait_for_bestmove()?
    } else {
        engine.cancel_active_search()?;
        engine.session.search(request)?
    };
    engine.predicted_fen = None;

    if ponder
        && CANCEL_EPOCH.load(Ordering::Acquire) == cancel_epoch
        && engine.key.protocol == ExternalEngineProtocol::Uci
        && let Some(predicted_fen) =
            predicted_position_fen(&request.fen, &info.best_move, info.ponder_move.as_deref())
    {
        let ponder_request = SearchRequest {
            fen: predicted_fen.clone(),
            moves: Vec::new(),
            depth: request.depth,
            movetime: None,
            node_limit: None,
        };
        if engine.session.start_ponder(&ponder_request).is_ok() {
            engine.predicted_fen = Some(predicted_fen);
        }
    }

    Ok((info, ponder_hit))
}

fn predicted_position_fen(fen: &str, best_move: &str, ponder_move: Option<&str>) -> Option<String> {
    let mut board = types::Board::from_fen(fen).ok()?;
    let best = board
        .generate_legal_moves()
        .iter()
        .find(|candidate| candidate.to_uci() == best_move)
        .copied()?;
    board.make_move(best);
    let ponder = ponder_move.and_then(|ponder_move| {
        board
            .generate_legal_moves()
            .iter()
            .find(|candidate| candidate.to_uci() == ponder_move)
            .copied()
    })?;
    board.make_move(ponder);
    Some(board.to_fen())
}

pub fn cancel_all_pondering() {
    CANCEL_EPOCH.fetch_add(1, Ordering::AcqRel);
    let Some(pool) = ENGINE_POOL.get() else {
        return;
    };
    // Never freeze the GUI behind a foreground engine query. The epoch above
    // prevents that query from starting a new ponder when it finishes; an
    // already-idle pool is stopped and drained immediately.
    if let Ok(mut pool) = pool.try_lock() {
        pool.retain_mut(|engine| engine.cancel_active_search().is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_display() {
        assert_eq!(ExternalEngineProtocol::Uci.to_string(), "UCI");
        assert_eq!(ExternalEngineProtocol::Xboard.to_string(), "XBoard");
    }

    #[test]
    fn external_engine_memory_includes_bounded_overhead() {
        assert_eq!(external_memory_limit_mb(64), 256);
        assert_eq!(external_memory_limit_mb(512), 704);
        assert_eq!(external_memory_limit_mb(4096), 4096);
        assert_eq!(external_memory_limit_mb(16_384), 4096);
    }

    #[test]
    fn telemetry_includes_ponder_hash_and_pv() {
        let info = ExternalMoveResult {
            best_move: "e2e4".to_owned(),
            ponder_move: Some("e7e5".to_owned()),
            depth: 15,
            seldepth: 18,
            score: 13,
            nodes: 129_000,
            nps: 881_000,
            time_ms: 147,
            hashfull: 10,
            tablebase_hits: 0,
            current_move: Some("b2b4".to_owned()),
            pv: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            ponder_hit: true,
        };
        let label = info.telemetry();
        assert!(label.contains("ponder e7e5"));
        assert!(label.contains("hashfull 10/1000"));
        assert!(label.contains("pv e2e4 e7e5"));
        assert!(label.contains("ponderhit"));
    }

    #[test]
    fn predicted_ponder_position_requires_two_legal_moves() {
        types::init();
        let predicted = predicted_position_fen(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "e2e4",
            Some("e7e5"),
        )
        .expect("legal predicted position");
        assert!(predicted.contains(" b ") || predicted.contains(" w "));
        assert!(
            predicted_position_fen(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                "e2e5",
                Some("e7e5"),
            )
            .is_none()
        );
    }

    #[test]
    fn mujrim_resource_options_do_not_force_stockfish_evalfile() {
        let options = uci_resource_options(Path::new("C:/engines/mujrim.exe"), false, true, None);
        assert!(
            !options
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("EvalFile")),
            "mujrim must keep its embedded network unless the user sets EvalFile: {options:?}"
        );
        assert!(options.iter().any(|(n, v)| n == "UseNNUE" && v == "true"));
        assert!(options.iter().any(|(n, v)| n == "MoveOverhead" && v == "150"));
    }

    #[test]
    fn explicit_evalfile_is_forwarded_for_any_engine() {
        let options = uci_resource_options(
            Path::new("C:/engines/mujrim.exe"),
            true,
            false,
            Some(r"C:\nets\custom.nnue"),
        );
        assert!(options.iter().any(|(n, v)| n == "EvalFile" && v.ends_with("custom.nnue")));
        assert!(options.iter().any(|(n, v)| n == "UseNNUE" && v == "false"));
        assert!(options.iter().any(|(n, v)| n == "Ponder" && v == "true"));
    }

    #[test]
    fn stockfish_uses_spaced_move_overhead_option_name() {
        let options =
            uci_resource_options(Path::new("C:/engines/stockfish.exe"), false, true, None);
        assert!(
            options.iter().any(|(n, _)| n == "Move Overhead"),
            "{options:?}"
        );
    }

    #[test]
    fn engine_search_rejects_illegal_uci_without_fallback_move() {
        types::init();
        let mut board = types::Board::new();
        let legal = board.generate_legal_moves();
        assert!(
            legal.iter().all(|mv| mv.to_uci() != "e2e5"),
            "sanity: e2e5 must be illegal from startpos"
        );
        // The GUI must surface this as Err, never substitute legal.iter().next().
        let resolved = legal.iter().find(|m| m.to_uci() == "e2e5").copied();
        assert!(resolved.is_none());
    }
}
