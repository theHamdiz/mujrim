#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]
#![allow(unexpected_cfgs)]
//! Mujrim GUI — premium chess interface.

mod analysis;
mod arrows;
mod audio;
mod board_view;
mod eval_graph;
mod game;
mod gif_export;
mod motion;
mod noise;
mod pieces;
mod premove;
mod recording;
mod tournament_arena;
mod tournament_live;
mod tournament_results;
mod tournament_setup;
mod uci_process;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::widget::{
    Image, Space, button, column, container, mouse_area, pick_list, row, scrollable, slider, text,
    text_input, toggler,
};
use iced::{Alignment, Color, Element, Font, Length, Subscription, Task, Theme};
use motion::AnimPace;
use mujrim_protocols::catalog::{
    DiscoveredEngine, RuntimeCompatibility, discover_bundled_engines_from_environment,
};
use mujrim_study::annotation::{AnnotationContext, MoveAnnotation};
use mujrim_study::board_marks::BoardArrow;
use mujrim_study::database::{EngineMetadata, GameMetadata, GameQuery, GameSummary, StudyDatabase};
use mujrim_study::gambit::{self, GambitLesson};
use mujrim_study::opening::OpeningExplorer;
use mujrim_study::tournament::{Entrant, TournamentFormat};
use mujrim_study::tournament_store::StoredTournament;
use mujrim_study::training::Puzzle;
use mujrim_study::training_store::{TrainingItem, TrainingStore};

use pieces::PieceAssets;
use uci_process::ExternalEngineProtocol;

/// Custom display font embedded from assets.
#[allow(dead_code)]
const CURIOUS_FONT_BYTES: &[u8] = include_bytes!("../assets/CuriousTrack.ttf");
const CURIOUS_FONT: Font = Font::with_name("Curious Track");
const MAX_GUI_HASH_MB: i32 = 512;

fn bounded_hash_mb(value: i32) -> usize {
    value.clamp(1, MAX_GUI_HASH_MB) as usize
}

// ──────────────────────────────────────────────────────────────
// Colors — fallback constants (themes override via GuiPalette)
#[allow(dead_code)]
// ──────────────────────────────────────────────────────────────
const BG_DARK: Color = Color::from_rgb(0.102, 0.102, 0.180);
#[allow(dead_code)]
const BG_PANEL: Color = Color::from_rgb(0.086, 0.129, 0.243);
#[allow(dead_code)]
const BG_SIDEBAR: Color = Color::from_rgb(0.059, 0.204, 0.376);
#[allow(dead_code)]
const TEXT_PRIMARY: Color = Color::from_rgb(0.96, 0.96, 0.96);
#[allow(dead_code)]
const TEXT_SECONDARY: Color = Color::from_rgb(0.627, 0.627, 0.690);
const ACCENT: Color = Color::from_rgb(0.914, 0.271, 0.376);
const ACCENT_TEAL: Color = Color::from_rgb(0.325, 0.749, 0.616);
const ACCENT_GOLD: Color = Color::from_rgb(0.706, 0.569, 0.235);
#[allow(dead_code)]
const BORDER_SUBTLE: Color = Color::from_rgb(0.16, 0.18, 0.28);

fn theme_fn(_: &App) -> Theme {
    Theme::Dark
}

fn main() -> iced::Result {
    let mut win_settings = main_window_settings();
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../../../assets/branding/mujrim-icon.png"),
        None,
    )
    .ok();
    if let Some(icon) = icon {
        win_settings.icon = Some(icon);
    }

    iced::application(App::boot, App::update, App::view)
        .title("Mujrim Chess")
        .subscription(App::subscription)
        .theme(theme_fn)
        .window_size((1280.0, 850.0))
        .window(win_settings)
        .run()
}

fn main_window_settings() -> iced::window::Settings {
    iced::window::Settings {
        decorations: false,
        resizable: true,
        transparent: false,
        min_size: Some(iced::Size::new(800.0, 600.0)),
        ..Default::default()
    }
}

/// Set the macOS Dock / launcher icon to the embedded logo at runtime.
#[cfg(target_os = "macos")]
fn set_macos_dock_icon() {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        // Ensure the app shows in Dock (important for non-.app binaries)
        let nsapp_class = Class::get("NSApplication").unwrap();
        let app: *mut Object = msg_send![nsapp_class, sharedApplication];
        // NSApplicationActivationPolicyRegular = 0
        let _: () = msg_send![app, setActivationPolicy: 0i64];

        let png_data: &[u8] = include_bytes!("../../../assets/branding/mujrim-icon.png");

        // NSData from bytes
        let nsdata_class = Class::get("NSData").unwrap();
        let data: *mut Object = msg_send![nsdata_class, alloc];
        let data: *mut Object = msg_send![data, initWithBytes:png_data.as_ptr()
                                                       length:png_data.len()];

        // NSImage from data
        let nsimage_class = Class::get("NSImage").unwrap();
        let image: *mut Object = msg_send![nsimage_class, alloc];
        let image: *mut Object = msg_send![image, initWithData:data];

        if !image.is_null() {
            // [app setApplicationIconImage:image]
            let _: () = msg_send![app, setApplicationIconImage:image];
        }
    }
}

/// Engine configuration.
#[derive(Debug, Clone)]
struct EngineConfig {
    hash_mb: i32,
    threads: i32,
    max_depth: i32,
    time_per_move: i32,
    ponder: bool,
    use_book: bool,
    use_nnue: bool,
    eval_file: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            hash_mb: 64,
            threads: 1,
            max_depth: 64,
            time_per_move: 3,
            ponder: false,
            use_book: true,
            use_nnue: true,
            eval_file: None,
        }
    }
}

/// Style for capture animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CaptureAnimStyle {
    Instant,
    Explosion,
    Fire,
}

impl std::fmt::Display for CaptureAnimStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Instant => write!(f, "Instant"),
            Self::Explosion => write!(f, "Explosion"),
            Self::Fire => write!(f, "Fire"),
        }
    }
}

/// Position of board coordinate labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CoordPosition {
    Inside,
    Outside,
}

impl std::fmt::Display for CoordPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inside => write!(f, "Inside"),
            Self::Outside => write!(f, "Outside"),
        }
    }
}

/// User-facing application settings (persisted in options modal).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct AppSettings {
    board_theme: board_view::BoardTheme,
    piece_set: pieces::PieceSet,
    show_coords: bool,
    /// Animation speed multiplier: 0=fast 1=normal 2=slow.
    anim_speed: i32,
    sfx_on: bool,
    bgm_volume: i32, // 0–100
    game_mood: audio::GameMood,
    sound_theme: audio::SoundTheme,
    auto_flip_black: bool,
    show_legal_moves: bool,
    show_last_move: bool,
    premoves_enabled: bool,
    capture_anim_style: CaptureAnimStyle,
    coord_position: CoordPosition,
    multi_premoves: bool,
    draw_arrows: bool,
    arrow_shape: arrows::ArrowShape,
    arrow_color: arrows::ArrowColor,
    arrow_size: arrows::ArrowSize,
    /// Enable interpolated piece slide overlays.
    piece_slide: bool,
    /// Enable hub / modal entrance motion.
    system_motion: bool,
    /// Draw last-move as a solid arrow in addition to square tint.
    last_move_arrow: bool,
    /// Draw ponder suggestion as a translucent arrow.
    ponder_arrow: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            board_theme: board_view::BoardTheme::Classic,
            piece_set: pieces::PieceSet::Cburnett,
            show_coords: true,
            anim_speed: 1,
            sfx_on: true,
            bgm_volume: 50,
            game_mood: audio::GameMood::Mystique,
            sound_theme: audio::SoundTheme::Wood,
            auto_flip_black: false,
            show_legal_moves: true,
            show_last_move: true,
            premoves_enabled: true,
            capture_anim_style: CaptureAnimStyle::Explosion,
            coord_position: CoordPosition::Inside,
            multi_premoves: true,
            draw_arrows: true,
            arrow_shape: arrows::ArrowShape::Smart,
            arrow_color: arrows::ArrowColor::Orange,
            arrow_size: arrows::ArrowSize::Normal,
            piece_slide: true,
            system_motion: true,
            last_move_arrow: true,
            ponder_arrow: true,
        }
    }
}

impl AppSettings {
    fn config_path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("mujrim");
        p.push("settings.toml");
        p
    }

    fn load() -> Self {
        let path = Self::config_path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            toml::from_str(&contents).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, toml_str);
        }
    }
}

struct App {
    screen: Screen,
    game: Option<game::GameState>,
    game_generation: u64,
    /// Automatic engine-search retries remaining after a protocol/spawn failure.
    engine_move_retries: u8,
    selected_mode: GameMode,
    white_player: PlayerConfig,
    black_player: PlayerConfig,
    engine_cfg: EngineConfig,
    bundled_engines: Vec<DiscoveredEngine>,
    external_engine_catalog: Vec<EngineMetadata>,
    study_database: Option<StudyDatabase>,
    study_query: String,
    study_results: Vec<GameSummary>,
    training_store: Option<TrainingStore>,
    training_due: Vec<TrainingItem>,
    active_puzzle: Option<TrainingItem>,
    opening_explorer: OpeningExplorer,
    opening_indexed_games: usize,
    settings: AppSettings,
    show_options: bool,
    options_tab: OptionsTab,
    options_offset: iced::Vector,
    options_drag: Option<(iced::Point, iced::Vector)>,
    cursor_position: iced::Point,
    move_log: Vec<String>,
    move_annotations: Vec<Option<MoveAnnotation>>,
    analysis_scores_cp: Vec<Option<i32>>,
    review_board: Option<types::Board>,
    review_ply: Option<usize>,
    initial_fen: String,
    status: String,
    engine_info: String,
    assets: PieceAssets,
    _bg_pattern: iced::widget::image::Handle,
    chess_bg: iced::widget::image::Handle,
    /// Subtle noise grain overlay for material-textured feel.
    _panel_grain: iced::widget::image::Handle,
    logo: iced::widget::image::Handle,
    #[cfg(feature = "book")]
    book: Option<search::book::OpeningBook>,
    sound: Option<audio::SoundEngine>,
    animation: Option<AnimationState>,
    window_width: f32,
    window_height: f32,
    bgm_on: bool,
    coin_flip: CoinFlipState,
    recorder: recording::RecordingEngine,
    window_id: Option<iced::window::Id>,
    // Syzygy state
    syzygy_status: String,
    syzygy_wdl_count: usize,
    syzygy_dtz_count: usize,
    syzygy_piece_set: updater::syzygy::SyzygyPieceSet,
    // NNUE network state
    nnue_status: String,
    nnue_installed_count: usize,
    // Tuning state
    tuning_params: Option<updater::tuning::TunableParams>,
    tuning_status: String,
    tournament_format: TournamentFormat,
    tournament_status: String,
    stored_tournaments: Vec<StoredTournament>,
    live_tournament: Option<tournament_live::LiveTournamentHandle>,
    live_tournament_view: tournament_live::LiveTournamentSnapshot,
    selected_tournament_id: Option<String>,
    selected_tournament_game_id: Option<usize>,
    tournament_review_active: bool,
    tournament_setup: tournament_setup::TournamentSetup,
    show_tournament_setup: bool,
    tournament_setup_offset: iced::Vector,
    tournament_setup_drag: Option<(iced::Point, iced::Vector)>,
    show_tournament_results: bool,
    /// Latest multi-engine analysis arrows for the analysis/review board.
    analysis_arrows: Vec<BoardArrow>,
    analysis_status: String,
    analysis_engines_selected: Vec<String>,
    analysis_multipv: i32,
    ponder_hint: Option<(types::Square, types::Square)>,
    active_gambit: Option<&'static GambitLesson>,
    gambit_ply: usize,
    hub_opened_at: Instant,
}

/// State of an in-progress piece move animation.
struct AnimationState {
    /// The move being animated.
    mv: types::Move,
    /// Piece being moved.
    _piece: types::Piece,
    /// Color of piece being moved.
    _color: types::Color,
    /// Captured piece (if any) for fade-out.
    captured: Option<(types::Piece, types::Color)>,
    /// Whether this was a capture.
    is_capture: bool,
    /// Animation start time.
    start: Instant,
    /// Duration of the animation.
    duration: Duration,
    /// Info string from engine (if engine move).
    engine_info: Option<String>,
    /// Whether to trigger the engine after animation completes.
    trigger_engine_after: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnalyzedPly {
    annotation: MoveAnnotation,
    /// Evaluation after the move, normalized to White's perspective.
    score_cp: i32,
}

#[derive(Clone, Debug)]
struct QuickTournamentEngine {
    name: String,
    path: PathBuf,
    search_limits: mujrim_protocols::catalog::SearchLimitSupport,
}

/// Coin flip animation state.
#[derive(Debug, Clone)]
enum CoinFlipState {
    /// No flip in progress.
    Idle,
    /// Flipping — cycling between W/B display.
    Flipping {
        start: Instant,
        /// The final result: true = heads (current player is White), false = tails (swap).
        result: bool,
    },
    /// Done — shows the result.
    Done(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Playing,
    Study,
    Tournaments,
    Analysis,
}

/// Tab within the options modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionsTab {
    Settings,
    Tools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameMode {
    HumanVsHuman,
    HumanVsEngine,
    EngineVsEngine,
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HumanVsHuman => write!(f, "Human vs Human"),
            Self::HumanVsEngine => write!(f, "Human vs Engine"),
            Self::EngineVsEngine => write!(f, "Engine vs Engine"),
        }
    }
}

#[derive(Debug, Clone)]
enum PlayerConfig {
    Human,
    BuiltIn {
        depth: i32,
    },
    External {
        path: String,
        protocol: ExternalEngineProtocol,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundledEngineChoice {
    index: usize,
    label: String,
}

impl std::fmt::Display for BundledEngineChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn bundled_engine_label(engine: &DiscoveredEngine) -> String {
    let execution = match engine.compatibility {
        RuntimeCompatibility::Native => "native",
        RuntimeCompatibility::Emulated => "x64 emulation",
    };
    format!("{} ({execution})", engine.display_name)
}

fn bundled_engine_choices(engines: &[DiscoveredEngine]) -> Vec<BundledEngineChoice> {
    engines
        .iter()
        .enumerate()
        .map(|(index, engine)| BundledEngineChoice {
            index,
            label: bundled_engine_label(engine),
        })
        .collect()
}

fn selected_bundled_engine(
    engines: &[DiscoveredEngine],
    player: &PlayerConfig,
) -> Option<BundledEngineChoice> {
    let PlayerConfig::External { path, .. } = player else {
        return None;
    };
    engines
        .iter()
        .position(|engine| engine.path == std::path::Path::new(path))
        .map(|index| BundledEngineChoice {
            index,
            label: bundled_engine_label(&engines[index]),
        })
}

impl std::fmt::Display for PlayerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "Human"),
            Self::BuiltIn { depth } => write!(f, "Mujrim (depth {depth})"),
            Self::External { path, protocol } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                write!(f, "{protocol}: {name}")
            }
        }
    }
}

type EngineMoveOk = (types::Move, String, Option<(types::Square, types::Square)>);

#[derive(Debug, Clone)]
enum Msg {
    SelectMode(GameMode),
    StartGame,
    OpenHome,
    OpenStudy,
    OpenTournaments,
    OpenAnalysis,
    SelectTournamentFormat(TournamentFormat),
    RunQuickTournament,
    CancelTournament,
    TournamentTick(Instant),
    QuickTournamentFinished(Box<mujrim_benchmarker::strength::TournamentSummary>),
    SelectTournament(String),
    SelectTournamentGame(usize),
    ToggleTournamentResults,
    TournamentEventNameChanged(String),
    TournamentSiteChanged(String),
    TournamentGamesPerEncounterChanged(i32),
    TournamentHashChanged(i32),
    TournamentThreadsChanged(i32),
    TournamentTimeControlChanged(tournament_setup::TimeControlPreset),
    TournamentToggleEngine(String),
    ToggleTournamentSetup,
    StartTournamentSetupDrag,
    StopTournamentSetupDrag,
    EngineCatalogProbed(Vec<EngineMetadata>),
    AnalyzeGame,
    GameAnalysisFinished(Result<Vec<AnalyzedPly>, String>),
    RunMultiEngineAnalysis,
    MultiEngineAnalysisFinished(analysis::AnalysisSnapshot),
    ToggleAnalysisEngine(String),
    SetAnalysisMultiPv(i32),
    ViewPly(usize),
    ReturnToLivePosition,
    StartGambitLesson(String),
    GambitStep(i32),
    RefreshTournamentHistory,
    LoadWhiteUciEngine,
    LoadWhiteXboardEngine,
    LoadBlackUciEngine,
    LoadBlackXboardEngine,
    WhiteEngineSelected(Option<String>, ExternalEngineProtocol),
    BlackEngineSelected(Option<String>, ExternalEngineProtocol),
    SelectBundledWhite(BundledEngineChoice),
    SelectBundledBlack(BundledEngineChoice),
    BoardClick(usize, usize),
    EngineMove(u64, Result<EngineMoveOk, String>),
    NewGame,
    FlipBoard,
    Resign,
    ExportPGN,
    SaveToLibrary,
    ImportPgn,
    PgnSelected(Option<String>),
    StudyQueryChanged(String),
    SearchLibrary,
    LoadLibraryGame(String),
    SeedTraining,
    StartTraining(String),
    GradeTraining(u8),
    StudyOpeningMove(String),
    OpeningIndexFinished(OpeningExplorer, usize),
    ExportGIF,
    ExitApp,
    #[allow(dead_code)]
    EngineInfo(String),
    AnimTick(Instant),
    ToggleBGM,
    CoinFlip,
    CoinFlipTick(Instant),
    HubTick(Instant),
    ToggleRecording,
    RecordCaptureTick,
    TakeScreenshot,
    ScreenshotDone(String),
    GifExportDone(String),
    RecordingSaved(String),
    // Engine config
    CfgHashChanged(i32),
    CfgThreadsChanged(i32),
    CfgDepthChanged(i32),
    CfgTimeChanged(i32),
    CfgTogglePonder,
    CfgToggleBook,
    CfgToggleNnue,
    LoadBuiltinEvalFile,
    BuiltinEvalFileSelected(Option<String>),
    ClearBuiltinEvalFile,
    // Options modal
    ToggleOptions,
    StartOptionsDrag,
    StopOptionsDrag,
    CursorMoved(iced::Point),
    SetBoardTheme(board_view::BoardTheme),
    SetPieceSet(pieces::PieceSet),
    SetShowCoords(bool),
    SetAnimSpeed(i32),
    SetSfx(bool),
    SetBgmVolume(i32),
    SetGameMood(audio::GameMood),
    SetSoundTheme(audio::SoundTheme),
    SetAutoFlip(bool),
    SetShowLegal(bool),
    SetShowLastMove(bool),
    SetPremoves(bool),
    SetCaptureAnim(CaptureAnimStyle),
    SetCoordPosition(CoordPosition),
    SetMultiPremoves(bool),
    SetDrawArrows(bool),
    SetArrowShape(arrows::ArrowShape),
    SetArrowColor(arrows::ArrowColor),
    SetArrowSize(arrows::ArrowSize),
    BoardRightDown(usize, usize),
    BoardRightUp(usize, usize),
    BoardPointerDown(usize, usize),
    BoardPointerMove(usize, usize),
    BoardPointerUp(usize, usize),
    SetPieceSlide(bool),
    SetSystemMotion(bool),
    SetLastMoveArrow(bool),
    SetPonderArrow(bool),
    // Tools panel
    SwitchOptionsTab(OptionsTab),
    SelectSyzygyPieceSet(updater::syzygy::SyzygyPieceSet),
    SyzygyDownload,
    SyzygyDownloadDone(String),
    NnueDownload,
    NnueDownloadDone(String),
    TuneLoad,
    TuneSetParam(String, String, f64),
    TuneSave,
    CheckForUpdates,
    DragWindow,
    ResizeWindow(iced::window::Direction),
    MinimizeWindow,
    ToggleMaximizeWindow,
    WindowOpened(iced::window::Id),
    WindowResized(iced::window::Id, iced::Size),
    #[allow(dead_code)]
    FontLoaded,
}

impl Default for App {
    fn default() -> Self {
        // Set macOS Dock icon (must happen after NSApplication exists)
        #[cfg(target_os = "macos")]
        set_macos_dock_icon();

        let settings = AppSettings::load();
        let mut sound = audio::SoundEngine::new();
        if let Some(ref mut s) = sound {
            s.set_volume(settings.bgm_volume as f32 / 100.0);
            s.set_mood(settings.game_mood);
            s.set_sound_theme(settings.sound_theme);
            s.play_bgm(audio::BgmTrack::Menu);
        }
        let study_database = StudyDatabase::open(study_database_path()).ok();
        let external_engine_catalog = study_database
            .as_ref()
            .and_then(|database| database.engine_catalog().ok())
            .unwrap_or_default();
        let stored_tournaments = study_database
            .as_ref()
            .and_then(|database| database.list_tournaments().ok())
            .unwrap_or_default();
        Self {
            screen: Screen::Menu,
            game: None,
            game_generation: 0,
            engine_move_retries: 1,
            selected_mode: GameMode::HumanVsHuman,
            white_player: PlayerConfig::Human,
            black_player: PlayerConfig::Human,
            engine_cfg: EngineConfig::default(),
            bundled_engines: discover_bundled_engines_from_environment().unwrap_or_default(),
            external_engine_catalog,
            study_database,
            study_query: String::new(),
            study_results: Vec::new(),
            training_store: TrainingStore::open(training_database_path()).ok(),
            training_due: Vec::new(),
            active_puzzle: None,
            opening_explorer: OpeningExplorer::default(),
            opening_indexed_games: 0,
            settings,
            show_options: false,
            options_tab: OptionsTab::Settings,
            options_offset: iced::Vector::new(0.0, 0.0),
            options_drag: None,
            cursor_position: iced::Point::ORIGIN,
            move_log: Vec::new(),
            move_annotations: Vec::new(),
            analysis_scores_cp: Vec::new(),
            review_board: None,
            review_ply: None,
            initial_fen: mujrim_study::opening::START_FEN.to_owned(),
            status: String::from("Welcome to Mujrim!"),
            engine_info: String::new(),
            assets: PieceAssets::load(),
            _bg_pattern: noise::pharaonic_pattern(256),
            chess_bg: noise::chess_blur_background(512, 384),
            _panel_grain: noise::macos_grain_panel(),
            logo: iced::widget::image::Handle::from_bytes(
                include_bytes!("../../../assets/branding/mujrim-icon.png").as_slice(),
            ),
            #[cfg(feature = "book")]
            book: search::book::OpeningBook::load_embedded().ok(),
            sound,
            animation: None,
            window_width: 1280.0,
            window_height: 850.0,
            bgm_on: true,
            coin_flip: CoinFlipState::Idle,
            recorder: recording::RecordingEngine::new(),
            window_id: None,
            syzygy_status: String::new(),
            syzygy_wdl_count: 0,
            syzygy_dtz_count: 0,
            syzygy_piece_set: updater::syzygy::SyzygyPieceSet::Standard,
            nnue_status: String::new(),
            nnue_installed_count: 0,
            tuning_params: None,
            tuning_status: String::new(),
            tournament_format: TournamentFormat::RoundRobin,
            tournament_status: String::new(),
            stored_tournaments,
            live_tournament: None,
            live_tournament_view: tournament_live::LiveTournamentSnapshot::default(),
            selected_tournament_id: None,
            selected_tournament_game_id: None,
            tournament_review_active: false,
            tournament_setup: tournament_setup::TournamentSetup::default(),
            show_tournament_setup: true,
            tournament_setup_offset: iced::Vector::new(0.0, 0.0),
            tournament_setup_drag: None,
            show_tournament_results: false,
            analysis_arrows: Vec::new(),
            analysis_status: "Pick engines and run multi-engine analysis.".to_owned(),
            analysis_engines_selected: vec!["builtin".to_owned()],
            analysis_multipv: 2,
            ponder_hint: None,
            active_gambit: None,
            gambit_ply: 0,
            hub_opened_at: Instant::now(),
        }
    }
}

impl App {
    fn invalidate_engine_tasks(&mut self) {
        self.game_generation = self.game_generation.wrapping_add(1);
        self.engine_move_retries = 1;
        uci_process::cancel_all_pondering();
    }

    fn clear_review_state(&mut self) {
        self.analysis_scores_cp.clear();
        self.review_board = None;
        self.review_ply = None;
    }

    fn load_tournament_game(&mut self, game: &tournament_live::PlayedGame) -> Result<(), String> {
        self.invalidate_engine_tasks();
        self.animation = None;
        self.active_puzzle = None;
        self.selected_tournament_game_id = Some(game.id);
        self.tournament_review_active = true;
        self.initial_fen = game.initial_fen.clone();
        self.move_log = game.moves.clone();
        self.move_annotations = vec![None; game.moves.len()];
        self.clear_review_state();
        let mut state = replay_study_game(&game.initial_fen, &game.moves)?;
        state.refresh_move_overlays(self.settings.show_last_move, None, &[]);
        self.game = Some(state);
        self.engine_info = format!(
            "{} (White) vs {} (Black)\nResult {}",
            game.white,
            game.black,
            game.result_label()
        );
        self.status = format!("Tournament game · {}", game.title());
        Ok(())
    }

    fn load_live_tournament_board(
        &mut self,
        live: &tournament_live::LiveGameBoard,
    ) -> Result<(), String> {
        if self.tournament_review_active
            && self.selected_tournament_game_id.is_none()
            && self.initial_fen == live.initial_fen
            && self.move_log == live.moves
        {
            self.engine_info = format!(
                "{} (White) vs {} (Black)\nLive · {} · d{} · {} nodes",
                live.white,
                live.black,
                tournament_arena::score_text(live.score_cp),
                live.depth,
                live.nodes
            );
            return Ok(());
        }
        self.invalidate_engine_tasks();
        self.animation = None;
        self.active_puzzle = None;
        self.selected_tournament_game_id = None;
        self.tournament_review_active = true;
        self.initial_fen = live.initial_fen.clone();
        self.move_log = live.moves.clone();
        self.move_annotations = vec![None; live.moves.len()];
        self.clear_review_state();
        let mut state = replay_study_game(&live.initial_fen, &live.moves)?;
        state.refresh_move_overlays(self.settings.show_last_move, None, &[]);
        self.game = Some(state);
        self.engine_info = format!(
            "{} (White) vs {} (Black)\nLive · {} · d{} · {} nodes",
            live.white,
            live.black,
            tournament_arena::score_text(live.score_cp),
            live.depth,
            live.nodes
        );
        self.status = format!(
            "Live · R{} · {} vs {} · ply {}",
            live.round,
            live.white,
            live.black,
            live.moves.len()
        );
        Ok(())
    }

    fn sync_tournament_arena_board(&mut self) -> Task<Msg> {
        if self.live_tournament_view.running {
            let visible = tournament_arena::visible_live_boards(
                &self.live_tournament_view.live_games,
                self.tournament_setup.concurrency as usize,
            );
            if let Some(live) = visible.last().cloned()
                && let Err(error) = self.load_live_tournament_board(&live)
            {
                self.tournament_status = format!("Could not open live board: {error}");
            }
            // Never auto-follow/analyze finished boards while a tournament is live —
            // stacking review workers was crashing the UI under engine load.
            return Task::none();
        }
        self.follow_latest_tournament_game()
    }

    fn follow_latest_tournament_game(&mut self) -> Task<Msg> {
        let Some(latest) = self.live_tournament_view.latest_game_id() else {
            return Task::none();
        };
        if self.selected_tournament_game_id == Some(latest) {
            return Task::none();
        }
        let Some(game) = self.live_tournament_view.game(latest).cloned() else {
            return Task::none();
        };
        match self.load_tournament_game(&game) {
            Ok(()) => Task::none(),
            Err(error) => {
                self.tournament_status = format!("Could not open tournament game: {error}");
                Task::none()
            }
        }
    }

    /// Boot function for iced 0.14 — returns (State, Task).
    fn boot() -> (Self, Task<Msg>) {
        let load_lucide = iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(|_| Msg::FontLoaded);
        let probe_engines = Task::perform(probe_adjacent_engines(), Msg::EngineCatalogProbed);
        (Self::default(), Task::batch([load_lucide, probe_engines]))
    }

    /// Subscription: animate at ~60fps while a move animation is in progress,
    /// also tick for coin flip animation and recording capture.
    fn subscription(&self) -> Subscription<Msg> {
        let mut subs: Vec<Subscription<Msg>> = vec![
            iced::window::open_events().map(Msg::WindowOpened),
            iced::window::resize_events().map(|(id, size)| Msg::WindowResized(id, size)),
        ];

        if self.animation.is_some() {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Msg::AnimTick));
        }

        if self.show_options {
            subs.push(iced::event::listen_with(
                |event, _status, _id| match event {
                    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                        Some(Msg::CursorMoved(position))
                    }
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left,
                    )) => Some(Msg::StopOptionsDrag),
                    _ => None,
                },
            ));
        }
        if self.show_tournament_setup {
            subs.push(iced::event::listen_with(
                |event, _status, _id| match event {
                    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                        Some(Msg::CursorMoved(position))
                    }
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left,
                    )) => Some(Msg::StopTournamentSetupDrag),
                    _ => None,
                },
            ));
        }

        if matches!(self.coin_flip, CoinFlipState::Flipping { .. }) {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Msg::CoinFlipTick));
        }

        if matches!(self.screen, Screen::Menu)
            && self.settings.system_motion
            && self.hub_opened_at.elapsed() < Duration::from_millis(1000)
        {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Msg::HubTick));
        }

        if self
            .live_tournament
            .as_ref()
            .is_some_and(|handle| handle.clone_snapshot().running)
        {
            subs.push(iced::time::every(Duration::from_millis(120)).map(Msg::TournamentTick));
        }

        if self.recorder.state() == recording::RecordState::Recording {
            // Capture at ~10fps
            subs.push(
                iced::time::every(Duration::from_millis(100)).map(|_| Msg::RecordCaptureTick),
            );
        }

        if subs.is_empty() {
            Subscription::none()
        } else {
            Subscription::batch(subs)
        }
    }

    fn current_pgn(&self) -> (String, &'static str) {
        let result = if let Some(ref game) = self.game {
            if game.game_over {
                if game.board.clone().is_checkmate() {
                    if game.board.side_to_move == types::Color::White {
                        "0-1"
                    } else {
                        "1-0"
                    }
                } else {
                    "1/2-1/2"
                }
            } else {
                "*"
            }
        } else {
            "*"
        };
        let pgn = build_annotated_pgn(
            &self.white_player.to_string(),
            &self.black_player.to_string(),
            &self.move_log,
            &self.move_annotations,
            &self.analysis_scores_cp,
            result,
        );
        (pgn, result)
    }

    fn persist_current_game(&mut self, result: &str) -> Result<String, String> {
        let pgn = build_annotated_pgn(
            &self.white_player.to_string(),
            &self.black_player.to_string(),
            &self.move_log,
            &self.move_annotations,
            &self.analysis_scores_cp,
            result,
        );
        let metadata = GameMetadata {
            event: "Mujrim Game".to_owned(),
            site: "Local".to_owned(),
            white: self.white_player.to_string(),
            black: self.black_player.to_string(),
            result: result.to_owned(),
            ..Default::default()
        };
        self.study_database
            .as_mut()
            .ok_or_else(|| "Study library is unavailable.".to_owned())?
            .import_pgn(metadata, &pgn)
    }

    fn refresh_study_results(&mut self) {
        let text = self.study_query.trim();
        self.study_results = self
            .study_database
            .as_ref()
            .map_or_else(Vec::new, |database| {
                database.search(&GameQuery {
                    text: (!text.is_empty()).then(|| text.to_owned()),
                    ..GameQuery::default()
                })
            });
        self.training_due = self
            .training_store
            .as_ref()
            .map_or_else(Vec::new, |store| store.due(today_day(), 100));
    }

    fn refresh_tournament_history(&mut self) {
        self.stored_tournaments = self
            .study_database
            .as_ref()
            .and_then(|database| database.list_tournaments().ok())
            .unwrap_or_default();
    }

    fn sync_board_overlays(&mut self) {
        let show_last = self.settings.last_move_arrow;
        let ponder = if self.settings.ponder_arrow {
            self.ponder_hint
        } else {
            None
        };
        let analysis = self.analysis_arrows.clone();
        if let Some(gs) = self.game.as_mut() {
            gs.refresh_move_overlays(show_last, ponder, &analysis);
        }
    }

    fn run_multi_engine_analysis_task(&self) -> Task<Msg> {
        let fen = self
            .review_board
            .as_ref()
            .or_else(|| self.game.as_ref().map(|gs| &gs.board))
            .map(|board| board.to_fen())
            .unwrap_or_else(|| self.initial_fen.clone());
        let mut engines = Vec::new();
        if self
            .analysis_engines_selected
            .iter()
            .any(|id| id == "builtin")
        {
            engines.push(analysis::AnalysisEngineSpec {
                id: "builtin".into(),
                name: "Mujrim".into(),
                path: None,
                protocol: ExternalEngineProtocol::Uci,
                builtin: true,
            });
        }
        for bundled in &self.bundled_engines {
            let id = bundled.path.to_string_lossy().into_owned();
            if self
                .analysis_engines_selected
                .iter()
                .any(|selected| selected == &id)
            {
                engines.push(analysis::AnalysisEngineSpec {
                    id: id.clone(),
                    name: bundled.display_name.to_owned(),
                    path: Some(bundled.path.clone()),
                    protocol: ExternalEngineProtocol::Uci,
                    builtin: false,
                });
            }
        }
        for external in &self.external_engine_catalog {
            if self
                .analysis_engines_selected
                .iter()
                .any(|selected| selected == &external.path)
            {
                let protocol = if external.protocol.eq_ignore_ascii_case("xboard") {
                    ExternalEngineProtocol::Xboard
                } else {
                    ExternalEngineProtocol::Uci
                };
                engines.push(analysis::AnalysisEngineSpec {
                    id: external.path.clone(),
                    name: external.name.clone(),
                    path: Some(PathBuf::from(&external.path)),
                    protocol,
                    builtin: false,
                });
            }
        }
        let request = analysis::AnalysisRequest {
            fen,
            depth: self.engine_cfg.max_depth.max(1),
            movetime: Duration::from_millis((self.engine_cfg.time_per_move.max(1) * 1000) as u64),
            hash_mb: bounded_hash_mb(self.engine_cfg.hash_mb),
            threads: self.engine_cfg.threads.max(1) as usize,
            multipv: self.analysis_multipv.max(1) as u32,
            engines,
            max_pv_plies: 6,
        };
        let depth = request.depth;
        Task::perform(
            async move {
                analysis::run_multi_engine_analysis(request, move |fen, _depth| {
                    builtin_analysis_line(fen, depth)
                })
            },
            Msg::MultiEngineAnalysisFinished,
        )
    }

    /// Start a move animation (call BEFORE applying the move to the board).
    fn start_animation(
        &mut self,
        mv: types::Move,
        engine_info: Option<String>,
        trigger_engine_after: bool,
    ) {
        if let Some(ref gs) = self.game {
            let piece_info = gs.board.piece_on(mv.from);
            let captured = gs.board.piece_on(mv.to);
            let is_capture = captured.is_some();

            if let Some((piece, color)) = piece_info {
                // Play sound
                if let Some(ref sound) = self.sound {
                    if is_capture {
                        sound.play_capture();
                    } else {
                        sound.play_move();
                    }
                }

                let pace = AnimPace::from_setting(self.settings.anim_speed);
                let anim_duration = if !self.settings.piece_slide {
                    Duration::from_millis(1)
                } else if is_capture {
                    match self.settings.capture_anim_style {
                        CaptureAnimStyle::Instant => pace.capture_instant(),
                        CaptureAnimStyle::Explosion => pace.capture_explosion(),
                        CaptureAnimStyle::Fire => pace.capture_fire(),
                    }
                } else {
                    pace.quiet_move()
                };

                self.animation = Some(AnimationState {
                    mv,
                    _piece: piece,
                    _color: color,
                    captured,
                    is_capture,
                    start: Instant::now(),
                    duration: anim_duration,
                    engine_info,
                    trigger_engine_after,
                });
            }
        }
    }

    /// Complete the animation and apply the move to the board.
    fn finish_animation(&mut self) -> Task<Msg> {
        let anim = match self.animation.take() {
            Some(a) => a,
            None => return Task::none(),
        };

        if let Some(ref info) = anim.engine_info {
            self.engine_info = info.clone();
        }

        if let Some(ref mut gs) = self.game {
            gs.last_move_squares = vec![anim.mv.from, anim.mv.to];
            // Clear drawing arrows on any move
            gs.arrows.clear();
            self.analysis_arrows.clear();
            gs.board.make_move(anim.mv);
            // Chess notation: append check (+) or checkmate (#)
            let mut notation = anim.mv.to_uci();
            if gs.board.is_checkmate() {
                notation.push('#');
            } else if gs.board.in_check() {
                notation.push('+');
            }
            self.move_log.push(notation);
            self.move_annotations.push(None);
            self.analysis_scores_cp.push(None);
            self.review_board = None;
            self.review_ply = None;

            if let Some(item) = &self.active_puzzle {
                let ply = self.move_log.len() - 1;
                let played = self.move_log[ply].trim_end_matches(['+', '#']);
                if item.puzzle.solution.get(ply).map(String::as_str) != Some(played) {
                    let flipped = gs.flipped;
                    match types::Board::from_fen(&item.puzzle.fen) {
                        Ok(board) => {
                            *gs = game::GameState::new(board);
                            gs.flipped = flipped;
                            self.move_log.clear();
                            self.move_annotations.clear();
                            self.analysis_scores_cp.clear();
                            self.review_board = None;
                            self.review_ply = None;
                            self.status = "That move is not the solution. Try again.".to_owned();
                        }
                        Err(error) => {
                            self.status = format!("Training position failed to reset: {error}")
                        }
                    }
                } else if self.move_log.len() == item.puzzle.solution.len() {
                    self.status =
                        "Solved. Grade the position to schedule the next review.".to_owned();
                } else {
                    self.status = "Correct. Find the continuation.".to_owned();
                }
                return Task::none();
            }

            if gs.board.is_game_over() {
                uci_process::cancel_all_pondering();
                gs.game_over = true;
                gs.premove_queue.clear();
                let result = if gs.board.is_checkmate() {
                    if gs.board.side_to_move == types::Color::White {
                        "0-1"
                    } else {
                        "1-0"
                    }
                } else {
                    "1/2-1/2"
                };
                self.status = if gs.board.is_checkmate() {
                    let w = if gs.board.side_to_move == types::Color::White {
                        "Black"
                    } else {
                        "White"
                    };
                    format!("Checkmate! {w} wins!")
                } else {
                    String::from("Game drawn")
                };
                self.status = match self.persist_current_game(result) {
                    Ok(id) => format!("{} · Autosaved as {id}.", self.status),
                    Err(error) => format!("{} · Autosave failed: {error}", self.status),
                };
                return Task::none();
            }

            let stm = if gs.board.side_to_move == types::Color::White {
                "White"
            } else {
                "Black"
            };
            self.status = format!("{stm} to move");

            // Check for queued premoves — execute if it's now human's turn
            let is_next_human = match gs.board.side_to_move {
                types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
            };

            if is_next_human && !gs.premove_queue.is_empty() && self.settings.premoves_enabled {
                let queued = gs.premove_queue.remove(0);
                if let Some(mv) = premove::resolve_legal(&mut gs.board, queued) {
                    gs.deselect();
                    // Determine if engine plays after this premove
                    let is_next_next_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.black_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.white_player, PlayerConfig::Human),
                    };
                    self.start_animation(mv, None, !is_next_next_human);
                    return Task::none();
                }
                // Illegal in the live position — drop the rest (chess.com behavior).
                gs.clear_premoves();
            }

            if anim.trigger_engine_after && !is_next_human && !gs.game_over {
                self.sync_board_overlays();
                return self.trigger_engine_move();
            }
        }
        self.sync_board_overlays();
        Task::none()
    }

    fn persist_tournament_summary(
        &mut self,
        summary: &mujrim_benchmarker::strength::TournamentSummary,
    ) {
        let Some(database) = self.study_database.as_mut() else {
            return;
        };
        if summary.engines.len() < 2 {
            return;
        }
        let entrants = summary
            .engines
            .iter()
            .enumerate()
            .map(|(index, engine)| Entrant {
                id: format!("engine-{index}"),
                name: engine.engine.name.clone(),
                seed_elo: engine.established_elo,
            })
            .collect::<Vec<_>>();
        let id = format!(
            "t-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let status = if summary.cancelled {
            format!("Cancelled · {}", format_tournament_summary(summary))
        } else if let Some(error) = &summary.error {
            format!("Stopped · {error}")
        } else {
            format!("Finished · {}", format_tournament_summary(summary))
        };
        let tournament = StoredTournament {
            id: id.clone(),
            name: format!("{} quick tournament", summary.format),
            format: summary.format,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            status,
            entrants,
            results: summary.game_results.clone(),
        };
        let _ = database.save_tournament(&tournament);
        self.selected_tournament_id = Some(id);
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::SelectMode(mode) => {
                self.selected_mode = mode;
                match mode {
                    GameMode::HumanVsHuman => {
                        self.white_player = PlayerConfig::Human;
                        self.black_player = PlayerConfig::Human;
                    }
                    GameMode::HumanVsEngine => {
                        self.white_player = PlayerConfig::Human;
                        self.black_player = PlayerConfig::BuiltIn { depth: 64 };
                    }
                    GameMode::EngineVsEngine => {
                        self.white_player = PlayerConfig::BuiltIn { depth: 64 };
                        self.black_player = PlayerConfig::BuiltIn { depth: 64 };
                    }
                }
                // Reset coin flip when mode changes
                self.coin_flip = CoinFlipState::Idle;
                Task::none()
            }
            Msg::OpenHome => {
                self.invalidate_engine_tasks();
                self.hub_opened_at = Instant::now();
                self.screen = Screen::Menu;
                Task::none()
            }
            Msg::OpenStudy => {
                self.screen = Screen::Study;
                self.refresh_study_results();
                Task::perform(
                    index_openings(study_database_path()),
                    |(explorer, count)| Msg::OpeningIndexFinished(explorer, count),
                )
            }
            Msg::OpenTournaments => {
                self.screen = Screen::Tournaments;
                self.show_tournament_setup = self.live_tournament.is_none();
                self.tournament_setup_drag = None;
                self.refresh_tournament_history();
                Task::none()
            }
            Msg::OpenAnalysis => {
                if self.game.is_none() {
                    let mut gs = game::GameState::new(types::Board::new());
                    if self.settings.auto_flip_black {
                        gs.flipped = false;
                    }
                    self.game = Some(gs);
                    self.move_log.clear();
                    self.move_annotations.clear();
                    self.clear_review_state();
                    self.initial_fen = mujrim_study::opening::START_FEN.to_owned();
                }
                self.screen = Screen::Analysis;
                Task::none()
            }
            Msg::RefreshTournamentHistory => {
                self.refresh_tournament_history();
                Task::none()
            }
            Msg::StartGambitLesson(id) => {
                if let Some(lesson) = gambit::find_gambit(&id) {
                    self.active_gambit = Some(lesson);
                    self.gambit_ply = lesson.key_ply.min(lesson.moves.len());
                    if let Ok(fen) = lesson.fen_after_plies(self.gambit_ply)
                        && let Ok(board) = types::Board::from_fen(&fen)
                    {
                        let mut gs = game::GameState::new(board);
                        if let Ok(arrows) = lesson.coaching_arrows(self.gambit_ply.max(1)) {
                            gs.overlay_arrows = arrows;
                        }
                        self.game = Some(gs);
                        self.initial_fen = fen;
                        self.move_log = lesson.moves[..self.gambit_ply]
                            .iter()
                            .map(|mv| (*mv).to_owned())
                            .collect();
                        self.move_annotations = vec![None; self.move_log.len()];
                        self.clear_review_state();
                        self.screen = Screen::Analysis;
                        self.status = format!("Gambit: {} ({})", lesson.name, lesson.eco);
                    }
                }
                Task::none()
            }
            Msg::GambitStep(delta) => {
                if let Some(lesson) = self.active_gambit {
                    let next = (self.gambit_ply as i32 + delta).clamp(0, lesson.moves.len() as i32)
                        as usize;
                    self.gambit_ply = next;
                    if let Ok(fen) = lesson.fen_after_plies(next)
                        && let Ok(board) = types::Board::from_fen(&fen)
                    {
                        if let Some(gs) = self.game.as_mut() {
                            gs.board = board;
                            gs.overlay_arrows =
                                lesson.coaching_arrows(next.max(1)).unwrap_or_default();
                        }
                        self.initial_fen = fen;
                        self.move_log = lesson.moves[..next]
                            .iter()
                            .map(|mv| (*mv).to_owned())
                            .collect();
                        self.move_annotations = vec![None; self.move_log.len()];
                    }
                }
                Task::none()
            }
            Msg::ToggleAnalysisEngine(id) => {
                if let Some(index) = self
                    .analysis_engines_selected
                    .iter()
                    .position(|existing| existing == &id)
                {
                    self.analysis_engines_selected.remove(index);
                } else {
                    self.analysis_engines_selected.push(id);
                }
                Task::none()
            }
            Msg::SetAnalysisMultiPv(value) => {
                self.analysis_multipv = value.clamp(1, 5);
                Task::none()
            }
            Msg::RunMultiEngineAnalysis => self.run_multi_engine_analysis_task(),
            Msg::MultiEngineAnalysisFinished(snapshot) => {
                let consensus = snapshot.consensus.as_deref().unwrap_or("no consensus");
                self.analysis_status = format!(
                    "{} · {} opinions",
                    snapshot.status,
                    snapshot.analysis.opinions.len()
                );
                self.analysis_arrows = snapshot.arrows.clone();
                self.sync_board_overlays();
                self.status = format!("Analysis ready · consensus {consensus}");
                Task::none()
            }
            Msg::SelectTournamentFormat(format) => {
                self.tournament_format = format;
                self.tournament_setup.format = format;
                self.tournament_status.clear();
                Task::none()
            }
            Msg::TournamentEventNameChanged(value) => {
                self.tournament_setup.event = value;
                Task::none()
            }
            Msg::TournamentSiteChanged(value) => {
                self.tournament_setup.site = value;
                Task::none()
            }
            Msg::TournamentGamesPerEncounterChanged(value) => {
                self.tournament_setup.games_per_encounter = value.max(1) as u32;
                Task::none()
            }
            Msg::TournamentHashChanged(value) => {
                self.tournament_setup.hash_mb = value.max(1) as u32;
                Task::none()
            }
            Msg::TournamentThreadsChanged(value) => {
                self.tournament_setup.engine_threads = value.max(1) as u32;
                Task::none()
            }
            Msg::TournamentTimeControlChanged(preset) => {
                self.tournament_setup.time_control = preset;
                Task::none()
            }
            Msg::ToggleTournamentSetup => {
                if self.live_tournament.is_none() {
                    self.show_tournament_setup = !self.show_tournament_setup;
                    self.tournament_setup_drag = None;
                }
                Task::none()
            }
            Msg::StartTournamentSetupDrag => {
                self.tournament_setup_drag =
                    Some((self.cursor_position, self.tournament_setup_offset));
                Task::none()
            }
            Msg::StopTournamentSetupDrag => {
                self.tournament_setup_drag = None;
                Task::none()
            }
            Msg::TournamentToggleEngine(path) => {
                let path = PathBuf::from(path);
                if let Some(index) = self
                    .tournament_setup
                    .selected_engine_paths
                    .iter()
                    .position(|existing| existing == &path)
                {
                    self.tournament_setup.selected_engine_paths.remove(index);
                } else {
                    self.tournament_setup.selected_engine_paths.push(path);
                }
                Task::none()
            }
            Msg::ToggleTournamentResults => {
                self.show_tournament_results = !self.show_tournament_results;
                self.live_tournament_view.show_results_panel = self.show_tournament_results;
                Task::none()
            }
            Msg::RunQuickTournament => {
                if self.live_tournament.is_some() {
                    self.tournament_status =
                        "A tournament is already running. Cancel it before starting another."
                            .to_owned();
                    return Task::none();
                }
                let roster =
                    tournament_engine_roster(&self.bundled_engines, &self.external_engine_catalog);
                if self.tournament_setup.selected_engine_paths.is_empty() {
                    self.tournament_setup.selected_engine_paths =
                        roster.iter().map(|engine| engine.path.clone()).collect();
                }
                if let Err(error) = self.tournament_setup.validate() {
                    self.tournament_status = error;
                    return Task::none();
                }
                let engines = roster
                    .into_iter()
                    .filter(|engine| {
                        self.tournament_setup
                            .selected_engine_paths
                            .iter()
                            .any(|path| path == &engine.path)
                    })
                    .collect::<Vec<_>>();
                let selected_count = engines.len();
                let engines = match preflight_tournament_engines(engines) {
                    Ok(engines) => engines,
                    Err(error) => {
                        self.tournament_status = error;
                        return Task::none();
                    }
                };
                let skipped = selected_count.saturating_sub(engines.len());
                // Keep selection aligned with engines that actually launched.
                self.tournament_setup.selected_engine_paths =
                    engines.iter().map(|engine| engine.path.clone()).collect();
                let handle =
                    tournament_live::LiveTournamentHandle::new(self.tournament_setup.format);
                self.live_tournament_view = handle.clone_snapshot();
                self.live_tournament = Some(handle.clone());
                self.selected_tournament_game_id = None;
                self.tournament_review_active = false;
                self.show_tournament_results = false;
                self.show_tournament_setup = false;
                self.tournament_setup_drag = None;
                self.tournament_setup.concurrency = 1;
                self.tournament_format = self.tournament_setup.format;
                self.tournament_status = if skipped == 0 {
                    format!(
                        "Running {} — {} · {} engines · one full board, real clocks.",
                        self.tournament_setup.event,
                        self.tournament_setup.time_control.label(),
                        engines.len()
                    )
                } else {
                    format!(
                        "Running {} — {} · {} engines (skipped {skipped} that failed preflight) · one full board, real clocks.",
                        self.tournament_setup.event,
                        self.tournament_setup.time_control.label(),
                        engines.len()
                    )
                };
                let setup = self.tournament_setup.clone();
                Task::perform(run_quick_tournament(engines, setup, handle), |summary| {
                    Msg::QuickTournamentFinished(Box::new(summary))
                })
            }
            Msg::CancelTournament => {
                if let Some(handle) = &self.live_tournament {
                    handle.request_cancel();
                    self.live_tournament_view = handle.clone_snapshot();
                    self.tournament_status = self.live_tournament_view.status_line.clone();
                } else {
                    self.tournament_status = "No tournament is running.".to_owned();
                }
                Task::none()
            }
            Msg::TournamentTick(_now) => {
                if let Some(handle) = &self.live_tournament {
                    self.live_tournament_view = handle.clone_snapshot();
                    self.live_tournament_view.show_results_panel = self.show_tournament_results;
                    if !self.live_tournament_view.status_line.is_empty() {
                        self.tournament_status = self.live_tournament_view.status_line.clone();
                    }
                    if self.live_tournament_view.running {
                        return self.sync_tournament_arena_board();
                    }
                }
                Task::none()
            }
            Msg::QuickTournamentFinished(summary) => {
                // Keep the UI alive even if persistence/standings formatting panics.
                let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(handle) = &self.live_tournament {
                        self.live_tournament_view = handle.clone_snapshot();
                    }
                    self.live_tournament = None;
                    self.live_tournament_view.running = false;
                    self.live_tournament_view.finished = true;
                    self.persist_tournament_summary(&summary);
                    self.tournament_status = if summary.cancelled {
                        format!(
                            "Tournament cancelled. {}",
                            format_tournament_summary(&summary)
                        )
                    } else if let Some(error) = &summary.error {
                        format!("Tournament finished with notes: {error}")
                    } else {
                        format!(
                            "Tournament complete. {}",
                            format_tournament_summary(&summary)
                        )
                    };
                    self.live_tournament_view.status_line = self.tournament_status.clone();
                    let names = summary
                        .engines
                        .iter()
                        .map(|engine| engine.engine.name.clone())
                        .collect::<Vec<_>>();
                    self.live_tournament_view.standings =
                        tournament_live::standing_rows(&names, &summary.standings);
                    self.live_tournament_view.game_results = summary.game_results.clone();
                    let games = mujrim_benchmarker::strength::games_from_summary(&summary);
                    if self.live_tournament_view.played_games.len() < games.len() {
                        self.live_tournament_view.played_games.clear();
                        self.live_tournament_view.append_games(games);
                    }
                    self.refresh_tournament_history();
                }));
                if finished.is_err() {
                    self.live_tournament = None;
                    self.live_tournament_view.running = false;
                    self.live_tournament_view.finished = true;
                    self.tournament_status =
                        "Tournament finished, but updating the results panel failed. Standings may be incomplete."
                            .to_owned();
                    self.live_tournament_view.status_line = self.tournament_status.clone();
                }
                self.follow_latest_tournament_game()
            }
            Msg::SelectTournament(id) => {
                self.selected_tournament_id = Some(id);
                Task::none()
            }
            Msg::SelectTournamentGame(id) => {
                let Some(game) = self.live_tournament_view.game(id).cloned() else {
                    self.tournament_status = format!("Tournament game #{id} is unavailable.");
                    return Task::none();
                };
                match self.load_tournament_game(&game) {
                    Ok(()) => {
                        if game.moves.is_empty() {
                            Task::none()
                        } else {
                            self.status =
                                "Analyzing tournament game for the eval graph…".to_owned();
                            Task::perform(
                                analyze_game(self.initial_fen.clone(), self.move_log.clone()),
                                Msg::GameAnalysisFinished,
                            )
                        }
                    }
                    Err(error) => {
                        self.tournament_status = format!("Could not open tournament game: {error}");
                        Task::none()
                    }
                }
            }
            Msg::EngineCatalogProbed(engines) => {
                let bundled_paths: std::collections::HashSet<_> = self
                    .bundled_engines
                    .iter()
                    .map(|engine| engine.path.clone())
                    .collect();
                let bundled_stems: std::collections::HashSet<_> = self
                    .bundled_engines
                    .iter()
                    .filter_map(|engine| {
                        engine
                            .path
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                    })
                    .chain(
                        self.bundled_engines
                            .iter()
                            .map(|engine| engine.id.to_owned()),
                    )
                    .collect();
                let engines: Vec<_> = engines
                    .into_iter()
                    .filter(|engine| {
                        let path = PathBuf::from(&engine.path);
                        !bundled_paths.contains(&path)
                            && !bundled_stems.contains(&engine.name)
                            && path_is_under_local_engines(&path)
                    })
                    .collect();
                for engine in &engines {
                    if let Some(database) = self.study_database.as_mut() {
                        let _ = database.upsert_engine(engine);
                    }
                }
                self.external_engine_catalog = engines;
                Task::none()
            }
            Msg::AnalyzeGame => {
                if self.move_log.is_empty() {
                    self.status = "Make or load moves before requesting a review.".to_owned();
                    return Task::none();
                }
                self.status = "Analyzing every move with the coaching engine…".to_owned();
                Task::perform(
                    analyze_game(self.initial_fen.clone(), self.move_log.clone()),
                    Msg::GameAnalysisFinished,
                )
            }
            Msg::GameAnalysisFinished(result) => {
                match result {
                    Ok(analysis) => {
                        self.move_annotations =
                            analysis.iter().map(|ply| Some(ply.annotation)).collect();
                        self.analysis_scores_cp =
                            analysis.iter().map(|ply| Some(ply.score_cp)).collect();
                        self.status = "Game review complete.".to_owned();
                    }
                    Err(error) => self.status = format!("Game review failed: {error}"),
                }
                Task::none()
            }
            Msg::ViewPly(ply) => {
                match board_at_ply(&self.initial_fen, &self.move_log, ply) {
                    Ok(board) => {
                        self.review_board = Some(board);
                        self.review_ply = Some(ply);
                        self.status = format!("Reviewing position after ply {ply}.");
                    }
                    Err(error) => self.status = format!("Could not navigate game: {error}"),
                }
                Task::none()
            }
            Msg::ReturnToLivePosition => {
                self.review_board = None;
                self.review_ply = None;
                self.status = "Live position restored.".to_owned();
                Task::none()
            }
            Msg::LoadWhiteUciEngine => Task::perform(async { pick_engine_file().await }, |p| {
                Msg::WhiteEngineSelected(p, ExternalEngineProtocol::Uci)
            }),
            Msg::LoadWhiteXboardEngine => Task::perform(async { pick_engine_file().await }, |p| {
                Msg::WhiteEngineSelected(p, ExternalEngineProtocol::Xboard)
            }),
            Msg::LoadBlackUciEngine => Task::perform(async { pick_engine_file().await }, |p| {
                Msg::BlackEngineSelected(p, ExternalEngineProtocol::Uci)
            }),
            Msg::LoadBlackXboardEngine => Task::perform(async { pick_engine_file().await }, |p| {
                Msg::BlackEngineSelected(p, ExternalEngineProtocol::Xboard)
            }),
            Msg::WhiteEngineSelected(path, protocol) => {
                self.invalidate_engine_tasks();
                if let Some(p) = path {
                    self.white_player = PlayerConfig::External { path: p, protocol };
                }
                Task::none()
            }
            Msg::BlackEngineSelected(path, protocol) => {
                self.invalidate_engine_tasks();
                if let Some(p) = path {
                    self.black_player = PlayerConfig::External { path: p, protocol };
                }
                Task::none()
            }
            Msg::SelectBundledWhite(choice) => {
                self.invalidate_engine_tasks();
                if let Some(engine) = self.bundled_engines.get(choice.index) {
                    self.white_player = PlayerConfig::External {
                        path: engine.path.to_string_lossy().into_owned(),
                        protocol: ExternalEngineProtocol::Uci,
                    };
                }
                Task::none()
            }
            Msg::SelectBundledBlack(choice) => {
                self.invalidate_engine_tasks();
                if let Some(engine) = self.bundled_engines.get(choice.index) {
                    self.black_player = PlayerConfig::External {
                        path: engine.path.to_string_lossy().into_owned(),
                        protocol: ExternalEngineProtocol::Uci,
                    };
                }
                Task::none()
            }
            Msg::StartGame => {
                self.invalidate_engine_tasks();
                // Apply coin flip result if applicable
                if let CoinFlipState::Done(heads) = self.coin_flip
                    && !heads
                    && matches!(self.selected_mode, GameMode::HumanVsEngine)
                {
                    // Tails: human plays Black, engine plays White
                    let engine = self.black_player.clone();
                    self.black_player = PlayerConfig::Human;
                    self.white_player = engine;
                }

                types::init();
                let board = types::Board::new();
                self.game = Some(game::GameState::new(board));
                self.initial_fen = mujrim_study::opening::START_FEN.to_owned();

                // Auto-flip board when playing Black
                if let CoinFlipState::Done(heads) = self.coin_flip
                    && !heads
                    && let Some(ref mut gs) = self.game
                {
                    gs.flipped = true;
                }

                self.move_log.clear();
                self.move_annotations.clear();
                self.clear_review_state();
                self.active_puzzle = None;
                self.engine_info.clear();
                self.status = String::from("Game started — White to move");
                self.screen = Screen::Playing;
                self.coin_flip = CoinFlipState::Idle;

                // Switch BGM to game track
                if self.bgm_on
                    && let Some(ref mut s) = self.sound
                {
                    s.play_bgm(audio::BgmTrack::Game);
                }

                if !matches!(self.white_player, PlayerConfig::Human) {
                    return self.trigger_engine_move();
                }
                Task::none()
            }
            Msg::BoardClick(row, col) => {
                if self.review_ply.is_some() {
                    self.review_board = None;
                    self.review_ply = None;
                    self.status = "Live position restored; click again to move.".to_owned();
                    return Task::none();
                }
                // Ignore clicks during animation
                if self.animation.is_some() {
                    return Task::none();
                }
                // Left-click clears all drawn arrows
                if let Some(ref mut gs) = self.game {
                    gs.arrows.clear();
                }
                if let Some(ref mut gs) = self.game {
                    if gs.game_over {
                        return Task::none();
                    }
                    let is_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
                    };
                    if !is_human {
                        // Chess.com-style premove / multi-premove while the opponent thinks.
                        if self.settings.premoves_enabled {
                            let human = gs.board.side_to_move.opponent();
                            let clicked_sq = game::display_to_square(row, col, gs.flipped);
                            if gs.selected_square.is_some() {
                                if gs.queue_premove(clicked_sq, human, self.settings.multi_premoves)
                                {
                                    self.status = if self.settings.multi_premoves {
                                        format!(
                                            "Premoves queued ({}/{}).",
                                            gs.premove_queue.len(),
                                            premove::MAX_PREMOVES
                                        )
                                    } else {
                                        "Premove queued.".to_owned()
                                    };
                                } else if premove::can_select_for_premove(
                                    &gs.board,
                                    &gs.premove_queue,
                                    human,
                                    clicked_sq,
                                ) {
                                    gs.select_premove_square(clicked_sq, human);
                                } else {
                                    gs.deselect();
                                }
                            } else if premove::can_select_for_premove(
                                &gs.board,
                                &gs.premove_queue,
                                human,
                                clicked_sq,
                            ) {
                                gs.select_premove_square(clicked_sq, human);
                            }
                        }
                        return Task::none();
                    }

                    let clicked_sq = game::display_to_square(row, col, gs.flipped);

                    if gs.selected_square.is_some() {
                        // Check if this is a legal move destination
                        let from = gs.selected_square.unwrap();
                        let legal = gs.board.generate_legal_moves();
                        if let Some(mv) = legal
                            .iter()
                            .find(|m| m.from == from && m.to == clicked_sq)
                            .copied()
                        {
                            // Determine if engine plays next
                            let is_next_human = match gs.board.side_to_move {
                                types::Color::White => {
                                    matches!(self.black_player, PlayerConfig::Human)
                                }
                                types::Color::Black => {
                                    matches!(self.white_player, PlayerConfig::Human)
                                }
                            };
                            gs.deselect();
                            // Start animation (don't apply move yet)
                            self.start_animation(mv, None, !is_next_human);
                        } else {
                            // Not a valid move target — try selecting the clicked piece instead
                            if let Some((_, color)) = gs.board.piece_on(clicked_sq)
                                && color == gs.board.side_to_move
                            {
                                gs.select_square(clicked_sq);
                                return Task::none();
                            }
                            gs.deselect();
                        }
                    } else {
                        // No piece selected — select if it's our piece
                        if let Some((_, color)) = gs.board.piece_on(clicked_sq)
                            && color == gs.board.side_to_move
                        {
                            gs.select_square(clicked_sq);
                        }
                    }
                }
                Task::none()
            }
            Msg::EngineMove(generation, result) => {
                if generation != self.game_generation {
                    return Task::none();
                }
                match result {
                    Ok((mv, info, ponder)) => {
                        self.engine_move_retries = 1;
                        let legal = self.game.as_mut().is_some_and(|game| {
                            game.board.generate_legal_moves().iter().any(|candidate| {
                                candidate.from == mv.from
                                    && candidate.to == mv.to
                                    && candidate.promotion == mv.promotion
                            })
                        });
                        if !legal {
                            self.status = "Discarded a stale or illegal engine result.".to_owned();
                            self.engine_info = info;
                            uci_process::cancel_all_pondering();
                            return Task::none();
                        }
                        self.ponder_hint = ponder;
                        self.start_animation(mv, Some(info), true);
                        Task::none()
                    }
                    Err(error) => {
                        self.status = format!("Engine failed: {error}");
                        self.engine_info = error.clone();
                        uci_process::cancel_all_pondering();
                        if self.engine_move_retries > 0 {
                            self.engine_move_retries -= 1;
                            self.status = format!("Engine failed: {error} — retrying…");
                            return self.trigger_engine_move();
                        }
                        Task::none()
                    }
                }
            }
            Msg::EngineInfo(info) => {
                self.engine_info = info;
                Task::none()
            }
            Msg::NewGame => {
                self.invalidate_engine_tasks();
                self.screen = Screen::Menu;
                self.game = None;
                self.move_log.clear();
                self.move_annotations.clear();
                self.clear_review_state();
                self.active_puzzle = None;
                self.engine_info.clear();
                self.status = String::from("Set up a new game.");

                // Switch BGM back to menu track
                if self.bgm_on
                    && let Some(ref mut s) = self.sound
                {
                    s.play_bgm(audio::BgmTrack::Menu);
                }

                Task::none()
            }
            Msg::FlipBoard => {
                if let Some(ref mut gs) = self.game {
                    gs.flipped = !gs.flipped;
                }
                Task::none()
            }
            Msg::Resign => {
                self.invalidate_engine_tasks();
                let mut result = None;
                if let Some(ref mut gs) = self.game {
                    gs.game_over = true;
                    result = Some(if gs.board.side_to_move == types::Color::White {
                        "0-1"
                    } else {
                        "1-0"
                    });
                    let loser = if gs.board.side_to_move == types::Color::White {
                        "White"
                    } else {
                        "Black"
                    };
                    self.status = format!("{loser} resigns!");
                }
                if let Some(result) = result {
                    self.status = match self.persist_current_game(result) {
                        Ok(id) => format!("{} · Autosaved as {id}.", self.status),
                        Err(error) => format!("{} · Autosave failed: {error}", self.status),
                    };
                }
                // Return to menu
                self.screen = Screen::Menu;
                self.game = None;
                if let Some(ref mut s) = self.sound {
                    s.play_bgm(audio::BgmTrack::Menu);
                }
                Task::none()
            }
            Msg::AnimTick(_now) => {
                if let Some(ref anim) = self.animation {
                    let elapsed = anim.start.elapsed();
                    if elapsed >= anim.duration {
                        return self.finish_animation();
                    }
                }
                Task::none()
            }
            Msg::ExportPGN => {
                let (pgn, _) = self.current_pgn();

                // Copy to clipboard via arboard (if available) or print
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(pgn);
                    self.status = String::from("PGN copied to clipboard!");
                } else {
                    self.status = String::from("Could not access clipboard.");
                }
                Task::none()
            }
            Msg::SaveToLibrary => {
                let (_, result) = self.current_pgn();
                self.status = match self.persist_current_game(result) {
                    Ok(id) => format!("Saved to library as {id}."),
                    Err(error) => format!("Library save failed: {error}"),
                };
                self.refresh_study_results();
                Task::perform(
                    index_openings(study_database_path()),
                    |(explorer, count)| Msg::OpeningIndexFinished(explorer, count),
                )
            }
            Msg::ImportPgn => Task::perform(pick_pgn_file(), Msg::PgnSelected),
            Msg::PgnSelected(path) => {
                if let Some(path) = path {
                    self.status = match self.study_database.as_mut() {
                        Some(database) => match database.import_pgn_file(&path) {
                            Ok(report) => format!(
                                "Imported {} of {} games; {} already existed.",
                                report.imported, report.discovered, report.duplicates
                            ),
                            Err(error) => format!("PGN import failed: {error}"),
                        },
                        None => "Study library is unavailable.".to_owned(),
                    };
                    self.refresh_study_results();
                    return Task::perform(index_openings(study_database_path()), |result| {
                        Msg::OpeningIndexFinished(result.0, result.1)
                    });
                }
                Task::none()
            }
            Msg::StudyQueryChanged(query) => {
                self.study_query = query;
                self.refresh_study_results();
                Task::none()
            }
            Msg::SearchLibrary => {
                self.refresh_study_results();
                Task::none()
            }
            Msg::LoadLibraryGame(id) => {
                self.invalidate_engine_tasks();
                let loaded = self
                    .study_database
                    .as_ref()
                    .ok_or_else(|| "Study library is unavailable.".to_owned())
                    .and_then(|database| database.load_game(&id))
                    .and_then(|game| {
                        replay_study_game(&game.initial_fen, &game.moves).map(|state| (game, state))
                    });
                match loaded {
                    Ok((loaded, state)) => {
                        let summary = format!(
                            "Loaded {} vs {} ({}) for review.",
                            display_metadata(&loaded.metadata.white, "White"),
                            display_metadata(&loaded.metadata.black, "Black"),
                            loaded.result
                        );
                        self.white_player = PlayerConfig::Human;
                        self.black_player = PlayerConfig::Human;
                        self.initial_fen = loaded.initial_fen.clone();
                        self.move_log = loaded.moves;
                        self.move_annotations = vec![None; self.move_log.len()];
                        self.clear_review_state();
                        self.game = Some(state);
                        self.engine_info.clear();
                        self.animation = None;
                        self.active_puzzle = None;
                        self.status = summary;
                        self.screen = Screen::Playing;
                    }
                    Err(error) => self.status = format!("Could not load game: {error}"),
                }
                Task::none()
            }
            Msg::SeedTraining => {
                self.status = match self.training_store.as_mut() {
                    Some(store) => match seed_training(store) {
                        Ok(added) => format!("Added {added} starter training positions."),
                        Err(error) => format!("Training setup failed: {error}"),
                    },
                    None => "Training database is unavailable.".to_owned(),
                };
                self.refresh_study_results();
                Task::none()
            }
            Msg::StartTraining(id) => {
                self.invalidate_engine_tasks();
                let item = self
                    .training_store
                    .as_ref()
                    .and_then(|store| store.get(&id))
                    .cloned();
                match item {
                    Some(item) => match replay_study_game(&item.puzzle.fen, &[]) {
                        Ok(state) => {
                            self.white_player = PlayerConfig::Human;
                            self.black_player = PlayerConfig::Human;
                            self.initial_fen = item.puzzle.fen.clone();
                            self.game = Some(state);
                            self.move_log.clear();
                            self.move_annotations.clear();
                            self.clear_review_state();
                            self.animation = None;
                            self.status = format!(
                                "Training: {}. Find the best continuation.",
                                item.puzzle.themes.join(", ")
                            );
                            self.active_puzzle = Some(item);
                            self.screen = Screen::Playing;
                        }
                        Err(error) => self.status = format!("Could not load training: {error}"),
                    },
                    None => self.status = format!("Training position '{id}' was not found."),
                }
                Task::none()
            }
            Msg::GradeTraining(grade) => {
                let Some(id) = self
                    .active_puzzle
                    .as_ref()
                    .map(|item| item.puzzle.id.clone())
                else {
                    return Task::none();
                };
                self.status = match self.training_store.as_mut() {
                    Some(store) => match store.review(&id, grade, today_day()) {
                        Ok(schedule) => format!(
                            "Review saved. Next due in {} day(s).",
                            schedule.interval_days
                        ),
                        Err(error) => format!("Could not save review: {error}"),
                    },
                    None => "Training database is unavailable.".to_owned(),
                };
                self.active_puzzle = None;
                self.game = None;
                self.move_log.clear();
                self.move_annotations.clear();
                self.clear_review_state();
                self.screen = Screen::Study;
                self.refresh_study_results();
                Task::none()
            }
            Msg::StudyOpeningMove(uci) => {
                self.invalidate_engine_tasks();
                types::init();
                let continuing = self.game.is_some();
                let mut board = self
                    .game
                    .as_ref()
                    .map_or_else(types::Board::new, |state| state.board.clone());
                match apply_opening_move(&mut board, &uci) {
                    Ok(mv) => {
                        let mut state = game::GameState::new(board);
                        state.last_move_squares = vec![mv.from, mv.to];
                        if !continuing {
                            self.initial_fen = mujrim_study::opening::START_FEN.to_owned();
                            self.move_log.clear();
                            self.move_annotations.clear();
                            self.clear_review_state();
                        }
                        self.move_log.push(uci);
                        self.move_annotations.push(None);
                        self.analysis_scores_cp.push(None);
                        self.game = Some(state);
                        self.active_puzzle = None;
                        self.status =
                            "Opening move loaded. Return to Study to inspect the next position."
                                .to_owned();
                        self.screen = Screen::Playing;
                    }
                    Err(error) => self.status = error,
                }
                Task::none()
            }
            Msg::OpeningIndexFinished(explorer, count) => {
                self.opening_explorer = explorer;
                self.opening_indexed_games = count;
                Task::none()
            }
            Msg::ExportGIF => {
                if self.move_log.is_empty() {
                    self.status = String::from("No moves to export.");
                    return Task::none();
                }

                // Strip check/checkmate notation suffixes for UCI parsing
                let moves: Vec<String> = self
                    .move_log
                    .iter()
                    .map(|m| m.trim_end_matches(['+', '#']).to_string())
                    .collect();
                self.status = String::from("Exporting GIF...");

                Task::perform(
                    async move {
                        let gif_data = match gif_export::export_game_gif(&moves, 100) {
                            Ok(data) => data,
                            Err(error) => return format!("Failed to export GIF: {error}"),
                        };

                        // Open save dialog
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Save Game as GIF")
                            .set_file_name("mujrim_game.gif")
                            .add_filter("GIF", &["gif"])
                            .save_file()
                            .await;

                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            match std::fs::write(&path, &gif_data) {
                                Ok(_) => format!("GIF saved to {}", path.display()),
                                Err(e) => format!("Failed to save GIF: {e}"),
                            }
                        } else {
                            String::from("GIF export cancelled.")
                        }
                    },
                    Msg::GifExportDone,
                )
            }
            Msg::GifExportDone(msg) => {
                self.status = msg;
                Task::none()
            }
            Msg::ExitApp => {
                uci_process::cancel_all_pondering();
                iced::exit()
            }
            Msg::DragWindow => {
                if let Some(id) = self.window_id {
                    iced::window::drag(id)
                } else {
                    Task::none()
                }
            }
            Msg::ResizeWindow(direction) => {
                if let Some(id) = self.window_id {
                    iced::window::drag_resize(id, direction)
                } else {
                    Task::none()
                }
            }
            Msg::MinimizeWindow => {
                if let Some(id) = self.window_id {
                    iced::window::minimize(id, true)
                } else {
                    Task::none()
                }
            }
            Msg::ToggleMaximizeWindow => {
                if let Some(id) = self.window_id {
                    iced::window::toggle_maximize(id)
                } else {
                    Task::none()
                }
            }
            Msg::WindowOpened(id) => {
                self.window_id = Some(id);
                Task::none()
            }
            Msg::WindowResized(id, size) => {
                if self.window_id.is_none() || self.window_id == Some(id) {
                    self.window_id = Some(id);
                    self.window_width = size.width.max(1.0);
                    self.window_height = size.height.max(1.0);
                }
                Task::none()
            }
            Msg::ToggleBGM => {
                let track = match self.screen {
                    Screen::Menu | Screen::Study | Screen::Tournaments => audio::BgmTrack::Menu,
                    Screen::Playing | Screen::Analysis => audio::BgmTrack::Game,
                };
                if let Some(ref mut sound) = self.sound {
                    self.bgm_on = sound.toggle_bgm(track);
                }
                Task::none()
            }
            // ── Engine config handlers ──
            Msg::CfgHashChanged(v) => {
                self.engine_cfg.hash_mb = v;
                Task::none()
            }
            Msg::CfgThreadsChanged(v) => {
                self.engine_cfg.threads = v;
                Task::none()
            }
            Msg::CfgDepthChanged(v) => {
                self.engine_cfg.max_depth = v;
                Task::none()
            }
            Msg::CfgTimeChanged(v) => {
                self.engine_cfg.time_per_move = v;
                Task::none()
            }
            Msg::CfgTogglePonder => {
                self.engine_cfg.ponder = !self.engine_cfg.ponder;
                if !self.engine_cfg.ponder {
                    uci_process::cancel_all_pondering();
                }
                Task::none()
            }
            Msg::CfgToggleBook => {
                self.engine_cfg.use_book = !self.engine_cfg.use_book;
                Task::none()
            }
            Msg::CfgToggleNnue => {
                self.engine_cfg.use_nnue = !self.engine_cfg.use_nnue;
                Task::none()
            }
            Msg::LoadBuiltinEvalFile => Task::perform(
                async { pick_nnue_file().await },
                Msg::BuiltinEvalFileSelected,
            ),
            Msg::BuiltinEvalFileSelected(path) => {
                self.engine_cfg.eval_file = path;
                Task::none()
            }
            Msg::ClearBuiltinEvalFile => {
                self.engine_cfg.eval_file = None;
                Task::none()
            }
            Msg::CoinFlip => {
                // Start coin flip animation (1.5 seconds)
                use rand::Rng;
                let result = rand::rng().random_bool(0.5);
                self.coin_flip = CoinFlipState::Flipping {
                    start: Instant::now(),
                    result,
                };
                Task::none()
            }
            Msg::CoinFlipTick(_now) => {
                if let CoinFlipState::Flipping { start, result } = self.coin_flip
                    && start.elapsed() >= Duration::from_millis(1500)
                {
                    self.coin_flip = CoinFlipState::Done(result);
                    if matches!(self.selected_mode, GameMode::HumanVsEngine) {
                        self.status = if result {
                            String::from("Heads! You play White.")
                        } else {
                            String::from("Tails! You play Black.")
                        };
                    }
                }
                Task::none()
            }
            Msg::HubTick(_now) => Task::none(),
            Msg::ToggleRecording => {
                match self.recorder.state() {
                    recording::RecordState::Idle => {
                        self.recorder.start();
                        self.status = String::from("Recording...");
                    }
                    recording::RecordState::Recording => {
                        let recorder = self.recorder.clone();
                        self.status = String::from("Saving recording...");

                        return Task::perform(
                            async move {
                                let file = rfd::AsyncFileDialog::new()
                                    .set_title("Save Recording")
                                    .set_file_name("mujrim_recording.mp4")
                                    .add_filter("Video", &["mp4", "gif"])
                                    .save_file()
                                    .await;

                                if let Some(file) = file {
                                    let path = file.path().to_path_buf();
                                    match recorder.stop_and_save(path.clone()) {
                                        Ok(frames) => {
                                            format!("Saved {} frames to {}", frames, path.display())
                                        }
                                        Err(e) => format!("Recording error: {e}"),
                                    }
                                } else {
                                    recorder.cancel();
                                    String::from("Recording cancelled.")
                                }
                            },
                            Msg::RecordingSaved,
                        );
                    }
                    recording::RecordState::Saving => {
                        // Already saving, ignore
                    }
                }
                Task::none()
            }
            Msg::RecordCaptureTick => {
                if self.recorder.capture_frame() == recording::CaptureOutcome::MemoryLimitReached {
                    self.status = String::from(
                        "Recording memory limit reached; stop recording to save captured frames.",
                    );
                }
                Task::none()
            }
            Msg::RecordingSaved(msg) => {
                self.status = msg;
                Task::none()
            }
            Msg::TakeScreenshot => {
                self.status = String::from("Taking screenshot...");
                Task::perform(
                    async {
                        let image = {
                            let monitors = xcap::Monitor::all().unwrap_or_default();
                            monitors
                                .first()
                                .and_then(|monitor| monitor.capture_image().ok())
                        };
                        if let Some(image) = image {
                            let file = rfd::AsyncFileDialog::new()
                                .set_title("Save Screenshot")
                                .set_file_name("mujrim_screenshot.png")
                                .add_filter("PNG", &["png"])
                                .save_file()
                                .await;
                            if let Some(file) = file {
                                let path = file.path().to_path_buf();
                                match image.save(&path) {
                                    Ok(_) => {
                                        return format!("Screenshot saved to {}", path.display());
                                    }
                                    Err(e) => return format!("Save error: {e}"),
                                }
                            }
                        }
                        String::from("Screenshot cancelled.")
                    },
                    Msg::ScreenshotDone,
                )
            }
            Msg::ScreenshotDone(msg) => {
                self.status = msg;
                Task::none()
            }
            // ── Options modal & settings ──
            Msg::ToggleOptions => {
                self.show_options = !self.show_options;
                self.options_drag = None;
                Task::none()
            }
            Msg::StartOptionsDrag => {
                self.options_drag = Some((self.cursor_position, self.options_offset));
                Task::none()
            }
            Msg::StopOptionsDrag => {
                self.options_drag = None;
                Task::none()
            }
            Msg::CursorMoved(position) => {
                self.cursor_position = position;
                if let Some((anchor, original)) = self.options_drag {
                    let max_x = ((self.window_width - 568.0) * 0.5).max(0.0);
                    let max_y = ((self.window_height - 620.0) * 0.5).max(0.0);
                    self.options_offset = iced::Vector::new(
                        (original.x + position.x - anchor.x).clamp(-max_x, max_x),
                        (original.y + position.y - anchor.y).clamp(-max_y, max_y),
                    );
                }
                if let Some((anchor, original)) = self.tournament_setup_drag {
                    let max_x = ((self.window_width - 620.0) * 0.5).max(0.0);
                    let max_y = ((self.window_height - 680.0) * 0.5).max(0.0);
                    self.tournament_setup_offset = iced::Vector::new(
                        (original.x + position.x - anchor.x).clamp(-max_x, max_x),
                        (original.y + position.y - anchor.y).clamp(-max_y, max_y),
                    );
                }
                Task::none()
            }
            Msg::SetBoardTheme(t) => {
                self.settings.board_theme = t;
                self.settings.save();
                Task::none()
            }
            Msg::SetPieceSet(piece_set) => {
                self.settings.piece_set = piece_set;
                self.settings.save();
                Task::none()
            }
            Msg::SetShowCoords(v) => {
                self.settings.show_coords = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetAnimSpeed(v) => {
                self.settings.anim_speed = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetSfx(v) => {
                self.settings.sfx_on = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetBgmVolume(v) => {
                self.settings.bgm_volume = v;
                if let Some(ref mut s) = self.sound {
                    s.set_volume(v as f32 / 100.0);
                }
                self.settings.save();
                Task::none()
            }
            Msg::SetGameMood(m) => {
                self.settings.game_mood = m;
                if let Some(ref mut s) = self.sound {
                    s.set_mood(m);
                }
                self.settings.save();
                Task::none()
            }
            Msg::SetSoundTheme(theme) => {
                self.settings.sound_theme = theme;
                if let Some(ref mut sound) = self.sound {
                    sound.set_sound_theme(theme);
                }
                self.settings.save();
                Task::none()
            }
            Msg::SetAutoFlip(v) => {
                self.settings.auto_flip_black = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetShowLegal(v) => {
                self.settings.show_legal_moves = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetShowLastMove(v) => {
                self.settings.show_last_move = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetPremoves(v) => {
                self.settings.premoves_enabled = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetCaptureAnim(v) => {
                self.settings.capture_anim_style = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetCoordPosition(v) => {
                self.settings.coord_position = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetMultiPremoves(v) => {
                self.settings.multi_premoves = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetDrawArrows(v) => {
                self.settings.draw_arrows = v;
                self.settings.save();
                Task::none()
            }
            Msg::SetArrowShape(value) => {
                self.settings.arrow_shape = value;
                self.settings.save();
                Task::none()
            }
            Msg::SetArrowColor(value) => {
                self.settings.arrow_color = value;
                self.settings.save();
                Task::none()
            }
            Msg::SetArrowSize(value) => {
                self.settings.arrow_size = value;
                self.settings.save();
                Task::none()
            }
            Msg::BoardRightDown(row, col) => {
                if let Some(ref mut gs) = self.game {
                    // Chess.com: right-click cancels the entire premove queue.
                    if !gs.premove_queue.is_empty() {
                        gs.clear_premoves();
                        self.status = "Premoves cancelled.".to_owned();
                        return Task::none();
                    }
                    if self.settings.draw_arrows {
                        let from = game::display_to_square(row, col, gs.flipped);
                        gs.begin_arrow(from);
                    }
                }
                Task::none()
            }
            Msg::BoardRightUp(row, col) => {
                if self.settings.draw_arrows
                    && let Some(ref mut gs) = self.game
                {
                    let to = game::display_to_square(row, col, gs.flipped);
                    gs.finish_arrow(to, self.settings.arrow_color);
                } else if let Some(ref mut gs) = self.game {
                    // Release outside an armed drag should not leave a stale start.
                    gs.arrow_start = None;
                }
                Task::none()
            }
            Msg::BoardPointerDown(row, col) => {
                if let Some(ref mut gs) = self.game {
                    let sq = game::display_to_square(row, col, gs.flipped);
                    let is_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
                    };
                    let human = if is_human {
                        gs.board.side_to_move
                    } else {
                        gs.board.side_to_move.opponent()
                    };
                    let piece_present = if is_human {
                        gs.board.piece_on(sq).is_some()
                    } else {
                        premove::can_select_for_premove(&gs.board, &gs.premove_queue, human, sq)
                    };
                    if piece_present {
                        if is_human {
                            gs.begin_drag(sq);
                        } else {
                            gs.select_premove_square(sq, human);
                            gs.drag_from = Some(sq);
                            gs.drag_over = Some(sq);
                        }
                        Task::none()
                    } else {
                        self.update(Msg::BoardClick(row, col))
                    }
                } else {
                    Task::none()
                }
            }
            Msg::BoardPointerMove(row, col) => {
                if let Some(ref mut gs) = self.game
                    && gs.drag_from.is_some()
                {
                    let sq = game::display_to_square(row, col, gs.flipped);
                    gs.update_drag(sq);
                }
                Task::none()
            }
            Msg::BoardPointerUp(row, col) => {
                let drag = self.game.as_mut().and_then(|gs| {
                    if gs.drag_from.is_some() {
                        let sq = game::display_to_square(row, col, gs.flipped);
                        gs.update_drag(sq);
                        gs.end_drag()
                    } else {
                        None
                    }
                });
                if let Some((from, to)) = drag {
                    if let Some(gs) = self.game.as_mut() {
                        gs.select_square(from);
                    }
                    let flipped = self.game.as_ref().is_some_and(|g| g.flipped);
                    let display_row = if flipped { to.rank() } else { 7 - to.rank() } as usize;
                    let display_col = if flipped { 7 - to.file() } else { to.file() } as usize;
                    self.update(Msg::BoardClick(display_row, display_col))
                } else {
                    self.update(Msg::BoardClick(row, col))
                }
            }
            Msg::SetPieceSlide(value) => {
                self.settings.piece_slide = value;
                self.settings.save();
                Task::none()
            }
            Msg::SetSystemMotion(value) => {
                self.settings.system_motion = value;
                self.settings.save();
                Task::none()
            }
            Msg::SetLastMoveArrow(value) => {
                self.settings.last_move_arrow = value;
                self.settings.save();
                self.sync_board_overlays();
                Task::none()
            }
            Msg::SetPonderArrow(value) => {
                self.settings.ponder_arrow = value;
                self.settings.save();
                self.sync_board_overlays();
                Task::none()
            }
            Msg::SwitchOptionsTab(tab) => {
                self.options_tab = tab;
                // Auto-load Syzygy status and tuning params when switching to Tools
                if tab == OptionsTab::Tools {
                    let syzygy_dir = updater::syzygy::default_syzygy_path();
                    let (wdl, dtz) = updater::syzygy::check_installed(&syzygy_dir);
                    self.syzygy_wdl_count = wdl;
                    self.syzygy_dtz_count = dtz;
                    if wdl > 0 {
                        let usage = updater::syzygy::disk_usage(&syzygy_dir);
                        let mb = usage as f64 / (1024.0 * 1024.0);
                        self.syzygy_status =
                            format!("{} WDL + {} DTZ files ({:.1} MB)", wdl, dtz, mb);
                    } else {
                        self.syzygy_status = "Not installed".to_string();
                    }
                    // Load tuning params if not already loaded
                    if self.tuning_params.is_none() {
                        let path = updater::tuning::TunableParams::default_path();
                        match updater::tuning::TunableParams::load(&path) {
                            Ok(params) => {
                                self.tuning_params = Some(params);
                                self.tuning_status = "Loaded".to_string();
                            }
                            Err(e) => {
                                self.tuning_status = format!("Load error: {e}");
                            }
                        }
                    }
                }
                Task::none()
            }
            Msg::SelectSyzygyPieceSet(piece_set) => {
                self.syzygy_piece_set = piece_set;
                self.syzygy_status = format!("Selected {piece_set}");
                Task::none()
            }
            Msg::SyzygyDownload => {
                let piece_set = self.syzygy_piece_set;
                self.syzygy_status = format!("Downloading {piece_set} (resumable)...");
                Task::perform(
                    async move {
                        // Task::perform runs in a separate async context
                        let dest = updater::syzygy::default_syzygy_path();
                        match updater::syzygy::download_tables(&dest, piece_set, None) {
                            Ok(s) => format!(
                                "Downloaded: {}, skipped: {}, failed: {}",
                                s.downloaded, s.skipped, s.failed
                            ),
                            Err(e) => format!("Download error: {e}"),
                        }
                    },
                    Msg::SyzygyDownloadDone,
                )
            }
            Msg::SyzygyDownloadDone(result) => {
                self.syzygy_status = result;
                let syzygy_dir = updater::syzygy::default_syzygy_path();
                let (wdl, dtz) = updater::syzygy::check_installed(&syzygy_dir);
                self.syzygy_wdl_count = wdl;
                self.syzygy_dtz_count = dtz;
                // Also refresh NNUE status
                let nnue_dir = updater::nnue::default_nnue_path();
                let installed = updater::nnue::check_installed(&nnue_dir);
                self.nnue_installed_count = installed
                    .iter()
                    .filter(|(_, s)| *s != updater::nnue::NetStatus::Missing)
                    .count();
                if self.nnue_installed_count > 0 {
                    let usage = updater::nnue::disk_usage(&nnue_dir);
                    let mb = usage as f64 / (1024.0 * 1024.0);
                    self.nnue_status = format!(
                        "{}/{} networks ({:.1} MB)",
                        self.nnue_installed_count,
                        updater::nnue::NETWORKS.len(),
                        mb
                    );
                } else {
                    self.nnue_status = "Not installed".to_string();
                }
                Task::none()
            }
            Msg::NnueDownload => {
                self.nnue_status = "Downloading NNUE networks...".to_string();
                Task::perform(
                    async {
                        let dest = updater::nnue::default_nnue_path();
                        match updater::nnue::download_all(&dest, None) {
                            Ok(s) => format!("Downloaded: {}, failed: {}", s.downloaded, s.failed),
                            Err(e) => format!("Download error: {e}"),
                        }
                    },
                    Msg::NnueDownloadDone,
                )
            }
            Msg::NnueDownloadDone(result) => {
                self.nnue_status = result;
                let nnue_dir = updater::nnue::default_nnue_path();
                let installed = updater::nnue::check_installed(&nnue_dir);
                self.nnue_installed_count = installed
                    .iter()
                    .filter(|(_, s)| *s != updater::nnue::NetStatus::Missing)
                    .count();
                Task::none()
            }
            Msg::TuneLoad => {
                let path = updater::tuning::TunableParams::default_path();
                match updater::tuning::TunableParams::load(&path) {
                    Ok(params) => {
                        self.tuning_params = Some(params);
                        self.tuning_status = "Loaded".to_string();
                    }
                    Err(e) => self.tuning_status = format!("Error: {e}"),
                }
                Task::none()
            }
            Msg::TuneSetParam(section, name, value) => {
                if let Some(ref mut params) = self.tuning_params {
                    params.set_value(&section, &name, value);
                }
                Task::none()
            }
            Msg::TuneSave => {
                if let Some(ref params) = self.tuning_params {
                    let path = updater::tuning::TunableParams::default_path();
                    match params.save(&path) {
                        Ok(()) => self.tuning_status = "Saved".to_string(),
                        Err(e) => self.tuning_status = format!("Save error: {e}"),
                    }
                }
                Task::none()
            }
            Msg::CheckForUpdates => {
                self.status = "Checking for updates...".to_string();
                Task::none()
            }
            Msg::FontLoaded => Task::none(),
        }
    }

    fn trigger_engine_move(&self) -> Task<Msg> {
        let Some(ref gs) = self.game else {
            return Task::none();
        };

        let side_player = match gs.board.side_to_move {
            types::Color::White => self.white_player.clone(),
            types::Color::Black => self.black_player.clone(),
        };
        let generation = self.game_generation;

        // ── Try opening book first (instant response) ─────────────
        #[cfg(feature = "book")]
        if self.engine_cfg.use_book
            && let Some(ref book) = self.book
            && let Some(book_move) = book.probe(&gs.board)
        {
            let legal = gs.board.clone().generate_legal_moves();
            if legal
                .iter()
                .any(|m| m.from == book_move.from && m.to == book_move.to)
            {
                return Task::perform(
                    async move { Ok((book_move, String::from("Book move"), None)) },
                    move |result| Msg::EngineMove(generation, result),
                );
            }
        }

        let mut board_clone = gs.board.clone();
        let time_secs = self.engine_cfg.time_per_move as u64;
        let hash_mb = bounded_hash_mb(self.engine_cfg.hash_mb);
        let threads = self.engine_cfg.threads as usize;
        let max_depth = self.engine_cfg.max_depth;
        let use_nnue = self.engine_cfg.use_nnue;
        let use_book = self.engine_cfg.use_book;
        let ponder = self.engine_cfg.ponder;
        let eval_file = self.engine_cfg.eval_file.clone();

        Task::perform(
            async move {
                let handle = std::thread::Builder::new()
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || -> Result<EngineMoveOk, String> {
                        types::init();
                        match side_player {
                            PlayerConfig::Human => {
                                Err("No engine selected for this side.".to_owned())
                            }
                            PlayerConfig::BuiltIn { .. } => {
                                let (mv, info) = builtin_engine_search(
                                    &mut board_clone,
                                    hash_mb,
                                    threads,
                                    use_nnue,
                                    eval_file.as_deref(),
                                    std::time::Duration::from_secs(time_secs),
                                    max_depth,
                                )?;
                                Ok((mv, info, None))
                            }
                            PlayerConfig::External { path, protocol } => {
                                let fen = board_clone.to_fen();
                                let legal = board_clone.generate_legal_moves();
                                let search = uci_process::ExternalSearchConfig {
                                    ponder,
                                    use_nnue,
                                    own_book: use_book,
                                    eval_file,
                                };
                                let info = uci_process::query_best_move(
                                    &path,
                                    protocol,
                                    &fen,
                                    max_depth,
                                    std::time::Duration::from_secs(time_secs),
                                    hash_mb,
                                    threads,
                                    &search,
                                )?;
                                let mv = legal
                                    .iter()
                                    .find(|m| m.to_uci() == info.best_move)
                                    .copied()
                                    .ok_or_else(|| {
                                        format!(
                                            "{protocol} returned illegal move '{}'",
                                            info.best_move
                                        )
                                    })?;
                                let ponder_sq =
                                    info.ponder_move.as_deref().and_then(|ponder_uci| {
                                        let mut predicted = board_clone.clone();
                                        predicted.make_move(mv);
                                        predicted
                                            .generate_legal_moves()
                                            .into_iter()
                                            .find(|candidate| candidate.to_uci() == ponder_uci)
                                            .map(|ponder_mv| (ponder_mv.from, ponder_mv.to))
                                    });
                                Ok((mv, format!("{protocol} {}", info.telemetry()), ponder_sq))
                            }
                        }
                    })
                    .expect("Failed to spawn engine thread");
                match handle.join() {
                    Ok(result) => result,
                    Err(_) => Err("Engine thread panicked".to_owned()),
                }
            },
            move |result| Msg::EngineMove(generation, result),
        )
    }

    fn view(&self) -> Element<'_, Msg> {
        let content = match self.screen {
            Screen::Menu => self.view_menu(),
            Screen::Playing => self.view_game(),
            Screen::Study => self.view_study_hub(),
            Screen::Tournaments => self.view_tournament_hub(),
            Screen::Analysis => self.view_analysis(),
        };

        // Wrap in options modal overlay if open
        let page: Element<'_, Msg> = if self.show_options {
            let modal = self.view_options_modal();
            iced::widget::stack![content, modal].into()
        } else if matches!(self.screen, Screen::Tournaments) && self.show_tournament_setup {
            let modal = self.view_tournament_setup_modal();
            iced::widget::stack![content, modal].into()
        } else {
            content
        };

        let page: Element<'_, Msg> = container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true)
            .style(move |_theme| {
                let pal = self.settings.board_theme.gui_palette();
                container::Style {
                    background: Some(iced::Background::Color(pal.bg)),
                    border: iced::Border {
                        radius: 12.0.into(),
                        width: 1.5,
                        color: Color::from_rgba(pal.border.r, pal.border.g, pal.border.b, 0.7),
                    },
                    shadow: iced::Shadow {
                        color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 12.0,
                    },
                    ..Default::default()
                }
            })
            .into();

        window_resize_frame(page)
    }

    fn view_title_bar(&self) -> Element<'_, Msg> {
        let pal = self.settings.board_theme.gui_palette();

        // ── Left: Logo + two-line title ──
        let logo_icon: Image<iced::widget::image::Handle> =
            Image::new(self.logo.clone()).width(24).height(24);
        let title_block = column![
            text("Mujrim").size(14).color(pal.text_primary),
            text("Chess Engine • v2.0").size(10).color(Color::from_rgba(
                pal.accent.r,
                pal.accent.g,
                pal.accent.b,
                0.7
            )),
        ]
        .spacing(1);
        let left = row![logo_icon, title_block]
            .spacing(8)
            .align_y(Alignment::Center);

        // ── Center: Pill-shaped action buttons ──
        let mut actions = row![
            pill_button(
                lucide_icon(iced_fonts::lucide::house),
                "Home",
                pal,
                matches!(self.screen, Screen::Menu),
                Msg::OpenHome,
            ),
            pill_button(
                lucide_icon(iced_fonts::lucide::settings),
                "Options",
                pal,
                self.show_options,
                Msg::ToggleOptions,
            ),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        if !matches!(self.screen, Screen::Playing | Screen::Analysis) {
            actions = actions
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::search),
                    "Analyze",
                    pal,
                    matches!(self.screen, Screen::Analysis),
                    Msg::OpenAnalysis,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::database),
                    "Study",
                    pal,
                    matches!(self.screen, Screen::Study),
                    Msg::OpenStudy,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::trophy),
                    "Tournaments",
                    pal,
                    matches!(self.screen, Screen::Tournaments),
                    Msg::OpenTournaments,
                ));
        }
        if matches!(self.screen, Screen::Playing | Screen::Analysis) {
            actions = actions
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::camera),
                    "Shot",
                    pal,
                    false,
                    Msg::TakeScreenshot,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::plus),
                    "New",
                    pal,
                    false,
                    Msg::NewGame,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::arrow_up_down),
                    "Flip",
                    pal,
                    false,
                    Msg::FlipBoard,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::flag),
                    "Resign",
                    pal,
                    true,
                    Msg::Resign,
                ))
                .push(pill_sep(pal))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::clipboard_copy),
                    "PGN",
                    pal,
                    false,
                    Msg::ExportPGN,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::database),
                    "Library",
                    pal,
                    false,
                    Msg::SaveToLibrary,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::sparkles),
                    "Review",
                    pal,
                    false,
                    Msg::AnalyzeGame,
                ))
                .push(pill_button(
                    lucide_icon(iced_fonts::lucide::film),
                    "GIF",
                    pal,
                    false,
                    Msg::ExportGIF,
                ));
            let (rec_icon, rec_label): (Element<'_, Msg>, &str) = match self.recorder.state() {
                recording::RecordState::Idle => (lucide_icon(iced_fonts::lucide::circle), "Rec"),
                recording::RecordState::Recording => {
                    (lucide_icon(iced_fonts::lucide::circle_stop), "Stop")
                }
                recording::RecordState::Saving => (lucide_icon(iced_fonts::lucide::save), "…"),
            };
            actions = actions.push(pill_button(
                rec_icon,
                rec_label,
                pal,
                false,
                Msg::ToggleRecording,
            ));
        } else {
            // Menu screen: show Start Game button
            actions = actions.push(pill_button(
                lucide_icon(iced_fonts::lucide::play),
                "Start",
                pal,
                false,
                Msg::StartGame,
            ));
        }

        // Wrap action buttons in a pill container
        let action_pill =
            container(actions)
                .padding([2, 6])
                .style(move |_theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        pal.bg.r * 0.8 + 0.04,
                        pal.bg.g * 0.8 + 0.04,
                        pal.bg.b * 0.8 + 0.04,
                        0.85,
                    ))),
                    border: iced::Border {
                        radius: 999.0.into(),
                        width: 1.0,
                        color: Color::from_rgba(pal.border.r, pal.border.g, pal.border.b, 0.6),
                    },
                    ..Default::default()
                });

        // ── Right: Status + Exit ──
        let right = row![
            text(&self.status).size(10).color(pal.text_secondary),
            window_icon_button(
                lucide_icon(iced_fonts::lucide::minus),
                "Minimize",
                pal,
                false,
                Msg::MinimizeWindow
            ),
            window_icon_button(
                lucide_icon(iced_fonts::lucide::square),
                "Maximize",
                pal,
                false,
                Msg::ToggleMaximizeWindow
            ),
            window_icon_button(
                lucide_icon(iced_fonts::lucide::x),
                "Close",
                pal,
                true,
                Msg::ExitApp
            ),
        ]
        .spacing(3)
        .align_y(Alignment::Center);

        // ── Assemble: Three-column layout ──
        let bar_content = row![
            container(left).width(Length::FillPortion(1)),
            container(action_pill).center_x(Length::FillPortion(2)),
            container(right)
                .width(Length::FillPortion(1))
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([0, 14]);

        // Title bar with translucent gradient bg and bottom accent glow
        let accent_glow = Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.12);
        let title_bar_widget: Element<'_, Msg> = container(bar_content)
            .width(Length::Fill)
            .height(48)
            .center_y(48)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    pal.sidebar.r * 0.7 + pal.panel.r * 0.3,
                    pal.sidebar.g * 0.7 + pal.panel.g * 0.3,
                    pal.sidebar.b * 0.7 + pal.panel.b * 0.3,
                    0.95,
                ))),
                border: iced::Border {
                    radius: 0.0.into(),
                    width: 0.0,
                    color: accent_glow,
                },
                shadow: iced::Shadow {
                    color: accent_glow,
                    offset: iced::Vector::new(0.0, 1.0),
                    blur_radius: 4.0,
                },
                ..Default::default()
            })
            .into();

        // Wrap entire title bar in a mouse_area so free space is draggable
        mouse_area(title_bar_widget)
            .interaction(iced::mouse::Interaction::Grab)
            .on_press(Msg::DragWindow)
            .into()
    }

    // ══════════════════════════════════════════════════════════
    // Menu screen — two-column layout with blurred chess bg
    // ══════════════════════════════════════════════════════════
    fn view_menu(&self) -> Element<'_, Msg> {
        let pal = self.settings.board_theme.gui_palette();
        let entrance = if self.settings.system_motion {
            motion::hub_entrance(self.hub_opened_at.elapsed().as_millis() as u64, 700)
        } else {
            1.0
        };
        let logo_img: Image<iced::widget::image::Handle> =
            Image::new(self.logo.clone()).width(128).height(128);

        let title = text("MUJRIM")
            .size(64)
            .color(Color::from_rgba(
                pal.text_primary.r,
                pal.text_primary.g,
                pal.text_primary.b,
                entrance,
            ))
            .font(CURIOUS_FONT);
        let subtitle = text("Play · Analyze · Prepare · Compete")
            .size(18)
            .color(Color::from_rgba(
                pal.accent_alt.r,
                pal.accent_alt.g,
                pal.accent_alt.b,
                0.55 + 0.45 * entrance,
            ));
        let tagline = text("A full desktop chess studio for every UCI engine on your machine.")
            .size(14)
            .color(pal.text_secondary);
        let quick_actions = row![
            styled_button("Analyze Position", Msg::OpenAnalysis),
            styled_button("Open Study", Msg::OpenStudy),
            styled_button("Engine Tournament", Msg::OpenTournaments),
        ]
        .spacing(10);

        let elapsed_ms = self.hub_opened_at.elapsed().as_millis() as u64;
        let left_in = if self.settings.system_motion {
            motion::hub_stagger(elapsed_ms, 120, 520)
        } else {
            1.0
        };
        let right_in = if self.settings.system_motion {
            motion::hub_stagger(elapsed_ms, 220, 520)
        } else {
            1.0
        };
        let start_in = if self.settings.system_motion {
            motion::hub_stagger(elapsed_ms, 340, 480)
        } else {
            1.0
        };
        let panel_height = (self.window_height - 390.0).clamp(240.0, 460.0);

        // ── Left column: Game Setup ──
        let mode_picker = pick_list(
            vec![
                GameMode::HumanVsHuman,
                GameMode::HumanVsEngine,
                GameMode::EngineVsEngine,
            ],
            Some(self.selected_mode),
            Msg::SelectMode,
        )
        .width(Length::Fill);
        let bundled_choices = bundled_engine_choices(&self.bundled_engines);
        let selected_white = selected_bundled_engine(&self.bundled_engines, &self.white_player);
        let selected_black = selected_bundled_engine(&self.bundled_engines, &self.black_player);

        let badge_w = 36.0;
        let w_badge = container(text("W").size(15).color(Color::from_rgb(0.20, 0.15, 0.10)))
            .center_x(badge_w)
            .center_y(badge_w)
            .width(badge_w)
            .height(badge_w)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.94, 0.88, 0.76))),
                border: iced::Border {
                    radius: 8.0.into(),
                    width: 1.0,
                    color: Color::from_rgb(0.80, 0.75, 0.60),
                },
                ..Default::default()
            });
        let b_badge = container(text("B").size(15).color(Color::from_rgb(0.80, 0.80, 0.85)))
            .center_x(badge_w)
            .center_y(badge_w)
            .width(badge_w)
            .height(badge_w)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.14, 0.12, 0.16))),
                border: iced::Border {
                    radius: 8.0.into(),
                    width: 1.0,
                    color: Color::from_rgb(0.30, 0.30, 0.35),
                },
                ..Default::default()
            });

        let mut left_col = column![
            text("Game Setup").size(16).color(Color::from_rgba(
                pal.accent_alt.r,
                pal.accent_alt.g,
                pal.accent_alt.b,
                left_in,
            )),
            text("Choose players and load engines")
                .size(12)
                .color(Color::from_rgba(
                    pal.text_secondary.r,
                    pal.text_secondary.g,
                    pal.text_secondary.b,
                    0.85 * left_in,
                )),
            Space::new().height(10),
            text("Mode").size(12).color(pal.text_secondary),
            mode_picker,
            Space::new().height(10),
            text("White").size(12).color(pal.text_secondary),
            row![
                w_badge,
                text(format!("{}", self.white_player))
                    .size(13)
                    .color(pal.text_primary)
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .width(Length::Fill);

        if matches!(self.selected_mode, GameMode::EngineVsEngine) {
            left_col = left_col
                .push(
                    pick_list(
                        bundled_choices.clone(),
                        selected_white,
                        Msg::SelectBundledWhite,
                    )
                    .placeholder("Auto-detected White engine")
                    .width(Length::Fill),
                )
                .push(
                    row![
                        styled_button("Load White UCI", Msg::LoadWhiteUciEngine),
                        styled_button("Load White XBoard", Msg::LoadWhiteXboardEngine),
                    ]
                    .spacing(8),
                );
        }
        left_col = left_col
            .push(Space::new().height(8))
            .push(text("Black").size(12).color(pal.text_secondary))
            .push(
                row![
                    b_badge,
                    text(format!("{}", self.black_player))
                        .size(13)
                        .color(pal.text_primary)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        if matches!(
            self.selected_mode,
            GameMode::HumanVsEngine | GameMode::EngineVsEngine
        ) {
            left_col = left_col
                .push(
                    pick_list(bundled_choices, selected_black, Msg::SelectBundledBlack)
                        .placeholder("Auto-detected Black engine")
                        .width(Length::Fill),
                )
                .push(
                    row![
                        styled_button("Load Black UCI", Msg::LoadBlackUciEngine),
                        styled_button("Load Black XBoard", Msg::LoadBlackXboardEngine),
                    ]
                    .spacing(8),
                );
        }

        // Coin flip (HumanVsEngine)
        if matches!(self.selected_mode, GameMode::HumanVsEngine) {
            left_col = left_col
                .push(Space::new().height(10))
                .push(text("Side selection").size(12).color(pal.text_secondary));
            let flip_el: Element<'_, Msg> = match &self.coin_flip {
                CoinFlipState::Idle => button(
                    container(
                        row![
                            iced_fonts::lucide::coins().size(18),
                            text(" Flip for side").size(13).color(Color::WHITE)
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .center_x(180)
                    .center_y(38)
                    .width(180)
                    .height(38),
                )
                .on_press(Msg::CoinFlip)
                .style(|_theme, status| {
                    let bg = if matches!(status, button::Status::Hovered) {
                        Color::from_rgb(0.55, 0.45, 0.20)
                    } else {
                        ACCENT_GOLD
                    };
                    button::Style {
                        background: Some(iced::Background::Color(bg)),
                        border: iced::Border {
                            radius: 10.0.into(),
                            ..Default::default()
                        },
                        text_color: Color::WHITE,
                        ..Default::default()
                    }
                })
                .into(),
                CoinFlipState::Flipping { start, .. } => {
                    let sym = ["◉", "○", "◉", "●", "◉", "◇"]
                        [(start.elapsed().as_millis() / 100 % 6) as usize];
                    row![
                        text("Flipping...").size(13).color(ACCENT_GOLD),
                        text(sym).size(24)
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into()
                }
                CoinFlipState::Done(heads) => {
                    let (icon, label, c) = if *heads {
                        ("○", "You play White!", ACCENT_TEAL)
                    } else {
                        ("●", "You play Black!", ACCENT)
                    };
                    row![text(icon).size(22), text(label).size(14).color(c)]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .into()
                }
            };
            left_col = left_col.push(flip_el);
        }

        let left_scroll = scrollable(left_col.padding([0, 4]))
            .height(Length::Fixed(panel_height))
            .width(Length::Fill);
        let left_card = container(glass_card(
            column![
                Space::new().height(motion::hub_slide_y(left_in, 18.0)),
                left_scroll,
            ]
            .into(),
            pal,
            left_in,
        ))
        .width(360)
        .height(Length::Fixed(panel_height + 28.0));

        // ── Right column: Engine Settings ──
        let cfg = &self.engine_cfg;
        let eval_file_label = cfg
            .eval_file
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Embedded".to_string());
        let right_col = column![
            text("Engine Settings").size(16).color(Color::from_rgba(
                pal.accent_alt.r,
                pal.accent_alt.g,
                pal.accent_alt.b,
                right_in,
            )),
            text("Tune search, book, and network")
                .size(12)
                .color(Color::from_rgba(
                    pal.text_secondary.r,
                    pal.text_secondary.g,
                    pal.text_secondary.b,
                    0.85 * right_in,
                )),
            Space::new().height(10),
            config_slider(
                "Time / Move",
                cfg.time_per_move,
                "s",
                1,
                30,
                Msg::CfgTimeChanged
            ),
            config_slider("Max Depth", cfg.max_depth, "", 1, 64, Msg::CfgDepthChanged),
            config_slider(
                "Hash (MB)",
                cfg.hash_mb.min(MAX_GUI_HASH_MB),
                "MB",
                1,
                MAX_GUI_HASH_MB,
                Msg::CfgHashChanged,
            ),
            config_slider("Threads", cfg.threads, "", 1, 32, Msg::CfgThreadsChanged),
            Space::new().height(8),
            settings_row(
                "Ponder",
                toggler(cfg.ponder)
                    .on_toggle(|_| Msg::CfgTogglePonder)
                    .size(18)
                    .into()
            ),
            settings_row(
                "Opening Book",
                toggler(cfg.use_book)
                    .on_toggle(|_| Msg::CfgToggleBook)
                    .size(18)
                    .into()
            ),
            settings_row(
                "NNUE Eval",
                toggler(cfg.use_nnue)
                    .on_toggle(|_| Msg::CfgToggleNnue)
                    .size(18)
                    .into()
            ),
            Space::new().height(8),
            row![
                text("Eval Net")
                    .size(12)
                    .color(pal.text_secondary)
                    .width(75),
                text(eval_file_label).size(12).color(pal.text_primary),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                styled_button("Load NNUE File", Msg::LoadBuiltinEvalFile),
                styled_button("Use Embedded", Msg::ClearBuiltinEvalFile),
            ]
            .spacing(8),
        ]
        .spacing(6)
        .width(Length::Fill);

        let right_scroll = scrollable(right_col.padding([0, 4]))
            .height(Length::Fixed(panel_height))
            .width(Length::Fill);
        let right_card = container(glass_card(
            column![
                Space::new().height(motion::hub_slide_y(right_in, 18.0)),
                right_scroll,
            ]
            .into(),
            pal,
            right_in,
        ))
        .width(360)
        .height(Length::Fixed(panel_height + 28.0));

        // ── Start button ──
        let start_btn = button(
            container(text("Start Game").size(16).color(Color::WHITE))
                .center_x(220)
                .center_y(48)
                .width(220)
                .height(48),
        )
        .on_press(Msg::StartGame)
        .style(move |_theme, status| {
            let base = Color::from_rgba(ACCENT.r, ACCENT.g, ACCENT.b, 0.35 + 0.65 * start_in);
            let bg = if matches!(status, button::Status::Hovered) {
                Color::from_rgba(0.30, 0.60, 1.0, 0.35 + 0.65 * start_in)
            } else {
                base
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::Border {
                    radius: 12.0.into(),
                    ..Default::default()
                },
                text_color: Color::from_rgba(1.0, 1.0, 1.0, 0.45 + 0.55 * start_in),
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.25 * start_in),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 12.0,
                },
                ..Default::default()
            }
        });

        // ── Two-column layout ──
        let two_cols = row![left_card, Space::new().width(24), right_card]
            .spacing(0)
            .align_y(Alignment::Start);

        let menu_content = column![
            Space::new().height(20),
            logo_img,
            Space::new().height(6),
            title,
            subtitle,
            tagline,
            Space::new().height(12),
            quick_actions,
            Space::new().height(16),
            two_cols,
            Space::new().height(motion::hub_slide_y(start_in, 14.0) + 12.0),
            start_btn,
            Space::new().height(8),
            text("Studio v2 · multi-engine ready")
                .size(11)
                .color(pal.text_secondary),
            Space::new().height(20),
        ]
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::Fill);

        // chess_bg as blurred background behind menu
        let bg_img: Image<iced::widget::image::Handle> = Image::new(self.chess_bg.clone())
            .width(Length::Fill)
            .height(Length::Fill);
        let bg_layer = container(bg_img)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: None,
                ..Default::default()
            });
        let menu_fg = container(
            scrollable(
                container(menu_content)
                    .center_x(Length::Fill)
                    .width(Length::Fill)
                    .padding([0, 16]),
            )
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        column![
            self.view_title_bar(),
            iced::widget::stack![bg_layer, menu_fg],
        ]
        .into()
    }

    // ══════════════════════════════════════════════════════════
    // Game screen
    // ══════════════════════════════════════════════════════════
    fn view_game(&self) -> Element<'_, Msg> {
        let gs = match &self.game {
            Some(gs) => gs,
            None => return text("No game").into(),
        };

        let title_bar_h = 48.0_f32;
        let padding = 40.0_f32;
        let available_h = (self.window_height - title_bar_h - padding).max(200.0);
        let panel_w = (self.window_width * 0.23).clamp(250.0, 400.0);
        let available_board_w = (self.window_width - panel_w - padding * 2.0 - 20.0).max(224.0);
        let sq_size = (available_h * 0.90 / 8.0)
            .min(available_board_w / 8.0)
            .clamp(28.0, 120.0);

        let anim_info = self
            .animation
            .as_ref()
            .filter(|_| self.review_board.is_none())
            .map(|anim| {
                let progress =
                    (anim.start.elapsed().as_secs_f32() / anim.duration.as_secs_f32()).min(1.0);
                board_view::AnimInfo {
                    from_sq: anim.mv.from,
                    to_sq: anim.mv.to,
                    _piece: anim._piece,
                    _color: anim._color,
                    progress,
                    captured: anim.captured,
                    is_capture: anim.is_capture,
                }
            });

        let board_view = board_view::view_board(
            gs,
            &self.assets,
            board_view::BoardViewOptions {
                sq_size,
                anim: anim_info,
                theme: self.settings.board_theme,
                piece_set: self.settings.piece_set,
                show_coords: self.settings.show_coords,
                coord_position: self.settings.coord_position,
                capture_anim_style: self.settings.capture_anim_style,
                overlay_arrows: &gs.overlay_arrows,
                user_arrows: &gs.arrows,
                arrow_appearance: arrows::ArrowAppearance {
                    shape: self.settings.arrow_shape,
                    color: self.settings.arrow_color,
                    size: self.settings.arrow_size,
                },
                display_board: self.review_board.as_ref(),
                show_legal_moves: self.settings.show_legal_moves,
                show_last_move: self.settings.show_last_move,
                annotation_badge: review_annotation_badge(
                    &self.initial_fen,
                    &self.move_log,
                    self.review_ply,
                    &self.move_annotations,
                ),
            },
        );
        let board_total = sq_size * 8.0;

        // Move history — wider two-column layout
        let pal = self.settings.board_theme.gui_palette();

        let moves_content: Element<'_, Msg> = if self.move_log.is_empty() {
            text("No moves yet.")
                .size(13)
                .color(pal.text_secondary)
                .into()
        } else {
            let mut moves_col = column![].spacing(2).padding([4, 8]);
            for (i, pair) in self.move_log.chunks(2).enumerate() {
                let num_text = text(format!("{}.", i + 1))
                    .size(12)
                    .color(pal.text_secondary)
                    .width(32);
                let white_index = i * 2;
                let white_label = annotated_move_label(
                    &pair[0],
                    self.move_annotations.get(white_index).copied().flatten(),
                );
                let white_move = move_history_button(
                    white_label,
                    white_index + 1,
                    self.review_ply == Some(white_index + 1),
                    pal,
                );
                let black_move = if let Some(b) = pair.get(1) {
                    move_history_button(
                        annotated_move_label(
                            b,
                            self.move_annotations
                                .get(white_index + 1)
                                .copied()
                                .flatten(),
                        ),
                        white_index + 2,
                        self.review_ply == Some(white_index + 2),
                        pal,
                    )
                } else {
                    container(text("...").size(13).color(pal.text_secondary))
                        .width(88)
                        .into()
                };
                moves_col = moves_col.push(
                    row![num_text, white_move, black_move]
                        .spacing(6)
                        .align_y(Alignment::Center),
                );
            }
            moves_col.into()
        };

        let moves_panel = container(scrollable(moves_content).height(Length::Fill))
            .padding(8)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(pal.sidebar)),
                border: iced::Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: pal.border,
                },
                ..Default::default()
            });

        let engine_panel = if self.engine_info.is_empty() {
            container(text("Engine idle").size(11).color(pal.text_secondary))
                .padding(8)
                .width(Length::Fill)
                .style(move |_theme| container::Style {
                    background: Some(iced::Background::Color(pal.panel)),
                    border: iced::Border {
                        radius: 4.0.into(),
                        width: 1.0,
                        color: pal.border,
                    },
                    ..Default::default()
                })
        } else {
            container(
                column![
                    text("Engine Analysis").size(11).color(pal.accent_alt),
                    text(&self.engine_info).size(12).color(pal.text_primary),
                ]
                .spacing(4),
            )
            .padding(8)
            .width(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(pal.panel)),
                border: iced::Border {
                    radius: 4.0.into(),
                    width: 1.0,
                    color: pal.accent_alt,
                },
                ..Default::default()
            })
        };

        let mut side_content = column![
            text("Moves").size(13).color(pal.text_secondary),
            moves_panel,
        ]
        .spacing(6)
        .width(panel_w);
        if self.analysis_scores_cp.iter().any(Option::is_some) {
            side_content = side_content
                .push(Space::new().height(6))
                .push(text("Game advancement").size(13).color(pal.text_secondary))
                .push(
                    container(eval_graph::view(&self.analysis_scores_cp, 104.0))
                        .width(Length::Fill)
                        .padding(6)
                        .style(move |_theme| container::Style {
                            background: Some(iced::Background::Color(pal.sidebar)),
                            border: iced::Border {
                                radius: 6.0.into(),
                                width: 1.0,
                                color: pal.border,
                            },
                            ..Default::default()
                        }),
                );
        }
        if self.review_ply.is_some() {
            side_content = side_content.push(styled_button(
                "Return to live position",
                Msg::ReturnToLivePosition,
            ));
        }
        side_content = side_content
            .push(Space::new().height(8))
            .push(text("Engine").size(13).color(pal.text_secondary))
            .push(engine_panel);
        if matches!(self.screen, Screen::Analysis) {
            side_content = side_content
                .push(Space::new().height(10))
                .push(scrollable(self.analysis_sidebar_panel(pal)).height(Length::Fill));
        }
        if let Some(item) = &self.active_puzzle {
            let solved = puzzle_line_matches(&self.move_log, &item.puzzle.solution);
            let mut controls = column![
                text(format!("Training · {}", item.puzzle.rating))
                    .size(13)
                    .color(pal.text_primary),
                text(item.puzzle.themes.join(", "))
                    .size(12)
                    .color(pal.text_secondary),
            ]
            .spacing(6);
            if solved {
                controls = controls.push(
                    row![
                        styled_button("Again", Msg::GradeTraining(1)),
                        styled_button("Hard", Msg::GradeTraining(3)),
                        styled_button("Good", Msg::GradeTraining(4)),
                        styled_button("Easy", Msg::GradeTraining(5)),
                    ]
                    .spacing(5),
                );
            } else {
                controls = controls.push(
                    text(format!(
                        "Solution progress: {}/{} plies",
                        self.move_log.len(),
                        item.puzzle.solution.len()
                    ))
                    .size(11)
                    .color(pal.text_secondary),
                );
            }
            controls = controls.push(styled_button("Return to Study", Msg::OpenStudy));
            side_content = side_content
                .push(Space::new().height(8))
                .push(container(controls).padding(8).width(Length::Fill));
        }
        let side_panel = container(
            scrollable(side_content)
                .height(Length::Fixed(board_total))
                .width(Length::Fill),
        )
        .padding(12)
        .height(board_total)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(pal.panel)),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: pal.border,
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.22),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 12.0,
            },
            ..Default::default()
        });

        let game_layout = row![board_view, side_panel]
            .spacing(20)
            .padding(20)
            .align_y(Alignment::Start);

        column![
            self.view_title_bar(),
            container(game_layout)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .into()
    }

    // ══════════════════════════════════════════════════════════
    // Options modal overlay
    // ══════════════════════════════════════════════════════════
    fn view_study_hub(&self) -> Element<'_, Msg> {
        let palette = self.settings.board_theme.gui_palette();
        let game_count = self.study_database.as_ref().map_or(0, StudyDatabase::len);
        let mut game_rows = column![].spacing(7);
        for summary in self.study_results.iter().take(100) {
            let (title, detail) = game_summary_label(summary);
            game_rows = game_rows.push(
                button(
                    column![
                        text(title).size(14).color(palette.text_primary),
                        text(detail).size(12).color(palette.text_secondary),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                )
                .on_press(Msg::LoadLibraryGame(summary.id.clone()))
                .padding(9)
                .width(Length::Fill),
            );
        }
        if self.study_results.is_empty() {
            game_rows = game_rows.push(
                text(if self.study_query.trim().is_empty() {
                    "Import a PGN collection to begin your library."
                } else {
                    "No games match this search."
                })
                .size(13)
                .color(palette.text_secondary),
            );
        }
        let training_count = self.training_store.as_ref().map_or(0, TrainingStore::len);
        let mut due_rows = column![].spacing(6);
        for item in self.training_due.iter().take(20) {
            due_rows = due_rows.push(
                button(
                    row![
                        text(item.puzzle.themes.join(", "))
                            .size(13)
                            .color(palette.text_primary),
                        Space::new().width(Length::Fill),
                        text(format!("{} Elo", item.puzzle.rating))
                            .size(12)
                            .color(palette.text_secondary),
                    ]
                    .align_y(Alignment::Center),
                )
                .on_press(Msg::StartTraining(item.puzzle.id.clone()))
                .padding(8)
                .width(Length::Fill),
            );
        }
        if self.training_due.is_empty() {
            due_rows = due_rows.push(
                text(if training_count == 0 {
                    "Add the starter set to begin spaced repetition."
                } else {
                    "Nothing is due today."
                })
                .size(13)
                .color(palette.text_secondary),
            );
        }
        let explorer_fen = self.game.as_ref().map_or_else(
            || mujrim_study::opening::START_FEN.to_owned(),
            |state| state.board.to_fen(),
        );
        let explorer_moves = self.opening_explorer.moves(&explorer_fen);
        let mut opening_rows = column![].spacing(6);
        for (uci, statistics) in explorer_moves.iter().take(16) {
            let decisive = statistics.white_wins + statistics.black_wins;
            let score = statistics
                .white_wins
                .saturating_mul(100)
                .saturating_add(statistics.draws.saturating_mul(50))
                .checked_div(statistics.games)
                .unwrap_or(0);
            opening_rows = opening_rows.push(
                button(
                    row![
                        text(*uci).size(14).color(palette.text_primary),
                        Space::new().width(Length::Fill),
                        text(format!(
                            "{} games · {}% White · {} decisive",
                            statistics.games, score, decisive
                        ))
                        .size(12)
                        .color(palette.text_secondary),
                    ]
                    .align_y(Alignment::Center),
                )
                .on_press(Msg::StudyOpeningMove((*uci).to_owned()))
                .padding(8)
                .width(Length::Fill),
            );
        }
        if explorer_moves.is_empty() {
            opening_rows = opening_rows.push(
                text("No library games reach this position.")
                    .size(13)
                    .color(palette.text_secondary),
            );
        }
        let library = settings_card(
            iced_fonts::lucide::database,
            "Game Library",
            column![
                text(format!("{game_count} locally indexed games"))
                    .size(20)
                    .color(palette.text_primary),
                text("PGN files are deduplicated, stored locally, and searchable by player, event, ECO, and Elo.")
                    .size(13)
                    .color(palette.text_secondary),
                row![
                    text_input("Search player, event, site, or ECO", &self.study_query)
                        .on_input(Msg::StudyQueryChanged)
                        .on_submit(Msg::SearchLibrary)
                        .padding(9)
                        .width(Length::Fill),
                    styled_button("Import PGN", Msg::ImportPgn),
                    styled_button("Save current", Msg::SaveToLibrary),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                scrollable(game_rows).height(Length::Fixed(340.0)),
            ]
            .spacing(10)
            .into(),
        );
        let coaching = settings_card(
            iced_fonts::lucide::graduation_cap,
            "Coach & Review",
            column![
                text("Move-quality vocabulary ready")
                    .size(18)
                    .color(palette.text_primary),
                text("Aura !!!, Brilliant !!, Great !, Best, Excellent, Good, OK, Book, Novelty, Inaccuracy, Mistake, and Blunder share one review model.")
                    .size(13)
                    .color(palette.text_secondary),
                text("Analysis uses the architecture-selected engine catalog with bounded memory.")
                    .size(13)
                    .color(palette.text_secondary),
            ]
            .spacing(10)
            .into(),
        );
        let training = settings_card(
            iced_fonts::lucide::target,
            "Training Queue",
            column![
                text(format!(
                    "{} positions · {} due today",
                    training_count,
                    self.training_due.len()
                ))
                .size(18)
                .color(palette.text_primary),
                text("Legal puzzle replay with persisted spaced-repetition scheduling.")
                    .size(13)
                    .color(palette.text_secondary),
                styled_button("Install starter set", Msg::SeedTraining),
                scrollable(due_rows).height(Length::Fixed(190.0)),
            ]
            .spacing(9)
            .into(),
        );
        let opening = settings_card(
            iced_fonts::lucide::book_open,
            "Opening Explorer",
            column![
                text(format!(
                    "{} games indexed · first 12 moves",
                    self.opening_indexed_games
                ))
                .size(18)
                .color(palette.text_primary),
                text("Select a move to continue legally on the board, then return here for the next position.")
                    .size(13)
                    .color(palette.text_secondary),
                scrollable(opening_rows).height(Length::Fixed(230.0)),
            ]
            .spacing(9)
            .into(),
        );
        let mut gambit_rows = column![].spacing(6);
        for lesson in gambit::catalog() {
            let id = lesson.id.to_owned();
            gambit_rows = gambit_rows.push(
                row![
                    column![
                        text(format!("{} ({})", lesson.name, lesson.eco))
                            .size(14)
                            .color(palette.text_primary),
                        text(lesson.summary).size(12).color(palette.text_secondary),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    styled_button("Learn", Msg::StartGambitLesson(id)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        let gambits = settings_card(
            iced_fonts::lucide::swords,
            "Gambit Laboratory",
            column![
                text("Interactive lines with numbered coaching arrows.")
                    .size(13)
                    .color(palette.text_secondary),
                scrollable(gambit_rows).height(Length::Fixed(220.0)),
            ]
            .spacing(9)
            .into(),
        );
        let content = column![
            text("Study Workspace").size(30).color(palette.text_primary),
            text("Games, coaching, openings, gambits, and training — interactive everywhere.")
                .size(14)
                .color(palette.text_secondary),
            library,
            row![coaching, training].spacing(16).width(Length::Fill),
            row![opening, gambits].spacing(16).width(Length::Fill),
            text(&self.status).size(13).color(palette.accent_alt),
        ]
        .spacing(16)
        .padding(24)
        .width(Length::Fill);
        column![
            self.view_title_bar(),
            scrollable(content).height(Length::Fill)
        ]
        .into()
    }

    fn view_analysis(&self) -> Element<'_, Msg> {
        // Analysis reuses the interactive board chrome and injects studio controls
        // into the game sidebar via `analysis_sidebar_panel`.
        if self.game.is_some() {
            self.view_game()
        } else {
            let palette = self.settings.board_theme.gui_palette();
            column![
                self.view_title_bar(),
                container(self.analysis_sidebar_panel(palette)).padding(24),
            ]
            .into()
        }
    }

    fn analysis_sidebar_panel(&self, palette: board_view::GuiPalette) -> Element<'_, Msg> {
        let mut engine_toggles = column![].spacing(6);
        let builtin_on = self
            .analysis_engines_selected
            .iter()
            .any(|id| id == "builtin");
        engine_toggles = engine_toggles.push(settings_row(
            "Mujrim (built-in)",
            toggler(builtin_on)
                .on_toggle(|_| Msg::ToggleAnalysisEngine("builtin".into()))
                .size(18)
                .into(),
        ));
        for engine in &self.bundled_engines {
            let id = engine.path.to_string_lossy().into_owned();
            let on = self.analysis_engines_selected.iter().any(|s| s == &id);
            engine_toggles = engine_toggles.push(settings_row(
                engine.display_name,
                toggler(on)
                    .on_toggle(move |_| Msg::ToggleAnalysisEngine(id.clone()))
                    .size(18)
                    .into(),
            ));
        }
        for engine in &self.external_engine_catalog {
            let id = engine.path.clone();
            let on = self.analysis_engines_selected.iter().any(|s| s == &id);
            let name = engine.name.clone();
            engine_toggles = engine_toggles.push(settings_row(
                name.as_str(),
                toggler(on)
                    .on_toggle(move |_| Msg::ToggleAnalysisEngine(id.clone()))
                    .size(18)
                    .into(),
            ));
        }
        let mut opinion_lines = column![].spacing(4);
        for arrow in self.analysis_arrows.iter().take(12) {
            let label = arrow
                .label
                .clone()
                .unwrap_or_else(|| format!("{}→{}", arrow.from, arrow.to));
            opinion_lines = opinion_lines.push(
                text(format!("{}. {label}", arrow.step.unwrap_or(0)))
                    .size(12)
                    .color(palette.text_secondary),
            );
        }
        if self.analysis_arrows.is_empty() {
            opinion_lines = opinion_lines.push(
                text("No multi-engine arrows yet.")
                    .size(12)
                    .color(palette.text_secondary),
            );
        }
        let gambit_controls: Element<'_, Msg> = if let Some(lesson) = self.active_gambit {
            column![
                text(format!("{} · {}", lesson.name, lesson.eco))
                    .size(14)
                    .color(palette.accent_alt),
                text(lesson.summary).size(12).color(palette.text_secondary),
                row![
                    styled_button("◀ Step", Msg::GambitStep(-1)),
                    text(format!("Ply {}", self.gambit_ply))
                        .size(13)
                        .color(palette.text_primary),
                    styled_button("Step ▶", Msg::GambitStep(1)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .into()
        } else {
            text("Load a gambit from Study for stepped coaching arrows.")
                .size(12)
                .color(palette.text_secondary)
                .into()
        };
        column![
            text("Multi-Engine Studio")
                .size(16)
                .color(palette.text_primary),
            text(&self.analysis_status)
                .size(12)
                .color(palette.text_secondary),
            config_slider(
                "MultiPV",
                self.analysis_multipv,
                "",
                1,
                5,
                Msg::SetAnalysisMultiPv,
            ),
            engine_toggles,
            styled_button("Run Multi-Engine Analysis", Msg::RunMultiEngineAnalysis),
            styled_button("Review Current Game", Msg::AnalyzeGame),
            text("Engine PV arrows")
                .size(13)
                .color(palette.text_primary),
            opinion_lines,
            text("Gambit coach").size(13).color(palette.text_primary),
            gambit_controls,
        ]
        .spacing(8)
        .into()
    }

    fn view_tournament_game_board(&self) -> Element<'_, Msg> {
        let palette = self.settings.board_theme.gui_palette();
        let live_focus = self
            .selected_tournament_game_id
            .is_none()
            .then(|| {
                tournament_arena::visible_live_boards(&self.live_tournament_view.live_games, 1)
                    .into_iter()
                    .next_back()
            })
            .flatten();
        let played = self
            .selected_tournament_game_id
            .and_then(|id| self.live_tournament_view.game(id));
        if played.is_none() && live_focus.is_none() && !self.tournament_review_active {
            return container(
                column![
                    text("Configure the tournament, then Start.")
                        .size(18)
                        .color(palette.text_primary),
                    text("Games play with real clocks on one full board — like Engine vs Engine.")
                        .size(13)
                        .color(palette.text_secondary),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }
        let Some(gs) = self.game.as_ref().filter(|_| self.tournament_review_active) else {
            return text("Loading tournament board…")
                .size(13)
                .color(palette.text_secondary)
                .into();
        };

        let title_bar_h = 48.0_f32;
        let padding = 40.0_f32;
        let available_h = (self.window_height - title_bar_h - padding - 72.0).max(200.0);
        let panel_w = (self.window_width * 0.23).clamp(250.0, 400.0);
        let available_board_w = (self.window_width - panel_w - padding * 2.0 - 20.0).max(224.0);
        let sq_size = (available_h * 0.90 / 8.0)
            .min(available_board_w / 8.0)
            .clamp(28.0, 120.0);

        let board = board_view::view_board(
            gs,
            &self.assets,
            board_view::BoardViewOptions {
                sq_size,
                anim: None,
                theme: self.settings.board_theme,
                piece_set: self.settings.piece_set,
                show_coords: self.settings.show_coords,
                coord_position: self.settings.coord_position,
                capture_anim_style: self.settings.capture_anim_style,
                overlay_arrows: &gs.overlay_arrows,
                user_arrows: &gs.arrows,
                arrow_appearance: arrows::ArrowAppearance {
                    shape: self.settings.arrow_shape,
                    color: self.settings.arrow_color,
                    size: self.settings.arrow_size,
                },
                display_board: self.review_board.as_ref(),
                show_legal_moves: false,
                show_last_move: self.settings.show_last_move,
                annotation_badge: review_annotation_badge(
                    &self.initial_fen,
                    &self.move_log,
                    self.review_ply,
                    &self.move_annotations,
                ),
            },
        );

        let title = if let Some(played) = played {
            played.title()
        } else if let Some(live) = &live_focus {
            format!("Live R{} · {} vs {}", live.round, live.white, live.black)
        } else {
            "Tournament board".to_owned()
        };
        let white_clock = live_focus
            .as_ref()
            .and_then(|live| live.white_clock_ms)
            .or_else(|| {
                Some(
                    self.tournament_setup
                        .time_control
                        .match_clock()
                        .initial
                        .as_millis() as u64,
                )
            });
        let black_clock = live_focus
            .as_ref()
            .and_then(|live| live.black_clock_ms)
            .or(white_clock);
        let clock_row = row![
            text(format!(
                "White {}  {}",
                live_focus
                    .as_ref()
                    .map(|live| live.white.as_str())
                    .or_else(|| played.map(|game| game.white.as_str()))
                    .unwrap_or("—"),
                tournament_live::format_clock_ms(white_clock)
            ))
            .size(14)
            .color(palette.text_primary),
            Space::new().width(Length::Fill),
            text(format!(
                "{}  {} Black",
                tournament_live::format_clock_ms(black_clock),
                live_focus
                    .as_ref()
                    .map(|live| live.black.as_str())
                    .or_else(|| played.map(|game| game.black.as_str()))
                    .unwrap_or("—"),
            ))
            .size(14)
            .color(palette.text_primary),
        ]
        .width(Length::Fill);

        let subtitle = if let Some(played) = played {
            format!("Result {}", played.result_label())
        } else if let Some(live) = &live_focus {
            format!(
                "Last {} · {} · d{} · {} nodes",
                if live.last_uci.is_empty() {
                    "—"
                } else {
                    live.last_uci.as_str()
                },
                tournament_arena::score_text(live.score_cp),
                live.depth,
                live.nodes
            )
        } else {
            self.engine_info.clone()
        };

        let moves_content: Element<'_, Msg> = if self.move_log.is_empty() {
            text("No moves yet.")
                .size(13)
                .color(palette.text_secondary)
                .into()
        } else {
            let mut moves_col = column![].spacing(2).padding([4, 8]);
            for (i, pair) in self.move_log.chunks(2).enumerate() {
                let white_index = i * 2;
                let white_move = move_history_button(
                    annotated_move_label(
                        &pair[0],
                        self.move_annotations.get(white_index).copied().flatten(),
                    ),
                    white_index + 1,
                    self.review_ply == Some(white_index + 1),
                    palette,
                );
                let black_move = if let Some(black) = pair.get(1) {
                    move_history_button(
                        annotated_move_label(
                            black,
                            self.move_annotations
                                .get(white_index + 1)
                                .copied()
                                .flatten(),
                        ),
                        white_index + 2,
                        self.review_ply == Some(white_index + 2),
                        palette,
                    )
                } else {
                    container(text("…").size(13).color(palette.text_secondary))
                        .width(88)
                        .into()
                };
                moves_col = moves_col.push(
                    row![
                        text(format!("{}.", i + 1))
                            .size(12)
                            .color(palette.text_secondary)
                            .width(32),
                        white_move,
                        black_move
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
            scrollable(moves_col).height(Length::Fill).into()
        };

        let mut side = column![
            text(title).size(16).color(palette.text_primary),
            text(subtitle).size(12).color(palette.text_secondary),
            text("Moves").size(13).color(palette.text_secondary),
            container(moves_content)
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_theme| container::Style {
                    background: Some(iced::Background::Color(palette.sidebar)),
                    border: iced::Border {
                        radius: 6.0.into(),
                        width: 1.0,
                        color: palette.border,
                    },
                    ..Default::default()
                }),
        ]
        .spacing(8)
        .width(panel_w);

        if self.review_ply.is_some() {
            side = side.push(styled_button(
                "Return to final position",
                Msg::ReturnToLivePosition,
            ));
        }
        if played.is_some() {
            side = side.push(
                row![
                    styled_button("Analyze game", Msg::AnalyzeGame),
                    styled_button("Open in Analysis", Msg::OpenAnalysis),
                ]
                .spacing(8),
            );
        }

        column![
            clock_row,
            Space::new().height(8),
            row![container(board).width(Length::Fill), side,]
                .spacing(16)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_tournament_hub(&self) -> Element<'_, Msg> {
        let palette = self.settings.board_theme.gui_palette();
        let live = &self.live_tournament_view;
        let running = self.live_tournament.is_some();

        let mut toolbar = row![].spacing(8);
        if !running {
            toolbar = toolbar.push(styled_button(
                if self.show_tournament_setup {
                    "Hide setup"
                } else {
                    "Tournament setup"
                },
                Msg::ToggleTournamentSetup,
            ));
        } else {
            toolbar = toolbar.push(styled_button("Cancel safely", Msg::CancelTournament));
        }
        toolbar = toolbar.push(styled_button(
            if self.show_tournament_results {
                "Hide results"
            } else {
                "Results"
            },
            Msg::ToggleTournamentResults,
        ));

        let status = column![
            text(if running {
                "Tournament · Live"
            } else if live.finished {
                "Tournament · Finished"
            } else {
                "Tournament"
            })
            .size(22)
            .color(palette.text_primary),
            text(format!(
                "{} · {} · {}/{} ({:.0}%) · {}",
                live.format_label,
                self.tournament_setup.time_control.label(),
                live.completed_matches,
                live.total_matches.max(live.completed_matches),
                live.progress_fraction() * 100.0,
                live.current_match_label()
            ))
            .size(13)
            .color(palette.text_secondary),
            text(&self.tournament_status)
                .size(12)
                .color(palette.accent_alt),
            toolbar,
        ]
        .spacing(6);

        let mut body = column![
            status,
            Space::new().height(8),
            self.view_tournament_game_board()
        ]
        .spacing(6)
        .padding([12, 20])
        .width(Length::Fill)
        .height(Length::Fill);

        if tournament_results::panel_open(live, self.show_tournament_results) {
            let mut standings_col =
                column![text("Standings").size(14).color(palette.text_primary)].spacing(4);
            if !tournament_results::standings_ready(&live.standings) {
                standings_col = standings_col.push(
                    text("Standings appear after the first finished pairing.")
                        .size(12)
                        .color(palette.text_secondary),
                );
            } else {
                for row in &live.standings {
                    let perf = row
                        .performance
                        .map(|elo| format!(" · {elo:.0} Elo"))
                        .unwrap_or_default();
                    standings_col = standings_col.push(
                        text(format!(
                            "{}. {}  {:.1}  ({}-{}-{}){perf}",
                            row.rank, row.name, row.points, row.wins, row.draws, row.losses
                        ))
                        .size(12)
                        .color(palette.text_primary),
                    );
                }
            }
            let mut games_col =
                column![text("Games").size(14).color(palette.text_primary)].spacing(4);
            for row in live.finished_matches.iter().rev().take(6) {
                games_col =
                    games_col.push(text(row.label()).size(11).color(palette.text_secondary));
            }
            for game in tournament_arena::finished_strip(&live.played_games, 16) {
                games_col = games_col.push(tournament_game_button(
                    game.title(),
                    format!("{} plies", game.moves.len()),
                    game.id,
                    self.selected_tournament_game_id == Some(game.id),
                    palette,
                ));
            }

            let mut history = column![
                text("Saved events").size(14).color(palette.text_primary),
                styled_button("Refresh history", Msg::RefreshTournamentHistory),
            ]
            .spacing(4);
            if self.stored_tournaments.is_empty() {
                history = history.push(
                    text("No saved tournaments yet.")
                        .size(12)
                        .color(palette.text_secondary),
                );
            } else {
                for tournament in self.stored_tournaments.iter().take(8) {
                    let selected_id =
                        self.selected_tournament_id.as_deref() == Some(tournament.id.as_str());
                    history = history.push(tournament_history_button(
                        tournament.name.clone(),
                        tournament.status.clone(),
                        tournament.id.clone(),
                        selected_id,
                        palette,
                    ));
                }
            }

            body = body.push(Space::new().height(8)).push(
                row![
                    container(scrollable(standings_col).height(Length::Fixed(160.0)))
                        .width(Length::FillPortion(3))
                        .padding(8)
                        .style(move |_theme| container::Style {
                            background: Some(iced::Background::Color(palette.sidebar)),
                            border: iced::Border {
                                radius: 8.0.into(),
                                width: 1.0,
                                color: palette.border,
                            },
                            ..Default::default()
                        }),
                    Space::new().width(12),
                    container(scrollable(games_col).height(Length::Fixed(160.0)))
                        .width(Length::FillPortion(2))
                        .padding(8)
                        .style(move |_theme| container::Style {
                            background: Some(iced::Background::Color(palette.sidebar)),
                            border: iced::Border {
                                radius: 8.0.into(),
                                width: 1.0,
                                color: palette.border,
                            },
                            ..Default::default()
                        }),
                    Space::new().width(12),
                    container(scrollable(history).height(Length::Fixed(160.0)))
                        .width(Length::FillPortion(2))
                        .padding(8)
                        .style(move |_theme| container::Style {
                            background: Some(iced::Background::Color(palette.sidebar)),
                            border: iced::Border {
                                radius: 8.0.into(),
                                width: 1.0,
                                color: palette.border,
                            },
                            ..Default::default()
                        }),
                ]
                .width(Length::Fill),
            );
        }

        column![self.view_title_bar(), body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_tournament_setup_modal(&self) -> Element<'_, Msg> {
        let palette = self.settings.board_theme.gui_palette();
        let setup = &self.tournament_setup;
        let roster_engines =
            tournament_engine_roster(&self.bundled_engines, &self.external_engine_catalog);

        let mut engines = column![].spacing(6);
        engines = engines.push(
            text("Engines from this UI's local engines/ folder. Native builds preferred; x64 may run via emulation on Arm64.")
                .size(12)
                .color(palette.text_secondary),
        );
        let mut engine_rows = column![].spacing(6);
        for engine in &roster_engines {
            let selected = setup
                .selected_engine_paths
                .iter()
                .any(|path| path == &engine.path);
            let path_key = engine.path.display().to_string();
            engine_rows = engine_rows.push(
                row![
                    styled_button(
                        if selected { "Selected" } else { "Select" },
                        Msg::TournamentToggleEngine(path_key),
                    ),
                    text(engine.name.clone())
                        .size(14)
                        .color(palette.text_primary)
                        .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if roster_engines.is_empty() {
            engine_rows = engine_rows.push(
                text("No UCI engines were found under the local engines/ folder.")
                    .size(13)
                    .color(palette.text_secondary),
            );
        }
        engines = engines.push(
            scrollable(engine_rows)
                .height(Length::Fixed(240.0))
                .width(Length::Fill),
        );

        let setup_form = column![
            text_input("Event name", &setup.event)
                .on_input(Msg::TournamentEventNameChanged)
                .width(Length::Fill),
            text_input("Site (optional)", &setup.site)
                .on_input(Msg::TournamentSiteChanged)
                .width(Length::Fill),
            settings_row(
                "Format",
                pick_list(
                    TournamentFormat::ALL,
                    Some(setup.format),
                    Msg::SelectTournamentFormat,
                )
                .width(220)
                .into(),
            ),
            settings_row(
                "Time control",
                pick_list(
                    tournament_setup::TimeControlPreset::ALL,
                    Some(setup.time_control),
                    Msg::TournamentTimeControlChanged,
                )
                .width(220)
                .into(),
            ),
            config_slider(
                "Games / pairing",
                setup.games_per_encounter as i32,
                "",
                1,
                8,
                Msg::TournamentGamesPerEncounterChanged,
            ),
            config_slider(
                "Hash",
                setup.hash_mb as i32,
                "MiB",
                16,
                512,
                Msg::TournamentHashChanged,
            ),
            config_slider(
                "Threads",
                setup.engine_threads as i32,
                "",
                1,
                8,
                Msg::TournamentThreadsChanged,
            ),
            text("Games use one board at a time with live clocks (not instant).")
                .size(12)
                .color(palette.text_secondary),
        ]
        .spacing(8);

        let modal_header = mouse_area(
            container(
                row![
                    iced_fonts::lucide::grip_horizontal()
                        .size(16)
                        .color(TEXT_SECONDARY),
                    text("Tournament Setup").size(20).color(TEXT_PRIMARY),
                    Space::new().width(Length::Fill),
                    text("Drag to move").size(10).color(TEXT_SECONDARY),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([5, 6]),
        )
        .interaction(iced::mouse::Interaction::Grab)
        .on_press(Msg::StartTournamentSetupDrag);

        let modal_content = container(
            column![
                modal_header,
                Space::new().height(8),
                scrollable(
                    column![
                        settings_card(iced_fonts::lucide::cpu, "Players", engines.into()),
                        settings_card(iced_fonts::lucide::trophy, "Event", setup_form.into()),
                        text(&self.tournament_status)
                            .size(12)
                            .color(palette.accent_alt),
                        row![
                            styled_button("Start tournament", Msg::RunQuickTournament),
                            styled_button("Close", Msg::ToggleTournamentSetup),
                        ]
                        .spacing(8),
                    ]
                    .spacing(12)
                    .width(Length::Fill),
                )
                .height(520),
            ]
            .spacing(0)
            .width(560),
        )
        .padding(24)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.08, 0.08, 0.16, 0.96,
            ))),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            ..Default::default()
        });

        let modal_width = 608.0;
        let modal_height = 680.0;
        let left = ((self.window_width - modal_width) * 0.5 + self.tournament_setup_offset.x)
            .clamp(0.0, (self.window_width - modal_width).max(0.0));
        let top = ((self.window_height - modal_height) * 0.5 + self.tournament_setup_offset.y)
            .clamp(0.0, (self.window_height - modal_height).max(0.0));

        container(
            column![
                Space::new().height(top),
                row![Space::new().width(left), modal_content],
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.0, 0.0, 0.0, 0.55,
            ))),
            ..Default::default()
        })
        .into()
    }

    fn view_options_modal(&self) -> Element<'_, Msg> {
        let s = &self.settings;

        // Display section
        let theme_picker = pick_list(
            board_view::BoardTheme::ALL.to_vec(),
            Some(s.board_theme),
            Msg::SetBoardTheme,
        )
        .width(160);
        let piece_set_picker = pick_list(
            pieces::PieceSet::ALL.to_vec(),
            Some(s.piece_set),
            Msg::SetPieceSet,
        )
        .width(160);

        let anim_label = AnimPace::from_setting(s.anim_speed).label();

        let capture_anim_picker = pick_list(
            vec![
                CaptureAnimStyle::Explosion,
                CaptureAnimStyle::Fire,
                CaptureAnimStyle::Instant,
            ],
            Some(s.capture_anim_style),
            Msg::SetCaptureAnim,
        )
        .width(120);

        let coord_pos_picker = pick_list(
            vec![CoordPosition::Inside, CoordPosition::Outside],
            Some(s.coord_position),
            Msg::SetCoordPosition,
        )
        .width(120);

        let display_section = settings_card(
            iced_fonts::lucide::monitor,
            "Display",
            column![
                settings_row("Board Theme", theme_picker.into(),),
                settings_row("Piece Set", piece_set_picker.into(),),
                settings_row(
                    "Show Coordinates",
                    toggler(s.show_coords)
                        .on_toggle(Msg::SetShowCoords)
                        .size(18)
                        .into(),
                ),
                settings_row("Coord Position", coord_pos_picker.into(),),
                settings_row(
                    "Animation",
                    row![
                        slider(0..=2, s.anim_speed, Msg::SetAnimSpeed).width(100),
                        text(anim_label).size(12).color(TEXT_PRIMARY).width(60),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
                ),
                settings_row("Capture Effect", capture_anim_picker.into(),),
            ]
            .spacing(2)
            .into(),
        );

        // Audio section
        let mood_picker = pick_list(
            vec![
                audio::GameMood::Playful,
                audio::GameMood::Joyful,
                audio::GameMood::Mystique,
            ],
            Some(s.game_mood),
            Msg::SetGameMood,
        )
        .width(120);
        let sound_theme_picker = pick_list(
            vec![
                audio::SoundTheme::Wood,
                audio::SoundTheme::Crystal,
                audio::SoundTheme::Soft,
            ],
            Some(s.sound_theme),
            Msg::SetSoundTheme,
        )
        .width(120);

        let audio_section = settings_card(
            iced_fonts::lucide::volume,
            "Audio",
            column![
                settings_row(
                    "Background Music",
                    toggler(self.bgm_on)
                        .on_toggle(|_| Msg::ToggleBGM)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Sound Effects",
                    toggler(s.sfx_on).on_toggle(Msg::SetSfx).size(18).into(),
                ),
                settings_row("Sound Theme", sound_theme_picker.into(),),
                settings_row(
                    "BGM Volume",
                    row![
                        slider(0..=100, s.bgm_volume, Msg::SetBgmVolume).width(100),
                        text(format!("{}%", s.bgm_volume))
                            .size(12)
                            .color(TEXT_PRIMARY)
                            .width(50),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
                ),
                settings_row("Game Mood", mood_picker.into(),),
            ]
            .spacing(2)
            .into(),
        );

        let arrow_shape_picker = pick_list(
            vec![arrows::ArrowShape::Smart, arrows::ArrowShape::Straight],
            Some(s.arrow_shape),
            Msg::SetArrowShape,
        )
        .width(140);
        let arrow_color_picker = pick_list(
            vec![
                arrows::ArrowColor::Orange,
                arrows::ArrowColor::Green,
                arrows::ArrowColor::Blue,
                arrows::ArrowColor::Red,
            ],
            Some(s.arrow_color),
            Msg::SetArrowColor,
        )
        .width(120);
        let arrow_size_picker = pick_list(
            vec![
                arrows::ArrowSize::Slim,
                arrows::ArrowSize::Normal,
                arrows::ArrowSize::Bold,
            ],
            Some(s.arrow_size),
            Msg::SetArrowSize,
        )
        .width(120);

        // Game section
        let game_section = settings_card(
            iced_fonts::lucide::gamepad,
            "Game",
            column![
                settings_row(
                    "Auto-flip for Black",
                    toggler(s.auto_flip_black)
                        .on_toggle(Msg::SetAutoFlip)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Show Legal Moves",
                    toggler(s.show_legal_moves)
                        .on_toggle(Msg::SetShowLegal)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Show Last Move",
                    toggler(s.show_last_move)
                        .on_toggle(Msg::SetShowLastMove)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Premoves",
                    toggler(s.premoves_enabled)
                        .on_toggle(Msg::SetPremoves)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Multi-Premoves",
                    toggler(s.multi_premoves)
                        .on_toggle(Msg::SetMultiPremoves)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Draw Arrows",
                    toggler(s.draw_arrows)
                        .on_toggle(Msg::SetDrawArrows)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Last-Move Arrow",
                    toggler(s.last_move_arrow)
                        .on_toggle(Msg::SetLastMoveArrow)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Ponder Arrow",
                    toggler(s.ponder_arrow)
                        .on_toggle(Msg::SetPonderArrow)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Piece Slide",
                    toggler(s.piece_slide)
                        .on_toggle(Msg::SetPieceSlide)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "System Motion",
                    toggler(s.system_motion)
                        .on_toggle(Msg::SetSystemMotion)
                        .size(18)
                        .into(),
                ),
                settings_row("Arrow Shape", arrow_shape_picker.into()),
                settings_row("Arrow Color", arrow_color_picker.into()),
                settings_row("Arrow Weight", arrow_size_picker.into()),
            ]
            .spacing(2)
            .into(),
        );

        let close_btn = button(
            container(
                row![
                    iced_fonts::lucide::x().size(14),
                    text("Close").size(14).color(Color::WHITE),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .center_x(100)
            .center_y(36)
            .width(100)
            .height(36),
        )
        .on_press(Msg::ToggleOptions)
        .style(|_theme, status| {
            let bg = if matches!(status, button::Status::Hovered) {
                ACCENT
            } else {
                Color::from_rgb(0.3, 0.2, 0.2)
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                text_color: Color::WHITE,
                ..Default::default()
            }
        });

        // Tab switcher
        let settings_tab_active = self.options_tab == OptionsTab::Settings;
        let settings_color = if settings_tab_active {
            Color::WHITE
        } else {
            TEXT_SECONDARY
        };
        let tools_color = if !settings_tab_active {
            Color::WHITE
        } else {
            TEXT_SECONDARY
        };
        let tab_buttons = row![
            button(
                row![
                    iced_fonts::lucide::settings()
                        .size(13)
                        .color(settings_color),
                    text(" Settings").size(13).color(settings_color)
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            )
            .on_press(Msg::SwitchOptionsTab(OptionsTab::Settings))
            .padding([6, 16])
            .style(move |_theme, _status| button::Style {
                background: Some(iced::Background::Color(if settings_tab_active {
                    ACCENT
                } else {
                    Color::TRANSPARENT
                })),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                text_color: Color::WHITE,
                ..Default::default()
            }),
            button(
                row![
                    iced_fonts::lucide::wrench().size(13).color(tools_color),
                    text(" Tools").size(13).color(tools_color)
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            )
            .on_press(Msg::SwitchOptionsTab(OptionsTab::Tools))
            .padding([6, 16])
            .style(move |_theme, _status| button::Style {
                background: Some(iced::Background::Color(if !settings_tab_active {
                    ACCENT
                } else {
                    Color::TRANSPARENT
                })),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                text_color: Color::WHITE,
                ..Default::default()
            }),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        // Build tab content
        let tab_content: Element<'_, Msg> = if settings_tab_active {
            scrollable(column![display_section, audio_section, game_section,].spacing(12))
                .height(450)
                .into()
        } else {
            // Tools tab
            let syzygy_set_picker = pick_list(
                vec![
                    updater::syzygy::SyzygyPieceSet::Standard,
                    updater::syzygy::SyzygyPieceSet::Extended,
                    updater::syzygy::SyzygyPieceSet::Full,
                ],
                Some(self.syzygy_piece_set),
                Msg::SelectSyzygyPieceSet,
            )
            .width(245);
            let syzygy_warning = match self.syzygy_piece_set {
                updater::syzygy::SyzygyPieceSet::Standard => {
                    "Complete 3-5 piece WDL + DTZ; recommended for this device."
                }
                updater::syzygy::SyzygyPieceSet::Extended => {
                    "Requires about 149 GiB. Verify free storage before starting."
                }
                updater::syzygy::SyzygyPieceSet::Full => {
                    "Requires about 16.7 TiB. Intended for dedicated tablebase storage."
                }
            };
            let syzygy_section = settings_card(
                iced_fonts::lucide::database,
                "Syzygy Tablebases",
                column![
                    settings_row(
                        "Status",
                        text(&self.syzygy_status)
                            .size(12)
                            .color(TEXT_PRIMARY)
                            .into()
                    ),
                    settings_row(
                        "Path",
                        text(updater::syzygy::default_syzygy_path().display().to_string())
                            .size(12)
                            .color(TEXT_PRIMARY)
                            .into()
                    ),
                    settings_row("Coverage", syzygy_set_picker.into()),
                    text(syzygy_warning).size(11).color(TEXT_SECONDARY),
                    Space::new().height(4),
                    styled_button_with_icon(
                        iced_fonts::lucide::download,
                        "Download / Resume Selected Tables",
                        Msg::SyzygyDownload
                    ),
                ]
                .spacing(2)
                .into(),
            );

            let nnue_section = settings_card(
                iced_fonts::lucide::brain,
                "NNUE Networks",
                column![
                    settings_row(
                        "Status",
                        text(&self.nnue_status).size(12).color(TEXT_PRIMARY).into()
                    ),
                    settings_row("Path", text("./nnue/").size(12).color(TEXT_PRIMARY).into()),
                    Space::new().height(4),
                    styled_button_with_icon(
                        iced_fonts::lucide::download,
                        "Download All NNUE Networks",
                        Msg::NnueDownload
                    ),
                ]
                .spacing(2)
                .into(),
            );

            let mut tuning_content = column![settings_row(
                "Status",
                text(&self.tuning_status)
                    .size(12)
                    .color(TEXT_PRIMARY)
                    .into()
            ),]
            .spacing(2);

            if let Some(ref params) = self.tuning_params {
                let flat = params.flat_list();
                let mut last_section = String::new();
                for (section, name, param) in flat.into_iter().take(20) {
                    if section != last_section {
                        let sec_label = section.clone();
                        tuning_content =
                            tuning_content.push(text(sec_label).size(11).color(ACCENT_TEAL));
                        last_section = section.clone();
                    }
                    let sec = section;
                    let nm = name.clone();
                    let name_label = name;
                    let min_i = param.min_i32();
                    let max_i = param.max_i32();
                    let val_i = param.value_i32();
                    tuning_content = tuning_content.push(settings_row(
                        &name_label,
                        row![
                            slider(min_i..=max_i, val_i, move |v| Msg::TuneSetParam(
                                sec.clone(),
                                nm.clone(),
                                v as f64
                            ))
                            .width(100),
                            text(format!("{val_i}"))
                                .size(11)
                                .color(TEXT_PRIMARY)
                                .width(50),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .into(),
                    ));
                }
                tuning_content = tuning_content.push(Space::new().height(4));
                tuning_content = tuning_content.push(
                    row![
                        styled_button("Save params.toml", Msg::TuneSave),
                        styled_button("Reload", Msg::TuneLoad),
                    ]
                    .spacing(8),
                );
            } else {
                tuning_content =
                    tuning_content.push(styled_button("Load params.toml", Msg::TuneLoad));
            }

            let tuning_section = settings_card(
                iced_fonts::lucide::sliders_horizontal,
                "Parameter Tuning",
                tuning_content.into(),
            );

            let updates_section = settings_card(
                iced_fonts::lucide::refresh_cw,
                "Updates",
                column![styled_button("Check for Updates", Msg::CheckForUpdates),]
                    .spacing(4)
                    .into(),
            );

            scrollable(
                column![
                    syzygy_section,
                    nnue_section,
                    tuning_section,
                    updates_section,
                ]
                .spacing(12),
            )
            .height(450)
            .into()
        };

        let modal_header = mouse_area(
            container(
                row![
                    iced_fonts::lucide::grip_horizontal()
                        .size(16)
                        .color(TEXT_SECONDARY),
                    text("Options").size(20).color(TEXT_PRIMARY),
                    Space::new().width(Length::Fill),
                    text("Drag to move").size(10).color(TEXT_SECONDARY),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([5, 6]),
        )
        .interaction(iced::mouse::Interaction::Grab)
        .on_press(Msg::StartOptionsDrag);

        let modal_content = container(
            column![
                modal_header,
                Space::new().height(8),
                tab_buttons,
                Space::new().height(12),
                tab_content,
                Space::new().height(16),
                close_btn,
            ]
            .spacing(0)
            .align_x(Alignment::Center)
            .width(520),
        )
        .padding(24)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.08, 0.08, 0.16, 0.96,
            ))),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            ..Default::default()
        });

        let modal_width = 568.0;
        let modal_height = 620.0;
        let left = ((self.window_width - modal_width) * 0.5 + self.options_offset.x)
            .clamp(0.0, (self.window_width - modal_width).max(0.0));
        let top = ((self.window_height - modal_height) * 0.5 + self.options_offset.y)
            .clamp(0.0, (self.window_height - modal_height).max(0.0));

        // Dark backdrop with an in-window draggable panel.
        container(
            column![
                Space::new().height(top),
                row![Space::new().width(left), modal_content],
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.0, 0.0, 0.0, 0.6,
            ))),
            ..Default::default()
        })
        .into()
    }
}

/// Translucent glass card with rounded corners for landing screen.
fn glass_card(
    content: Element<'_, Msg>,
    pal: board_view::GuiPalette,
    alpha: f32,
) -> Element<'_, Msg> {
    let alpha = alpha.clamp(0.0, 1.0);
    container(content)
        .padding(18)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                pal.panel.r,
                pal.panel.g,
                pal.panel.b,
                0.72 + 0.18 * alpha,
            ))),
            border: iced::Border {
                radius: 16.0.into(),
                width: 1.0,
                color: Color::from_rgba(
                    pal.border.r,
                    pal.border.g,
                    pal.border.b,
                    0.35 + 0.45 * alpha,
                ),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.28 * alpha),
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 18.0,
            },
            ..Default::default()
        })
        .into()
}

/// Styled secondary button matching the macOS dark theme.
fn styled_button(label: &str, msg: Msg) -> Element<'_, Msg> {
    let label_text = label.to_string();
    button(
        container(text(label_text).size(12).color(TEXT_PRIMARY))
            .center_x(Length::Shrink)
            .center_y(30)
            .height(30)
            .padding([0, 12]),
    )
    .on_press(msg)
    .style(|_theme, status| {
        let bg = if matches!(status, button::Status::Hovered) {
            Color::from_rgb(0.28, 0.28, 0.30)
        } else {
            BG_PANEL
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            text_color: TEXT_PRIMARY,
            ..Default::default()
        }
    })
    .into()
}

/// Creates a Lucide icon Element at a standard pill-button size.
fn lucide_icon<'a>(
    icon_fn: fn() -> iced::widget::Text<'a, iced::Theme, iced::Renderer>,
) -> Element<'a, Msg> {
    icon_fn().size(14).into()
}

/// Adds explicit resize hit targets to borderless custom chrome. Winit cannot
/// infer these edges once native decorations are disabled.
fn window_resize_frame<'a>(content: Element<'a, Msg>) -> Element<'a, Msg> {
    use iced::alignment::{Horizontal, Vertical};
    use iced::window::Direction;

    const EDGE: f32 = 7.0;
    const CORNER: f32 = 14.0;

    let horizontal = |direction, top: bool| {
        let interaction = iced::mouse::Interaction::ResizingVertically;
        container(
            mouse_area(Space::new().width(Length::Fill).height(EDGE))
                .interaction(interaction)
                .on_press(Msg::ResizeWindow(direction)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(if top { Vertical::Top } else { Vertical::Bottom })
    };
    let vertical = |direction, left: bool| {
        let interaction = iced::mouse::Interaction::ResizingHorizontally;
        container(
            mouse_area(Space::new().width(EDGE).height(Length::Fill))
                .interaction(interaction)
                .on_press(Msg::ResizeWindow(direction)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(if left {
            Horizontal::Left
        } else {
            Horizontal::Right
        })
    };
    let corner = |direction, horizontal, vertical, interaction| {
        container(
            mouse_area(Space::new().width(CORNER).height(CORNER))
                .interaction(interaction)
                .on_press(Msg::ResizeWindow(direction)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(horizontal)
        .align_y(vertical)
    };

    iced::widget::stack![
        content,
        horizontal(Direction::North, true),
        horizontal(Direction::South, false),
        vertical(Direction::West, true),
        vertical(Direction::East, false),
        corner(
            Direction::NorthWest,
            Horizontal::Left,
            Vertical::Top,
            iced::mouse::Interaction::ResizingDiagonallyDown,
        ),
        corner(
            Direction::NorthEast,
            Horizontal::Right,
            Vertical::Top,
            iced::mouse::Interaction::ResizingDiagonallyUp,
        ),
        corner(
            Direction::SouthWest,
            Horizontal::Left,
            Vertical::Bottom,
            iced::mouse::Interaction::ResizingDiagonallyUp,
        ),
        corner(
            Direction::SouthEast,
            Horizontal::Right,
            Vertical::Bottom,
            iced::mouse::Interaction::ResizingDiagonallyDown,
        ),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Compact icon-only window control with a hover tooltip.
fn window_icon_button<'a>(
    icon: Element<'a, Msg>,
    label: &'a str,
    pal: board_view::GuiPalette,
    destructive: bool,
    msg: Msg,
) -> Element<'a, Msg> {
    let control = button(
        container(icon)
            .center_x(28)
            .center_y(28)
            .width(28)
            .height(28),
    )
    .on_press(msg)
    .padding(0)
    .style(move |_theme, status| {
        let background = match status {
            button::Status::Hovered if destructive => Color::from_rgba(0.90, 0.18, 0.22, 0.75),
            button::Status::Hovered => {
                Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.24)
            }
            button::Status::Pressed => {
                Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.36)
            }
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(iced::Background::Color(background)),
            border: iced::Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            text_color: pal.text_primary,
            ..Default::default()
        }
    });

    iced::widget::tooltip(
        control,
        container(text(label).size(11)).padding([4, 7]),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(4)
    .into()
}

/// Styled secondary button with a Lucide icon prefix.
fn styled_button_with_icon<'a>(
    icon_fn: fn() -> iced::widget::Text<'a, iced::Theme, iced::Renderer>,
    label: &str,
    msg: Msg,
) -> Element<'a, Msg> {
    let label_text = label.to_string();
    button(
        container(
            row![
                icon_fn().size(12),
                text(label_text).size(12).color(TEXT_PRIMARY)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Shrink)
        .center_y(30)
        .height(30)
        .padding([0, 12]),
    )
    .on_press(msg)
    .style(|_theme, status| {
        let bg = if matches!(status, button::Status::Hovered) {
            Color::from_rgb(0.28, 0.28, 0.30)
        } else {
            BG_PANEL
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            text_color: TEXT_PRIMARY,
            ..Default::default()
        }
    })
    .into()
}

/// Pill-shaped title bar button with icon + label, accent hover glow.
fn pill_button<'a>(
    icon: Element<'a, Msg>,
    label: &'a str,
    pal: board_view::GuiPalette,
    destructive: bool,
    msg: Msg,
) -> Element<'a, Msg> {
    let content = row![icon, text(label).size(11),]
        .spacing(4)
        .align_y(Alignment::Center);

    button(content)
        .on_press(msg)
        .padding([4, 10])
        .style(move |_theme, status| {
            let (bg, border_c) = match status {
                button::Status::Hovered => {
                    if destructive {
                        (
                            Color::from_rgba(0.85, 0.20, 0.25, 0.30),
                            Color::from_rgba(0.85, 0.20, 0.25, 0.50),
                        )
                    } else {
                        (
                            Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.18),
                            Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.35),
                        )
                    }
                }
                button::Status::Pressed => (
                    Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.28),
                    Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.50),
                ),
                _ => (Color::TRANSPARENT, Color::TRANSPARENT),
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::Border {
                    radius: 999.0.into(),
                    width: 1.0,
                    color: border_c,
                },
                text_color: if destructive {
                    Color::from_rgba(
                        pal.text_primary.r,
                        pal.text_primary.g,
                        pal.text_primary.b,
                        0.85,
                    )
                } else {
                    pal.text_primary
                },
                ..Default::default()
            }
        })
        .into()
}

/// Subtle vertical separator between button groups in the pill bar.
fn pill_sep<'a>(pal: board_view::GuiPalette) -> Element<'a, Msg> {
    container(Space::new().width(0))
        .width(1)
        .height(20)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                pal.border.r,
                pal.border.g,
                pal.border.b,
                0.5,
            ))),
            ..Default::default()
        })
        .into()
}

/// Engine config slider row: "Label  [====]  Value Unit"
fn config_slider<'a, F>(
    label: &'a str,
    value: i32,
    unit: &str,
    min: i32,
    max: i32,
    on_change: F,
) -> Element<'a, Msg>
where
    F: 'a + Fn(i32) -> Msg,
{
    let display = if unit.is_empty() {
        format!("{value}")
    } else {
        format!("{value} {unit}")
    };
    row![
        text(label).size(12).color(TEXT_SECONDARY).width(90),
        slider(min..=max, value, on_change).width(130),
        text(display).size(12).color(TEXT_PRIMARY).width(60),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// A settings row with a label and a control widget, consistently aligned.
fn settings_row<'a>(label: &str, control: Element<'a, Msg>) -> Element<'a, Msg> {
    let label_owned = label.to_string();
    container(
        row![
            text(label_owned).size(12).color(TEXT_SECONDARY).width(180),
            control,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([6, 8])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: None,
        ..Default::default()
    })
    .into()
}

/// A glass-card section with a Lucide icon header and inner content.
fn settings_card<'a>(
    icon_fn: fn() -> iced::widget::Text<'a, iced::Theme, iced::Renderer>,
    title: &str,
    content: Element<'a, Msg>,
) -> Element<'a, Msg> {
    let title_owned = title.to_string();
    container(
        column![
            row![
                icon_fn().size(14).color(ACCENT_TEAL),
                text(title_owned).size(14).color(ACCENT_TEAL),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            Space::new().height(6),
            content,
        ]
        .spacing(0),
    )
    .padding(12)
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(Color::from_rgba(
            1.0, 1.0, 1.0, 0.04,
        ))),
        border: iced::Border {
            radius: 8.0.into(),
            width: 1.0,
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.06),
        },
        ..Default::default()
    })
    .into()
}

async fn pick_engine_file() -> Option<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Select UCI Engine Executable")
        .pick_file()
        .await?;
    Some(file.path().to_string_lossy().to_string())
}

async fn pick_pgn_file() -> Option<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Import PGN Collection")
        .add_filter("Portable Game Notation", &["pgn"])
        .pick_file()
        .await?;
    Some(file.path().to_string_lossy().into_owned())
}

fn strip_move_annotations(notation: &str) -> &str {
    notation.trim().trim_end_matches(['+', '#', '!', '?'])
}

fn normalize_logged_uci(notation: &str) -> String {
    // UCI tokens are lowercase; keep this separate from SAN PGN export.
    strip_move_annotations(notation).to_ascii_lowercase()
}

fn find_logged_move(board: &mut types::Board, notation: &str) -> Option<types::Move> {
    let uci = normalize_logged_uci(notation);
    board
        .generate_legal_moves()
        .iter()
        .find(|mv| mv.to_uci() == uci)
        .copied()
}

/// Destination-square badge for the ply under review (chess.com Game Review style).
fn review_annotation_badge(
    initial_fen: &str,
    moves: &[String],
    review_ply: Option<usize>,
    annotations: &[Option<MoveAnnotation>],
) -> Option<(types::Square, MoveAnnotation)> {
    let ply = review_ply.filter(|ply| *ply > 0)?;
    let annotation = annotations.get(ply - 1).copied().flatten()?;
    if !annotation.shows_board_badge() {
        return None;
    }
    let mut board = types::Board::from_fen(initial_fen).ok()?;
    for notation in moves.iter().take(ply - 1) {
        let mv = find_logged_move(&mut board, notation)?;
        board.make_move(mv);
    }
    let played = find_logged_move(&mut board, moves.get(ply - 1)?)?;
    Some((played.to, annotation))
}

fn replay_study_game(initial_fen: &str, moves: &[String]) -> Result<game::GameState, String> {
    types::init();
    let mut state = game::GameState::new(types::Board::from_fen(initial_fen)?);
    for (ply, notation) in moves.iter().enumerate() {
        let mv = find_logged_move(&mut state.board, notation)
            .ok_or_else(|| format!("illegal move {notation} at ply {}", ply + 1))?;
        state.last_move_squares = vec![mv.from, mv.to];
        state.board.make_move(mv);
    }
    state.game_over = state.board.is_game_over();
    Ok(state)
}

fn display_metadata<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn game_summary_label(summary: &GameSummary) -> (String, String) {
    let metadata = &summary.metadata;
    let white = display_metadata(&metadata.white, "White");
    let black = display_metadata(&metadata.black, "Black");
    let ratings = match (metadata.white_elo, metadata.black_elo) {
        (Some(white), Some(black)) => format!("{white}–{black}"),
        _ => "unrated".to_owned(),
    };
    let event = display_metadata(&metadata.event, "Casual game");
    let eco = display_metadata(&metadata.eco, "—");
    (
        format!("{white} vs {black}  {}", metadata.result),
        format!("{event} · {eco} · {ratings}"),
    )
}

fn study_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Mujrim")
        .join("library")
}

fn training_database_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Mujrim")
        .join("training")
}

async fn index_openings(path: PathBuf) -> (OpeningExplorer, usize) {
    std::thread::Builder::new()
        .name("mujrim-opening-index".to_owned())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            let Ok(database) = StudyDatabase::open(path) else {
                return (OpeningExplorer::default(), 0);
            };
            let summaries = database.search(&GameQuery::default());
            let mut explorer = OpeningExplorer::default();
            let mut indexed = 0;
            for summary in summaries.iter().take(5_000) {
                let Ok(game) = database.load_game(&summary.id) else {
                    continue;
                };
                let plies = game.moves.len().min(24);
                if explorer
                    .record_game(&game.initial_fen, &game.moves[..plies], &game.result)
                    .is_ok()
                {
                    indexed += 1;
                }
            }
            (explorer, indexed)
        })
        .ok()
        .and_then(|worker| worker.join().ok())
        .unwrap_or_default()
}

fn apply_opening_move(board: &mut types::Board, uci: &str) -> Result<types::Move, String> {
    let mv = board
        .generate_legal_moves()
        .iter()
        .find(|mv| mv.to_uci() == uci)
        .copied()
        .ok_or_else(|| format!("Opening move '{uci}' is no longer legal."))?;
    board.make_move(mv);
    Ok(mv)
}

fn today_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() / 86_400)
}

async fn probe_adjacent_engines() -> Vec<EngineMetadata> {
    std::thread::Builder::new()
        .name("mujrim-engine-discovery".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            list_local_engine_binaries()
                .into_iter()
                .filter_map(|path| probe_engine_protocol(&path))
                .collect()
        })
        .ok()
        .and_then(|worker| worker.join().ok())
        .unwrap_or_default()
}

fn preferred_engine_arch_folders() -> Vec<String> {
    let mut folders = Vec::with_capacity(6);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("avx2") {
        folders.push("windows-x86_64-avx2".to_owned());
    }
    folders.push(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        folders.push("windows-arm64".to_owned());
        // After native ARM picks, allow x64 engines via Prism.
        folders.push("windows-x86_64-avx2".to_owned());
        folders.push("windows-x86_64".to_owned());
    }
    folders
}

/// Executables under `<ui>/engines/`, one binary per stem, native arch preferred.
fn list_local_engine_binaries() -> Vec<PathBuf> {
    let Some(root) = std::env::current_exe()
        .ok()
        .as_ref()
        .and_then(|exe| mujrim_protocols::catalog::local_engines_root(exe))
    else {
        return Vec::new();
    };
    if !root.is_dir() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    collect_engine_executables(&root, 0, &mut candidates);
    let preferred = preferred_engine_arch_folders();
    candidates.sort_by(|left, right| {
        engine_path_rank(left, &preferred)
            .cmp(&engine_path_rank(right, &preferred))
            .then_with(|| {
                // Prefer host-native PE when ranks tie.
                let left_native = mujrim_protocols::is_host_native_binary(left);
                let right_native = mujrim_protocols::is_host_native_binary(right);
                right_native.cmp(&left_native)
            })
            .then_with(|| left.cmp(right))
    });
    let mut seen_stems = std::collections::HashSet::new();
    candidates.retain(|path| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        seen_stems.insert(stem)
    });
    candidates.truncate(64);
    candidates
}

fn engine_path_rank(path: &std::path::Path, preferred: &[String]) -> usize {
    path.components()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(|component| {
            preferred
                .iter()
                .position(|folder| folder.eq_ignore_ascii_case(component))
        })
        .unwrap_or(usize::MAX)
}

fn collect_engine_executables(root: &std::path::Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth > 5 || output.len() >= 128 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_engine_executables(&path, depth + 1, output);
        } else if is_engine_executable(&path) {
            // Include foreign-ISA binaries under the local engines tree (e.g. x64 on ARM).
            output.push(path);
        }
        if output.len() >= 128 {
            return;
        }
    }
}

fn is_engine_executable(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
}

fn probe_engine_protocol(path: &std::path::Path) -> Option<EngineMetadata> {
    use mujrim_protocols::{EngineSession, ProtocolKind};

    const PROBE_MEMORY: u64 = 256 * 1024 * 1024;
    let protocol = if EngineSession::spawn_with_args_and_memory_limit(
        path,
        &[],
        ProtocolKind::Uci,
        Some(PROBE_MEMORY),
    )
    .is_ok()
    {
        "UCI"
    } else if EngineSession::spawn_with_args_and_memory_limit(
        path,
        &[],
        ProtocolKind::Xboard,
        Some(PROBE_MEMORY),
    )
    .is_ok()
    {
        "XBoard"
    } else {
        return None;
    };

    let name = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Chess engine".to_owned());
    let architecture = path
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str())
        .find(|component| {
            component.contains("arm64")
                || component.contains("aarch64")
                || component.contains("x86_64")
        })
        .unwrap_or(std::env::consts::ARCH)
        .to_owned();
    Some(EngineMetadata {
        path: path.to_string_lossy().into_owned(),
        name,
        protocol: protocol.to_owned(),
        architecture,
        author: String::new(),
    })
}

fn starter_puzzles() -> Vec<Puzzle> {
    vec![
        Puzzle {
            id: "starter-development".to_owned(),
            fen: mujrim_study::opening::START_FEN.to_owned(),
            solution: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            themes: vec!["opening fundamentals".to_owned()],
            rating: 600,
        },
        Puzzle {
            id: "starter-mate-white".to_owned(),
            fen: "r1bqkbnr/pppp1ppp/2n5/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 3".to_owned(),
            solution: vec!["h5f7".to_owned()],
            themes: vec!["mate in one".to_owned()],
            rating: 750,
        },
        Puzzle {
            id: "starter-mate-black".to_owned(),
            fen: "rnbqkbnr/pppp1ppp/4p3/8/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2".to_owned(),
            solution: vec!["d8h4".to_owned()],
            themes: vec!["mate in one".to_owned(), "king safety".to_owned()],
            rating: 800,
        },
    ]
}

fn seed_training(store: &mut TrainingStore) -> Result<usize, String> {
    let mut added = 0;
    for puzzle in starter_puzzles() {
        if store.add(puzzle)? {
            added += 1;
        }
    }
    Ok(added)
}

fn path_is_under_local_engines(path: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(root) = mujrim_protocols::catalog::local_engines_root(&exe) else {
        return false;
    };
    let root = root.canonicalize().unwrap_or(root);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.starts_with(&root)
}

fn catalog_display_name(stem: &str, bundled: &[DiscoveredEngine]) -> String {
    if let Some(engine) = bundled.iter().find(|engine| {
        engine.id.eq_ignore_ascii_case(stem)
            || engine
                .path
                .file_stem()
                .is_some_and(|name| name.eq_ignore_ascii_case(stem))
    }) {
        return if engine.compatibility == RuntimeCompatibility::Emulated
            || !mujrim_protocols::is_host_native_binary(&engine.path)
        {
            bundled_engine_label(engine)
        } else {
            engine.display_name.to_owned()
        };
    }
    for &(id, display) in mujrim_protocols::catalog::BUNDLED_ENGINES {
        if id.eq_ignore_ascii_case(stem) {
            return display.to_owned();
        }
    }
    stem.to_owned()
}

fn tournament_engine_roster(
    bundled: &[DiscoveredEngine],
    discovered: &[EngineMetadata],
) -> Vec<QuickTournamentEngine> {
    let mut roster = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut seen_stems = std::collections::HashSet::new();

    // Source of truth: whatever is actually present under the UI-local engines/ tree.
    for path in list_local_engine_binaries() {
        if !path_is_under_local_engines(&path) || !seen_paths.insert(path.clone()) {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "engine".to_owned());
        if !seen_stems.insert(stem.clone()) {
            continue;
        }
        let mut name = catalog_display_name(&stem, bundled);
        if !mujrim_protocols::is_host_native_binary(&path)
            && !name.to_ascii_lowercase().contains("emulation")
            && !name.to_ascii_lowercase().contains("x64")
        {
            name = format!("{name} (x64 emulation)");
        }
        let search_limits = bundled
            .iter()
            .find(|engine| engine.path == path || engine.id.eq_ignore_ascii_case(&stem))
            .map(|engine| engine.search_limits)
            .unwrap_or(mujrim_protocols::catalog::SearchLimitSupport::STANDARD);
        roster.push(QuickTournamentEngine {
            name,
            path,
            search_limits,
        });
    }

    // Keep any already-probed UCI engines that live under local engines/ and were missed.
    for engine in discovered {
        let path = PathBuf::from(&engine.path);
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| engine.name.clone());
        if engine.protocol.eq_ignore_ascii_case("UCI")
            && path.is_file()
            && path_is_under_local_engines(&path)
            && seen_paths.insert(path.clone())
            && seen_stems.insert(stem)
        {
            roster.push(QuickTournamentEngine {
                name: engine.name.clone(),
                path,
                search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
            });
        }
    }
    roster.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    roster
}

async fn run_quick_tournament(
    engines: Vec<QuickTournamentEngine>,
    setup: tournament_setup::TournamentSetup,
    handle: tournament_live::LiveTournamentHandle,
) -> mujrim_benchmarker::strength::TournamentSummary {
    let cancel = Arc::clone(&handle.cancel);
    let snapshot = Arc::clone(&handle.snapshot);
    let format = setup.format;
    let worker = std::thread::Builder::new()
        .name("mujrim-tournament".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_quick_tournament_body(engines, setup, cancel, snapshot)
            }))
            .unwrap_or_else(|_| mujrim_benchmarker::strength::TournamentSummary {
                format,
                engines: Vec::new(),
                matches: Vec::new(),
                standings: Vec::new(),
                game_results: Vec::new(),
                cancelled: false,
                error: Some(
                    "Tournament worker panicked. The UI stayed up — check engine compatibility (Arm64/Prism) and try fewer engines."
                        .to_owned(),
                ),
            })
        });
    match worker {
        Ok(worker) => match worker.join() {
            Ok(summary) => summary,
            Err(_) => mujrim_benchmarker::strength::TournamentSummary {
                format,
                engines: Vec::new(),
                matches: Vec::new(),
                standings: Vec::new(),
                game_results: Vec::new(),
                cancelled: false,
                error: Some("Tournament worker failed unexpectedly.".to_owned()),
            },
        },
        Err(error) => mujrim_benchmarker::strength::TournamentSummary {
            format,
            engines: Vec::new(),
            matches: Vec::new(),
            standings: Vec::new(),
            game_results: Vec::new(),
            cancelled: false,
            error: Some(format!("Could not start tournament worker: {error}")),
        },
    }
}

fn run_quick_tournament_body(
    engines: Vec<QuickTournamentEngine>,
    setup: tournament_setup::TournamentSetup,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    snapshot: Arc<std::sync::Mutex<tournament_live::LiveTournamentSnapshot>>,
) -> mujrim_benchmarker::strength::TournamentSummary {
    use mujrim_benchmarker::strength::{
        EngineSpec, TournamentConfig, TournamentEngine, TournamentEvent, TournamentProgress,
        run_tournament_with_control,
    };

    let format = setup.format;
    let roster: Vec<TournamentEngine> = engines
        .into_iter()
        .map(|engine| {
            let mut spec = EngineSpec::new(engine.path.clone());
            spec.name = engine.name;
            spec.uci_options = uci_process::uci_resource_options(&engine.path, false, true, None);
            TournamentEngine {
                engine: spec,
                established_elo: None,
                search_limits: engine.search_limits,
            }
        })
        .collect();
    let initial_clock_ms = setup.time_control.match_clock().initial.as_millis() as u64;
    let progress: TournamentProgress = Arc::new({
        let snapshot = Arc::clone(&snapshot);
        move |event: TournamentEvent| {
            let snapshot = Arc::clone(&snapshot);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let Ok(mut guard) = snapshot.lock() else {
                    return;
                };
                match event {
                    TournamentEvent::Planned {
                        total_matches,
                        engine_names,
                    } => {
                        guard.total_matches = total_matches;
                        guard.engine_names = engine_names;
                        guard.status_line =
                            format!("Scheduled {total_matches} pairings. Starting…");
                    }
                    TournamentEvent::MatchStarted {
                        index,
                        total,
                        round,
                        white,
                        black,
                    } => {
                        guard.total_matches = total.max(guard.total_matches);
                        guard.current_round = round;
                        guard.current_white = white.clone();
                        guard.current_black = black.clone();
                        guard.status_line =
                            format!("Playing {index}/{total} · Round {round} · {white} vs {black}");
                    }
                    TournamentEvent::GameStarted {
                        game_key,
                        match_index,
                        round,
                        white,
                        black,
                        initial_fen,
                    } => {
                        guard.upsert_live_game(tournament_live::LiveGameBoard {
                            game_key,
                            match_index,
                            round,
                            white: white.clone(),
                            black: black.clone(),
                            initial_fen,
                            moves: Vec::new(),
                            last_uci: String::new(),
                            score_cp: 0,
                            depth: 0,
                            nodes: 0,
                            white_clock_ms: Some(initial_clock_ms),
                            black_clock_ms: Some(initial_clock_ms),
                        });
                        guard.current_round = round;
                        guard.current_white = white;
                        guard.current_black = black;
                    }
                    TournamentEvent::PlyPlayed {
                        game_key,
                        ply,
                        uci,
                        score_cp,
                        depth,
                        nodes,
                        moves,
                        white_clock_ms,
                        black_clock_ms,
                    } => {
                        guard.apply_ply(
                            &game_key,
                            ply,
                            uci,
                            score_cp,
                            depth,
                            nodes,
                            moves,
                            white_clock_ms,
                            black_clock_ms,
                        );
                    }
                    TournamentEvent::GameFinished {
                        game_key,
                        white_score,
                        moves,
                    } => {
                        guard.finish_live_game(&game_key, white_score, moves);
                    }
                    TournamentEvent::MatchFinished {
                        index,
                        total,
                        round,
                        white,
                        black,
                        white_points,
                        black_points,
                        error,
                        standings,
                        game_results,
                        games,
                    } => {
                        guard.completed_matches = index;
                        guard.total_matches = total.max(guard.total_matches);
                        guard
                            .finished_matches
                            .push(tournament_live::FinishedMatchRow {
                                index,
                                round,
                                white: white.clone(),
                                black: black.clone(),
                                white_points,
                                black_points,
                                error: error.clone(),
                            });
                        guard.standings =
                            tournament_live::standing_rows(&guard.engine_names, &standings);
                        guard.game_results = game_results;
                        let already_live = guard
                            .played_games
                            .iter()
                            .any(|game| game.match_index == index);
                        if !already_live {
                            guard.append_games(games);
                        }
                        guard.current_white.clear();
                        guard.current_black.clear();
                        guard.status_line = if let Some(error) = error {
                            format!("Match {index}/{total}: {error}")
                        } else {
                            format!(
                                "Finished {index}/{total} · {white} {} {}",
                                tournament_live::score_label(white_points, black_points),
                                black
                            )
                        };
                    }
                    TournamentEvent::Cancelled {
                        standings,
                        game_results,
                    } => {
                        guard.cancelled = true;
                        guard.running = false;
                        guard.standings =
                            tournament_live::standing_rows(&guard.engine_names, &standings);
                        guard.game_results = game_results;
                        guard.status_line =
                            "Tournament cancelled. Partial standings are available.".to_owned();
                    }
                }
            }));
        }
    });
    let mut match_config = setup.to_match_config();
    match_config.stop_flag = Some(Arc::clone(&cancel));
    let summary = run_tournament_with_control(
        roster,
        TournamentConfig {
            match_config,
            format,
            swiss_rounds: matches!(format, TournamentFormat::Swiss)
                .then_some(setup.swiss_rounds.max(1) as usize),
            checkpoint_directory: study_database_path().parent().map(|path| {
                path.join("tournaments")
                    .join(tournament_directory_name(format))
            }),
        },
        cancel,
        Some(progress),
    );
    if let Ok(mut guard) = snapshot.lock() {
        guard.running = false;
        guard.finished = true;
        guard.cancelled = summary.cancelled;
        guard.error = summary.error.clone();
        let names = summary
            .engines
            .iter()
            .map(|engine| engine.engine.name.clone())
            .collect::<Vec<_>>();
        guard.engine_names = names.clone();
        guard.standings = tournament_live::standing_rows(&names, &summary.standings);
        guard.game_results = summary.game_results.clone();
        let games = mujrim_benchmarker::strength::games_from_summary(&summary);
        if guard.played_games.len() < games.len() {
            guard.played_games.clear();
            guard.append_games(games);
        }
    }
    summary
}

fn tournament_directory_name(format: TournamentFormat) -> &'static str {
    match format {
        TournamentFormat::RoundRobin => "round-robin",
        TournamentFormat::DoubleRoundRobin => "double-round-robin",
        TournamentFormat::Swiss => "swiss",
        TournamentFormat::Knockout => "knockout",
    }
}

fn format_tournament_summary(summary: &mujrim_benchmarker::strength::TournamentSummary) -> String {
    let podium = summary
        .standings
        .iter()
        .take(3)
        .enumerate()
        .filter_map(|(rank, standing)| {
            let name = summary
                .engines
                .get(standing.entrant)
                .map(|engine| engine.engine.name.as_str())?;
            let rating = standing.performance.map_or_else(
                || "rating pending".to_owned(),
                |estimate| format!("{:.0} Elo", estimate.elo),
            );
            Some(format!(
                "{}. {name} — {:.1} points, {rating}",
                rank + 1,
                standing.points
            ))
        })
        .collect::<Vec<_>>()
        .join("  ·  ");
    if podium.is_empty() {
        format!("{} finished without completed games.", summary.format)
    } else {
        format!("{} · {podium}", summary.format)
    }
}

/// Probe each selected engine before a tournament so Arm64/Prism spawn failures
/// never take down the UI mid-event.
fn preflight_tournament_engines(
    engines: Vec<QuickTournamentEngine>,
) -> Result<Vec<QuickTournamentEngine>, String> {
    use mujrim_protocols::{EngineSession, ProtocolKind};

    const PREFLIGHT_MEMORY: u64 = 256 * 1024 * 1024;
    let mut healthy = Vec::with_capacity(engines.len());
    let mut failures = Vec::new();
    for engine in engines {
        match EngineSession::spawn_with_args_and_memory_limit(
            &engine.path,
            &[],
            ProtocolKind::Uci,
            Some(PREFLIGHT_MEMORY),
        ) {
            Ok(_session) => healthy.push(engine),
            Err(error) => failures.push(format!("{} ({})", engine.name, error)),
        }
    }
    if healthy.len() < 2 {
        let detail = if failures.is_empty() {
            "need at least two engines that can start".to_owned()
        } else {
            format!(
                "only {} ready; failed: {}",
                healthy.len(),
                failures.join("; ")
            )
        };
        return Err(format!("Tournament preflight failed — {detail}"));
    }
    if !failures.is_empty() {
        // Keep going with healthy engines; surface skips in the status via Err only when <2.
        let _ = failures;
    }
    Ok(healthy)
}

async fn analyze_game(initial_fen: String, moves: Vec<String>) -> Result<Vec<AnalyzedPly>, String> {
    std::thread::Builder::new()
        .name("mujrim-game-review".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || analyze_game_at_depth_from(&initial_fen, &moves, 10))
        .map_err(|error| format!("could not start review worker: {error}"))?
        .join()
        .map_err(|_| "review worker failed unexpectedly".to_owned())?
}

#[cfg(test)]
fn analyze_game_at_depth(moves: &[String], depth: i32) -> Result<Vec<AnalyzedPly>, String> {
    analyze_game_at_depth_from(mujrim_study::opening::START_FEN, moves, depth)
}

fn analyze_game_at_depth_from(
    initial_fen: &str,
    moves: &[String],
    depth: i32,
) -> Result<Vec<AnalyzedPly>, String> {
    types::init();
    let mut board = types::Board::from_fen(initial_fen)
        .map_err(|error| format!("invalid initial position: {error}"))?;
    let mut engine = search::SearchEngine::new(32, 1);
    let mut analysis = Vec::with_capacity(moves.len());
    for (ply, notation) in moves.iter().enumerate() {
        let legal_moves = board.generate_legal_moves();
        let played = find_logged_move(&mut board, notation).ok_or_else(|| {
            format!(
                "illegal move '{}' at ply {}",
                normalize_logged_uci(notation),
                ply + 1
            )
        })?;
        let moving_value = board
            .piece_on(played.from)
            .map_or(0, |(piece, _)| piece_value(piece));
        let captured_value = board
            .piece_on(played.to)
            .map_or(0, |(piece, _)| piece_value(piece));

        let mut before = board.clone();
        let best = engine.search_depth(&mut before, depth.max(1));
        let is_best_move = best.best_move.from == played.from
            && best.best_move.to == played.to
            && best.best_move.promotion == played.promotion;

        let mut after = board.clone();
        after.make_move(played);
        let can_be_recaptured = after
            .generate_legal_moves()
            .iter()
            .any(|reply| reply.to == played.to && reply.is_capture());
        let reply = engine.search_depth(&mut after, depth.max(1));
        let played_score = reply.score.saturating_neg();
        let annotation = AnnotationContext {
            best_score_cp: best.score,
            played_score_cp: played_score,
            second_best_score_cp: None,
            is_sacrifice: moving_value >= 300
                && captured_value.saturating_add(100) < moving_value
                && can_be_recaptured,
            is_best_move,
            is_only_move: legal_moves.len() == 1,
            position_in_opening_database: false,
            move_in_opening_database: false,
        }
        .classify();
        analysis.push(AnalyzedPly {
            annotation,
            score_cp: if ply % 2 == 0 {
                played_score
            } else {
                played_score.saturating_neg()
            },
        });
        board.make_move(played);
    }
    Ok(analysis)
}

fn board_at_ply(initial_fen: &str, moves: &[String], ply: usize) -> Result<types::Board, String> {
    if ply > moves.len() {
        return Err(format!("ply {ply} is beyond the {}-ply game", moves.len()));
    }
    let state = replay_study_game(initial_fen, &moves[..ply])?;
    Ok(state.board)
}

const fn piece_value(piece: types::Piece) -> i32 {
    match piece {
        types::Piece::Pawn => 100,
        types::Piece::Knight => 320,
        types::Piece::Bishop => 330,
        types::Piece::Rook => 500,
        types::Piece::Queen => 900,
        types::Piece::King => 20_000,
    }
}

fn annotated_move_label(notation: &str, annotation: Option<MoveAnnotation>) -> String {
    annotation.map_or_else(
        || notation.to_owned(),
        |annotation| {
            let symbol = annotation.symbol();
            if symbol.is_empty() {
                notation.to_owned()
            } else {
                format!("{notation} {symbol}")
            }
        },
    )
}

fn tournament_history_button<'a>(
    name: String,
    status: String,
    id: String,
    selected: bool,
    pal: board_view::GuiPalette,
) -> Element<'a, Msg> {
    button(
        column![
            text(name).size(13).color(pal.text_primary),
            text(status).size(11).color(pal.text_secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .on_press(Msg::SelectTournament(id))
    .padding(8)
    .width(Length::Fill)
    .style(move |_theme, button_status| {
        let background = if selected {
            Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.28)
        } else if matches!(button_status, button::Status::Hovered) {
            Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.14)
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(iced::Background::Color(background)),
            border: iced::Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            text_color: pal.text_primary,
            ..Default::default()
        }
    })
    .into()
}

fn tournament_game_button<'a>(
    title: String,
    detail: String,
    id: usize,
    selected: bool,
    pal: board_view::GuiPalette,
) -> Element<'a, Msg> {
    button(
        column![
            text(title).size(12).color(pal.text_primary),
            text(detail).size(11).color(pal.text_secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .on_press(Msg::SelectTournamentGame(id))
    .padding(7)
    .width(Length::Fill)
    .style(move |_theme, button_status| {
        let background = if selected {
            Color::from_rgba(pal.accent_alt.r, pal.accent_alt.g, pal.accent_alt.b, 0.30)
        } else if matches!(button_status, button::Status::Hovered) {
            Color::from_rgba(pal.accent_alt.r, pal.accent_alt.g, pal.accent_alt.b, 0.14)
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(iced::Background::Color(background)),
            border: iced::Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            text_color: pal.text_primary,
            ..Default::default()
        }
    })
    .into()
}

fn move_history_button<'a>(
    label: String,
    ply: usize,
    selected: bool,
    pal: board_view::GuiPalette,
) -> Element<'a, Msg> {
    button(container(text(label).size(13)).width(88))
        .on_press(Msg::ViewPly(ply))
        .padding([3, 4])
        .style(move |_theme, status| {
            let background = if selected {
                Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.30)
            } else if matches!(status, button::Status::Hovered) {
                Color::from_rgba(pal.accent.r, pal.accent.g, pal.accent.b, 0.16)
            } else {
                Color::TRANSPARENT
            };
            button::Style {
                background: Some(iced::Background::Color(background)),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                text_color: pal.text_primary,
                ..Default::default()
            }
        })
        .into()
}

fn puzzle_line_matches(played: &[String], solution: &[String]) -> bool {
    played.len() == solution.len()
        && played
            .iter()
            .zip(solution)
            .all(|(played, expected)| played.trim_end_matches(['+', '#']) == expected)
}

fn build_pgn(white: &str, black: &str, moves: &[String], result: &str) -> String {
    let mut pgn = format!(
        "[Event \"Mujrim Game\"]\n[Site \"Local\"]\n[Date \"????.??.??\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n\n"
    );
    for (index, pair) in moves.chunks(2).enumerate() {
        pgn.push_str(&format!(
            "{}. {}",
            index + 1,
            strip_move_annotations(&pair[0])
        ));
        if let Some(black_move) = pair.get(1) {
            pgn.push(' ');
            pgn.push_str(strip_move_annotations(black_move));
        }
        pgn.push(' ');
    }
    pgn.push_str(result);
    pgn
}

fn build_annotated_pgn(
    white: &str,
    black: &str,
    moves: &[String],
    annotations: &[Option<MoveAnnotation>],
    scores_cp: &[Option<i32>],
    result: &str,
) -> String {
    if !annotations.iter().any(Option::is_some) && !scores_cp.iter().any(Option::is_some) {
        return build_pgn(white, black, moves, result);
    }

    let mut pgn = format!(
        "[Event \"Mujrim Game\"]\n[Site \"Local\"]\n[Date \"????.??.??\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n[Annotator \"Mujrim Game Review\"]\n\n"
    );
    for (ply, notation) in moves.iter().enumerate() {
        if ply % 2 == 0 {
            pgn.push_str(&format!("{}. ", ply / 2 + 1));
        }
        pgn.push_str(&normalize_logged_uci(notation));

        let annotation = annotations.get(ply).copied().flatten();
        let score = scores_cp.get(ply).copied().flatten();
        if annotation.is_some() || score.is_some() {
            pgn.push_str(" {");
            if let Some(score) = score {
                pgn.push_str(&format!("[%eval {:.2}]", score as f64 / 100.0));
            }
            if let Some(annotation) = annotation {
                if score.is_some() {
                    pgn.push(' ');
                }
                pgn.push_str(annotation.label());
                let symbol = annotation.symbol();
                if !symbol.is_empty() {
                    pgn.push(' ');
                    pgn.push_str(symbol);
                }
            }
            pgn.push('}');
        }
        pgn.push(' ');
    }
    pgn.push_str(result);
    pgn
}

async fn pick_nnue_file() -> Option<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Select NNUE Network")
        .add_filter("NNUE", &["bin", "nnue"])
        .pick_file()
        .await?;
    Some(file.path().to_string_lossy().to_string())
}

/// Persistent built-in search so consecutive GUI moves reuse TT / history,
/// matching how CuteChess keeps an engine process alive across moves.
fn builtin_analysis_line(fen: &str, depth: i32) -> Result<(String, i32, Vec<String>), String> {
    types::init();
    let mut board = types::Board::from_fen(fen)?;
    let (mv, _info) = builtin_engine_search(
        &mut board,
        64,
        1,
        true,
        None,
        Duration::from_millis(250),
        depth.max(1),
    )?;
    // Reconstruct a short PV by searching once; expose best move as the PV head.
    Ok((mv.to_uci(), 0, vec![mv.to_uci()]))
}

fn builtin_engine_search(
    board: &mut types::Board,
    hash_mb: usize,
    threads: usize,
    use_nnue: bool,
    eval_file: Option<&str>,
    time: std::time::Duration,
    max_depth: i32,
) -> Result<(types::Move, String), String> {
    use std::sync::{Mutex, OnceLock};

    struct BuiltinCache {
        hash_mb: usize,
        threads: usize,
        use_nnue: bool,
        eval_file: Option<String>,
        engine: search::SearchEngine,
    }

    static CACHE: OnceLock<Mutex<Option<BuiltinCache>>> = OnceLock::new();
    let mut guard = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let needs_rebuild = guard.as_ref().is_none_or(|cached| {
        cached.hash_mb != hash_mb
            || cached.threads != threads
            || cached.use_nnue != use_nnue
            || cached.eval_file.as_deref() != eval_file
    });

    if needs_rebuild {
        let mut engine = search::SearchEngine::new(hash_mb, threads);
        engine.set_use_nnue(use_nnue);
        if let Some(path) = eval_file {
            let net = eval::nnue::load_network(std::path::Path::new(path))
                .map_err(|err| format!("EvalFile error: {err}"))?;
            engine.set_nnue_network(net);
        }
        *guard = Some(BuiltinCache {
            hash_mb,
            threads,
            use_nnue,
            eval_file: eval_file.map(str::to_owned),
            engine,
        });
    }

    let cached = guard.as_mut().expect("builtin cache just initialized");
    let result = cached.engine.search_time(board, time, max_depth);
    let note = eval_file
        .map(|_| format!(" | net {}", cached.engine.nnue_info().name))
        .unwrap_or_default();
    Ok((
        result.best_move,
        format!(
            "depth {} | score {} cp | {} nodes | {:.0} nps{}",
            result.depth,
            result.score,
            result.nodes,
            result.nodes as f64 / result.elapsed.as_secs_f64().max(0.001),
            note,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use mujrim_protocols::catalog::{DiscoveredEngine, RuntimeCompatibility};

    use super::{
        EngineConfig, ExternalEngineProtocol, PlayerConfig, analyze_game_at_depth,
        annotated_move_label, apply_opening_move, board_at_ply, bounded_hash_mb,
        build_annotated_pgn, build_pgn, bundled_engine_choices, engine_path_rank, find_logged_move,
        format_tournament_summary, game_summary_label, main_window_settings, normalize_logged_uci,
        puzzle_line_matches, replay_study_game, review_annotation_badge, selected_bundled_engine,
        starter_puzzles, tournament_directory_name,
    };
    use mujrim_study::annotation::MoveAnnotation;
    use mujrim_study::database::{GameMetadata, GameSummary};
    use mujrim_study::tournament::TournamentFormat;

    #[test]
    fn test_engine_config_defaults_to_embedded_eval() {
        let cfg = EngineConfig::default();
        assert!(cfg.eval_file.is_none());
        assert!(cfg.use_nnue);
    }

    #[test]
    fn main_window_uses_resizable_custom_chrome() {
        let settings = main_window_settings();
        assert!(!settings.decorations);
        assert!(settings.resizable);
        assert!(!settings.transparent);
    }

    #[test]
    fn engine_hash_is_bounded_for_low_memory_desktops() {
        assert_eq!(bounded_hash_mb(-1), 1);
        assert_eq!(bounded_hash_mb(64), 64);
        assert_eq!(bounded_hash_mb(4096), 512);
    }

    #[test]
    fn bundled_engine_choices_expose_execution_mode_and_selection() {
        let engines = vec![DiscoveredEngine {
            id: "obsidian",
            display_name: "Obsidian",
            path: PathBuf::from(r"C:\Mujrim\engines\obsidian.exe"),
            target_directory: "windows-x86_64-avx2".to_owned(),
            compatibility: RuntimeCompatibility::Emulated,
            search_limits: mujrim_protocols::catalog::SearchLimitSupport::STANDARD,
        }];
        let choices = bundled_engine_choices(&engines);
        assert_eq!(choices[0].label, "Obsidian (x64 emulation)");

        let selected = selected_bundled_engine(
            &engines,
            &PlayerConfig::External {
                path: engines[0].path.to_string_lossy().into_owned(),
                protocol: ExternalEngineProtocol::Uci,
            },
        )
        .expect("bundled engine should be selected");
        assert_eq!(selected, choices[0]);
    }

    #[test]
    fn engine_path_rank_prefers_primary_host_arch_folder() {
        let preferred = vec!["windows-aarch64".to_owned(), "windows-arm64".to_owned()];
        let primary =
            PathBuf::from(r"C:\Mujrim\engines\mujrim\bin\windows-aarch64\mujrim-elite.exe");
        let alias = PathBuf::from(r"C:\Mujrim\engines\mujrim\bin\windows-arm64\mujrim-elite.exe");
        assert!(engine_path_rank(&primary, &preferred) < engine_path_rank(&alias, &preferred));
    }

    #[test]
    fn all_tournament_formats_have_stable_checkpoint_directories() {
        let names = TournamentFormat::ALL.map(tournament_directory_name);
        assert_eq!(
            names,
            ["round-robin", "double-round-robin", "swiss", "knockout"]
        );
    }

    #[test]
    fn empty_tournament_summary_reports_the_selected_format() {
        let summary = mujrim_benchmarker::strength::TournamentSummary {
            format: TournamentFormat::Swiss,
            engines: Vec::new(),
            matches: Vec::new(),
            standings: Vec::new(),
            game_results: Vec::new(),
            cancelled: false,
            error: None,
        };
        assert_eq!(
            format_tournament_summary(&summary),
            "Swiss finished without completed games."
        );
    }

    #[test]
    fn tournament_summary_ignores_out_of_range_standings() {
        let summary = mujrim_benchmarker::strength::TournamentSummary {
            format: TournamentFormat::RoundRobin,
            engines: Vec::new(),
            matches: Vec::new(),
            standings: vec![mujrim_study::tournament::Standing {
                entrant: 99,
                played: 0,
                wins: 0,
                draws: 0,
                losses: 0,
                points: 0.0,
                performance: None,
            }],
            game_results: Vec::new(),
            cancelled: false,
            error: None,
        };
        assert_eq!(
            format_tournament_summary(&summary),
            "Round robin finished without completed games."
        );
    }

    #[test]
    fn review_badge_uses_destination_square_and_classification() {
        types::init();
        let badge = review_annotation_badge(
            mujrim_study::opening::START_FEN,
            &["e2e4".to_owned()],
            Some(1),
            &[Some(MoveAnnotation::Brilliant)],
        )
        .expect("badge");
        assert_eq!(badge.0, types::Square::from_index(28));
        assert_eq!(badge.1, MoveAnnotation::Brilliant);
        assert!(
            review_annotation_badge(
                mujrim_study::opening::START_FEN,
                &["e2e4".to_owned()],
                Some(1),
                &[Some(MoveAnnotation::Ok)],
            )
            .is_none()
        );
    }

    #[test]
    fn pgn_builder_numbers_moves_and_preserves_result() {
        let pgn = build_pgn(
            "White",
            "Black",
            &["e4".to_owned(), "e5".to_owned(), "Nf3".to_owned()],
            "1-0",
        );
        assert!(pgn.contains("[White \"White\"]"));
        assert!(pgn.ends_with("1. e4 e5 2. Nf3 1-0"));
    }

    #[test]
    fn analyzed_pgn_contains_evaluations_and_coaching_labels() {
        let pgn = build_annotated_pgn(
            "White",
            "Black",
            &["e2e4".to_owned(), "e7e5".to_owned()],
            &[Some(MoveAnnotation::Brilliant), Some(MoveAnnotation::Good)],
            &[Some(34), Some(-12)],
            "*",
        );
        assert!(pgn.contains("[Annotator \"Mujrim Game Review\"]"));
        assert!(pgn.contains("e2e4 {[%eval 0.34] Brilliant !!}"));
        assert!(pgn.contains("e7e5 {[%eval -0.12] Good ✓}"));
    }

    #[test]
    fn history_navigation_rebuilds_without_mutating_live_state() {
        let moves = ["e2e4".to_owned(), "e7e5".to_owned()];
        let after_white = board_at_ply(mujrim_study::opening::START_FEN, &moves, 1).unwrap();
        assert_eq!(after_white.side_to_move, types::Color::Black);
        assert!(board_at_ply(mujrim_study::opening::START_FEN, &moves, 3).is_err());
    }

    #[test]
    fn move_review_replays_uci_moves_and_formats_annotations() {
        let annotations = analyze_game_at_depth(&["e2e4".to_owned()], 1).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(
            annotated_move_label("e2e4", Some(MoveAnnotation::Brilliant)),
            "e2e4 !!"
        );
    }

    #[test]
    fn library_game_replay_validates_every_move() {
        let state = replay_study_game(
            mujrim_study::opening::START_FEN,
            &["e2e4".to_owned(), "e7e5".to_owned(), "g1f3".to_owned()],
        )
        .unwrap();
        assert_eq!(state.board.side_to_move, types::Color::Black);
        assert_eq!(state.last_move_squares[1].to_string(), "f3");
        assert!(replay_study_game(mujrim_study::opening::START_FEN, &["e2e5".to_owned()]).is_err());
    }

    #[test]
    fn replay_and_review_accept_check_mate_uci_suffixes() {
        let moves = ["e2e4+".to_owned(), "e7e5#".to_owned(), "g1f3!".to_owned()];
        let state = replay_study_game(mujrim_study::opening::START_FEN, &moves).unwrap();
        assert_eq!(state.board.side_to_move, types::Color::Black);
        let board = board_at_ply(mujrim_study::opening::START_FEN, &moves, 2).unwrap();
        assert_eq!(board.side_to_move, types::Color::White);
        assert_eq!(normalize_logged_uci("e8e4+"), "e8e4");
        assert_eq!(
            find_logged_move(&mut types::Board::new(), "e2e4#")
                .unwrap()
                .to_uci(),
            "e2e4"
        );
    }

    #[test]
    fn build_pgn_strips_check_suffixes_for_portable_export() {
        let pgn = build_pgn(
            "White",
            "Black",
            &["e2e4+".to_owned(), "e7e5#".to_owned()],
            "1-0",
        );
        assert!(pgn.contains("1. e2e4 e7e5"));
        assert!(!pgn.contains("e2e4+"));
        assert!(!pgn.contains("e7e5#"));
    }

    #[test]
    fn library_summary_has_human_readable_fallbacks() {
        let summary = GameSummary {
            id: "game".to_owned(),
            metadata: GameMetadata {
                black: "Kasparov".to_owned(),
                result: "1/2-1/2".to_owned(),
                ..GameMetadata::default()
            },
        };
        let (title, detail) = game_summary_label(&summary);
        assert_eq!(title, "White vs Kasparov  1/2-1/2");
        assert!(detail.contains("Casual game · — · unrated"));
    }

    #[test]
    fn starter_training_is_legal_and_solution_matching_ignores_check_suffixes() {
        let puzzles = starter_puzzles();
        assert_eq!(puzzles.len(), 3);
        assert!(puzzles.iter().all(|puzzle| puzzle.validate().is_ok()));
        assert!(puzzle_line_matches(
            &["h5f7#".to_owned()],
            &["h5f7".to_owned()]
        ));
        assert!(!puzzle_line_matches(
            &["h5e5".to_owned()],
            &["h5f7".to_owned()]
        ));
    }

    #[test]
    fn opening_navigation_accepts_only_legal_uci_moves() {
        types::init();
        let mut board = types::Board::new();
        assert_eq!(
            apply_opening_move(&mut board, "e2e4").unwrap().to_uci(),
            "e2e4"
        );
        assert_eq!(board.side_to_move, types::Color::Black);
        assert!(apply_opening_move(&mut board, "e2e4").is_err());
    }
}
