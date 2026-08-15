//! Home-hub helpers shared by the Floem UI without GUI types.

use super::engine::{GameMode, PlayerConfig};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoinFlipState {
    #[default]
    Idle,
    Flipping,
    Done {
        heads: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinFlipAssignment {
    pub white: PlayerConfig,
    pub black: PlayerConfig,
    pub flip_board: bool,
    pub status: &'static str,
}

pub fn engine_picker_visible(mode: GameMode, white: bool) -> bool {
    if white {
        matches!(mode, GameMode::EngineVsEngine)
    } else {
        matches!(mode, GameMode::HumanVsEngine | GameMode::EngineVsEngine)
    }
}

pub fn apply_coin_flip(
    heads: bool,
    white: PlayerConfig,
    black: PlayerConfig,
) -> CoinFlipAssignment {
    if heads {
        CoinFlipAssignment {
            white,
            black,
            flip_board: false,
            status: "Heads! You play White.",
        }
    } else {
        CoinFlipAssignment {
            white: black,
            black: white,
            flip_board: true,
            status: "Tails! You play Black.",
        }
    }
}

pub fn clamp_cfg_time(value: i32) -> i32 {
    value.clamp(1, 30)
}

pub fn clamp_cfg_depth(value: i32) -> i32 {
    value.clamp(1, 64)
}

pub fn clamp_cfg_threads(value: i32) -> i32 {
    value.clamp(1, 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickers_follow_game_mode() {
        assert!(!engine_picker_visible(GameMode::HumanVsHuman, true));
        assert!(!engine_picker_visible(GameMode::HumanVsHuman, false));
        assert!(!engine_picker_visible(GameMode::HumanVsEngine, true));
        assert!(engine_picker_visible(GameMode::HumanVsEngine, false));
        assert!(engine_picker_visible(GameMode::EngineVsEngine, true));
        assert!(engine_picker_visible(GameMode::EngineVsEngine, false));
    }

    #[test]
    fn coin_flip_swaps_sides_on_tails() {
        let human = PlayerConfig::Human;
        let engine = PlayerConfig::BuiltIn { depth: 16 };
        let heads = apply_coin_flip(true, human.clone(), engine.clone());
        assert!(matches!(heads.white, PlayerConfig::Human));
        assert!(!heads.flip_board);
        let tails = apply_coin_flip(false, human, engine);
        assert!(matches!(tails.white, PlayerConfig::BuiltIn { .. }));
        assert!(matches!(tails.black, PlayerConfig::Human));
        assert!(tails.flip_board);
    }

    #[test]
    fn engine_setting_clamps_match_home_ranges() {
        assert_eq!(clamp_cfg_time(0), 1);
        assert_eq!(clamp_cfg_time(99), 30);
        assert_eq!(clamp_cfg_depth(0), 1);
        assert_eq!(clamp_cfg_depth(80), 64);
        assert_eq!(clamp_cfg_threads(0), 1);
        assert_eq!(clamp_cfg_threads(64), 32);
    }
}
