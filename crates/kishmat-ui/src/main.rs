//! KishMat GUI — premium chess interface with macOS-style design.
//!
//! Features:
//! - macOS grain noise texture backgrounds
//! - High-fidelity colored chess pieces (image-based)
//! - Human vs Human, Human vs Engine, Engine vs Engine modes
//! - Styled title bar matching macOS aesthetic

mod board_view;
mod game;
mod noise;
mod pieces;
mod uci_process;

use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, text, Image, Space,
};
use iced::{Alignment, Color, Element, Length, Task, Theme};

use pieces::PieceAssets;

// ──────────────────────────────────────────────────────────────
// Colors — macOS-inspired palette
// ──────────────────────────────────────────────────────────────
const BG_DARK: Color = Color::from_rgb(0.176, 0.176, 0.188);       // #2D2D30
const BG_PANEL: Color = Color::from_rgb(0.204, 0.204, 0.220);      // #343438
const BG_SIDEBAR: Color = Color::from_rgb(0.165, 0.165, 0.178);    // #2A2A2D
const TEXT_PRIMARY: Color = Color::from_rgb(0.93, 0.93, 0.94);     // #EDEDED
const TEXT_SECONDARY: Color = Color::from_rgb(0.60, 0.60, 0.62);   // #999A9E
const ACCENT: Color = Color::from_rgb(0.25, 0.55, 0.96);           // Apple blue
const BORDER_SUBTLE: Color = Color::from_rgb(0.24, 0.24, 0.26);    // #3D3D42

fn main() -> iced::Result {
    // Embed the logo for the window icon
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../assets/logo.png"),
        None,
    ).ok();

    let mut app = iced::application("KishMat Chess", App::update, App::view)
        .theme(|_| Theme::Dark)
        .window_size((1120.0, 760.0));

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
    move_log: Vec<String>,
    status: String,
    engine_info: String,
    /// Pre-loaded piece images.
    assets: PieceAssets,
    /// Noise texture handles for backgrounds.
    noise_bg: iced::widget::image::Handle,
    noise_title: iced::widget::image::Handle,
    /// App logo handle.
    logo: iced::widget::image::Handle,
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
    /// Standalone engine info update (for running search progress).
    #[allow(dead_code)]
    EngineInfo(String),
    /// Periodic tick for UI updates (clock, animation, etc.).
    #[allow(dead_code)]
    Tick,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::Menu,
            game: None,
            selected_mode: GameMode::HumanVsHuman,
            white_player: PlayerConfig::Human,
            black_player: PlayerConfig::Human,
            move_log: Vec::new(),
            status: String::from("Welcome to KishMat!"),
            engine_info: String::new(),
            assets: PieceAssets::load(),
            noise_bg: noise::macos_grain_dark(),
            noise_title: noise::macos_grain_panel(),
            logo: iced::widget::image::Handle::from_bytes(
                include_bytes!("../assets/logo.png").as_slice(),
            ),
        }
    }
}

impl App {
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
                        self.black_player = PlayerConfig::BuiltIn { depth: 8 };
                    }
                    GameMode::EngineVsEngine => {
                        self.white_player = PlayerConfig::BuiltIn { depth: 8 };
                        self.black_player = PlayerConfig::BuiltIn { depth: 8 };
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
                        // Try to make a move to the clicked square
                        if let Some(mv) = gs.try_move(clicked_sq) {
                            self.move_log.push(mv.to_uci());

                            if gs.game_over {
                                self.status = if gs.board.is_checkmate() {
                                    let w = if gs.board.side_to_move == types::Color::White {
                                        "Black"
                                    } else {
                                        "White"
                                    };
                                    format!("Checkmate! {w} wins! 🎉")
                                } else {
                                    String::from("Game drawn — ½–½")
                                };
                                return Task::none();
                            }

                            let stm = if gs.board.side_to_move == types::Color::White {
                                "White"
                            } else {
                                "Black"
                            };
                            self.status = format!("{stm} to move");

                            let is_next_human = match gs.board.side_to_move {
                                types::Color::White => {
                                    matches!(self.white_player, PlayerConfig::Human)
                                }
                                types::Color::Black => {
                                    matches!(self.black_player, PlayerConfig::Human)
                                }
                            };
                            if !is_next_human {
                                return self.trigger_engine_move();
                            }
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
                self.engine_info = info;
                if let Some(ref mut gs) = self.game {
                    gs.last_move_squares = vec![mv.from, mv.to];
                    gs.board.make_move(mv);
                    self.move_log.push(mv.to_uci());

                    if gs.board.is_game_over() {
                        gs.game_over = true;
                        self.status = if gs.board.is_checkmate() {
                            let w = if gs.board.side_to_move == types::Color::White {
                                "Black"
                            } else {
                                "White"
                            };
                            format!("Checkmate! {w} wins! 🎉")
                        } else {
                            String::from("Game drawn — ½–½")
                        };
                        return Task::none();
                    }

                    let stm = if gs.board.side_to_move == types::Color::White {
                        "White"
                    } else {
                        "Black"
                    };
                    self.status = format!("{stm} to move");

                    let is_next_human = match gs.board.side_to_move {
                        types::Color::White => matches!(self.white_player, PlayerConfig::Human),
                        types::Color::Black => matches!(self.black_player, PlayerConfig::Human),
                    };
                    if !is_next_human && !gs.game_over {
                        return self.trigger_engine_move();
                    }
                }
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
            Msg::Tick => {
                // Tick can be used for periodic UI updates (e.g., clock, animations)
                if let Some(ref gs) = self.game {
                    if !gs.game_over {
                        let stm = if gs.board.side_to_move == types::Color::White {
                            "White"
                        } else {
                            "Black"
                        };
                        self.status = format!("{stm} to move");
                    }
                }
                Task::none()
            }
        }
    }

    fn trigger_engine_move(&self) -> Task<Msg> {
        if let Some(ref gs) = self.game {
            let mut board_clone = gs.board.clone();
            let depth = match gs.board.side_to_move {
                types::Color::White => match &self.white_player {
                    PlayerConfig::BuiltIn { depth } => *depth,
                    _ => 8,
                },
                types::Color::Black => match &self.black_player {
                    PlayerConfig::BuiltIn { depth } => *depth,
                    _ => 8,
                },
            };

            Task::perform(
                async move {
                    // No need for 16MB stack — ThreadState arrays are now heap-allocated.
                    let handle = std::thread::Builder::new()
                        .spawn(move || {
                            types::init();
                            let mut engine = search::SearchEngine::new(64, 1);
                            let result = engine.search_depth(&mut board_clone, depth);
                            (result.best_move, format!(
                                "depth {} | score {} cp | {} nodes | {:.0} nps",
                                result.depth, result.score, result.nodes,
                                result.nodes as f64 / result.elapsed.as_secs_f64().max(0.001),
                            ))
                        })
                        .expect("Failed to spawn engine thread");
                    handle.join().expect("Engine thread panicked")
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
        let title_grain: Image<iced::widget::image::Handle> = Image::new(self.noise_title.clone())
            .width(Length::Fill)
            .height(38);
        let _ = title_grain; // noise_title ready for stack/overlay use
        let title = row![
            logo_icon,
            text(" KishMat").size(16).color(TEXT_PRIMARY),
            text(" Chess Engine").size(14).color(TEXT_SECONDARY),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let status = text(&self.status)
            .size(12)
            .color(TEXT_SECONDARY);

        container(
            row![
                title,
                horizontal_space(),
                status,
            ]
            .align_y(Alignment::Center)
            .padding([0, 16]),
        )
        .width(Length::Fill)
        .height(38)
        .center_y(38)
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

        let mut config_col = column![].spacing(8).align_x(Alignment::Center);

        config_col = config_col.push(
            text(format!("⬜ White: {}", self.white_player))
                .size(14)
                .color(TEXT_PRIMARY),
        );
        if matches!(self.selected_mode, GameMode::EngineVsEngine) {
            config_col = config_col.push(styled_button("Load White UCI Engine", Msg::LoadWhiteEngine));
        }
        config_col = config_col.push(
            text(format!("⬛ Black: {}", self.black_player))
                .size(14)
                .color(TEXT_PRIMARY),
        );
        if matches!(
            self.selected_mode,
            GameMode::HumanVsEngine | GameMode::EngineVsEngine
        ) {
            config_col = config_col.push(styled_button("Load Black UCI Engine", Msg::LoadBlackEngine));
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

        let menu_content = column![
            Space::with_height(40),
            logo_img,
            Space::with_height(8),
            title,
            subtitle,
            Space::with_height(30),
            text("Game Mode").size(13).color(TEXT_SECONDARY),
            mode_picker,
            Space::with_height(16),
            config_col,
            Space::with_height(30),
            start_btn,
        ]
        .spacing(8)
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

        let board_view = board_view::view_board(gs, &self.assets);

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
            scrollable(text(move_history_text).size(13).color(TEXT_PRIMARY)).height(220),
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

        // Engine info
        let engine_text = if self.engine_info.is_empty() {
            String::from("Engine idle")
        } else {
            self.engine_info.clone()
        };

        let engine_panel = container(text(engine_text).size(11).color(TEXT_SECONDARY))
            .padding(8)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(BG_SIDEBAR)),
                border: iced::Border {
                    radius: 4.0.into(),
                    width: 1.0,
                    color: BORDER_SUBTLE,
                },
                ..Default::default()
            });

        // Control buttons
        let btn_row = row![
            styled_button("New Game", Msg::NewGame),
            styled_button("⇄ Flip", Msg::FlipBoard),
            styled_button("🏳 Resign", Msg::Resign),
        ]
        .spacing(6);

        let side_panel = container(
            column![
                text("Moves").size(12).color(TEXT_SECONDARY),
                moves_panel,
                Space::with_height(8),
                text("Engine").size(12).color(TEXT_SECONDARY),
                engine_panel,
                Space::with_height(12),
                btn_row,
            ]
            .spacing(6)
            .width(260),
        )
        .padding(12)
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

async fn pick_engine_file() -> Option<String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Select UCI Engine Executable")
        .pick_file()
        .await?;
    Some(file.path().to_string_lossy().to_string())
}
