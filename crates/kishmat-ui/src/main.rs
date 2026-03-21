#![allow(unexpected_cfgs)]
//! KishMat GUI — premium chess interface.

mod arrows;
mod audio;
mod board_view;
mod game;
mod gif_export;
mod noise;
mod pieces;
mod recording;
mod uci_process;

use std::path::PathBuf;

use iced::widget::{
    Image, Space, button, column, container, mouse_area, pick_list, row, scrollable, slider, text,
    toggler,
};
use iced::{Alignment, Color, Element, Font, Length, Subscription, Task, Theme};
use std::time::{Duration, Instant};

use pieces::PieceAssets;
use uci_process::ExternalEngineProtocol;

/// Custom display font embedded from assets.
#[allow(dead_code)]
const CURIOUS_FONT_BYTES: &[u8] = include_bytes!("../assets/CuriousTrack.ttf");
const CURIOUS_FONT: Font = Font::with_name("Curious Track");

// ──────────────────────────────────────────────────────────────
// Colors — fallback constants (themes override via GuiPalette)
#[allow(dead_code)]
// ──────────────────────────────────────────────────────────────
#[allow(dead_code)]
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
    // Embed the logo for the window icon
    let icon = iced::window::icon::from_file_data(include_bytes!("../assets/logo.png"), None).ok();

    let mut win_settings = iced::window::Settings {
        decorations: false,
        transparent: true,
        ..Default::default()
    };
    if let Some(icon) = icon {
        win_settings.icon = Some(icon);
    }

    iced::application(App::boot, App::update, App::view)
        .title("KishMat Chess")
        .subscription(App::subscription)
        .theme(theme_fn)
        .window_size((1280.0, 850.0))
        .transparent(true)
        .window(win_settings)
        .run()
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

        let png_data: &[u8] = include_bytes!("../assets/logo.png");

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
struct AppSettings {
    board_theme: board_view::BoardTheme,
    show_coords: bool,
    /// Animation speed multiplier: 0=fast 1=normal 2=slow.
    anim_speed: i32,
    sfx_on: bool,
    bgm_volume: i32, // 0–100
    game_mood: audio::GameMood,
    auto_flip_black: bool,
    show_legal_moves: bool,
    show_last_move: bool,
    premoves_enabled: bool,
    capture_anim_style: CaptureAnimStyle,
    coord_position: CoordPosition,
    multi_premoves: bool,
    draw_arrows: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            board_theme: board_view::BoardTheme::Classic,
            show_coords: true,
            anim_speed: 1,
            sfx_on: true,
            bgm_volume: 50,
            game_mood: audio::GameMood::Mystique,
            auto_flip_black: false,
            show_legal_moves: true,
            show_last_move: true,
            premoves_enabled: true,
            capture_anim_style: CaptureAnimStyle::Explosion,
            coord_position: CoordPosition::Inside,
            multi_premoves: true,
            draw_arrows: true,
        }
    }
}

impl AppSettings {
    fn config_path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("kishmat");
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
    selected_mode: GameMode,
    white_player: PlayerConfig,
    black_player: PlayerConfig,
    engine_cfg: EngineConfig,
    settings: AppSettings,
    show_options: bool,
    options_tab: OptionsTab,
    move_log: Vec<String>,
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
    window_height: f32,
    bgm_on: bool,
    coin_flip: CoinFlipState,
    recorder: recording::RecordingEngine,
    window_id: Option<iced::window::Id>,
    // Syzygy state
    syzygy_status: String,
    syzygy_wdl_count: usize,
    syzygy_dtz_count: usize,
    // NNUE network state
    nnue_status: String,
    nnue_installed_count: usize,
    // Tuning state
    tuning_params: Option<updater::tuning::TunableParams>,
    tuning_status: String,
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

impl std::fmt::Display for PlayerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "Human"),
            Self::BuiltIn { depth } => write!(f, "KishMat (depth {depth})"),
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

#[derive(Debug, Clone)]
enum Msg {
    SelectMode(GameMode),
    StartGame,
    LoadWhiteUciEngine,
    LoadWhiteXboardEngine,
    LoadBlackUciEngine,
    LoadBlackXboardEngine,
    WhiteEngineSelected(Option<String>, ExternalEngineProtocol),
    BlackEngineSelected(Option<String>, ExternalEngineProtocol),
    BoardClick(usize, usize),
    EngineMove(types::Move, String),
    NewGame,
    FlipBoard,
    Resign,
    ExportPGN,
    ExportGIF,
    ExitApp,
    #[allow(dead_code)]
    EngineInfo(String),
    AnimTick(Instant),
    ToggleBGM,
    CoinFlip,
    CoinFlipTick(Instant),
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
    SetBoardTheme(board_view::BoardTheme),
    SetShowCoords(bool),
    SetAnimSpeed(i32),
    SetSfx(bool),
    SetBgmVolume(i32),
    SetGameMood(audio::GameMood),
    SetAutoFlip(bool),
    SetShowLegal(bool),
    SetShowLastMove(bool),
    SetPremoves(bool),
    SetCaptureAnim(CaptureAnimStyle),
    SetCoordPosition(CoordPosition),
    SetMultiPremoves(bool),
    SetDrawArrows(bool),
    BoardRightDown(usize, usize),
    BoardRightUp(usize, usize),
    // Tools panel
    SwitchOptionsTab(OptionsTab),
    SyzygyDownload,
    SyzygyDownloadDone(String),
    NnueDownload,
    NnueDownloadDone(String),
    TuneLoad,
    TuneSetParam(String, String, f64),
    TuneSave,
    CheckForUpdates,
    DragWindow,
    WindowOpened(iced::window::Id),
    #[allow(dead_code)]
    FontLoaded,
}

impl Default for App {
    fn default() -> Self {
        // Set macOS Dock icon (must happen after NSApplication exists)
        #[cfg(target_os = "macos")]
        set_macos_dock_icon();

        let mut sound = audio::SoundEngine::new();
        if let Some(ref mut s) = sound {
            s.play_bgm(audio::BgmTrack::Menu);
        }
        Self {
            screen: Screen::Menu,
            game: None,
            selected_mode: GameMode::HumanVsHuman,
            white_player: PlayerConfig::Human,
            black_player: PlayerConfig::Human,
            engine_cfg: EngineConfig::default(),
            settings: AppSettings::load(),
            show_options: false,
            options_tab: OptionsTab::Settings,
            move_log: Vec::new(),
            status: String::from("Welcome to KishMat!"),
            engine_info: String::new(),
            assets: PieceAssets::load(),
            _bg_pattern: noise::pharaonic_pattern(256),
            chess_bg: noise::chess_blur_background(512, 384),
            _panel_grain: noise::macos_grain_panel(),
            logo: iced::widget::image::Handle::from_bytes(
                include_bytes!("../assets/logo.png").as_slice(),
            ),
            #[cfg(feature = "book")]
            book: search::book::OpeningBook::load_embedded().ok(),
            sound,
            animation: None,
            window_height: 850.0,
            bgm_on: true,
            coin_flip: CoinFlipState::Idle,
            recorder: recording::RecordingEngine::new(),
            window_id: None,
            syzygy_status: String::new(),
            syzygy_wdl_count: 0,
            syzygy_dtz_count: 0,
            nnue_status: String::new(),
            nnue_installed_count: 0,
            tuning_params: None,
            tuning_status: String::new(),
        }
    }
}

impl App {
    /// Boot function for iced 0.14 — returns (State, Task).
    fn boot() -> (Self, Task<Msg>) {
        let load_lucide = iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(|_| Msg::FontLoaded);
        (Self::default(), load_lucide)
    }

    /// Subscription: animate at ~60fps while a move animation is in progress,
    /// also tick for coin flip animation and recording capture.
    fn subscription(&self) -> Subscription<Msg> {
        let mut subs: Vec<Subscription<Msg>> =
            vec![iced::window::open_events().map(Msg::WindowOpened)];

        if self.animation.is_some() {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Msg::AnimTick));
        }

        if matches!(self.coin_flip, CoinFlipState::Flipping { .. }) {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Msg::CoinFlipTick));
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

                // Capture animation duration depends on style
                let anim_duration = if is_capture {
                    match self.settings.capture_anim_style {
                        CaptureAnimStyle::Instant => Duration::from_millis(50),
                        CaptureAnimStyle::Explosion => Duration::from_millis(350),
                        CaptureAnimStyle::Fire => Duration::from_millis(400),
                    }
                } else {
                    Duration::from_millis(150)
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
            gs.board.make_move(anim.mv);
            // Chess notation: append check (+) or checkmate (#)
            let mut notation = anim.mv.to_uci();
            if gs.board.is_checkmate() {
                notation.push('#');
            } else if gs.board.in_check() {
                notation.push('+');
            }
            self.move_log.push(notation);

            if gs.board.is_game_over() {
                gs.game_over = true;
                gs.premove_queue.clear();
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
                let (from, to) = gs.premove_queue.remove(0);
                let legal = gs.board.generate_legal_moves();
                if let Some(mv) = legal.iter().find(|m| m.from == from && m.to == to).copied() {
                    gs.deselect();
                    // Determine if engine plays after this premove
                    let is_next_next_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.black_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.white_player, PlayerConfig::Human),
                    };
                    self.start_animation(mv, None, !is_next_next_human);
                    return Task::none();
                } else {
                    // Premove was illegal — clear queue
                    gs.premove_queue.clear();
                }
            }

            if anim.trigger_engine_after {
                if !is_next_human && !gs.game_over {
                    return self.trigger_engine_move();
                }
            }
        }
        Task::none()
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
                if let Some(p) = path {
                    self.white_player = PlayerConfig::External { path: p, protocol };
                }
                Task::none()
            }
            Msg::BlackEngineSelected(path, protocol) => {
                if let Some(p) = path {
                    self.black_player = PlayerConfig::External { path: p, protocol };
                }
                Task::none()
            }
            Msg::StartGame => {
                // Apply coin flip result if applicable
                if let CoinFlipState::Done(heads) = self.coin_flip {
                    if !heads && matches!(self.selected_mode, GameMode::HumanVsEngine) {
                        // Tails: human plays Black, engine plays White
                        let engine = self.black_player.clone();
                        self.black_player = PlayerConfig::Human;
                        self.white_player = engine;
                    }
                }

                types::init();
                let board = types::Board::new();
                self.game = Some(game::GameState::new(board));

                // Auto-flip board when playing Black
                if let CoinFlipState::Done(heads) = self.coin_flip {
                    if !heads {
                        if let Some(ref mut gs) = self.game {
                            gs.flipped = true;
                        }
                    }
                }

                self.move_log.clear();
                self.engine_info.clear();
                self.status = String::from("Game started — White to move");
                self.screen = Screen::Playing;
                self.coin_flip = CoinFlipState::Idle;

                // Switch BGM to game track
                if self.bgm_on {
                    if let Some(ref mut s) = self.sound {
                        s.play_bgm(audio::BgmTrack::Game);
                    }
                }

                if !matches!(self.white_player, PlayerConfig::Human) {
                    return self.trigger_engine_move();
                }
                Task::none()
            }
            Msg::BoardClick(row, col) => {
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
                        // Queue premove if enabled
                        if self.settings.premoves_enabled {
                            let sq_index = if gs.flipped {
                                row * 8 + col
                            } else {
                                (7 - row) * 8 + col
                            };
                            let clicked_sq = types::Square::from_index(sq_index);
                            if let Some(from) = gs.selected_square {
                                // Queue the premove
                                if !self.settings.multi_premoves {
                                    gs.premove_queue.clear();
                                }
                                gs.premove_queue.push((from, clicked_sq));
                                gs.deselect();
                            } else {
                                // Select for premove
                                gs.selected_square = Some(clicked_sq);
                                gs.legal_highlights.clear();
                            }
                        }
                        return Task::none();
                    }

                    let sq_index = if gs.flipped {
                        row * 8 + col
                    } else {
                        (7 - row) * 8 + col
                    };
                    let clicked_sq = types::Square::from_index(sq_index);

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
                            if let Some((_, color)) = gs.board.piece_on(clicked_sq) {
                                if color == gs.board.side_to_move {
                                    gs.select_square(clicked_sq);
                                    return Task::none();
                                }
                            }
                            gs.deselect();
                        }
                    } else {
                        // No piece selected — select if it's our piece
                        if let Some((_, color)) = gs.board.piece_on(clicked_sq) {
                            if color == gs.board.side_to_move {
                                gs.select_square(clicked_sq);
                            }
                        }
                    }
                }
                Task::none()
            }
            Msg::EngineMove(mv, info) => {
                // Start animation for engine move
                self.start_animation(mv, Some(info), true);
                Task::none()
            }
            Msg::EngineInfo(info) => {
                self.engine_info = info;
                Task::none()
            }
            Msg::NewGame => {
                self.screen = Screen::Menu;
                self.game = None;
                self.move_log.clear();
                self.engine_info.clear();
                self.status = String::from("Set up a new game.");

                // Switch BGM back to menu track
                if self.bgm_on {
                    if let Some(ref mut s) = self.sound {
                        s.play_bgm(audio::BgmTrack::Menu);
                    }
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
                if let Some(ref mut gs) = self.game {
                    gs.game_over = true;
                    let loser = if gs.board.side_to_move == types::Color::White {
                        "White"
                    } else {
                        "Black"
                    };
                    self.status = format!("{loser} resigns!");
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
                // Build a minimal PGN string from the move log
                let mut pgn = String::new();
                pgn.push_str("[Event \"KishMat Game\"]\n");
                pgn.push_str("[Site \"Local\"]\n");
                pgn.push_str(&format!("[Date \"{}\"]\n", "????.??.??"));
                pgn.push_str(&format!("[White \"{}\"]\n", self.white_player));
                pgn.push_str(&format!("[Black \"{}\"]\n", self.black_player));

                let result = if let Some(ref gs) = self.game {
                    if gs.game_over {
                        if gs.board.clone().is_checkmate() {
                            if gs.board.side_to_move == types::Color::White {
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
                pgn.push_str(&format!("[Result \"{result}\"]\n\n"));

                // Format moves as numbered pairs
                for (i, pair) in self.move_log.chunks(2).enumerate() {
                    pgn.push_str(&format!("{}. {}", i + 1, pair[0]));
                    if let Some(b) = pair.get(1) {
                        pgn.push_str(&format!(" {b}"));
                    }
                    pgn.push(' ');
                }
                pgn.push_str(result);

                // Copy to clipboard via arboard (if available) or print
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(pgn);
                    self.status = String::from("PGN copied to clipboard!");
                } else {
                    self.status = String::from("Could not access clipboard.");
                }
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
                        // Generate GIF bytes
                        let gif_data = gif_export::export_game_gif(&moves, 100); // 1s per move

                        // Open save dialog
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Save Game as GIF")
                            .set_file_name("kishmat_game.gif")
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
            Msg::ExitApp => iced::exit(),
            Msg::DragWindow => {
                if let Some(id) = self.window_id {
                    iced::window::drag(id)
                } else {
                    Task::none()
                }
            }
            Msg::WindowOpened(id) => {
                self.window_id = Some(id);
                Task::none()
            }
            Msg::ToggleBGM => {
                let track = match self.screen {
                    Screen::Menu => audio::BgmTrack::Menu,
                    Screen::Playing => audio::BgmTrack::Game,
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
                if let CoinFlipState::Flipping { start, result } = self.coin_flip {
                    if start.elapsed() >= Duration::from_millis(1500) {
                        self.coin_flip = CoinFlipState::Done(result);
                        if matches!(self.selected_mode, GameMode::HumanVsEngine) {
                            self.status = if result {
                                String::from("Heads! You play White.")
                            } else {
                                String::from("Tails! You play Black.")
                            };
                        }
                    }
                }
                Task::none()
            }
            Msg::ToggleRecording => {
                match self.recorder.state() {
                    recording::RecordState::Idle => {
                        self.recorder.start();
                        self.status = String::from("🔴 Recording...");
                    }
                    recording::RecordState::Recording => {
                        let recorder = self.recorder.clone();
                        self.status = String::from("Saving recording...");

                        return Task::perform(
                            async move {
                                let file = rfd::AsyncFileDialog::new()
                                    .set_title("Save Recording")
                                    .set_file_name("kishmat_recording.mp4")
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
                                    // Cancel recording
                                    let _ = recorder
                                        .stop_and_save(std::path::PathBuf::from("/dev/null"));
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
                self.recorder.capture_frame();
                Task::none()
            }
            Msg::RecordingSaved(msg) => {
                self.status = msg;
                Task::none()
            }
            Msg::TakeScreenshot => {
                self.status = String::from("Taking screenshot...");
                return Task::perform(
                    async {
                        // Capture primary monitor
                        let monitors = xcap::Monitor::all().unwrap_or_default();
                        if let Some(monitor) = monitors.first() {
                            if let Ok(img) = monitor.capture_image() {
                                let file = rfd::AsyncFileDialog::new()
                                    .set_title("Save Screenshot")
                                    .set_file_name("kishmat_screenshot.png")
                                    .add_filter("PNG", &["png"])
                                    .save_file()
                                    .await;
                                if let Some(file) = file {
                                    let path = file.path().to_path_buf();
                                    match img.save(&path) {
                                        Ok(_) => {
                                            return format!(
                                                "Screenshot saved to {}",
                                                path.display()
                                            );
                                        }
                                        Err(e) => return format!("Save error: {e}"),
                                    }
                                }
                            }
                        }
                        String::from("Screenshot cancelled.")
                    },
                    Msg::ScreenshotDone,
                );
            }
            Msg::ScreenshotDone(msg) => {
                self.status = msg;
                Task::none()
            }
            // ── Options modal & settings ──
            Msg::ToggleOptions => {
                self.show_options = !self.show_options;
                Task::none()
            }
            Msg::SetBoardTheme(t) => {
                self.settings.board_theme = t;
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
            Msg::BoardRightDown(row, col) => {
                if self.settings.draw_arrows {
                    if let Some(ref mut gs) = self.game {
                        let sq_index = if gs.flipped {
                            row * 8 + col
                        } else {
                            (7 - row) * 8 + col
                        };
                        gs.arrow_start = Some(types::Square::from_index(sq_index));
                    }
                }
                Task::none()
            }
            Msg::BoardRightUp(row, col) => {
                if self.settings.draw_arrows {
                    if let Some(ref mut gs) = self.game {
                        let sq_index = if gs.flipped {
                            row * 8 + col
                        } else {
                            (7 - row) * 8 + col
                        };
                        let to_sq = types::Square::from_index(sq_index);
                        if let Some(from_sq) = gs.arrow_start.take() {
                            if from_sq != to_sq {
                                // Toggle arrow: remove if exists, add if not
                                let arrow = (from_sq, to_sq);
                                if let Some(idx) = gs.arrows.iter().position(|a| *a == arrow) {
                                    gs.arrows.remove(idx);
                                } else {
                                    gs.arrows.push(arrow);
                                }
                            } else {
                                // Right-click on same square: clear all arrows
                                gs.arrows.clear();
                            }
                        }
                    }
                }
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
            Msg::SyzygyDownload => {
                self.syzygy_status = "Downloading 3-4-5 piece tables...".to_string();
                Task::perform(
                    async {
                        // Task::perform runs in a separate async context
                        let dest = updater::syzygy::default_syzygy_path();
                        match updater::syzygy::download_tables(
                            &dest,
                            updater::syzygy::SyzygyPieceSet::Standard,
                            None,
                        ) {
                            Ok(s) => format!(
                                "✓ {} downloaded, {} skipped, {} failed",
                                s.downloaded, s.skipped, s.failed
                            ),
                            Err(e) => format!("✗ {e}"),
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
                            Ok(s) => format!("✓ {} downloaded, {} failed", s.downloaded, s.failed),
                            Err(e) => format!("✗ {e}"),
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
                        Ok(()) => self.tuning_status = "Saved ✓".to_string(),
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

        // ── Try opening book first (instant response) ─────────────
        #[cfg(feature = "book")]
        if self.engine_cfg.use_book {
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&gs.board) {
                    let legal = gs.board.clone().generate_legal_moves();
                    if legal
                        .iter()
                        .any(|m| m.from == book_move.from && m.to == book_move.to)
                    {
                        return Task::perform(
                            async move { (book_move, String::from("Book move")) },
                            |(mv, info)| Msg::EngineMove(mv, info),
                        );
                    }
                }
            }
        }

        let mut board_clone = gs.board.clone();
        let fallback_board_clone = gs.board.clone();
        let time_secs = self.engine_cfg.time_per_move as u64;
        let hash_mb = self.engine_cfg.hash_mb as usize;
        let threads = self.engine_cfg.threads as usize;
        let max_depth = self.engine_cfg.max_depth;
        let use_nnue = self.engine_cfg.use_nnue;
        let eval_file = self.engine_cfg.eval_file.clone();

        Task::perform(
            async move {
                let handle = std::thread::Builder::new()
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || {
                        types::init();
                        match side_player {
                            PlayerConfig::Human => {
                                let legal = board_clone.generate_legal_moves();
                                let mv = *legal.iter().next().expect("No legal moves");
                                (mv, String::from("No engine selected"))
                            }
                            PlayerConfig::BuiltIn { .. } => {
                                let mut engine = search::SearchEngine::new(hash_mb, threads);
                                engine.set_use_nnue(use_nnue);
                                let mut note = String::new();
                                if let Some(path) = eval_file.as_ref() {
                                    match eval::nnue::load_network(std::path::Path::new(path)) {
                                        Ok(net) => {
                                            engine.set_nnue_network(net);
                                            note = format!(" | net {}", engine.nnue_info().name);
                                        }
                                        Err(err) => {
                                            note = format!(" | EvalFile error: {err}");
                                        }
                                    }
                                }
                                let result = engine.search_time(
                                    &mut board_clone,
                                    std::time::Duration::from_secs(time_secs),
                                    max_depth,
                                );
                                (
                                    result.best_move,
                                    format!(
                                        "depth {} | score {} cp | {} nodes | {:.0} nps{}",
                                        result.depth,
                                        result.score,
                                        result.nodes,
                                        result.nodes as f64
                                            / result.elapsed.as_secs_f64().max(0.001),
                                        note,
                                    ),
                                )
                            }
                            PlayerConfig::External { path, protocol } => {
                                let fen = board_clone.to_fen();
                                let legal = board_clone.generate_legal_moves();
                                match uci_process::query_best_move(
                                    &path,
                                    protocol,
                                    &fen,
                                    max_depth,
                                    std::time::Duration::from_secs(time_secs),
                                    hash_mb,
                                    threads,
                                ) {
                                    Ok(info) => {
                                        if let Some(mv) = legal
                                            .iter()
                                            .find(|m| m.to_uci() == info.best_move)
                                            .copied()
                                        {
                                            (
                                                mv,
                                                format!(
                                                    "{protocol} depth {} | score {} cp | {} nodes | {} nps",
                                                    info.depth, info.score, info.nodes, info.nps
                                                ),
                                            )
                                        } else {
                                            let mv = *legal
                                                .iter()
                                                .next()
                                                .expect("No legal moves in external fallback");
                                            (
                                                mv,
                                                format!(
                                                    "{protocol} returned illegal move '{}' - fallback",
                                                    info.best_move
                                                ),
                                            )
                                        }
                                    }
                                    Err(e) => {
                                        let mv = *legal
                                            .iter()
                                            .next()
                                            .expect("No legal moves in external error fallback");
                                        (mv, format!("{protocol} error: {e} - fallback move"))
                                    }
                                }
                            }
                        }
                    })
                    .expect("Failed to spawn engine thread");
                match handle.join() {
                    Ok(result) => result,
                    Err(_) => {
                        let mut fb = fallback_board_clone;
                        types::init();
                        let moves = fb.generate_legal_moves();
                        let mv = *moves.iter().next().expect("No legal moves in fallback");
                        (mv, String::from("Engine error - fallback move"))
                    }
                }
            },
            |(mv, info)| Msg::EngineMove(mv, info),
        )
    }

    fn view(&self) -> Element<'_, Msg> {
        let content = match self.screen {
            Screen::Menu => self.view_menu(),
            Screen::Playing => self.view_game(),
        };

        // Wrap in options modal overlay if open
        let page: Element<'_, Msg> = if self.show_options {
            let modal = self.view_options_modal();
            iced::widget::stack![content, modal].into()
        } else {
            content
        };

        container(page)
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
            .into()
    }

    fn view_title_bar(&self) -> Element<'_, Msg> {
        let pal = self.settings.board_theme.gui_palette();

        // ── Left: Logo + two-line title ──
        let logo_icon: Image<iced::widget::image::Handle> =
            Image::new(self.logo.clone()).width(24).height(24);
        let title_block = column![
            text("KishMat").size(14).color(pal.text_primary),
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
        let mut actions = row![].spacing(3).align_y(Alignment::Center);
        actions = actions.push(pill_button(lucide_icon(iced_fonts::lucide::settings), "Options", pal, false, Msg::ToggleOptions));
        if matches!(self.screen, Screen::Playing) {
            actions = actions
                .push(pill_button(lucide_icon(iced_fonts::lucide::camera), "Shot", pal, false, Msg::TakeScreenshot))
                .push(pill_button(lucide_icon(iced_fonts::lucide::plus), "New", pal, false, Msg::NewGame))
                .push(pill_button(lucide_icon(iced_fonts::lucide::arrow_up_down), "Flip", pal, false, Msg::FlipBoard))
                .push(pill_button(lucide_icon(iced_fonts::lucide::flag), "Resign", pal, true, Msg::Resign))
                .push(pill_sep(pal))
                .push(pill_button(lucide_icon(iced_fonts::lucide::clipboard_copy), "PGN", pal, false, Msg::ExportPGN))
                .push(pill_button(lucide_icon(iced_fonts::lucide::film), "GIF", pal, false, Msg::ExportGIF));
            let (rec_icon, rec_label): (Element<'_, Msg>, &str) = match self.recorder.state() {
                recording::RecordState::Idle => (lucide_icon(iced_fonts::lucide::circle), "Rec"),
                recording::RecordState::Recording => (lucide_icon(iced_fonts::lucide::circle_stop), "Stop"),
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
            actions = actions.push(pill_button(lucide_icon(iced_fonts::lucide::play), "Start", pal, false, Msg::StartGame));
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
            pill_button(lucide_icon(iced_fonts::lucide::x), "Exit", pal, true, Msg::ExitApp),
        ]
        .spacing(10)
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
        let logo_img: Image<iced::widget::image::Handle> =
            Image::new(self.logo.clone()).width(100).height(100);

        let title = text("KishMat Chess")
            .size(42)
            .color(pal.text_primary)
            .font(CURIOUS_FONT);
        let subtitle = text("The First Arabian Chess Engine")
            .size(16)
            .color(pal.text_secondary);

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
        .width(280);

        let badge_w = 40.0;
        let w_badge = container(text("W").size(16).color(Color::from_rgb(0.20, 0.15, 0.10)))
            .center_x(badge_w)
            .center_y(badge_w)
            .width(badge_w)
            .height(badge_w)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.94, 0.88, 0.76))),
                border: iced::Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: Color::from_rgb(0.80, 0.75, 0.60),
                },
                ..Default::default()
            });
        let b_badge = container(text("B").size(16).color(Color::from_rgb(0.80, 0.80, 0.85)))
            .center_x(badge_w)
            .center_y(badge_w)
            .width(badge_w)
            .height(badge_w)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.14, 0.12, 0.16))),
                border: iced::Border {
                    radius: 6.0.into(),
                    width: 1.0,
                    color: Color::from_rgb(0.30, 0.30, 0.35),
                },
                ..Default::default()
            });

        let mut left_col = column![
            text("Game Setup").size(14).color(ACCENT_TEAL),
            Space::new().height(8),
            text("Mode").size(12).color(pal.text_secondary),
            mode_picker,
            Space::new().height(8),
            row![
                w_badge,
                text(format!(" {}", self.white_player))
                    .size(13)
                    .color(pal.text_primary)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(4)
        .width(340);

        if matches!(self.selected_mode, GameMode::EngineVsEngine) {
            left_col = left_col
                .push(styled_button("Load White UCI", Msg::LoadWhiteUciEngine))
                .push(styled_button(
                    "Load White XBoard",
                    Msg::LoadWhiteXboardEngine,
                ));
        }
        left_col = left_col.push(
            row![
                b_badge,
                text(format!(" {}", self.black_player))
                    .size(13)
                    .color(pal.text_primary)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        if matches!(
            self.selected_mode,
            GameMode::HumanVsEngine | GameMode::EngineVsEngine
        ) {
            left_col = left_col
                .push(styled_button("Load Black UCI", Msg::LoadBlackUciEngine))
                .push(styled_button(
                    "Load Black XBoard",
                    Msg::LoadBlackXboardEngine,
                ));
        }

        // Coin flip (HumanVsEngine)
        if matches!(self.selected_mode, GameMode::HumanVsEngine) {
            left_col = left_col.push(Space::new().height(8));
            let flip_el: Element<'_, Msg> = match &self.coin_flip {
                CoinFlipState::Idle => button(
                    container(
                        row![
                            iced_fonts::lucide::coins().size(18),
                            text(" Flip Coin").size(13).color(Color::WHITE)
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .center_x(160)
                    .center_y(36)
                    .width(160)
                    .height(36),
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
                            radius: 8.0.into(),
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

        let left_card = container(glass_card(left_col.into()))
            .width(340)
            .height(Length::Fill);

        // ── Right column: Engine Settings ──
        let cfg = &self.engine_cfg;
        let eval_file_label = cfg
            .eval_file
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Embedded".to_string());
        let right_col = column![
            text("Engine Settings").size(14).color(ACCENT_TEAL),
            Space::new().height(8),
            config_slider(
                "Time / Move",
                cfg.time_per_move,
                "s",
                1,
                30,
                Msg::CfgTimeChanged
            ),
            config_slider("Max Depth", cfg.max_depth, "", 1, 64, Msg::CfgDepthChanged),
            config_slider("Hash (MB)", cfg.hash_mb, "MB", 1, 4096, Msg::CfgHashChanged),
            config_slider("Threads", cfg.threads, "", 1, 32, Msg::CfgThreadsChanged),
            Space::new().height(4),
            settings_row("Ponder", toggler(cfg.ponder).on_toggle(|_| Msg::CfgTogglePonder).size(18).into()),
            settings_row("Opening Book", toggler(cfg.use_book).on_toggle(|_| Msg::CfgToggleBook).size(18).into()),
            settings_row("NNUE Eval", toggler(cfg.use_nnue).on_toggle(|_| Msg::CfgToggleNnue).size(18).into()),
            Space::new().height(4),
            row![
                text("Eval Net").size(12).color(TEXT_SECONDARY).width(75),
                text(eval_file_label).size(12).color(TEXT_PRIMARY),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                styled_button("Load NNUE File", Msg::LoadBuiltinEvalFile),
                styled_button("Use Embedded", Msg::ClearBuiltinEvalFile),
            ]
            .spacing(8),
        ]
        .spacing(4)
        .width(340);

        let right_card = container(glass_card(right_col.into()))
            .width(340)
            .height(Length::Fill);

        // ── Start button ──
        let start_btn = button(
            container(text("Start Game").size(16).color(Color::WHITE))
                .center_x(200)
                .center_y(48)
                .width(200)
                .height(48),
        )
        .on_press(Msg::StartGame)
        .style(|_theme, status| {
            let bg = if matches!(status, button::Status::Hovered) {
                Color::from_rgb(0.30, 0.60, 1.0)
            } else {
                ACCENT
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
        });

        // ── Two-column layout ──
        let two_cols =
            row![left_card, Space::new().width(20), right_card].align_y(Alignment::Start);

        let menu_content = column![
            Space::new().height(20),
            logo_img,
            Space::new().height(4),
            title,
            subtitle,
            Space::new().height(20),
            two_cols,
            Space::new().height(20),
            start_btn,
            Space::new().height(8),
            text("v2.0").size(11).color(pal.text_secondary),
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
            container(menu_content)
                .center_x(Length::Fill)
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

        let title_bar_h = 42.0_f32;
        let padding = 40.0_f32;
        let available_h = (self.window_height - title_bar_h - padding).max(200.0);
        let sq_size = (available_h * 0.90 / 8.0).min(120.0).max(40.0);

        let anim_info = self.animation.as_ref().map(|anim| {
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
            sq_size,
            anim_info,
            self.settings.board_theme,
            self.settings.show_coords,
            self.settings.coord_position,
            self.settings.capture_anim_style,
            &gs.arrows,
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
                let white_move = text(&pair[0]).size(13).color(pal.text_primary).width(70);
                let black_move = if let Some(b) = pair.get(1) {
                    text(b.as_str()).size(13).color(pal.text_primary).width(70)
                } else {
                    text("...").size(13).color(pal.text_secondary).width(70)
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

        let panel_w = (sq_size * 3.5).max(280.0).min(400.0); // Proportional side panel

        let side_panel = container(
            column![
                text("Moves").size(13).color(pal.text_secondary),
                moves_panel,
                Space::new().height(8),
                text("Engine").size(13).color(pal.text_secondary),
                engine_panel,
            ]
            .spacing(6)
            .width(panel_w),
        )
        .padding(12)
        .height(board_total)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(pal.panel)),
            border: iced::Border {
                radius: 8.0.into(),
                width: 1.0,
                color: pal.border,
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
    fn view_options_modal(&self) -> Element<'_, Msg> {
        let s = &self.settings;

        // Display section
        let theme_picker = pick_list(
            board_view::BoardTheme::ALL.to_vec(),
            Some(s.board_theme),
            Msg::SetBoardTheme,
        )
        .width(160);

        let anim_label = match s.anim_speed {
            0 => "Fast",
            2 => "Slow",
            _ => "Normal",
        };

        let capture_anim_picker = pick_list(
            vec![CaptureAnimStyle::Explosion, CaptureAnimStyle::Fire, CaptureAnimStyle::Instant],
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
                settings_row(
                    "Board Theme",
                    theme_picker.into(),
                ),
                settings_row(
                    "Show Coordinates",
                    toggler(s.show_coords)
                        .on_toggle(Msg::SetShowCoords)
                        .size(18)
                        .into(),
                ),
                settings_row(
                    "Coord Position",
                    coord_pos_picker.into(),
                ),
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
                settings_row(
                    "Capture Effect",
                    capture_anim_picker.into(),
                ),
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
                    toggler(s.sfx_on)
                        .on_toggle(Msg::SetSfx)
                        .size(18)
                        .into(),
                ),
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
                settings_row(
                    "Game Mood",
                    mood_picker.into(),
                ),
            ]
            .spacing(2)
            .into(),
        );

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
        let settings_color = if settings_tab_active { Color::WHITE } else { TEXT_SECONDARY };
        let tools_color = if !settings_tab_active { Color::WHITE } else { TEXT_SECONDARY };
        let tab_buttons = row![
            button(row![iced_fonts::lucide::settings().size(13).color(settings_color), text(" Settings").size(13).color(settings_color)].spacing(4).align_y(Alignment::Center))
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
            button(row![iced_fonts::lucide::wrench().size(13).color(tools_color), text(" Tools").size(13).color(tools_color)].spacing(4).align_y(Alignment::Center))
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
            scrollable(
                column![
                    display_section,
                    audio_section,
                    game_section,
                ]
                .spacing(12),
            )
            .height(450)
            .into()
        } else {
            // Tools tab
            let syzygy_section = settings_card(
                iced_fonts::lucide::database,
                "Syzygy Tablebases",
                column![
                    settings_row("Status", text(&self.syzygy_status).size(12).color(TEXT_PRIMARY).into()),
                    settings_row("Path", text("./syzygy/").size(12).color(TEXT_PRIMARY).into()),
                    Space::new().height(4),
                    styled_button_with_icon(iced_fonts::lucide::download, "Download 3-4-5 Piece Tables (~1 GB)", Msg::SyzygyDownload),
                ]
                .spacing(2)
                .into(),
            );

            let nnue_section = settings_card(
                iced_fonts::lucide::brain,
                "NNUE Networks",
                column![
                    settings_row("Status", text(&self.nnue_status).size(12).color(TEXT_PRIMARY).into()),
                    settings_row("Path", text("./nnue/").size(12).color(TEXT_PRIMARY).into()),
                    Space::new().height(4),
                    styled_button_with_icon(iced_fonts::lucide::download, "Download All NNUE Networks", Msg::NnueDownload),
                ]
                .spacing(2)
                .into(),
            );

            let mut tuning_content = column![
                settings_row("Status", text(&self.tuning_status).size(12).color(TEXT_PRIMARY).into()),
            ]
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
                    tuning_content = tuning_content.push(
                        settings_row(
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
                        ),
                    );
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
                column![
                    styled_button("Check for Updates", Msg::CheckForUpdates),
                ]
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

        let modal_content = container(
            column![
                text("Options").size(20).color(TEXT_PRIMARY),
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

        // Dark backdrop
        container(
            container(modal_content)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
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
fn glass_card(content: Element<'_, Msg>) -> Element<'_, Msg> {
    container(content)
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.086, 0.129, 0.243, 0.85,
            ))),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: Color::from_rgba(0.30, 0.35, 0.50, 0.6),
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

/// Styled secondary button with a Lucide icon prefix.
fn styled_button_with_icon<'a>(
    icon_fn: fn() -> iced::widget::Text<'a, iced::Theme, iced::Renderer>,
    label: &str,
    msg: Msg,
) -> Element<'a, Msg> {
    let label_text = label.to_string();
    button(
        container(
            row![icon_fn().size(12), text(label_text).size(12).color(TEXT_PRIMARY)]
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

async fn pick_nnue_file() -> Option<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Select NNUE Network")
        .add_filter("NNUE", &["bin", "nnue"])
        .pick_file()
        .await?;
    Some(file.path().to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::EngineConfig;

    #[test]
    fn test_engine_config_defaults_to_embedded_eval() {
        let cfg = EngineConfig::default();
        assert!(cfg.eval_file.is_none());
        assert!(cfg.use_nnue);
    }
}
