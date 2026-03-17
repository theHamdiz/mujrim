//! KishMat GUI — premium chess interface with macOS-style design.
//!
//! Features:
//! - macOS grain noise texture backgrounds
//! - High-fidelity colored chess pieces (image-based)
//! - Human vs Human, Human vs Engine, Engine vs Engine modes
//! - Styled title bar matching macOS aesthetic

mod audio;
mod board_view;
mod game;
mod noise;
mod pieces;
mod uci_process;

use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, slider, text, Image,
    Space,
};
use iced::{Alignment, Color, Element, Length, Subscription, Task, Theme};
use std::time::{Duration, Instant};

use pieces::PieceAssets;

// ──────────────────────────────────────────────────────────────
// Colors — premium dark theme
// ──────────────────────────────────────────────────────────────
const BG_DARK: Color = Color::from_rgb(0.102, 0.102, 0.180);       // #1A1A2E deep navy
const BG_PANEL: Color = Color::from_rgb(0.086, 0.129, 0.243);      // #16213E dark navy
const BG_SIDEBAR: Color = Color::from_rgb(0.059, 0.204, 0.376);    // #0F3460 midnight blue
const TEXT_PRIMARY: Color = Color::from_rgb(0.96, 0.96, 0.96);     // #F5F5F5
const TEXT_SECONDARY: Color = Color::from_rgb(0.627, 0.627, 0.690);// #A0A0B0
const ACCENT: Color = Color::from_rgb(0.914, 0.271, 0.376);        // #E94560 vibrant rose
const ACCENT_TEAL: Color = Color::from_rgb(0.325, 0.749, 0.616);   // #53BF9D teal
const BORDER_SUBTLE: Color = Color::from_rgb(0.16, 0.18, 0.28);    // #282D47

fn main() -> iced::Result {
    // Embed the logo for the window icon
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../assets/logo.png"),
        None,
    ).ok();

    let mut app = iced::application("KishMat Chess", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Dark)
        .window_size((1280.0, 850.0));

    if let Some(icon) = icon {
        app = app.window(iced::window::Settings {
            icon: Some(icon),
            ..Default::default()
        });
    }

    app.run()
}

/// The top-level application state.
struct App {
    screen: Screen,
    game: Option<game::GameState>,
    selected_mode: GameMode,
    white_player: PlayerConfig,
    black_player: PlayerConfig,
    /// Engine search time in seconds (configurable from menu).
    engine_time_secs: i32,
    move_log: Vec<String>,
    status: String,
    engine_info: String,
    /// Pre-loaded piece images.
    assets: PieceAssets,
    /// Noise texture handle for background.
    noise_bg: iced::widget::image::Handle,
    /// App logo handle.
    logo: iced::widget::image::Handle,
    /// Polyglot opening book (loaded once, probed before search).
    book: Option<search::book::OpeningBook>,
    /// Audio engine for move/capture sound effects.
    sound: Option<audio::SoundEngine>,
    /// Current move animation state.
    animation: Option<AnimationState>,
    /// Tracked window height for dynamic board sizing.
    window_height: f32,
}

/// State of an in-progress piece move animation.
struct AnimationState {
    /// The move being animated.
    mv: types::Move,
    /// Piece being moved.
    piece: types::Piece,
    /// Color of piece being moved.
    color: types::Color,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Playing,
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
    BuiltIn { depth: i32 },
    External { path: String },
}

impl std::fmt::Display for PlayerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => write!(f, "Human"),
            Self::BuiltIn { depth } => write!(f, "KishMat (depth {depth})"),
            Self::External { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                write!(f, "UCI: {name}")
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Msg {
    SelectMode(GameMode),
    StartGame,
    LoadWhiteEngine,
    LoadBlackEngine,
    WhiteEngineSelected(Option<String>),
    BlackEngineSelected(Option<String>),
    BoardClick(usize, usize),
    /// Engine finished searching: (best_move, search_info_string).
    EngineMove(types::Move, String),
    NewGame,
    FlipBoard,
    Resign,
    /// Export the current game as PGN.
    ExportPGN,
    /// Exit the application.
    ExitApp,
    /// Engine time slider changed.
    TimeChanged(i32),
    /// Standalone engine info update.
    #[allow(dead_code)]
    EngineInfo(String),
    /// Animation tick (60fps).
    AnimTick(Instant),
    /// Animation completed — apply the move.
    AnimComplete,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::Menu,
            game: None,
            selected_mode: GameMode::HumanVsHuman,
            white_player: PlayerConfig::Human,
            black_player: PlayerConfig::Human,
            engine_time_secs: 3,
            move_log: Vec::new(),
            status: String::from("Welcome to KishMat!"),
            engine_info: String::new(),
            assets: PieceAssets::load(),
            noise_bg: noise::macos_grain_dark(),
            logo: iced::widget::image::Handle::from_bytes(
                include_bytes!("../assets/logo.png").as_slice(),
            ),
            book: search::book::OpeningBook::load_embedded().ok(),
            sound: audio::SoundEngine::new(),
            animation: None,
            window_height: 850.0,
        }
    }
}

impl App {
    /// Subscription: animate at ~60fps while a move animation is in progress.
    fn subscription(&self) -> Subscription<Msg> {
        if self.animation.is_some() {
            iced::time::every(Duration::from_millis(16)).map(Msg::AnimTick)
        } else {
            Subscription::none()
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
                    if is_capture { sound.play_capture(); } else { sound.play_move(); }
                }

                self.animation = Some(AnimationState {
                    mv,
                    piece,
                    color,
                    captured,
                    is_capture,
                    start: Instant::now(),
                    duration: Duration::from_millis(150),
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
            gs.board.make_move(anim.mv);
            self.move_log.push(anim.mv.to_uci());

            if gs.board.is_game_over() {
                gs.game_over = true;
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

            if anim.trigger_engine_after {
                let is_next_human = match gs.board.side_to_move {
                    types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                    types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
                };
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
                Task::none()
            }
            Msg::LoadWhiteEngine => Task::perform(
                async { pick_engine_file().await },
                Msg::WhiteEngineSelected,
            ),
            Msg::LoadBlackEngine => Task::perform(
                async { pick_engine_file().await },
                Msg::BlackEngineSelected,
            ),
            Msg::WhiteEngineSelected(path) => {
                if let Some(p) = path {
                    self.white_player = PlayerConfig::External { path: p };
                }
                Task::none()
            }
            Msg::BlackEngineSelected(path) => {
                if let Some(p) = path {
                    self.black_player = PlayerConfig::External { path: p };
                }
                Task::none()
            }
            Msg::StartGame => {
                types::init();
                let board = types::Board::new();
                self.game = Some(game::GameState::new(board));
                self.move_log.clear();
                self.engine_info.clear();
                self.status = String::from("Game started — White to move");
                self.screen = Screen::Playing;
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
                if let Some(ref mut gs) = self.game {
                    if gs.game_over {
                        return Task::none();
                    }
                    let is_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
                    };
                    if !is_human {
                        return Task::none();
                    }

                    let sq_index =
                        if gs.flipped { row * 8 + col } else { (7 - row) * 8 + col };
                    let clicked_sq = types::Square::from_index(sq_index);

                    if gs.selected_square.is_some() {
                        // Check if this is a legal move destination
                        let from = gs.selected_square.unwrap();
                        let legal = gs.board.generate_legal_moves();
                        if let Some(mv) = legal.iter().find(|m| m.from == from && m.to == clicked_sq).copied() {
                            // Determine if engine plays next
                            let is_next_human = match gs.board.side_to_move {
                                types::Color::White => matches!(self.black_player, PlayerConfig::Human),
                                types::Color::Black => matches!(self.white_player, PlayerConfig::Human),
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
            Msg::AnimComplete => {
                self.finish_animation()
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
            Msg::TimeChanged(t) => {
                self.engine_time_secs = t;
                Task::none()
            }
            Msg::ExitApp => iced::exit(),
        }
    }

    fn trigger_engine_move(&self) -> Task<Msg> {
        if let Some(ref gs) = self.game {
            // ── Try opening book first (instant response) ─────────────
            if let Some(ref book) = self.book {
                if let Some(book_move) = book.probe(&gs.board) {
                    // Verify the book move is legal (defensive programming)
                    let legal = gs.board.clone().generate_legal_moves();
                    if legal.iter().any(|m| m.from == book_move.from && m.to == book_move.to) {
                        return Task::perform(
                            async move { (book_move, String::from("Book move")) },
                            |(mv, info)| Msg::EngineMove(mv, info),
                        );
                    }
                }
            }

            // ── Fall back to time-limited search ─────────────────────
            let mut board_clone = gs.board.clone();
            let fallback_board_clone = gs.board.clone();
            let time_secs = self.engine_time_secs as u64;

            Task::perform(
                async move {
                    // Use 8MB stack to handle the large ThreadState arrays
                    let handle = std::thread::Builder::new()
                        .stack_size(8 * 1024 * 1024)
                        .spawn(move || {
                            types::init();
                            let mut engine = search::SearchEngine::new(64, 1);
                            let result = engine.search_time(
                                &mut board_clone,
                                std::time::Duration::from_secs(time_secs),
                                64,
                            );
                            (result.best_move, format!(
                                "depth {} | score {} cp | {} nodes | {:.0} nps",
                                result.depth, result.score, result.nodes,
                                result.nodes as f64 / result.elapsed.as_secs_f64().max(0.001),
                            ))
                        })
                        .expect("Failed to spawn engine thread");
                    // Catch panics so the UI doesn't freeze
                    match handle.join() {
                        Ok(result) => result,
                        Err(_) => {
                            // Engine panicked — return a fallback: first legal move
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
        } else {
            Task::none()
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        let content = match self.screen {
            Screen::Menu => self.view_menu(),
            Screen::Playing => self.view_game(),
        };

        // Full window container with dark noise background
        let bg_image: Image<iced::widget::image::Handle> = Image::new(self.noise_bg.clone())
            .width(Length::Fill)
            .height(Length::Fill);
        let _ = bg_image; // noise_bg ready for stack/overlay use
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(BG_DARK)),
                ..Default::default()
            })
            .into()
    }

    // ══════════════════════════════════════════════════════════
    // Title bar — macOS-style with grain texture
    // ══════════════════════════════════════════════════════════
    fn view_title_bar(&self) -> Element<'_, Msg> {
        let logo_icon: Image<iced::widget::image::Handle> = Image::new(self.logo.clone())
            .width(28)
            .height(28);
        let title = row![
            logo_icon,
            text(" KishMat").size(16).color(TEXT_PRIMARY),
            text(" Chess Engine").size(14).color(TEXT_SECONDARY),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        // Right side: game buttons (only shown during play) + status
        let mut right_side = row![].spacing(8).align_y(Alignment::Center);

        if matches!(self.screen, Screen::Playing) {
            right_side = right_side
                .push(title_button("New Game", Msg::NewGame))
                .push(title_button("Flip", Msg::FlipBoard))
                .push(title_button("Resign", Msg::Resign))
                .push(title_button("Export PGN", Msg::ExportPGN));
        }
        right_side = right_side.push(title_button("Exit", Msg::ExitApp));

        right_side = right_side.push(
            text(&self.status).size(12).color(TEXT_SECONDARY)
        );

        container(
            row![
                title,
                horizontal_space(),
                right_side,
            ]
            .align_y(Alignment::Center)
            .padding([0, 16]),
        )
        .width(Length::Fill)
        .height(42)
        .center_y(42)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(BG_SIDEBAR)),
            border: iced::Border {
                radius: 0.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
            ..Default::default()
        })
        .into()
    }

    // ══════════════════════════════════════════════════════════
    // Menu screen
    // ══════════════════════════════════════════════════════════
    fn view_menu(&self) -> Element<'_, Msg> {
        let logo_img: Image<iced::widget::image::Handle> = Image::new(self.logo.clone())
            .width(120)
            .height(120);

        let title = text("KishMat Chess")
            .size(42)
            .color(TEXT_PRIMARY);

        let subtitle = text("The First Arabian Chess Engine")
            .size(16)
            .color(TEXT_SECONDARY);

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

        let mut config_col = column![].spacing(8).align_x(Alignment::Start).width(280);

        config_col = config_col.push(
            row![
                container(
                    text("W").size(16).color(Color::from_rgb(0.20, 0.15, 0.10)),
                )
                .padding([4, 10])
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(
                        Color::from_rgb(0.94, 0.88, 0.76),
                    )),
                    border: iced::Border { radius: 4.0.into(), ..Default::default() },
                    ..Default::default()
                }),
                text(format!(" {}", self.white_player))
                    .size(14)
                    .color(TEXT_PRIMARY),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        if matches!(self.selected_mode, GameMode::EngineVsEngine) {
            config_col = config_col.push(styled_button("Load White UCI Engine", Msg::LoadWhiteEngine));
        }
        config_col = config_col.push(
            row![
                container(
                    text("B").size(16).color(Color::from_rgb(0.80, 0.80, 0.85)),
                )
                .padding([4, 10])
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(
                        Color::from_rgb(0.14, 0.12, 0.16),
                    )),
                    border: iced::Border {
                        radius: 4.0.into(),
                        width: 1.0,
                        color: Color::from_rgb(0.30, 0.30, 0.35),
                    },
                    ..Default::default()
                }),
                text(format!(" {}", self.black_player))
                    .size(14)
                    .color(TEXT_PRIMARY),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        if matches!(
            self.selected_mode,
            GameMode::HumanVsEngine | GameMode::EngineVsEngine
        ) {
            config_col = config_col.push(styled_button("Load Black UCI Engine", Msg::LoadBlackEngine));
        }

        // Engine depth slider (only for engine modes)
        if !matches!(self.selected_mode, GameMode::HumanVsHuman) {
            config_col = config_col.push(Space::with_height(4));
            config_col = config_col.push(
                column![
                    text(format!("Engine Time: {}s", self.engine_time_secs))
                        .size(13)
                        .color(TEXT_SECONDARY),
                    slider(1..=10, self.engine_time_secs, Msg::TimeChanged)
                        .width(260),
                ]
                .spacing(4),
            );
        }

        let start_btn = button(
            container(
                text("Start Game").size(16).color(Color::WHITE),
            )
            .center_x(180)
            .center_y(44)
            .width(180)
            .height(44),
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
                    radius: 8.0.into(),
                    ..Default::default()
                },
                text_color: Color::WHITE,
                ..Default::default()
            }
        });



        // Wrap config in a styled card
        let config_card = container(
            column![
                text("Game Mode").size(13).color(TEXT_SECONDARY),
                mode_picker,
                Space::with_height(12),
                config_col,
            ]
            .spacing(8)
            .width(300),
        )
        .padding(20)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(BG_PANEL)),
            border: iced::Border {
                radius: 10.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            ..Default::default()
        });

        let menu_content = column![
            Space::with_height(30),
            logo_img,
            Space::with_height(8),
            title,
            subtitle,
            Space::with_height(24),
            config_card,
            Space::with_height(24),
            start_btn,
            Space::with_height(12),
            text("v2.0").size(11).color(TEXT_SECONDARY),
        ]
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::Fill);

        column![
            self.view_title_bar(),
            container(menu_content)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill),
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

        // ── Dynamic board sizing: 90% of available height ─────────
        let title_bar_h = 42.0_f32;
        let padding = 40.0_f32; // 20px padding on each side
        let available_h = (self.window_height - title_bar_h - padding).max(200.0);
        let sq_size = (available_h * 0.90 / 8.0).min(120.0).max(40.0);

        // ── Build animation info (Copy, so no lifetime issues) ───
        let anim_info = self.animation.as_ref().map(|anim| {
            let progress = (anim.start.elapsed().as_secs_f32()
                / anim.duration.as_secs_f32()).min(1.0);
            board_view::AnimInfo {
                from_sq: anim.mv.from,
                to_sq: anim.mv.to,
                piece: anim.piece,
                color: anim.color,
                progress,
                captured: anim.captured,
            }
        });

        let board_view = board_view::view_board(gs, &self.assets, sq_size, anim_info);
        let board_total = sq_size * 8.0;

        // Move history
        let move_history_text = if self.move_log.is_empty() {
            String::from("No moves yet.")
        } else {
            self.move_log
                .chunks(2)
                .enumerate()
                .map(|(i, pair)| {
                    let w = &pair[0];
                    let b = pair.get(1).map(|s| s.as_str()).unwrap_or("...");
                    format!(" {}. {} {}", i + 1, w, b)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let moves_panel = container(
            scrollable(text(move_history_text).size(13).color(TEXT_PRIMARY))
                .height(Length::Fill),
        )
        .padding(10)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(BG_SIDEBAR)),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            ..Default::default()
        });

        // Engine info — structured display
        let engine_panel = if self.engine_info.is_empty() {
            container(
                text("Engine idle").size(11).color(TEXT_SECONDARY)
            )
            .padding(8)
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(BG_PANEL)),
                border: iced::Border { radius: 4.0.into(), width: 1.0, color: BORDER_SUBTLE },
                ..Default::default()
            })
        } else {
            container(
                column![
                    text("Engine Analysis").size(11).color(ACCENT_TEAL),
                    text(&self.engine_info).size(12).color(TEXT_PRIMARY),
                ]
                .spacing(4),
            )
            .padding(8)
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(BG_PANEL)),
                border: iced::Border { radius: 4.0.into(), width: 1.0, color: ACCENT_TEAL },
                ..Default::default()
            })
        };

        // Side panel width scales with board but has a minimum
        let panel_w = (sq_size * 4.0).max(250.0);

        let side_panel = container(
            column![
                text("Moves").size(13).color(TEXT_SECONDARY),
                moves_panel,
                Space::with_height(8),
                text("Engine").size(13).color(TEXT_SECONDARY),
                engine_panel,
            ]
            .spacing(6)
            .width(panel_w),
        )
        .padding(12)
        .height(board_total)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(BG_PANEL)),
            border: iced::Border {
                radius: 8.0.into(),
                width: 1.0,
                color: BORDER_SUBTLE,
            },
            ..Default::default()
        });

        let game_layout = row![board_view, side_panel,]
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

/// Compact button for the title bar.
fn title_button(label: &str, msg: Msg) -> Element<'_, Msg> {
    let label_text = label.to_string();
    button(
        text(label_text).size(11).color(TEXT_PRIMARY),
    )
    .on_press(msg)
    .padding([4, 10])
    .style(|_theme, status| {
        let bg = if matches!(status, button::Status::Hovered) {
            Color::from_rgb(0.18, 0.22, 0.38)
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 4.0.into(),
                width: 0.0,
                color: Color::TRANSPARENT,
            },
            text_color: TEXT_PRIMARY,
            ..Default::default()
        }
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
