//! Live engine search that streams SearchInfo into ArcRwSignal telemetry.

use std::time::Duration;

use floem::ext_event::create_ext_action;
use floem::prelude::{SignalGet, SignalUpdate, SignalWith};
use mujrim_protocols::SearchInfo;
use types::Move;

use crate::app_core::engine::{
    EngineConfig, PlayerConfig, TelemetrySnapshot, apply_search_info, builtin_engine_search,
};
use crate::app_core::game::GameState;
use crate::app_core::match_controller::{self, FinishOutcome, MatchAction, MatchSnapshot};
use crate::app_core::uci_process::{self, ExternalSearchConfig};

use super::actions;
use super::state::{AppHandles, AppState, SlideAnim};

type EngineSearchOutcome =
    Result<(Move, String, Option<(types::Square, types::Square)>, bool), String>;

pub fn maybe_start_engine_turn(state: AppState, handles: &AppHandles) {
    let Some(game) = state.game.get_untracked() else {
        return;
    };
    let snap = MatchSnapshot::from_game(
        state.game_generation.get_untracked(),
        state.searching.get_untracked(),
        state.engine_retries.get_untracked(),
        &game,
        &state.white_player.get_untracked(),
        &state.black_player.get_untracked(),
    );
    if match_controller::next_action(&snap) != MatchAction::Think {
        return;
    }
    let player = match game.board.side_to_move {
        types::Color::White => state.white_player.get_untracked(),
        types::Color::Black => state.black_player.get_untracked(),
    };
    start_search(state, handles, player, game);
}

fn schedule_next_turn(state: AppState, handles: AppHandles) {
    floem::action::exec_after(Duration::from_millis(0), move |_| {
        maybe_start_engine_turn(state, &handles);
    });
}

fn start_search(state: AppState, handles: &AppHandles, player: PlayerConfig, game: GameState) {
    let mut searching = state.searching.get_untracked();
    match_controller::begin_search(&mut searching);
    state.searching.set(searching);
    let generation = state.game_generation.get_untracked();
    let cfg = state.engine_cfg.get_untracked();
    let telemetry = handles.telemetry.clone();
    telemetry.set(TelemetrySnapshot::from_label("searching…"));

    #[cfg(feature = "book")]
    if cfg.use_book
        && let Some(book) = handles.book.as_ref()
        && let Some(book_move) = book.probe(&game.board)
    {
        let legal = game.board.clone().generate_legal_moves();
        if legal
            .iter()
            .any(|mv| mv.from == book_move.from && mv.to == book_move.to)
        {
            finish_move(
                state,
                handles.clone(),
                generation,
                Ok((book_move, "Book move".to_owned(), None, false)),
            );
            return;
        }
    }

    let handles_done = handles.clone();
    let on_done = create_ext_action(handles.ui_scope, move |result| {
        finish_move(state, handles_done, generation, result)
    });
    let mut board = game.board.clone();
    std::thread::Builder::new()
        .name("mujrim-ui-engine".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            types::init();
            let result = search_side(&mut board, &player, &cfg, &telemetry);
            on_done(result);
        })
        .expect("engine thread");
}

fn search_side(
    board: &mut types::Board,
    player: &PlayerConfig,
    cfg: &EngineConfig,
    telemetry: &floem::ext_event::ArcRwSignal<TelemetrySnapshot>,
) -> EngineSearchOutcome {
    let hash_mb = crate::app_core::engine::bounded_hash_mb(cfg.hash_mb);
    let threads = cfg.threads.max(1) as usize;
    let time = Duration::from_secs(cfg.time_per_move.max(1) as u64);
    match player {
        PlayerConfig::Human => Err("No engine selected for this side.".to_owned()),
        PlayerConfig::BuiltIn { .. } => {
            let (mv, info) = builtin_engine_search(
                board,
                hash_mb,
                threads,
                cfg.use_nnue,
                cfg.eval_file.as_deref(),
                time,
                cfg.max_depth,
            )?;
            telemetry.set(TelemetrySnapshot::from_label(info.clone()));
            Ok((mv, info, None, false))
        }
        PlayerConfig::External { path, protocol } => {
            let fen = board.to_fen();
            let legal = board.generate_legal_moves();
            let search = ExternalSearchConfig {
                ponder: cfg.ponder,
                use_nnue: cfg.use_nnue,
                own_book: cfg.use_book,
                eval_file: cfg.eval_file.clone(),
            };
            let info = uci_process::query_best_move_streaming(
                path,
                *protocol,
                &fen,
                cfg.max_depth,
                time,
                hash_mb,
                threads,
                &search,
                |snapshot: &SearchInfo| {
                    apply_search_info(&mut telemetry.write(), snapshot, &protocol.to_string());
                },
            )?;
            let mv = legal
                .iter()
                .find(|candidate| candidate.to_uci() == info.best_move)
                .copied()
                .ok_or_else(|| format!("{protocol} returned illegal move '{}'", info.best_move))?;
            let ponder = info.ponder_move.as_deref().and_then(|uci| {
                let mut predicted = board.clone();
                predicted.make_move(mv);
                predicted
                    .generate_legal_moves()
                    .into_iter()
                    .find(|candidate| candidate.to_uci() == uci)
                    .map(|ponder_mv| (ponder_mv.from, ponder_mv.to))
            });
            Ok((
                mv,
                format!("{protocol} {}", info.telemetry()),
                ponder,
                info.ponder_hit,
            ))
        }
    }
}

fn finish_move(state: AppState, handles: AppHandles, generation: u64, result: EngineSearchOutcome) {
    let mut searching = state.searching.get_untracked();
    let mut retries = state.engine_retries.get_untracked();
    let live = state.game_generation.get_untracked();
    let ok = result.is_ok();
    let outcome =
        match_controller::finish_search(generation, live, &mut searching, &mut retries, ok);
    state.searching.set(searching);
    state.engine_retries.set(retries);
    match (outcome, result) {
        (FinishOutcome::Stale, _) => {}
        (FinishOutcome::Applied, Ok((mv, info, ponder, hit))) => {
            handles
                .telemetry
                .set(TelemetrySnapshot::from_label(info.clone()));
            state.status.set(info);
            let captured = state.game.with_untracked(|game| {
                game.as_ref()
                    .and_then(|gs| gs.board.piece_on(mv.to))
                    .is_some()
            });
            if captured {
                if let Some(sound) = handles.sound.borrow().as_ref() {
                    sound.play_capture();
                }
            } else if let Some(sound) = handles.sound.borrow().as_ref() {
                sound.play_move();
            }
            actions::apply_engine_move(state, mv, ponder, captured);
            if match_controller::should_cancel_ponder(state.engine_cfg.get_untracked().ponder, hit)
            {
                uci_process::cancel_all_pondering();
            }
            schedule_next_turn(state, handles);
        }
        (FinishOutcome::Retry, Err(error)) => {
            state
                .status
                .set(format!("Engine failed: {error} — retrying…"));
            schedule_next_turn(state, handles);
        }
        (FinishOutcome::Failed, Err(error)) => state.status.set(error),
        _ => {}
    }
}

pub fn begin_slide(state: AppState, from: types::Square, to: types::Square, captured: bool) {
    let settings = state.settings.get_untracked();
    if !settings.piece_slide {
        state.slide.set(None);
        state.slide_t.set(1.0);
        return;
    }
    state.slide.set(Some(SlideAnim { from, to, captured }));
    state.slide_t.set(0.0);
    if captured {
        state.capture_burst.set(1.0);
    }
    tick_slide(state);
}

fn tick_slide(state: AppState) {
    floem::action::exec_after(Duration::from_millis(16), move |_| {
        let pace = state.anim_pace();
        let step = match pace {
            crate::app_core::motion::AnimPace::Fast => 0.14,
            crate::app_core::motion::AnimPace::Normal => 0.08,
            crate::app_core::motion::AnimPace::Slow => 0.045,
        };
        let next = (state.slide_t.get_untracked() + step).min(1.0);
        state.slide_t.set(next);
        let burst = (state.capture_burst.get_untracked() - step).max(0.0);
        state.capture_burst.set(burst);
        if next < 1.0 {
            tick_slide(state);
        } else {
            state.slide.set(None);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mujrim_protocols::SearchInfo;

    #[test]
    fn streaming_reducer_keeps_pv_and_hashfull() {
        let info = SearchInfo {
            depth: 9,
            hashfull: 400,
            best_move: "g1f3".into(),
            pv: vec!["g1f3".into(), "b8c6".into()],
            ..SearchInfo::default()
        };
        let mut snap = TelemetrySnapshot::default();
        apply_search_info(&mut snap, &info, "UCI");
        assert_eq!(snap.hashfull, 400);
        assert_eq!(snap.pv.len(), 2);
        assert!(snap.label.contains("g1f3"));
    }

    #[test]
    fn next_search_is_scheduled_outside_the_completion_callback() {
        let src = include_str!("engine.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(production.contains("exec_after"));
        assert!(production.contains("handles.ui_scope"));
        assert!(!production.contains("Scope::current()"));
    }
}
