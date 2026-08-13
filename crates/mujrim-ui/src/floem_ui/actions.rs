//! Side-effecting UI actions shared by Floem screens and chrome.

use std::path::PathBuf;
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
    if game.game_over || state.review_ply.get_untracked().is_some() {
        return;
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
    let mut engines = vec![AnalysisEngineSpec {
        id: "builtin".into(),
        name: "Mujrim".into(),
        path: None,
        protocol: ExternalEngineProtocol::Uci,
        builtin: true,
    }];
    if let PlayerConfig::External { path, protocol } = state.black_player.get_untracked() {
        let name = PathBuf::from(&path)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        engines.push(AnalysisEngineSpec {
            id: path.clone(),
            name,
            path: Some(PathBuf::from(path)),
            protocol,
            builtin: false,
        });
    }
    let request = AnalysisRequest {
        fen,
        depth: cfg.max_depth.min(16),
        movetime: Duration::from_millis(400),
        hash_mb: crate::app_core::engine::bounded_hash_mb(cfg.hash_mb),
        threads: cfg.threads.max(1) as usize,
        multipv: 3,
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
                state.move_log.set(game.moves);
                state.game.set(Some(board));
                state.screen.set(Screen::Playing);
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
        if let Ok(game) = logic::replay_study_game(&item.puzzle.fen, &[]) {
            state.game.set(Some(game));
        }
        state.puzzle_line.set(Vec::new());
        state.active_puzzle.set(Some(item));
        state.screen.set(Screen::Playing);
        state.status.set("Solve the puzzle.".to_owned());
    } else {
        state.status.set("No puzzles due.".to_owned());
    }
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
    let mut setup = state.tournament_setup.get_untracked();
    let roster = logic::tournament_engine_roster(&handles.bundled, &handles.catalog.borrow());
    if setup.selected_engine_paths.is_empty() {
        setup.selected_engine_paths = roster.iter().map(|engine| engine.path.clone()).collect();
        state.tournament_setup.set(setup.clone());
    }
    let selected: Vec<_> = roster
        .into_iter()
        .filter(|engine| {
            setup
                .selected_engine_paths
                .iter()
                .any(|path| path == &engine.path)
        })
        .collect();
    if selected.len() < 2 {
        state
            .tournament_status
            .set("Select at least two engines.".to_owned());
        return;
    }
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
    let snapshot = handle.clone();
    let on_done = create_ext_action(handles.ui_scope, move |summary| {
        state
            .tournament_status
            .set(logic::format_tournament_summary(&summary));
        if let Ok(guard) = snapshot.snapshot.lock() {
            state.tournament_snapshot.set(guard.clone());
        }
    });
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let summary = logic::run_quick_tournament(selected, setup, handle);
            on_done(summary);
        })
        .ok();
    poll_tournament(state, handles.clone());
}

fn poll_tournament(state: AppState, handles: AppHandles) {
    floem::action::exec_after(Duration::from_millis(250), move |_| {
        if let Some(handle) = handles.tournament.borrow().as_ref()
            && let Ok(guard) = handle.snapshot.lock()
        {
            let running = guard.running;
            state.tournament_snapshot.set(guard.clone());
            state.tournament_status.set(guard.status_line.clone());
            if match_controller::should_sync_tournament_board(state.screen.get_untracked()) {
                sync_tournament_board(state, &guard);
            }
            if running {
                poll_tournament(state, handles.clone());
            }
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
            setup.selected_engine_paths = roster.iter().map(|engine| engine.path.clone()).collect();
        }
    });
    state.status.set(if roster.is_empty() {
        "No local engines found. Vendor them with scripts/vendor-linux-engines.sh.".to_owned()
    } else {
        format!("Detected {} local engines.", roster.len())
    });
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
