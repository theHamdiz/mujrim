//! Persisted GUI settings and navigation enums.

use std::path::PathBuf;

use super::arrows::{ArrowColor, ArrowShape, ArrowSize};
use super::audio::{GameMood, SoundTheme};
use super::palette::BoardTheme;
use super::pieces::PieceSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CaptureAnimStyle {
    Instant,
    Explosion,
    Fire,
}

impl CaptureAnimStyle {
    pub const ALL: [Self; 3] = [Self::Instant, Self::Explosion, Self::Fire];
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CoordPosition {
    Inside,
    Outside,
}

impl CoordPosition {
    pub const ALL: [Self; 2] = [Self::Inside, Self::Outside];
}

impl std::fmt::Display for CoordPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inside => write!(f, "Inside"),
            Self::Outside => write!(f, "Outside"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Menu,
    Playing,
    Study,
    Tournaments,
    Analysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsTab {
    Settings,
    Tools,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub board_theme: BoardTheme,
    pub piece_set: PieceSet,
    pub show_coords: bool,
    pub anim_speed: i32,
    pub sfx_on: bool,
    pub bgm_volume: i32,
    pub game_mood: GameMood,
    pub sound_theme: SoundTheme,
    pub auto_flip_black: bool,
    pub show_legal_moves: bool,
    pub show_last_move: bool,
    pub premoves_enabled: bool,
    pub capture_anim_style: CaptureAnimStyle,
    pub coord_position: CoordPosition,
    pub multi_premoves: bool,
    pub draw_arrows: bool,
    pub arrow_shape: ArrowShape,
    pub arrow_color: ArrowColor,
    pub arrow_size: ArrowSize,
    pub piece_slide: bool,
    pub system_motion: bool,
    pub last_move_arrow: bool,
    pub ponder_arrow: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            board_theme: BoardTheme::Classic,
            piece_set: PieceSet::Cburnett,
            show_coords: true,
            anim_speed: 1,
            sfx_on: true,
            bgm_volume: 50,
            game_mood: GameMood::Mystique,
            sound_theme: SoundTheme::Wood,
            auto_flip_black: false,
            show_legal_moves: true,
            show_last_move: true,
            premoves_enabled: true,
            capture_anim_style: CaptureAnimStyle::Explosion,
            coord_position: CoordPosition::Inside,
            multi_premoves: true,
            draw_arrows: true,
            arrow_shape: ArrowShape::Smart,
            arrow_color: ArrowColor::Orange,
            arrow_size: ArrowSize::Normal,
            piece_slide: true,
            system_motion: true,
            last_move_arrow: true,
            ponder_arrow: true,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("mujrim");
        p.push("settings.toml");
        p
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            toml::from_str(&contents).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, toml_str);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_classic_cburnett() {
        let settings = AppSettings::default();
        assert_eq!(settings.board_theme, BoardTheme::Classic);
        assert_eq!(settings.piece_set, PieceSet::Cburnett);
        assert!(settings.piece_slide);
        assert_eq!(BoardTheme::ALL.len(), 8);
        assert_eq!(CaptureAnimStyle::ALL.len(), 3);
        assert_eq!(CoordPosition::ALL.len(), 2);
    }
}
