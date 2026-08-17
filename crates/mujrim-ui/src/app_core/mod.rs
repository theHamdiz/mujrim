//! Shared GUI domain: chess/engine/settings with no GUI-framework types.
//!
//! Helpers stay public so the Floem UI can share them; not every entry point
//! is called from every screen.

#![allow(dead_code)]

pub mod analysis;
pub mod arrows;
pub mod ateed_resume;
pub mod ateed_studio;
pub mod audio;
pub mod engine;
pub mod fonts;
pub mod game;
pub mod game_resume;
pub mod gif_export;
pub mod hub;
pub mod layout;
pub mod logic;
pub mod match_controller;
pub mod motion;
pub mod noise;
pub mod palette;
pub mod pieces;
pub mod premove;
pub mod recording;
pub mod settings;
pub mod tournament_arena;
pub mod tournament_live;
pub mod tournament_results;
pub mod tournament_resume;
pub mod tournament_setup;
pub mod uci_process;
pub mod windowing;

#[allow(unused_imports)]
pub use engine::{
    BundledEngineChoice, EngineConfig, GameMode, PlayerConfig, QuickTournamentEngine,
    TelemetrySnapshot, apply_search_info, bounded_hash_mb, bundled_engine_choices,
    bundled_engine_label, discover_default_engine, resolve_engine_launch, selected_bundled_engine,
};
#[allow(unused_imports)]
pub use palette::{BoardTheme, GuiPalette, Rgba, ThemeColors};
#[allow(unused_imports)]
pub use pieces::{PieceAssets, PieceSet};
#[allow(unused_imports)]
pub use settings::{
    AppSettings, CaptureAnimStyle, CoordPosition, OptionsTab, PieceAnimStyle, Screen,
};
