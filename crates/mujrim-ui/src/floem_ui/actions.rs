//! Side-effecting UI actions shared by Floem screens and chrome.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::Duration;

use floem::ext_event::create_ext_action;
use floem::prelude::{SignalGet, SignalUpdate, SignalWith};
use mujrim_study::database::GameQuery;
use types::{Color, Move, Square};

use crate::app_core::analysis::{AnalysisEngineSpec, AnalysisRequest, run_multi_engine_analysis};
use crate::app_core::audio::BgmTrack;
use crate::app_core::engine::{GameMode, PlayerConfig};
use crate::app_core::game::GameState;
use crate::app_core::gif_export;
use crate::app_core::hub::{self, CoinFlipState};
use crate::app_core::layout::{self, DockTab};
use crate::app_core::logic;
use crate::app_core::match_controller;
use crate::app_core::recording::RecordState;
use crate::app_core::settings::{AppSettings, Screen};
use crate::app_core::uci_process::{self, ExternalEngineProtocol};

use super::engine;
use super::state::{AppHandles, AppState};

pub fn new_game(state: AppState, handles: &AppHandles) {
    types::init();
    uci_process::cancel_all_pondering();
    let mut generation = state.game_generation.get_untracked();
    let mut searching = state.searching.get_untracked();
    let mut retries = state.engine_retries.get_untracked();
    match_controller::bump_generation(&mut generation, &mut searching, &mut retries);
    state.game_generation.set(generation);
    state.searching.set(searching);
    state.engine_retries.set(retries);
    state.selected_tournament_game_id.set(None);

    let mut white = state.white_player.get_untracked();
    let mut black = state.black_player.get_untracked();
    let mut flipped = false;
    let mut status = "New game.".to_owned();
    if matches!(state.selected_mode.get_untracked(), GameMode::HumanVsEngine)
        && let CoinFlipState::Done { heads } = state.coin_flip.get_untracked()
    {
        let assigned = hub::apply_coin_flip(heads, white, black);
        white = assigned.white;
        black = assigned.black;
        flipped = assigned.flip_board;
        status = assigned.status.to_owned();
    }
    state.white_player.set(white.clone());
    state.black_player.set(black.clone());

    let mut game = GameState::new(types::Board::new());
    let settings = state.settings.get_untracked();
    if flipped
        || (settings.auto_flip_black
            && matches!(black, PlayerConfig::Human)
            && !matches!(white, PlayerConfig::Human))
    {
        game.flipped = true;
    }
    if settings.auto_flip_black
        && matches!(white, PlayerConfig::Human)
        && !matches!(black, PlayerConfig::Human)
    {
        game.flipped = false;
    }
    state.game.set(Some(game));
    state.move_log.set(Vec::new());
    state.move_annotations.set(Vec::new());
    state.analysis_scores.set(Vec::new());
    state.review_ply.set(None);
    state.analysis.set(None);
    state
        .initial_fen
        .set(mujrim_study::opening::START_FEN.to_owned());
    state.screen.set(Screen::Playing);
    state.status.set(status);
    if let Some(sound) = handles.sound.borrow_mut().as_mut() {
        sound.play_bgm(BgmTrack::Game);
    }
    let handles = handles.clone();
    floem::action::exec_after(Duration::from_millis(0), move |_| {
        engine::maybe_start_engine_turn(state, &handles);
    });
}

pub fn start_coin_flip(state: AppState) {
    use rand::Rng;
    let heads = rand::rng().random_bool(0.5);
    state.coin_flip.set(CoinFlipState::Flipping);
    floem::action::exec_after(Duration::from_millis(1500), move |_| {
        state.coin_flip.set(CoinFlipState::Done { heads });
        state.status.set(if heads {
            "Heads! You play White.".to_owned()
        } else {
            "Tails! You play Black.".to_owned()
        });
    });
}

pub fn resign(state: AppState, handles: &AppHandles) {
    state.game.update(|game| {
        if let Some(game) = game.as_mut() {
            game.game_over = true;
        }
    });
    state.searching.set(false);
    state.status.set("Resigned.".to_owned());
    if let Some(sound) = handles.sound.borrow_mut().as_mut() {
        sound.stop_bgm();
    }
}

pub fn on_board_press(state: AppState, handles: &AppHandles, square: Square) {
    let Some(mut game) = state.game.get_untracked() else {
        return;
    };
    if game.game_over {
        return;
    }
    if state.review_ply.get_untracked().is_some() {
        if matches!(
            state.screen.get_untracked(),
            Screen::Study | Screen::Learn | Screen::Analysis
        ) {
            resume_from_review(state);
        } else {
            return;
        }
    }
    let stm = game.board.side_to_move;
    let human_turn = player_is_human(state, stm);
    let settings = state.settings.get_untracked();
    if !human_turn && settings.premoves_enabled {
        if game.selected_square.is_some() {
            let _ = game.queue_premove(square, human_color(state), settings.multi_premoves);
        } else if game
            .board
            .piece_on(square)
            .is_some_and(|(_, color)| color == human_color(state))
        {
            game.select_premove_square(square, human_color(state));
        }
        state.game.set(Some(game));
        return;
    }
    if !human_turn {
        return;
    }
    game.begin_drag(square);
    state.game.set(Some(game));
    let _ = handles;
}

pub fn on_board_release(state: AppState, handles: &AppHandles, square: Square) {
    let Some(mut game) = state.game.get_untracked() else {
        return;
    };
    if game.game_over {
        return;
    }
    let drag = game.end_drag();
    let target = drag.map(|(_, to)| to).unwrap_or(square);
    let origin = game.selected_square.or(drag.map(|(from, _)| from));
    if let Some(from) = origin {
        game.selected_square = Some(from);
        let captured = game.board.piece_on(target).is_some();
        if let Some(mv) = game.try_move(target) {
            state.game.set(Some(game));
            apply_played_move(state, handles, mv, captured);
            return;
        }
        if game.board.piece_on(target).is_some() {
            game.select_square(target);
        } else {
            game.deselect();
        }
    } else if game.board.piece_on(target).is_some() {
        game.select_square(target);
    }
    state.game.set(Some(game));
}

pub fn apply_engine_move(
    state: AppState,
    mv: Move,
    ponder: Option<(Square, Square)>,
    captured: bool,
) {
    state.game.update(|game| {
        if let Some(game) = game.as_mut() {
            game.last_move_squares = vec![mv.from, mv.to];
            game.board.make_move(mv);
            game.deselect();
            game.game_over = game.board.is_game_over();
            let settings = state.settings.get_untracked();
            game.refresh_move_overlays(settings.last_move_arrow, ponder, &[]);
        }
    });
    state.move_log.update(|log| log.push(mv.to_uci()));
    state.move_annotations.update(|items| items.push(None));
    engine::begin_slide(state, mv.from, mv.to, captured);
    try_flush_premoves(state);
}

fn apply_played_move(state: AppState, handles: &AppHandles, mv: Move, captured: bool) {
    let mut captured = captured;
    state.game.update(|game| {
        if let Some(game) = game.as_mut() {
            captured = game.board.piece_on(mv.to).is_some() || captured;
        }
    });
    if let Some(sound) = handles.sound.borrow().as_ref() {
        if captured {
            sound.play_capture();
        } else {
            sound.play_move();
        }
    }
    state.move_log.update(|log| log.push(mv.to_uci()));
    state.move_annotations.update(|items| items.push(None));
    engine::begin_slide(state, mv.from, mv.to, captured);
    overlay_last_move(state);
    if state
        .game
        .with_untracked(|g| g.as_ref().is_some_and(|g| g.game_over))
    {
        state.status.set("Game over.".to_owned());
        return;
    }
    engine::maybe_start_engine_turn(state, handles);
}

fn overlay_last_move(state: AppState) {
    let settings = state.settings.get_untracked();
    state.game.update(|game| {
        if let Some(game) = game.as_mut() {
            game.refresh_move_overlays(settings.last_move_arrow, None, &[]);
        }
    });
}

fn try_flush_premoves(state: AppState) {
    let settings = state.settings.get_untracked();
    if !settings.premoves_enabled {
        return;
    }
    let human = human_color(state);
    state.game.update(|game| {
        let Some(game) = game.as_mut() else {
            return;
        };
        if game.board.side_to_move != human || game.premove_queue.is_empty() {
            return;
        }
        let entry = game.premove_queue.remove(0);
        game.selected_square = Some(entry.from);
        if game.try_move(entry.to).is_some() {
            state.move_log.update(|log| {
                log.push(format!("{}{}", entry.from, entry.to));
            });
        } else {
            game.clear_premoves();
        }
    });
}

fn player_is_human(state: AppState, color: Color) -> bool {
    matches!(
        match color {
            Color::White => state.white_player.get_untracked(),
            Color::Black => state.black_player.get_untracked(),
        },
        PlayerConfig::Human
    )
}

fn human_color(state: AppState) -> Color {
    if player_is_human(state, Color::White) {
        Color::White
    } else {
        Color::Black
    }
}

pub fn export_pgn(state: AppState) {
    let white = state.white_player.get_untracked().to_string();
    let black = state.black_player.get_untracked().to_string();
    let moves = state.move_log.get_untracked();
    let pgn = logic::build_pgn(&white, &black, &moves, "*");
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(&pgn);
        state.status.set("PGN copied to clipboard.".to_owned());
    }
}

pub fn save_to_library(state: AppState, handles: &AppHandles) {
    let mut study = handles.study.borrow_mut();
    let Some(database) = study.as_mut() else {
        state.status.set("Study library is unavailable.".to_owned());
        return;
    };
    let pgn = logic::build_pgn(
        &state.white_player.get_untracked().to_string(),
        &state.black_player.get_untracked().to_string(),
        &state.move_log.get_untracked(),
        "*",
    );
    match database.import_pgn_text(&pgn) {
        Ok(report) => state
            .status
            .set(format!("Saved to library ({} imported).", report.imported)),
        Err(error) => state.status.set(error),
    }
}

pub fn analyze_game(state: AppState, handles: &AppHandles) {
    state.screen.set(Screen::Analysis);
    let fen = state
        .game
        .get_untracked()
        .map(|game| game.board.to_fen())
        .unwrap_or_else(|| state.initial_fen.get_untracked());
    let cfg = state.engine_cfg.get_untracked();
    let selected = state.analysis_engines_selected.get_untracked();
    let mut engines = Vec::new();
    if selected.iter().any(|id| id == "builtin") {
        engines.push(AnalysisEngineSpec {
            id: "builtin".into(),
            name: "Mujrim".into(),
            path: None,
            protocol: ExternalEngineProtocol::Uci,
            builtin: true,
        });
    }
    let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
    for engine in roster {
        let id = engine.path.to_string_lossy().into_owned();
        if selected.iter().any(|selected| {
            selected == &id
                || logic::engine_identity_key(Path::new(selected))
                    == logic::engine_identity_key(&engine.path)
        }) {
            engines.push(AnalysisEngineSpec {
                id,
                name: engine.name,
                path: Some(engine.path),
                protocol: ExternalEngineProtocol::Uci,
                builtin: false,
            });
        }
    }
    if engines.is_empty() {
        engines.push(AnalysisEngineSpec {
            id: "builtin".into(),
            name: "Mujrim".into(),
            path: None,
            protocol: ExternalEngineProtocol::Uci,
            builtin: true,
        });
    }
    let request = AnalysisRequest {
        fen,
        depth: cfg.max_depth.min(16),
        movetime: Duration::from_millis((cfg.time_per_move.max(1) * 1000) as u64),
        hash_mb: crate::app_core::engine::bounded_hash_mb(cfg.hash_mb),
        threads: cfg.threads.max(1) as usize,
        multipv: state.analysis_multipv.get_untracked().clamp(1, 5) as u32,
        engines,
        max_pv_plies: 8,
    };
    let on_done = create_ext_action(
        handles.ui_scope,
        move |snapshot: crate::app_core::analysis::AnalysisSnapshot| {
            state.game.update(|game| {
                if let Some(game) = game.as_mut() {
                    game.overlay_arrows = snapshot.arrows.clone();
                }
            });
            state.analysis.set(Some(snapshot));
        },
    );
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            types::init();
            on_done(run_multi_engine_analysis(
                request,
                crate::app_core::engine::builtin_analysis_line,
            ));
        })
        .ok();
}

pub fn export_gif(state: AppState) {
    let moves = state.move_log.get_untracked();
    match gif_export::export_game_gif(&moves, 40) {
        Ok(bytes) => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("GIF", &["gif"])
                .save_file()
            {
                match std::fs::write(&path, bytes) {
                    Ok(()) => state.status.set(format!("GIF saved to {}", path.display())),
                    Err(error) => state.status.set(error.to_string()),
                }
            }
        }
        Err(error) => state.status.set(error),
    }
}

pub fn screenshot(state: AppState, _handles: &AppHandles) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG", &["png"])
        .save_file()
    else {
        return;
    };
    match capture_png() {
        Ok(bytes) => match std::fs::write(&path, bytes) {
            Ok(()) => state
                .status
                .set(format!("Screenshot saved to {}", path.display())),
            Err(error) => state.status.set(error.to_string()),
        },
        Err(error) => state.status.set(error),
    }
}

fn capture_png() -> Result<Vec<u8>, String> {
    use xcap::Monitor;
    let monitor = Monitor::all()
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "No monitor available.".to_owned())?;
    let image = monitor.capture_image().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub fn toggle_recording(state: AppState, handles: &AppHandles) {
    match handles.recorder.state() {
        RecordState::Idle => {
            handles.recorder.start();
            state.recording_label.set("Stop".to_owned());
            state.status.set("Recording…".to_owned());
            tick_recording(state, handles.clone());
        }
        RecordState::Recording => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Video / PNG seq", &["mp4", "png"])
                .save_file()
            {
                match handles.recorder.stop_and_save(path.clone()) {
                    Ok(frames) => state
                        .status
                        .set(format!("Saved {frames} frames to {}", path.display())),
                    Err(error) => state.status.set(error),
                }
            } else {
                handles.recorder.cancel();
            }
            state.recording_label.set("Record".to_owned());
        }
        RecordState::Saving => {}
    }
}

fn tick_recording(state: AppState, handles: AppHandles) {
    floem::action::exec_after(Duration::from_millis(120), move |_| {
        if handles.recorder.state() == RecordState::Recording {
            let _ = handles.recorder.capture_frame();
            state.recording_label.set("Recording…".to_owned());
            tick_recording(state, handles);
        }
    });
}

pub fn import_pgn(state: AppState, handles: &AppHandles) {
    let handles = handles.clone();
    let on_done = create_ext_action(handles.ui_scope, move |path: Option<PathBuf>| {
        let Some(path) = path else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            state.status.set("Could not read PGN.".to_owned());
            return;
        };
        let mut study = handles.study.borrow_mut();
        let Some(database) = study.as_mut() else {
            state.status.set("Study library is unavailable.".to_owned());
            return;
        };
        match database.import_pgn_text(&text) {
            Ok(report) => {
                state.status.set(format!(
                    "Imported {} / {} games.",
                    report.imported, report.discovered
                ));
                drop(study);
                refresh_study(state, &handles);
            }
            Err(error) => state.status.set(error),
        }
    });
    std::thread::spawn(move || {
        on_done(
            rfd::FileDialog::new()
                .add_filter("PGN", &["pgn", "txt"])
                .pick_file(),
        );
    });
}

pub fn pick_eval_file(state: AppState, handles: &AppHandles) {
    let on_done = create_ext_action(handles.ui_scope, move |path: Option<PathBuf>| {
        if let Some(path) = path {
            state
                .engine_cfg
                .update(|cfg| cfg.eval_file = Some(path.to_string_lossy().into_owned()));
        }
    });
    std::thread::spawn(move || {
        on_done(rfd::FileDialog::new().pick_file());
    });
}

pub fn clear_eval_file(state: AppState) {
    state.engine_cfg.update(|cfg| cfg.eval_file = None);
}

pub fn refresh_study(state: AppState, handles: &AppHandles) {
    let study = handles.study.borrow();
    let Some(database) = study.as_ref() else {
        return;
    };
    let query = GameQuery {
        text: {
            let q = state.study_query.get_untracked();
            if q.trim().is_empty() { None } else { Some(q) }
        },
        ..GameQuery::default()
    };
    state.study_results.set(database.search(&query));
}

pub fn load_library_game(state: AppState, handles: &AppHandles, id: String) {
    let loaded = handles
        .study
        .borrow()
        .as_ref()
        .ok_or_else(|| "Study library is unavailable.".to_owned())
        .and_then(|database| database.load_game(&id));
    match loaded {
        Ok(game) => match logic::replay_study_game(&game.initial_fen, &game.moves) {
            Ok(board) => {
                state.white_player.set(PlayerConfig::Human);
                state.black_player.set(PlayerConfig::Human);
                state.initial_fen.set(game.initial_fen);
                state.move_log.set(game.moves.clone());
                state.move_annotations.set(vec![None; game.moves.len()]);
                state.game.set(Some(board));
                state.review_ply.set(Some(game.moves.len()));
                if !matches!(
                    state.screen.get_untracked(),
                    Screen::Study | Screen::Learn | Screen::Analysis
                ) {
                    state.screen.set(Screen::Study);
                }
                state.status.set("Loaded library game.".to_owned());
            }
            Err(error) => state.status.set(error),
        },
        Err(error) => state.status.set(error),
    }
}

pub fn start_puzzle(state: AppState, handles: &AppHandles) {
    let due = handles
        .training
        .borrow()
        .as_ref()
        .map(|store| store.due(logic::today_day(), 8))
        .unwrap_or_default();
    state.training_due.set(due.clone());
    if let Some(item) = due.into_iter().next() {
        start_training(state, handles, item.puzzle.id);
    } else {
        state.status.set("No puzzles due.".to_owned());
    }
}

pub fn seed_training(state: AppState, handles: &AppHandles) {
    let mut training = handles.training.borrow_mut();
    state.status.set(match training.as_mut() {
        Some(store) => match logic::seed_training(store) {
            Ok(added) => format!("Added {added} starter training positions."),
            Err(error) => format!("Training setup failed: {error}"),
        },
        None => "Training database is unavailable.".to_owned(),
    });
    drop(training);
    refresh_training_due(state, handles);
}

fn refresh_training_due(state: AppState, handles: &AppHandles) {
    let due = handles
        .training
        .borrow()
        .as_ref()
        .map(|store| store.due(logic::today_day(), 32))
        .unwrap_or_default();
    state.training_due.set(due);
}

pub fn start_training(state: AppState, handles: &AppHandles, id: String) {
    let item = handles
        .training
        .borrow()
        .as_ref()
        .and_then(|store| store.get(&id).cloned());
    match item {
        Some(item) => match logic::replay_study_game(&item.puzzle.fen, &[]) {
            Ok(game) => {
                state.white_player.set(PlayerConfig::Human);
                state.black_player.set(PlayerConfig::Human);
                state.initial_fen.set(item.puzzle.fen.clone());
                state.move_log.set(Vec::new());
                state.move_annotations.set(Vec::new());
                state.game.set(Some(game));
                state.active_puzzle.set(Some(item.clone()));
                state.puzzle_line.set(Vec::new());
                state.active_gambit_id.set(None);
                state.screen.set(Screen::Learn);
                state.status.set(format!(
                    "Training: {}. Find the best continuation.",
                    item.puzzle.themes.join(", ")
                ));
            }
            Err(error) => state
                .status
                .set(format!("Could not load training: {error}")),
        },
        None => state
            .status
            .set(format!("Training position '{id}' was not found.")),
    }
}

pub fn resume_from_review(state: AppState) {
    let Some(ply) = state.review_ply.get_untracked() else {
        return;
    };
    let len = state.move_log.get_untracked().len();
    if ply < len {
        state.move_log.update(|moves| moves.truncate(ply));
        state.move_annotations.update(|items| items.truncate(ply));
    }
    state.review_ply.set(None);
}

pub fn study_opening_move(state: AppState, uci: String) {
    types::init();
    resume_from_review(state);
    let continuing = state.game.get_untracked().is_some();
    let mut board = state
        .game
        .get_untracked()
        .map_or_else(types::Board::new, |game| game.board.clone());
    match logic::apply_uci_move(&mut board, &uci) {
        Ok(mv) => {
            let mut game = GameState::new(board);
            game.last_move_squares = vec![mv.from, mv.to];
            if !continuing {
                state
                    .initial_fen
                    .set(mujrim_study::opening::START_FEN.to_owned());
                state.move_log.set(Vec::new());
                state.move_annotations.set(Vec::new());
            }
            state.move_log.update(|moves| moves.push(uci));
            state.move_annotations.update(|items| items.push(None));
            state.game.set(Some(game));
            state.active_puzzle.set(None);
            state.active_gambit_id.set(None);
            state
                .status
                .set("Opening move played on the study board.".to_owned());
            if !matches!(state.screen.get_untracked(), Screen::Study | Screen::Learn) {
                state.screen.set(Screen::Study);
            }
        }
        Err(error) => state.status.set(error),
    }
}

pub fn ensure_study_board(state: AppState) {
    if state.game.get_untracked().is_some() {
        return;
    }
    types::init();
    state
        .initial_fen
        .set(mujrim_study::opening::START_FEN.to_owned());
    state.move_log.set(Vec::new());
    state.move_annotations.set(Vec::new());
    state.review_ply.set(None);
    state.game.set(Some(GameState::new(types::Board::new())));
}

pub fn view_ply(state: AppState, ply: usize) {
    let fen = state.initial_fen.get_untracked();
    let moves = state.move_log.get_untracked();
    match logic::board_at_ply(&fen, &moves, ply) {
        Ok(board) => {
            state.review_ply.set(Some(ply));
            let last_move = if ply > 0 {
                logic::board_at_ply(&fen, &moves, ply - 1)
                    .ok()
                    .and_then(|mut previous| {
                        logic::apply_uci_move(&mut previous, &moves[ply - 1])
                            .ok()
                            .map(|mv| vec![mv.from, mv.to])
                    })
            } else {
                None
            };
            state.game.update(|game| {
                if let Some(game) = game.as_mut() {
                    game.board = board;
                    if let Some(squares) = last_move {
                        game.last_move_squares = squares;
                    }
                }
            });
            state.status.set(format!("Reviewing ply {ply}."));
        }
        Err(error) => state.status.set(format!("Could not navigate: {error}")),
    }
}

pub fn refresh_saved_lines(state: AppState, handles: &AppHandles) {
    if let Some(database) = handles.study.borrow().as_ref()
        && let Ok(lines) = database.list_lines()
    {
        state.saved_lines.set(lines);
    }
}

pub fn save_preparation(state: AppState, handles: &AppHandles) {
    let name = state.line_name.get_untracked();
    let name = if name.trim().is_empty() {
        "Untitled line".to_owned()
    } else {
        name
    };
    match logic::save_current_line(
        name,
        state.prep_side.get_untracked(),
        state.initial_fen.get_untracked(),
        state.move_log.get_untracked(),
        state.prep_notes.get_untracked(),
    ) {
        Ok(line) => {
            let result = handles
                .study
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| "Study library is unavailable.".to_owned())
                .and_then(|database| database.save_line(&line));
            match result {
                Ok(()) => {
                    state
                        .status
                        .set(format!("Saved preparation '{}'.", line.name));
                    refresh_saved_lines(state, handles);
                }
                Err(error) => state.status.set(error),
            }
        }
        Err(error) => state.status.set(error),
    }
}

pub fn load_preparation(state: AppState, handles: &AppHandles, id: String) {
    let line = state
        .saved_lines
        .get_untracked()
        .into_iter()
        .find(|line| line.id == id);
    let Some(line) = line else {
        refresh_saved_lines(state, handles);
        state
            .status
            .set("That preparation is no longer available.".to_owned());
        return;
    };
    match logic::replay_study_game(&line.initial_fen, &line.moves) {
        Ok(game) => {
            state.initial_fen.set(line.initial_fen);
            state.move_log.set(line.moves.clone());
            state.move_annotations.set(vec![None; line.moves.len()]);
            state.line_name.set(line.name.clone());
            state.prep_notes.set(line.notes.clone());
            state.prep_side.set(line.side);
            state.review_ply.set(Some(line.moves.len()));
            state.game.set(Some(game));
            state.screen.set(Screen::Study);
            state.status.set(format!("Loaded '{}'.", line.name));
        }
        Err(error) => state.status.set(error),
    }
}

pub fn delete_preparation(state: AppState, handles: &AppHandles, id: String) {
    let result = handles
        .study
        .borrow_mut()
        .as_mut()
        .ok_or_else(|| "Study library is unavailable.".to_owned())
        .and_then(|database| database.delete_line(&id));
    match result {
        Ok(()) => {
            state.status.set("Deleted preparation.".to_owned());
            refresh_saved_lines(state, handles);
        }
        Err(error) => state.status.set(error),
    }
}

pub fn start_gambit_lesson(state: AppState, id: String) {
    let Some(lesson) = mujrim_study::gambit::find_gambit(&id) else {
        state.status.set(format!("Gambit '{id}' was not found."));
        return;
    };
    let ply = lesson.key_ply.min(lesson.moves.len());
    match lesson.fen_after_plies(ply) {
        Ok(fen) => match types::Board::from_fen(&fen) {
            Ok(board) => {
                let mut game = GameState::new(board);
                if let Ok(arrows) = lesson.coaching_arrows(ply.max(1)) {
                    game.overlay_arrows = arrows;
                }
                state.game.set(Some(game));
                state.initial_fen.set(fen);
                state.move_log.set(
                    lesson.moves[..ply]
                        .iter()
                        .map(|mv| (*mv).to_owned())
                        .collect(),
                );
                state.move_annotations.set(vec![None; ply]);
                state.active_gambit_id.set(Some(id));
                state.gambit_ply.set(ply);
                state.active_puzzle.set(None);
                state.screen.set(Screen::Learn);
                state
                    .status
                    .set(format!("Gambit: {} ({})", lesson.name, lesson.eco));
            }
            Err(error) => state.status.set(error),
        },
        Err(error) => state.status.set(error),
    }
}

pub fn gambit_step(state: AppState, delta: i32) {
    let Some(id) = state.active_gambit_id.get_untracked() else {
        return;
    };
    let Some(lesson) = mujrim_study::gambit::find_gambit(&id) else {
        return;
    };
    let next = (state.gambit_ply.get_untracked() as i32 + delta).clamp(0, lesson.moves.len() as i32)
        as usize;
    if let Ok(fen) = lesson.fen_after_plies(next)
        && let Ok(board) = types::Board::from_fen(&fen)
    {
        state.game.update(|game| {
            if let Some(game) = game.as_mut() {
                game.board = board;
                game.overlay_arrows = lesson.coaching_arrows(next.max(1)).unwrap_or_default();
            }
        });
        state.initial_fen.set(fen);
        state.move_log.set(
            lesson.moves[..next]
                .iter()
                .map(|mv| (*mv).to_owned())
                .collect(),
        );
        state.move_annotations.set(vec![None; next]);
        state.gambit_ply.set(next);
    }
}

pub fn toggle_analysis_engine(state: AppState, id: String) {
    state.analysis_engines_selected.update(|selected| {
        if let Some(index) = selected.iter().position(|existing| existing == &id) {
            selected.remove(index);
        } else {
            selected.push(id);
        }
    });
}

pub fn index_openings(state: AppState, handles: &AppHandles) {
    let path = logic::study_database_path();
    let handles = handles.clone();
    let on_done = create_ext_action(handles.ui_scope, move |(explorer, count)| {
        *handles.explorer.borrow_mut() = explorer;
        state.opening_indexed.set(count);
        state.status.set(format!("Indexed {count} opening games."));
    });
    std::thread::spawn(move || {
        on_done(logic::index_openings(path));
    });
}

pub fn start_tournament(state: AppState, handles: &AppHandles) {
    if state.tournament_snapshot.get_untracked().running {
        state
            .tournament_status
            .set("A tournament is already running. Cancel it first.".to_owned());
        return;
    }
    let mut setup = state.tournament_setup.get_untracked();
    setup.event = state.tournament_event.get_untracked();
    setup.site = state.tournament_site.get_untracked();
    setup.sanitize_for_gui();
    state.tournament_setup.set(setup.clone());
    let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
    if setup.selected_engine_paths.is_empty() {
        setup.selected_engine_paths = logic::default_tournament_engine_paths(&roster);
        state.tournament_setup.set(setup.clone());
    }
    let selected: Vec<_> = roster
        .into_iter()
        .filter(|engine| logic::engine_is_selected(&setup.selected_engine_paths, &engine.path))
        .collect();
    if selected.len() < 2 {
        state.tournament_status.set(
            "Select at least two engines. Failures forfeit the game; they will not crash the UI."
                .to_owned(),
        );
        return;
    }
    uci_process::cancel_all_pondering();
    uci_process::shutdown_external_engines();
    state.selected_tournament_game_id.set(None);
    state.analysis_scores.set(Vec::new());
    state.move_log.set(Vec::new());
    state.game.set(None);
    state.dock_tab.set(DockTab::Histogram);
    state.dock_open.set(true);
    let handle = crate::app_core::tournament_live::LiveTournamentHandle::new(setup.format);
    *handles.tournament.borrow_mut() = Some(handle.clone());
    state.show_tournament_setup.set(false);
    state.screen.set(Screen::Tournaments);
    state.tournament_status.set(format!(
        "Starting {} engines · 1 board · forfeit on engine errors.",
        selected.len()
    ));
    let snapshot = handle.clone();
    let on_done = create_ext_action(handles.ui_scope, move |summary| {
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            state
                .tournament_status
                .set(logic::format_tournament_summary(&summary));
            if let Ok(guard) = snapshot.snapshot.lock() {
                state.tournament_snapshot.set(guard.clone());
            }
        }));
    });
    if std::thread::Builder::new()
        .name("mujrim-tournament-ui".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let format = setup.format;
            let summary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                logic::run_quick_tournament(selected, setup, handle)
            }))
            .unwrap_or_else(|_| mujrim_benchmarker::strength::TournamentSummary {
                format,
                engines: Vec::new(),
                matches: Vec::new(),
                standings: Vec::new(),
                game_results: Vec::new(),
                cancelled: false,
                error: Some(
                    "Tournament failed unexpectedly. Engine errors forfeit the game; the UI stayed up."
                        .to_owned(),
                ),
            });
            on_done(summary);
        })
        .is_err()
    {
        state
            .tournament_status
            .set("Could not start the tournament worker.".to_owned());
        *handles.tournament.borrow_mut() = None;
        return;
    }
    poll_tournament(state, handles.clone());
}

fn poll_tournament(state: AppState, handles: AppHandles) {
    floem::action::exec_after(Duration::from_millis(250), move |_| {
        let keep_polling = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let handle = handles.tournament.borrow().clone();
            let Some(handle) = handle else {
                return false;
            };
            let guard = match handle.snapshot.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let running = guard.running;
            state.tournament_snapshot.set(guard.clone());
            state.tournament_status.set(guard.status_line.clone());
            if match_controller::should_sync_tournament_board(state.screen.get_untracked()) {
                sync_tournament_board(state, &guard);
            }
            running
        }))
        .unwrap_or(false);
        if keep_polling {
            poll_tournament(state, handles.clone());
        }
    });
}

fn sync_tournament_board(
    state: AppState,
    snap: &crate::app_core::tournament_live::LiveTournamentSnapshot,
) {
    if let Some(id) = state.selected_tournament_game_id.get_untracked()
        && let Some(played) = snap.game(id).cloned()
        && let Ok(game) = logic::replay_study_game(&played.initial_fen, &played.moves)
    {
        state.game.set(Some(game));
        state.move_log.set(played.moves);
        state.initial_fen.set(played.initial_fen);
        return;
    }
    if let Some(live) = layout::focused_live_game(&snap.live_games).cloned() {
        if (state.move_log.get_untracked() != live.moves
            || state.initial_fen.get_untracked() != live.initial_fen)
            && let Ok(game) = logic::replay_study_game(&live.initial_fen, &live.moves)
        {
            state.game.set(Some(game));
            state.move_log.set(live.moves.clone());
            state.initial_fen.set(live.initial_fen.clone());
        }
        let mut scores = state.analysis_scores.get_untracked();
        layout::extend_histogram(&mut scores, live.moves.len(), live.score_cp);
        state.analysis_scores.set(scores);
        return;
    }
    if !snap.running
        && let Some(played) = snap.played_games.last().cloned()
        && let Ok(game) = logic::replay_study_game(&played.initial_fen, &played.moves)
    {
        state.game.set(Some(game));
        state.move_log.set(played.moves);
        state.initial_fen.set(played.initial_fen);
    }
}

pub fn cancel_tournament(handles: &AppHandles) {
    if let Some(handle) = handles.tournament.borrow().as_ref() {
        handle
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

pub fn refresh_updater_status(state: AppState) {
    let syzygy_dir = updater::syzygy::default_syzygy_path();
    let (wdl, dtz) = updater::syzygy::check_installed(&syzygy_dir);
    state.syzygy_status.set(if wdl + dtz == 0 {
        "Not installed".to_owned()
    } else {
        format!("{wdl} WDL / {dtz} DTZ tables")
    });
    let nnue_dir = updater::nnue::default_nnue_path();
    let installed = updater::nnue::check_installed(&nnue_dir);
    let count = installed
        .iter()
        .filter(|(_, status)| *status != updater::nnue::NetStatus::Missing)
        .count();
    state.nnue_status.set(format!(
        "{count} / {} networks",
        updater::nnue::NETWORKS.len()
    ));
    let path = updater::tuning::TunableParams::default_path();
    state
        .tuning_status
        .set(match updater::tuning::TunableParams::load(&path) {
            Ok(_) => "Loaded".to_owned(),
            Err(error) => format!("Load error: {error}"),
        });
}

pub fn download_syzygy(state: AppState, handles: &AppHandles) {
    let dest = updater::syzygy::default_syzygy_path();
    let piece_set = state.syzygy_piece_set.get_untracked();
    let on_done = create_ext_action(
        handles.ui_scope,
        move |result: Result<(), String>| match result {
            Ok(()) => {
                refresh_updater_status(state);
                state.status.set("Syzygy download finished.".to_owned());
            }
            Err(error) => state.status.set(error),
        },
    );
    std::thread::spawn(move || {
        on_done(updater::syzygy::download_tables(&dest, piece_set, None).map(|_| ()));
    });
}

pub fn download_nnue(state: AppState, handles: &AppHandles) {
    let dest = updater::nnue::default_nnue_path();
    let on_done = create_ext_action(
        handles.ui_scope,
        move |result: Result<updater::nnue::DownloadSummary, String>| match result {
            Ok(summary) => {
                refresh_updater_status(state);
                state
                    .status
                    .set(format!("Downloaded {} networks.", summary.downloaded));
            }
            Err(error) => state.status.set(error),
        },
    );
    std::thread::spawn(move || {
        on_done(updater::nnue::download_all(&dest, None));
    });
}

pub fn update_settings(state: AppState, patch: impl FnOnce(&mut AppSettings)) {
    state.settings.update(patch);
    state.persist_settings();
}

pub fn select_mode(state: AppState, handles: &AppHandles, mode: GameMode) {
    state.selected_mode.set(mode);
    let (white, black) =
        logic::players_for_detected_engines(mode, &handles.bundled, &handles.catalog.borrow());
    state.white_player.set(white);
    state.black_player.set(black);
    state.coin_flip.set(CoinFlipState::Idle);
}

pub fn open_tournament_setup(state: AppState, handles: &AppHandles) {
    let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
    state.tournament_setup.update(|setup| {
        if setup.selected_engine_paths.is_empty() {
            setup.selected_engine_paths = logic::default_tournament_engine_paths(&roster);
        }
        if setup.event.trim().is_empty() {
            setup.event = "Mujrim Tournament".to_owned();
        }
        setup.sanitize_for_gui();
    });
    let setup = state.tournament_setup.get_untracked();
    state.tournament_event.set(setup.event);
    state.tournament_site.set(setup.site);
    state.tournament_status.set(if roster.is_empty() {
        "No local engines found. Vendor them with scripts/vendor-linux-engines.sh.".to_owned()
    } else {
        format!("Detected {} local engines.", roster.len())
    });
    state.status.set(state.tournament_status.get_untracked());
    state.show_tournament_setup.set(true);
}

pub fn pick_external_engine(
    state: AppState,
    handles: &AppHandles,
    white: bool,
    protocol: ExternalEngineProtocol,
) {
    let on_done = create_ext_action(handles.ui_scope, move |path: Option<PathBuf>| {
        let Some(path) = path else {
            return;
        };
        let player = PlayerConfig::External {
            path: path.to_string_lossy().into_owned(),
            protocol,
        };
        if white {
            state.white_player.set(player);
        } else {
            state.black_player.set(player);
        }
    });
    std::thread::spawn(move || {
        on_done(rfd::FileDialog::new().pick_file());
    });
}

pub fn annotate_last_move(state: AppState) {
    let moves = state.move_log.get_untracked();
    if moves.is_empty() {
        return;
    }
    let fen = state.initial_fen.get_untracked();
    match logic::analyze_game_at_depth_from(&fen, &moves, 8) {
        Ok(plies) => {
            state
                .move_annotations
                .set(plies.iter().map(|ply| Some(ply.annotation)).collect());
            state
                .analysis_scores
                .set(plies.iter().map(|ply| Some(ply.score_cp)).collect());
        }
        Err(error) => state.status.set(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::engine::players_for_mode;

    #[test]
    fn human_side_defaults_to_white_when_white_is_human() {
        // PlayerConfig Display stays stable for PGN headers.
        assert_eq!(PlayerConfig::Human.to_string(), "Human");
        let (white, black) = players_for_mode(GameMode::HumanVsEngine, &[]);
        assert!(matches!(white, PlayerConfig::Human));
        assert!(matches!(black, PlayerConfig::BuiltIn { .. }));
    }
}
