use bevy::prelude::*;
use std::sync::{Mutex, mpsc};

use crate::state::{ChessGame, EngineConfig, TurnState};

/// Result from the engine search.
pub struct EngineResult {
    pub best_move: types::Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
}

/// Info about the engine's latest search, displayed in the HUD.
#[derive(Resource, Default, Clone)]
pub struct EngineInfo {
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub best_move_uci: String,
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

    let (tx, rx) = mpsc::channel();

    // Dedicated OS thread — never blocks the UI.
    std::thread::spawn(move || {
        let mut board = board;
        let mut engine = search::SearchEngine::new(64, 1);
        let result = engine.search_depth(&mut board, depth);
        let _ = tx.send(EngineResult {
            best_move: result.best_move,
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
        });
    });

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
        let rx = task.rx.lock().unwrap();
        rx.try_recv()
    };

    match result {
        Ok(result) => {
            engine_info.depth = result.depth;
            engine_info.score = result.score;
            engine_info.nodes = result.nodes;
            engine_info.best_move_uci = result.best_move.to_uci();

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
