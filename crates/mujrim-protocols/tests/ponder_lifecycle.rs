#![cfg(all(target_os = "windows", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use mujrim_protocols::{
    EngineOptions, EngineSearchState, EngineSession, ProtocolKind, SearchRequest,
};

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn bundled_stockfish() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("dist")
        .join("engines")
        .join("stockfish")
        .join("bin")
        .join("windows-aarch64")
        .join("stockfish.exe")
}

fn ponder_request(nodes: u64) -> SearchRequest {
    SearchRequest {
        fen: START_FEN.to_owned(),
        moves: Vec::new(),
        depth: 64,
        movetime: None,
        node_limit: Some(nodes),
    }
}

#[test]
fn bundled_engine_ponderhit_and_stop_are_drained_without_stale_bestmoves() {
    let engine = bundled_stockfish();
    if !engine.is_file() {
        eprintln!(
            "skipping bundled ponder lifecycle test: {}",
            engine.display()
        );
        return;
    }

    let mut session = EngineSession::spawn_with_args_and_memory_limit(
        &engine,
        &[],
        ProtocolKind::Uci,
        Some(384 * 1024 * 1024),
    )
    .expect("start bundled engine");
    session.set_read_timeout(Duration::from_secs(10));
    session
        .configure(&EngineOptions {
            hash_mb: Some(16),
            threads: Some(1),
            own_book: Some(false),
            custom: vec![("Ponder".to_owned(), "true".to_owned())],
        })
        .expect("configure pondering");
    session.new_game().expect("start first game");

    session
        .start_ponder(&ponder_request(20_000))
        .expect("start ponder search");
    assert_eq!(session.search_state(), EngineSearchState::Pondering);
    assert!(session.start_search(&ponder_request(1)).is_err());

    let poll_deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < poll_deadline {
        assert!(
            session.poll_search().expect("poll ponder search").is_none(),
            "a conforming UCI engine must wait for ponderhit or stop"
        );
        thread::sleep(Duration::from_millis(2));
    }

    session.ponder_hit().expect("accept predicted move");
    assert_eq!(session.search_state(), EngineSearchState::Searching);
    let hit_result = session
        .wait_for_bestmove()
        .expect("finish ponderhit search");
    assert!(!hit_result.best_move.is_empty());
    assert_ne!(hit_result.best_move, "0000");
    assert_eq!(session.search_state(), EngineSearchState::Idle);

    session.new_game().expect("start second game");
    session
        .start_ponder(&ponder_request(10_000_000))
        .expect("start cancellable ponder search");
    thread::sleep(Duration::from_millis(20));
    let stopped = session.stop_search().expect("stop and drain ponder search");
    assert!(!stopped.best_move.is_empty());
    assert_ne!(stopped.best_move, "0000");
    assert_eq!(session.search_state(), EngineSearchState::Idle);
    assert!(session.poll_search().is_err());
}
