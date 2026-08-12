use bevy::prelude::*;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use mujrim_protocols::{EngineOptions, EngineSession, ProtocolKind, SearchRequest};

use crate::state::{ChessGame, EngineConfig, TurnState};

/// Result from the engine search.
pub struct EngineResult {
    pub best_move: types::Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub engine_name: String,
}

/// Info about the engine's latest search, displayed in the HUD.
#[derive(Resource, Default, Clone)]
pub struct EngineInfo {
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub best_move_uci: String,
    pub engine_name: String,
}

/// Holds a channel receiver for the engine result.
/// Wrapped in `Mutex` because `mpsc::Receiver` is `!Sync`.
#[derive(Resource)]
pub struct EngineTask {
    rx: Mutex<mpsc::Receiver<EngineResult>>,
}

/// Initialize engine info on game start.
pub fn init_engine(mut commands: Commands) {
    commands.insert_resource(EngineInfo::default());
}

/// Start engine search on a dedicated OS thread.
pub fn start_engine_search(
    mut commands: Commands,
    game: Res<ChessGame>,
    config: Res<EngineConfig>,
    turn_state: Res<State<TurnState>>,
    existing_task: Option<Res<EngineTask>>,
) {
    if *turn_state.get() != TurnState::EngineTurn || existing_task.is_some() {
        return;
    }

    let board = game.board.clone();
    let depth = config.depth;
    let time_limit_ms = config.time_limit_ms;
    let external_engine = config.selected_bundled_engine().cloned();

    let (tx, rx) = mpsc::channel();

    // Dedicated OS thread — never blocks the UI.
    let spawn_result = std::thread::Builder::new()
        .name("mujrim-engine-search".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = external_engine
                .as_ref()
                .and_then(|engine| {
                    match search_external_engine(&board, engine, depth, time_limit_ms) {
                        Ok(result) => Some(result),
                        Err(error) => {
                            warn!(
                                "{} failed, using built-in engine: {error}",
                                engine.display_name
                            );
                            None
                        }
                    }
                })
                .unwrap_or_else(|| search_builtin_engine(board, depth));
            let _ = tx.send(result);
        });

    if let Err(error) = spawn_result {
        warn!("failed to start engine search thread: {error}");
        return;
    }

    commands.insert_resource(EngineTask { rx: Mutex::new(rx) });
}

/// Non-blocking poll: check the channel without stalling.
pub fn poll_engine_result(
    mut commands: Commands,
    mut game: ResMut<ChessGame>,
    task: Option<ResMut<EngineTask>>,
    mut turn_state: ResMut<NextState<TurnState>>,
    mut engine_info: ResMut<EngineInfo>,
    mut audio_messages: MessageWriter<crate::audio::SoundMessage>,
) {
    let Some(task) = task else { return };

    let result = {
        let rx = task
            .rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rx.try_recv()
    };

    match result {
        Ok(result) => {
            engine_info.depth = result.depth;
            engine_info.score = result.score;
            engine_info.nodes = result.nodes;
            engine_info.best_move_uci = result.best_move.to_uci();
            engine_info.engine_name = result.engine_name;

            let legal = game.board.generate_legal_moves();
            if let Some(mv) = legal
                .iter()
                .find(|m| {
                    m.from == result.best_move.from
                        && m.to == result.best_move.to
                        && m.promotion == result.best_move.promotion
                })
                .copied()
            {
                let is_capture = mv.is_capture();
                game.make_move(mv);

                if game.board.in_check() {
                    audio_messages.write(crate::audio::SoundMessage::Check);
                } else if is_capture {
                    audio_messages.write(crate::audio::SoundMessage::Capture);
                } else {
                    audio_messages.write(crate::audio::SoundMessage::Move);
                }
            }

            commands.remove_resource::<EngineTask>();

            if game.game_result.is_none() {
                turn_state.set(TurnState::PlayerTurn);
            }
        }
        Err(mpsc::TryRecvError::Empty) => {
            // Still computing — UI stays responsive
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            commands.remove_resource::<EngineTask>();
            turn_state.set(TurnState::PlayerTurn);
        }
    }
}

fn search_builtin_engine(mut board: types::Board, depth: i32) -> EngineResult {
    let mut engine = search::SearchEngine::new(64, 1);
    let result = engine.search_depth(&mut board, depth);
    EngineResult {
        best_move: result.best_move,
        score: result.score,
        depth: result.depth,
        nodes: result.nodes,
        engine_name: "Mujrim built-in".to_owned(),
    }
}

fn search_external_engine(
    board: &types::Board,
    engine: &mujrim_protocols::catalog::DiscoveredEngine,
    depth: i32,
    time_limit_ms: u64,
) -> Result<EngineResult, String> {
    const HASH_MB: usize = 64;
    const MEMORY_LIMIT_BYTES: u64 = 384 * 1024 * 1024;

    let mut session = EngineSession::spawn_with_args_and_memory_limit(
        &engine.path,
        &[],
        ProtocolKind::Uci,
        Some(MEMORY_LIMIT_BYTES),
    )?;
    session.configure(&EngineOptions {
        hash_mb: Some(HASH_MB),
        threads: Some(1),
        own_book: None,
        custom: Vec::new(),
    })?;
    let info = session.search(&SearchRequest {
        fen: board.to_fen(),
        moves: Vec::new(),
        depth,
        movetime: Some(Duration::from_millis(time_limit_ms.max(1))),
        node_limit: None,
        clock: None,
    })?;
    let best_move = find_legal_uci_move(board, &info.best_move).ok_or_else(|| {
        format!(
            "{} returned illegal move '{}'",
            engine.display_name, info.best_move
        )
    })?;

    Ok(EngineResult {
        best_move,
        score: info.score,
        depth: info.depth,
        nodes: info.nodes,
        engine_name: engine.display_name.to_owned(),
    })
}

fn find_legal_uci_move(board: &types::Board, uci: &str) -> Option<types::Move> {
    let mut board = board.clone();
    board
        .generate_legal_moves()
        .into_iter()
        .find(|candidate| candidate.to_uci() == uci)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::find_legal_uci_move;

    #[test]
    fn external_move_must_be_legal_in_current_position() {
        types::init();
        let board = types::Board::new();
        assert_eq!(
            find_legal_uci_move(&board, "e2e4").map(|mv| mv.to_uci()),
            Some("e2e4".to_owned())
        );
        assert!(find_legal_uci_move(&board, "e2e5").is_none());
    }
}
