//! Shared GUI domain: chess/engine/settings with no GUI-framework types.
//!
//! Helpers stay public so both Floem and Iced backends can share them; a given
//! backend may not call every entry point.

#![allow(dead_code)]

pub mod analysis;
pub mod arrows;
pub mod audio;
pub mod engine;
pub mod game;
pub mod gif_export;
pub mod logic;
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
pub mod tournament_setup;
pub mod uci_process;

#[allow(unused_imports)]
pub use engine::{
    BundledEngineChoice, EngineConfig, GameMode, PlayerConfig, QuickTournamentEngine,
    TelemetrySnapshot, apply_search_info, bounded_hash_mb, builtin_analysis_line,
    builtin_engine_search, bundled_engine_choices, bundled_engine_label, selected_bundled_engine,
};
#[allow(unused_imports)]
pub use palette::{BoardTheme, GuiPalette, Rgba, ThemeColors};
#[allow(unused_imports)]
pub use pieces::{PieceAssets, PieceSet};
#[allow(unused_imports)]
pub use settings::{AppSettings, CaptureAnimStyle, CoordPosition, OptionsTab, Screen};
