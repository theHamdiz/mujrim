#![allow(unexpected_cfgs)]
//! Mujrim Installer — guided setup wizard.

mod downloads;
mod embedded;
mod install;

use std::path::PathBuf;

use iced::widget::{
    Image, Space, button, column, container, mouse_area, pick_list, progress_bar, row, scrollable,
    text, text_input, toggler,
};
use iced::{Alignment, Color, Element, Length, Task, Theme};

use downloads::NnueSelection;
use updater::syzygy::SyzygyPieceSet;

// ──────────────────────────────────────────────────────────────
// Colors — matching mujrim-ui palette
// ──────────────────────────────────────────────────────────────
const BG_DARK: Color = Color::from_rgb(0.102, 0.102, 0.180);
const ACCENT: Color = Color::from_rgb(0.914, 0.271, 0.376);
const ACCENT_TEAL: Color = Color::from_rgb(0.325, 0.749, 0.616);
const ACCENT_GOLD: Color = Color::from_rgb(0.706, 0.569, 0.235);
const TEXT_PRIMARY: Color = Color::from_rgb(0.96, 0.96, 0.96);
const TEXT_SECONDARY: Color = Color::from_rgb(0.627, 0.627, 0.690);
const BG_PANEL: Color = Color::from_rgb(0.086, 0.129, 0.243);

fn theme_fn(_: &App) -> Theme {
    Theme::Dark
}

// ──────────────────────────────────────────────────────────────
// Entry point
// ──────────────────────────────────────────────────────────────

pub fn run() -> iced::Result {
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../../../assets/branding/mujrim-icon.png"),
        None,
    )
    .ok();

    let mut win = iced::window::Settings {
        decorations: false,
        transparent: true,
        ..Default::default()
    };
    if let Some(icon) = icon {
        win.icon = Some(icon);
    }

    #[cfg(target_os = "macos")]
    set_macos_dock_icon();

    iced::application(App::boot, App::update, App::view)
        .title("Mujrim Installer")
        .subscription(App::subscription)
        .theme(theme_fn)
        .window_size((720.0, 560.0))
        .transparent(true)
        .window(win)
        .run()
}

#[cfg(target_os = "macos")]
fn set_macos_dock_icon() {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    unsafe {
        let nsapp_class = Class::get("NSApplication").unwrap();
        let app: *mut Object = msg_send![nsapp_class, sharedApplication];
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let png_data: &[u8] = include_bytes!("../../../assets/branding/mujrim-icon.png");
        let nsdata_class = Class::get("NSData").unwrap();
        let data: *mut Object = msg_send![nsdata_class, alloc];
        let data: *mut Object =
            msg_send![data, initWithBytes:png_data.as_ptr() length:png_data.len()];
        let nsimage_class = Class::get("NSImage").unwrap();
        let image: *mut Object = msg_send![nsimage_class, alloc];
        let image: *mut Object = msg_send![image, initWithData:data];
        if !image.is_null() {
            let _: () = msg_send![app, setApplicationIconImage:image];
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Wizard steps
// ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    InstallPath,
    Downloads,
    Installing,
    Complete,
}

// ──────────────────────────────────────────────────────────────
// Syzygy tier wrapper for pick_list Display
// ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyzygyTier(SyzygyPieceSet);

impl std::fmt::Display for SyzygyTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            SyzygyPieceSet::Standard => write!(f, "3-4-5 pieces (~1 GB)"),
            SyzygyPieceSet::Extended => write!(f, "3-4-5-6 pieces (~150 GB)"),
            SyzygyPieceSet::Full => write!(f, "3-4-5-6-7 pieces (~140 TB)"),
        }
    }
}

const SYZYGY_TIERS: &[SyzygyTier] = &[
    SyzygyTier(SyzygyPieceSet::Standard),
    SyzygyTier(SyzygyPieceSet::Extended),
    SyzygyTier(SyzygyPieceSet::Full),
];

// ──────────────────────────────────────────────────────────────
// Application state
// ──────────────────────────────────────────────────────────────

struct App {
    step: Step,
    install_dir: String,
    nnue_selections: Vec<NnueSelection>,
    syzygy_tier: SyzygyTier,
    download_nnue: bool,
    download_syzygy: bool,
    progress_msg: String,
    progress_value: f32,
    error: Option<String>,
    install_result: Option<install::InstallResult>,
    logo: iced::widget::image::Handle,
    window_id: Option<iced::window::Id>,
}

#[derive(Debug, Clone)]
enum Msg {
    NextStep,
    PrevStep,
    SetInstallDir(String),
    BrowseDir,
    DirSelected(Option<String>),
    ToggleNnue(usize, bool),
    SetSyzygyTier(SyzygyTier),
    ToggleDownloadNnue(bool),
    ToggleDownloadSyzygy(bool),
    #[allow(dead_code)]
    StartInstall,
    InstallBinariesDone(Result<install::InstallResult, String>),
    NnueDownloadDone(Result<usize, String>),
    SyzygyDownloadDone(Result<usize, String>),
    ExitApp,
    LaunchApp,
    DragWindow,
    WindowOpened(iced::window::Id),
    #[allow(dead_code)]
    FontLoaded,
}

impl Default for App {
    fn default() -> Self {
        let default_dir = install::default_install_dir();
        Self {
            step: Step::Welcome,
            install_dir: default_dir.to_string_lossy().to_string(),
            nnue_selections: downloads::default_nnue_selections(),
            syzygy_tier: SyzygyTier(downloads::default_syzygy_tier()),
            download_nnue: true,
            download_syzygy: false,
            progress_msg: String::new(),
            progress_value: 0.0,
            error: None,
            install_result: None,
            logo: iced::widget::image::Handle::from_bytes(
                include_bytes!("../../../assets/branding/mujrim-icon.png").as_slice(),
            ),
            window_id: None,
        }
    }
}

impl App {
    fn boot() -> (Self, Task<Msg>) {
        let load_lucide = iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(|_| Msg::FontLoaded);
        (Self::default(), load_lucide)
    }

    fn subscription(&self) -> iced::Subscription<Msg> {
        iced::window::open_events().map(Msg::WindowOpened)
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::NextStep => {
                self.error = None;
                self.step = match self.step {
                    Step::Welcome => Step::InstallPath,
                    Step::InstallPath => Step::Downloads,
                    Step::Downloads => Step::Installing,
                    Step::Installing => Step::Complete,
                    Step::Complete => Step::Complete,
                };
                if self.step == Step::Installing {
                    return self.begin_installation();
                }
                Task::none()
            }
            Msg::PrevStep => {
                self.error = None;
                self.step = match self.step {
                    Step::Welcome => Step::Welcome,
                    Step::InstallPath => Step::Welcome,
                    Step::Downloads => Step::InstallPath,
                    Step::Installing => Step::Installing,
                    Step::Complete => Step::Complete,
                };
                Task::none()
            }
            Msg::SetInstallDir(dir) => {
                self.install_dir = dir;
                Task::none()
            }
            Msg::BrowseDir => Task::perform(async { browse_dir().await }, Msg::DirSelected),
            Msg::DirSelected(path) => {
                if let Some(p) = path {
                    self.install_dir = p;
                }
                Task::none()
            }
            Msg::ToggleNnue(idx, checked) => {
                if let Some(sel) = self.nnue_selections.get_mut(idx) {
                    sel.selected = checked;
                }
                Task::none()
            }
            Msg::SetSyzygyTier(tier) => {
                self.syzygy_tier = tier;
                Task::none()
            }
            Msg::ToggleDownloadNnue(v) => {
                self.download_nnue = v;
                Task::none()
            }
            Msg::ToggleDownloadSyzygy(v) => {
                self.download_syzygy = v;
                Task::none()
            }
            Msg::StartInstall => {
                self.step = Step::Installing;
                self.begin_installation()
            }
            Msg::InstallBinariesDone(result) => match result {
                Ok(res) => {
                    self.install_result = Some(res);
                    self.progress_value = 0.5;
                    self.progress_msg = "Binaries installed.".into();

                    if self.download_nnue && self.nnue_selections.iter().any(|s| s.selected) {
                        self.progress_msg = "Downloading NNUE networks…".into();
                        let sels = self.nnue_selections.clone();
                        let dir = downloads::nnue_dir(&PathBuf::from(&self.install_dir));
                        return Task::perform(
                            async move { downloads::download_nnue_blocking(&sels, &dir) },
                            Msg::NnueDownloadDone,
                        );
                    }

                    if self.download_syzygy {
                        self.progress_msg = "Downloading Syzygy tablebases…".into();
                        let tier = self.syzygy_tier.0;
                        let dir = downloads::syzygy_dir(&PathBuf::from(&self.install_dir));
                        return Task::perform(
                            async move { downloads::download_syzygy_blocking(tier, &dir) },
                            Msg::SyzygyDownloadDone,
                        );
                    }

                    self.progress_value = 1.0;
                    self.progress_msg = "Installation complete!".into();
                    self.step = Step::Complete;
                    Task::none()
                }
                Err(e) => {
                    self.error = Some(e);
                    self.progress_msg = "Installation failed.".into();
                    Task::none()
                }
            },
            Msg::NnueDownloadDone(result) => {
                match result {
                    Ok(count) => {
                        self.progress_value = 0.75;
                        self.progress_msg = format!("{count} NNUE network(s) downloaded.");
                    }
                    Err(e) => {
                        self.progress_msg = format!("NNUE download warning: {e}");
                    }
                }

                if self.download_syzygy {
                    self.progress_msg = "Downloading Syzygy tablebases…".into();
                    let tier = self.syzygy_tier.0;
                    let dir = downloads::syzygy_dir(&PathBuf::from(&self.install_dir));
                    return Task::perform(
                        async move { downloads::download_syzygy_blocking(tier, &dir) },
                        Msg::SyzygyDownloadDone,
                    );
                }

                self.progress_value = 1.0;
                self.step = Step::Complete;
                Task::none()
            }
            Msg::SyzygyDownloadDone(result) => {
                match result {
                    Ok(count) => {
                        self.progress_msg =
                            format!("Syzygy: {count} tablebase file(s) downloaded.");
                    }
                    Err(e) => {
                        self.progress_msg = format!("Syzygy warning: {e}");
                    }
                }
                self.progress_value = 1.0;
                self.step = Step::Complete;
                Task::none()
            }
            Msg::ExitApp => {
                if let Some(id) = self.window_id {
                    iced::window::close(id)
                } else {
                    std::process::exit(0);
                }
            }
            Msg::LaunchApp => {
                let exe = PathBuf::from(&self.install_dir).join("mujrim-ui");
                let _ = std::process::Command::new(&exe).spawn();
                if let Some(id) = self.window_id {
                    iced::window::close(id)
                } else {
                    std::process::exit(0);
                }
            }
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
            Msg::FontLoaded => Task::none(),
        }
    }

    fn begin_installation(&mut self) -> Task<Msg> {
        self.progress_value = 0.0;
        self.progress_msg = "Installing binaries…".into();
        self.error = None;

        let dir = PathBuf::from(&self.install_dir);
        Task::perform(
            async move { install::install_all(&dir) },
            Msg::InstallBinariesDone,
        )
    }

    // ─── View ────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Msg> {
        let title_bar = self.title_bar();
        let body: Element<'_, Msg> = match self.step {
            Step::Welcome => self.view_welcome(),
            Step::InstallPath => self.view_install_path(),
            Step::Downloads => self.view_downloads(),
            Step::Installing => self.view_installing(),
            Step::Complete => self.view_complete(),
        };

        let content = column![title_bar, body]
            .width(Length::Fill)
            .height(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(BG_DARK)),
                ..Default::default()
            })
            .into()
    }

    fn title_bar(&self) -> Element<'_, Msg> {
        let title = text("Mujrim Installer").size(16).color(TEXT_PRIMARY);

        let close_btn = button(iced_fonts::lucide::x().size(16))
            .on_press(Msg::ExitApp)
            .style(|_, _| button::Style {
                background: None,
                text_color: TEXT_SECONDARY,
                ..Default::default()
            })
            .padding([4, 8]);

        let drag_area = mouse_area(container(title).padding([8, 16]).width(Length::Fill))
            .on_press(Msg::DragWindow);

        container(
            row![drag_area, close_btn]
                .align_y(Alignment::Center)
                .width(Length::Fill),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(BG_PANEL)),
            ..Default::default()
        })
        .width(Length::Fill)
        .into()
    }

    // ── Step 1: Welcome ──

    fn view_welcome(&self) -> Element<'_, Msg> {
        let logo = Image::new(self.logo.clone()).width(128).height(128);

        let heading = text("Welcome to Mujrim").size(32).color(TEXT_PRIMARY);
        let subtitle = text("The First Arabian Chess Engine")
            .size(16)
            .color(TEXT_SECONDARY);
        let version = text("Version 1.0.0").size(14).color(ACCENT_GOLD);

        let payload_info = if embedded::has_payload() {
            let size = downloads::human_bytes(embedded::total_size());
            text(format!("Bundled payload: {size}"))
                .size(13)
                .color(ACCENT_TEAL)
        } else {
            text("⚠ No binaries embedded — rebuild with `just installer`")
                .size(13)
                .color(ACCENT)
        };

        let install_btn = accent_btn("Install").on_press(Msg::NextStep);

        column![
            Space::new().height(30),
            logo,
            Space::new().height(16),
            heading,
            Space::new().height(4),
            subtitle,
            Space::new().height(8),
            version,
            Space::new().height(4),
            payload_info,
            Space::new().height(32),
            install_btn,
        ]
        .align_x(Alignment::Center)
        .spacing(0)
        .padding(40)
        .width(Length::Fill)
        .into()
    }

    // ── Step 2: Install Path ──

    fn view_install_path(&self) -> Element<'_, Msg> {
        let heading = text("Choose Installation Directory")
            .size(24)
            .color(TEXT_PRIMARY);

        let path_input = text_input("Install path…", &self.install_dir)
            .on_input(Msg::SetInstallDir)
            .padding(10)
            .width(420);

        let browse_btn = panel_btn("Browse…").on_press(Msg::BrowseDir);

        let path_row = row![path_input, browse_btn]
            .spacing(8)
            .align_y(Alignment::Center);

        let note = text(
            "Binaries will be placed in this directory. \
             NNUE and Syzygy files go into subdirectories.",
        )
        .size(13)
        .color(TEXT_SECONDARY);

        let nav = nav_row(Msg::PrevStep, Msg::NextStep);

        column![
            Space::new().height(30),
            heading,
            Space::new().height(24),
            path_row,
            Space::new().height(8),
            note,
            Space::new().height(Length::Fill),
            nav,
            Space::new().height(20),
        ]
        .align_x(Alignment::Center)
        .spacing(0)
        .padding(40)
        .width(Length::Fill)
        .into()
    }

    // ── Step 3: Downloads ──

    fn view_downloads(&self) -> Element<'_, Msg> {
        let heading = text("Optional Downloads").size(24).color(TEXT_PRIMARY);

        // NNUE toggle
        let nnue_toggle = toggler(self.download_nnue)
            .on_toggle(Msg::ToggleDownloadNnue)
            .label("Download NNUE Networks")
            .size(18);

        let nnue_list: Element<'_, Msg> = if self.download_nnue {
            let items: Vec<Element<'_, Msg>> = self
                .nnue_selections
                .iter()
                .enumerate()
                .map(|(i, sel)| {
                    let label = format!(
                        "{} ({}) — {}",
                        sel.network.name,
                        sel.network.engine,
                        downloads::human_bytes(sel.network.approx_size),
                    );
                    toggler(sel.selected)
                        .on_toggle(move |v| Msg::ToggleNnue(i, v))
                        .label(label)
                        .size(14)
                        .into()
                })
                .collect();
            container(column(items).spacing(6)).padding([0, 24]).into()
        } else {
            Space::new().height(0).into()
        };

        // Syzygy toggle
        let syzygy_toggle = toggler(self.download_syzygy)
            .on_toggle(Msg::ToggleDownloadSyzygy)
            .label("Download Syzygy Tablebases")
            .size(18);

        let syzygy_picker: Element<'_, Msg> = if self.download_syzygy {
            let tier_label = text("Piece count:").size(13).color(TEXT_SECONDARY);
            let picker =
                pick_list(SYZYGY_TIERS, Some(self.syzygy_tier), Msg::SetSyzygyTier).text_size(13);

            let size_note = text(format!(
                "Estimated download: {}",
                downloads::human_bytes(downloads::syzygy_estimated_size(self.syzygy_tier.0)),
            ))
            .size(12)
            .color(ACCENT_GOLD);

            let warn: Element<'_, Msg> = if matches!(
                self.syzygy_tier.0,
                SyzygyPieceSet::Extended | SyzygyPieceSet::Full
            ) {
                text("⚠ Very large download — ensure sufficient disk space.")
                    .size(12)
                    .color(ACCENT)
                    .into()
            } else {
                Space::new().height(0).into()
            };

            column![
                row![tier_label, picker]
                    .spacing(8)
                    .align_y(Alignment::Center),
                size_note,
                warn,
            ]
            .spacing(6)
            .into()
        } else {
            Space::new().height(0).into()
        };

        let content = column![
            heading,
            Space::new().height(16),
            nnue_toggle,
            nnue_list,
            Space::new().height(16),
            syzygy_toggle,
            syzygy_picker,
        ]
        .spacing(4)
        .width(Length::Fill);

        let nav = nav_row(Msg::PrevStep, Msg::NextStep);
        let scrolled = scrollable(content).height(Length::Fill);

        column![
            Space::new().height(20),
            scrolled,
            Space::new().height(8),
            nav,
            Space::new().height(20),
        ]
        .padding(40)
        .width(Length::Fill)
        .into()
    }

    // ── Step 4: Installing ──

    fn view_installing(&self) -> Element<'_, Msg> {
        let heading = text("Installing…").size(24).color(TEXT_PRIMARY);

        let bar = container(progress_bar(0.0..=1.0, self.progress_value)).max_width(500);

        let status = text(&self.progress_msg).size(14).color(TEXT_SECONDARY);

        let err: Element<'_, Msg> = if let Some(ref e) = self.error {
            text(e).size(13).color(ACCENT).into()
        } else {
            Space::new().height(0).into()
        };

        column![
            Space::new().height(60),
            heading,
            Space::new().height(32),
            bar,
            Space::new().height(12),
            status,
            err,
        ]
        .align_x(Alignment::Center)
        .spacing(0)
        .padding(40)
        .width(Length::Fill)
        .into()
    }

    // ── Step 5: Complete ──

    fn view_complete(&self) -> Element<'_, Msg> {
        let icon = iced_fonts::lucide::circle_check()
            .size(64)
            .color(ACCENT_TEAL);

        let heading = text("Installation Complete!").size(28).color(TEXT_PRIMARY);

        let summary_text = if let Some(ref res) = self.install_result {
            format!(
                "Installed {} binaries and {} shortcuts to {}",
                res.binaries_written,
                res.shortcuts_created,
                res.install_dir.display(),
            )
        } else {
            String::from("Installation finished.")
        };

        let summary = text(summary_text).size(14).color(TEXT_SECONDARY);
        let status = text(&self.progress_msg).size(13).color(TEXT_SECONDARY);

        let launch_btn = accent_btn("Launch Mujrim").on_press(Msg::LaunchApp);
        let close_btn = panel_btn("Close").on_press(Msg::ExitApp);

        let buttons = row![launch_btn, close_btn].spacing(12);

        column![
            Space::new().height(40),
            icon,
            Space::new().height(16),
            heading,
            Space::new().height(12),
            summary,
            status,
            Space::new().height(32),
            buttons,
        ]
        .align_x(Alignment::Center)
        .spacing(0)
        .padding(40)
        .width(Length::Fill)
        .into()
    }
}

// ──────────────────────────────────────────────────────────────
// Styled buttons (matching mujrim-ui)
// ──────────────────────────────────────────────────────────────

fn accent_btn(label: &str) -> iced::widget::Button<'_, Msg> {
    button(
        text(label)
            .size(15)
            .color(TEXT_PRIMARY)
            .align_x(Alignment::Center),
    )
    .padding([10, 28])
    .style(|_, _| button::Style {
        background: Some(iced::Background::Color(ACCENT)),
        border: iced::Border {
            radius: 6.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        text_color: TEXT_PRIMARY,
        ..Default::default()
    })
}

fn panel_btn(label: &str) -> iced::widget::Button<'_, Msg> {
    button(
        text(label)
            .size(15)
            .color(TEXT_PRIMARY)
            .align_x(Alignment::Center),
    )
    .padding([10, 28])
    .style(|_, _| button::Style {
        background: Some(iced::Background::Color(BG_PANEL)),
        border: iced::Border {
            radius: 6.0.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        text_color: TEXT_PRIMARY,
        ..Default::default()
    })
}

fn nav_row<'a>(back_msg: Msg, next_msg: Msg) -> Element<'a, Msg> {
    row![
        panel_btn("Back").on_press(back_msg),
        Space::new().width(Length::Fill),
        accent_btn("Next").on_press(next_msg),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

async fn browse_dir() -> Option<String> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Choose Installation Directory")
        .pick_folder()
        .await;
    handle.map(|h| h.path().to_string_lossy().to_string())
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_app_state() {
        let app = App::default();
        assert_eq!(app.step, Step::Welcome);
        assert!(!app.install_dir.is_empty());
        assert!(app.download_nnue);
        assert!(!app.download_syzygy);
    }

    #[test]
    fn syzygy_tier_display() {
        let tier = SyzygyTier(SyzygyPieceSet::Extended);
        let s = format!("{tier}");
        assert!(s.contains("150 GB"));
    }

    #[test]
    fn step_ordering() {
        assert_ne!(Step::Welcome, Step::InstallPath);
        assert_ne!(Step::InstallPath, Step::Downloads);
    }
}
