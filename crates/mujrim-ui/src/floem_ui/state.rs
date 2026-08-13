//! Reactive application state shared across Floem views.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use floem::ext_event::ArcRwSignal;
use floem::prelude::{RwSignal, SignalGet, SignalUpdate};
use mujrim_protocols::catalog::DiscoveredEngine;
use mujrim_study::annotation::MoveAnnotation;
use mujrim_study::database::{EngineMetadata, GameSummary, StudyDatabase};
use mujrim_study::opening::{OpeningExplorer, PrepSide, SavedLine};
use mujrim_study::tournament_store::StoredTournament;
use mujrim_study::training_store::{TrainingItem, TrainingStore};
use types::Square;

use crate::app_core::analysis::AnalysisSnapshot;
use crate::app_core::audio::{BgmTrack, SoundEngine};
use crate::app_core::engine::{EngineConfig, GameMode, PlayerConfig};
use crate::app_core::game::GameState;
use crate::app_core::hub::CoinFlipState;
use crate::app_core::layout::DockTab;
use crate::app_core::match_controller;
use crate::app_core::motion::AnimPace;
use crate::app_core::pieces::PieceAssets;
use crate::app_core::recording::RecordingEngine;
use crate::app_core::settings::{AppSettings, OptionsTab, Screen};
use crate::app_core::tournament_live::{LiveTournamentHandle, LiveTournamentSnapshot};
use crate::app_core::tournament_setup::TournamentSetup;

#[derive(Clone, Copy)]
pub struct SlideAnim {
    pub from: Square,
    pub to: Square,
    pub captured: bool,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub screen: RwSignal<Screen>,
    pub game: RwSignal<Option<GameState>>,
    pub game_generation: RwSignal<u64>,
    pub searching: RwSignal<bool>,
    pub selected_mode: RwSignal<GameMode>,
    pub white_player: RwSignal<PlayerConfig>,
    pub black_player: RwSignal<PlayerConfig>,
    pub engine_cfg: RwSignal<EngineConfig>,
    pub settings: RwSignal<AppSettings>,
    pub show_options: RwSignal<bool>,
    pub options_tab: RwSignal<OptionsTab>,
    pub show_tournament_setup: RwSignal<bool>,
    pub move_log: RwSignal<Vec<String>>,
    pub move_annotations: RwSignal<Vec<Option<MoveAnnotation>>>,
    pub analysis_scores: RwSignal<Vec<Option<i32>>>,
    pub review_ply: RwSignal<Option<usize>>,
    pub initial_fen: RwSignal<String>,
    pub status: RwSignal<String>,
    pub analysis: RwSignal<Option<AnalysisSnapshot>>,
    pub slide: RwSignal<Option<SlideAnim>>,
    pub slide_t: RwSignal<f32>,
    pub capture_burst: RwSignal<f32>,
    pub board_geom: RwSignal<crate::app_core::layout::BoardGeom>,
    pub study_query: RwSignal<String>,
    pub study_results: RwSignal<Vec<GameSummary>>,
    pub opening_indexed: RwSignal<usize>,
    pub training_due: RwSignal<Vec<TrainingItem>>,
    pub active_puzzle: RwSignal<Option<TrainingItem>>,
    pub puzzle_line: RwSignal<Vec<String>>,
    pub tournament_setup: RwSignal<TournamentSetup>,
    pub tournament_snapshot: RwSignal<LiveTournamentSnapshot>,
    pub tournament_status: RwSignal<String>,
    pub selected_tournament_game_id: RwSignal<Option<usize>>,
    pub dock_open: RwSignal<bool>,
    pub dock_tab: RwSignal<DockTab>,
    pub syzygy_status: RwSignal<String>,
    pub nnue_status: RwSignal<String>,
    pub tuning_status: RwSignal<String>,
    pub syzygy_piece_set: RwSignal<updater::syzygy::SyzygyPieceSet>,
    pub recording_label: RwSignal<String>,
    pub hub_progress: RwSignal<f32>,
    pub coin_flip: RwSignal<CoinFlipState>,
    pub engine_retries: RwSignal<u8>,
    pub bgm_on: RwSignal<bool>,
    pub tournament_event: RwSignal<String>,
    pub tournament_site: RwSignal<String>,
    pub analysis_engines_selected: RwSignal<Vec<String>>,
    pub analysis_multipv: RwSignal<i32>,
    pub active_gambit_id: RwSignal<Option<String>>,
    pub gambit_ply: RwSignal<usize>,
    pub saved_lines: RwSignal<Vec<SavedLine>>,
    pub line_name: RwSignal<String>,
    pub prep_notes: RwSignal<String>,
    pub prep_side: RwSignal<PrepSide>,
    pub move_note: RwSignal<String>,
    pub tournament_history: RwSignal<Vec<StoredTournament>>,
    pub current_tournament_id: RwSignal<Option<String>>,
    pub persisted_tournament_games: RwSignal<usize>,
    pub resume_prompt:
        RwSignal<Option<crate::app_core::tournament_resume::ActiveTournamentCheckpoint>>,
}

#[derive(Clone)]
pub struct AppHandles {
    pub assets: Rc<PieceAssets>,
    pub sound: Rc<RefCell<Option<SoundEngine>>>,
    pub recorder: RecordingEngine,
    pub study: Rc<RefCell<Option<StudyDatabase>>>,
    pub training: Rc<RefCell<Option<TrainingStore>>>,
    pub explorer: Rc<RefCell<OpeningExplorer>>,
    pub tournament: Rc<RefCell<Option<LiveTournamentHandle>>>,
    pub bundled: Vec<DiscoveredEngine>,
    pub catalog: Rc<RefCell<Vec<EngineMetadata>>>,
    pub telemetry: ArcRwSignal<crate::app_core::engine::TelemetrySnapshot>,
    pub logo: Vec<u8>,
    pub chess_bg: Vec<u8>,
    pub ui_scope: floem::reactive::Scope,
    #[cfg(feature = "book")]
    pub book: Rc<Option<search::book::OpeningBook>>,
}

impl AppState {
    pub fn boot() -> (Self, AppHandles) {
        types::init();
        let settings = AppSettings::load();
        let sound = SoundEngine::new();
        let mut sound_cell = sound;
        if let Some(engine) = sound_cell.as_mut() {
            engine.set_volume(settings.bgm_volume as f32 / 100.0);
            engine.set_mood(settings.game_mood);
            engine.set_sound_theme(settings.sound_theme);
            engine.play_bgm(BgmTrack::Menu);
        }
        let bundled = mujrim_protocols::catalog::discover_bundled_engines_from_environment()
            .unwrap_or_default();
        let black = crate::app_core::logic::players_for_detected_engines(
            GameMode::HumanVsEngine,
            &bundled,
            &[],
        )
        .1;
        let state = Self {
            screen: RwSignal::new(Screen::Menu),
            game: RwSignal::new(None),
            game_generation: RwSignal::new(0),
            searching: RwSignal::new(false),
            selected_mode: RwSignal::new(GameMode::HumanVsEngine),
            white_player: RwSignal::new(PlayerConfig::Human),
            black_player: RwSignal::new(black),
            engine_cfg: RwSignal::new(EngineConfig::default()),
            settings: RwSignal::new(settings),
            show_options: RwSignal::new(false),
            options_tab: RwSignal::new(OptionsTab::Settings),
            show_tournament_setup: RwSignal::new(false),
            move_log: RwSignal::new(Vec::new()),
            move_annotations: RwSignal::new(Vec::new()),
            analysis_scores: RwSignal::new(Vec::new()),
            review_ply: RwSignal::new(None),
            initial_fen: RwSignal::new(mujrim_study::opening::START_FEN.to_owned()),
            status: RwSignal::new("Ready.".to_owned()),
            analysis: RwSignal::new(None),
            slide: RwSignal::new(None),
            slide_t: RwSignal::new(1.0),
            capture_burst: RwSignal::new(0.0),
            board_geom: RwSignal::new(crate::app_core::layout::BoardGeom::default()),
            study_query: RwSignal::new(String::new()),
            study_results: RwSignal::new(Vec::new()),
            opening_indexed: RwSignal::new(0),
            training_due: RwSignal::new(Vec::new()),
            active_puzzle: RwSignal::new(None),
            puzzle_line: RwSignal::new(Vec::new()),
            tournament_setup: RwSignal::new(TournamentSetup::default()),
            tournament_snapshot: RwSignal::new(LiveTournamentSnapshot::default()),
            tournament_status: RwSignal::new(String::new()),
            selected_tournament_game_id: RwSignal::new(None),
            dock_open: RwSignal::new(false),
            dock_tab: RwSignal::new(DockTab::Histogram),
            syzygy_status: RwSignal::new(String::new()),
            nnue_status: RwSignal::new(String::new()),
            tuning_status: RwSignal::new(String::new()),
            syzygy_piece_set: RwSignal::new(updater::syzygy::SyzygyPieceSet::Standard),
            recording_label: RwSignal::new("Record".to_owned()),
            hub_progress: RwSignal::new(0.0),
            coin_flip: RwSignal::new(CoinFlipState::Idle),
            engine_retries: RwSignal::new(match_controller::DEFAULT_ENGINE_RETRIES),
            bgm_on: RwSignal::new(true),
            tournament_event: RwSignal::new("Mujrim Tournament".to_owned()),
            tournament_site: RwSignal::new(String::new()),
            analysis_engines_selected: RwSignal::new(vec!["builtin".to_owned()]),
            analysis_multipv: RwSignal::new(3),
            active_gambit_id: RwSignal::new(None),
            gambit_ply: RwSignal::new(0),
            saved_lines: RwSignal::new(Vec::new()),
            line_name: RwSignal::new("New preparation".to_owned()),
            prep_notes: RwSignal::new(String::new()),
            prep_side: RwSignal::new(PrepSide::White),
            move_note: RwSignal::new(String::new()),
            tournament_history: RwSignal::new(Vec::new()),
            current_tournament_id: RwSignal::new(None),
            persisted_tournament_games: RwSignal::new(0),
            resume_prompt: RwSignal::new(None),
        };
        let study_path = crate::app_core::logic::study_database_path();
        let study = StudyDatabase::open(&study_path).ok();
        let training_path = study_path.parent().map_or_else(
            || PathBuf::from("training"),
            |parent| parent.join("training"),
        );
        let mut training = TrainingStore::open(training_path).ok();
        if let Some(store) = training.as_mut() {
            let _ = crate::app_core::logic::seed_training(store);
        }
        let due = training
            .as_ref()
            .map(|store| store.due(crate::app_core::logic::today_day(), 32))
            .unwrap_or_default();
        state.training_due.set(due);
        if let Some(database) = study.as_ref() {
            if let Ok(lines) = database.list_lines() {
                state.saved_lines.set(lines);
            }
            if let Ok(history) = database.list_tournaments() {
                state.tournament_history.set(history);
            }
        }
        let handles = AppHandles {
            assets: Rc::new(PieceAssets::load()),
            sound: Rc::new(RefCell::new(sound_cell)),
            recorder: RecordingEngine::new(),
            study: Rc::new(RefCell::new(study)),
            training: Rc::new(RefCell::new(training)),
            explorer: Rc::new(RefCell::new(OpeningExplorer::default())),
            tournament: Rc::new(RefCell::new(None)),
            bundled,
            catalog: Rc::new(RefCell::new(Vec::new())),
            telemetry: ArcRwSignal::new(crate::app_core::engine::TelemetrySnapshot::default()),
            logo: include_bytes!("../../../../assets/branding/mujrim-icon.png").to_vec(),
            chess_bg: crate::app_core::noise::chess_blur_background(512, 384).bytes,
            ui_scope: floem::reactive::Scope::current(),
            #[cfg(feature = "book")]
            book: Rc::new(search::book::OpeningBook::load_embedded().ok()),
        };
        (state, handles)
    }

    pub fn persist_settings(self) {
        self.settings.get_untracked().save();
    }

    pub fn anim_pace(self) -> AnimPace {
        AnimPace::from_setting(self.settings.get_untracked().anim_speed)
    }
}
