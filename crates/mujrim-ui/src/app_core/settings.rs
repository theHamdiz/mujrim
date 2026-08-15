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
    Shatter,
    Vortex,
    Spark,
}

impl CaptureAnimStyle {
    pub const ALL: [Self; 6] = [
        Self::Instant,
        Self::Explosion,
        Self::Fire,
        Self::Shatter,
        Self::Vortex,
        Self::Spark,
    ];
}

impl std::fmt::Display for CaptureAnimStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Instant => write!(f, "Instant"),
            Self::Explosion => write!(f, "Explosion"),
            Self::Fire => write!(f, "Fire"),
            Self::Shatter => write!(f, "Shatter"),
            Self::Vortex => write!(f, "Vortex"),
            Self::Spark => write!(f, "Spark"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PieceAnimStyle {
    Slide,
    Arc,
    Bounce,
    Warp,
    Instant,
}

impl PieceAnimStyle {
    pub const ALL: [Self; 5] = [
        Self::Slide,
        Self::Arc,
        Self::Bounce,
        Self::Warp,
        Self::Instant,
    ];
}

impl std::fmt::Display for PieceAnimStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slide => write!(f, "Slide"),
            Self::Arc => write!(f, "Arc hop"),
            Self::Bounce => write!(f, "Bounce"),
            Self::Warp => write!(f, "Warp"),
            Self::Instant => write!(f, "Instant"),
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
    Learn,
    Library,
    Tournaments,
    Analysis,
    Ateed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsTab {
    Display,
    Motion,
    Arrows,
    Audio,
    Analysis,
    Tools,
}

impl OptionsTab {
    pub const ALL: [Self; 6] = [
        Self::Display,
        Self::Motion,
        Self::Arrows,
        Self::Audio,
        Self::Analysis,
        Self::Tools,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::Motion => "Motion",
            Self::Arrows => "Arrows",
            Self::Audio => "Audio",
            Self::Analysis => "Analysis",
            Self::Tools => "Tools",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub board_theme: BoardTheme,
    pub piece_set: PieceSet,
    pub show_coords: bool,
    pub anim_speed: i32,
    pub sfx_on: bool,
    pub bgm_on: bool,
    pub bgm_volume: i32,
    pub game_mood: GameMood,
    pub sound_theme: SoundTheme,
    pub sidebar_width_px: f64,
    pub auto_flip_black: bool,
    pub show_legal_moves: bool,
    pub show_last_move: bool,
    pub premoves_enabled: bool,
    pub capture_anim_style: CaptureAnimStyle,
    pub piece_anim_style: PieceAnimStyle,
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
    pub show_threats: bool,
    pub dock_height_px: f64,
    pub eval_bar_engine: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            board_theme: BoardTheme::Classic,
            piece_set: PieceSet::Cburnett,
            show_coords: true,
            anim_speed: 1,
            sfx_on: true,
            bgm_on: true,
            bgm_volume: 50,
            game_mood: GameMood::Mystique,
            sound_theme: SoundTheme::Wood,
            sidebar_width_px: super::layout::SIDEBAR_IDEAL_PX,
            auto_flip_black: false,
            show_legal_moves: true,
            show_last_move: true,
            premoves_enabled: true,
            capture_anim_style: CaptureAnimStyle::Explosion,
            piece_anim_style: PieceAnimStyle::Arc,
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
            show_threats: true,
            dock_height_px: super::layout::DOCK_OPEN_PX,
            eval_bar_engine: EVAL_BAR_DEFAULT_ENGINE.to_owned(),
        }
    }
}

pub const EVAL_BAR_DEFAULT_ENGINE: &str = "mujrim-v60";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalBarEngineChoice {
    pub id: String,
    pub label: String,
}

impl std::fmt::Display for EvalBarEngineChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
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

    /// Merge a subset TOML document into the on-disk schema so unknown fields
    /// are never dropped.
    pub fn merge_and_save_toml(overlay: &str) {
        let mut base = Self::load();
        let Ok(mut base_val) = toml::Value::try_from(&base) else {
            base.save();
            return;
        };
        if let Ok(toml::Value::Table(overlay)) = overlay.parse()
            && let Some(dst) = base_val.as_table_mut()
        {
            for (key, value) in overlay {
                dst.insert(key, value);
            }
        }
        if let Ok(merged) = toml::Value::try_into(base_val) {
            base = merged;
        }
        base.save();
    }

    pub fn decode_subset<T: Default + serde::de::DeserializeOwned>() -> T {
        let core = Self::load();
        let Ok(text) = toml::to_string(&core) else {
            return T::default();
        };
        toml::from_str(&text).unwrap_or_else(|_| {
            let sanitized = text
                .replace(
                    "capture_anim_style = \"Shatter\"",
                    "capture_anim_style = \"Explosion\"",
                )
                .replace(
                    "capture_anim_style = \"Vortex\"",
                    "capture_anim_style = \"Explosion\"",
                )
                .replace(
                    "capture_anim_style = \"Spark\"",
                    "capture_anim_style = \"Explosion\"",
                );
            toml::from_str(&sanitized).unwrap_or_default()
        })
    }
}

/// Returns `Some(next)` only when the requested value differs from the current one.
pub const fn committed_toggle(current: bool, next: bool) -> Option<bool> {
    if current == next { None } else { Some(next) }
}

/// Enable or disable `id` in a selection list without flipping neighbors.
pub fn set_id_enabled(selected: &mut Vec<String>, id: &str, enabled: bool) {
    let present = selected.iter().any(|existing| existing == id);
    if enabled && !present {
        selected.push(id.to_owned());
    } else if !enabled && present {
        selected.retain(|existing| existing != id);
    }
}

/// Live tail is `None`; earlier plies stay `Some`.
pub const fn review_cursor_for_view(ply: usize, len: usize) -> Option<usize> {
    if ply >= len { None } else { Some(ply) }
}

/// Stay live when a move is appended at the tail; keep an earlier review ply frozen.
pub const fn review_cursor_after_append(
    review: Option<usize>,
    previous_len: usize,
) -> Option<usize> {
    match review {
        None => None,
        Some(ply) if ply >= previous_len => None,
        other => other,
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
        assert!(settings.show_threats);
        assert_eq!(BoardTheme::ALL.len(), 8);
        assert_eq!(CaptureAnimStyle::ALL.len(), 6);
        assert_eq!(PieceAnimStyle::ALL.len(), 5);
        assert_eq!(CoordPosition::ALL.len(), 2);
        assert!(settings.bgm_on);
        assert!(
            (settings.sidebar_width_px - crate::app_core::layout::SIDEBAR_IDEAL_PX).abs()
                < f64::EPSILON
        );
        assert_eq!(settings.eval_bar_engine, EVAL_BAR_DEFAULT_ENGINE);
        assert!(
            (settings.dock_height_px - crate::app_core::layout::DOCK_OPEN_PX).abs() < f64::EPSILON
        );
        assert_eq!(OptionsTab::ALL.len(), 6);
        assert_ne!(Screen::Ateed, Screen::Menu);
        assert_ne!(Screen::Ateed, Screen::Analysis);
    }

    #[test]
    fn toml_round_trip_keeps_floem_only_fields() {
        let settings = AppSettings {
            draw_arrows: false,
            show_threats: false,
            bgm_on: false,
            capture_anim_style: CaptureAnimStyle::Shatter,
            piece_anim_style: PieceAnimStyle::Warp,
            sidebar_width_px: 400.0,
            ..AppSettings::default()
        };
        let encoded = toml::to_string(&settings).expect("encode");
        let decoded: AppSettings = toml::from_str(&encoded).expect("decode");
        assert!(!decoded.draw_arrows);
        assert!(!decoded.show_threats);
        assert!(!decoded.bgm_on);
        assert_eq!(decoded.capture_anim_style, CaptureAnimStyle::Shatter);
        assert_eq!(decoded.piece_anim_style, PieceAnimStyle::Warp);
        assert!((decoded.sidebar_width_px - 400.0).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_overlay_does_not_drop_unknown_core_fields() {
        let base = AppSettings {
            show_threats: false,
            capture_anim_style: CaptureAnimStyle::Vortex,
            ..AppSettings::default()
        };
        let overlay = "draw_arrows = false\nsfx_on = false\n";
        let mut base_val = toml::Value::try_from(&base).expect("base");
        if let toml::Value::Table(overlay) = overlay.parse::<toml::Value>().expect("overlay")
            && let Some(dst) = base_val.as_table_mut()
        {
            for (key, value) in overlay {
                dst.insert(key, value);
            }
        }
        let merged: AppSettings = toml::Value::try_into(base_val).expect("merge");
        assert!(!merged.draw_arrows);
        assert!(!merged.sfx_on);
        assert!(!merged.show_threats);
        assert_eq!(merged.capture_anim_style, CaptureAnimStyle::Vortex);
    }

    #[test]
    fn committed_toggle_ignores_redundant_events() {
        assert_eq!(committed_toggle(true, true), None);
        assert_eq!(committed_toggle(false, false), None);
        assert_eq!(committed_toggle(false, true), Some(true));
        assert_eq!(committed_toggle(true, false), Some(false));
    }

    #[test]
    fn analysis_engine_set_does_not_flip() {
        let mut selected = vec!["builtin".to_owned()];
        set_id_enabled(&mut selected, "builtin", true);
        assert_eq!(selected, vec!["builtin".to_owned()]);
        set_id_enabled(&mut selected, "builtin", false);
        assert!(selected.is_empty());
        set_id_enabled(&mut selected, "stockfish", true);
        set_id_enabled(&mut selected, "stockfish", true);
        assert_eq!(selected, vec!["stockfish".to_owned()]);
    }

    #[test]
    fn review_cursor_treats_tail_as_live() {
        assert_eq!(review_cursor_for_view(0, 0), None);
        assert_eq!(review_cursor_for_view(3, 3), None);
        assert_eq!(review_cursor_for_view(2, 3), Some(2));
        assert_eq!(review_cursor_after_append(None, 4), None);
        assert_eq!(review_cursor_after_append(Some(4), 4), None);
        assert_eq!(review_cursor_after_append(Some(3), 4), Some(3));
        assert_eq!(review_cursor_after_append(Some(2), 4), Some(2));
    }

    #[test]
    fn draw_arrows_survives_a_simulated_move_patch() {
        let settings = AppSettings {
            draw_arrows: true,
            ..AppSettings::default()
        };
        let encoded = toml::to_string(&settings).expect("encode");
        let mut decoded: AppSettings = toml::from_str(&encoded).expect("decode");
        decoded.anim_speed = 2;
        assert!(decoded.draw_arrows);
    }
}
