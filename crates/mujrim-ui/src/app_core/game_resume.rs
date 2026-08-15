//! Durable play-game checkpoint so a sudden restart can restore the current game.

use mujrim_study::durable;

use super::engine::{GameMode, PlayerConfig};
use super::settings::AppSettings;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ActiveGameCheckpoint {
    pub mode: String,
    pub white: String,
    pub black: String,
    pub initial_fen: String,
    pub moves: Vec<String>,
    pub flipped: bool,
    pub game_over: bool,
}

impl Default for ActiveGameCheckpoint {
    fn default() -> Self {
        Self {
            mode: GameMode::HumanVsEngine.encode().to_owned(),
            white: PlayerConfig::Human.encode(),
            black: PlayerConfig::BuiltIn { depth: 16 }.encode(),
            initial_fen: mujrim_study::opening::START_FEN.to_owned(),
            moves: Vec::new(),
            flipped: false,
            game_over: false,
        }
    }
}

impl ActiveGameCheckpoint {
    pub fn path() -> std::path::PathBuf {
        let mut path = AppSettings::config_path();
        path.set_file_name("active-game.toml");
        path
    }

    pub fn load() -> Option<Self> {
        let contents = durable::read_text(&Self::path())?;
        toml::from_str(&contents).ok()
    }

    pub fn save(&self) {
        if let Ok(encoded) = toml::to_string_pretty(self) {
            let _ = durable::atomic_write_text(&Self::path(), &encoded);
        }
    }

    pub fn clear() {
        durable::remove_file(&Self::path());
    }

    pub fn capture(
        mode: GameMode,
        white: &PlayerConfig,
        black: &PlayerConfig,
        initial_fen: &str,
        moves: &[String],
        flipped: bool,
        game_over: bool,
    ) -> Self {
        Self {
            mode: mode.encode().to_owned(),
            white: white.encode(),
            black: black.encode(),
            initial_fen: if initial_fen.is_empty() {
                mujrim_study::opening::START_FEN.to_owned()
            } else {
                initial_fen.to_owned()
            },
            moves: moves.to_vec(),
            flipped,
            game_over,
        }
    }

    pub fn parsed_mode(&self) -> GameMode {
        GameMode::decode(&self.mode)
    }

    pub fn parsed_white(&self) -> PlayerConfig {
        PlayerConfig::decode(&self.white)
    }

    pub fn parsed_black(&self) -> PlayerConfig {
        PlayerConfig::decode(&self.black)
    }

    pub fn is_resumable(&self) -> bool {
        !self.game_over
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_round_trips_players_and_moves() {
        let checkpoint = ActiveGameCheckpoint::capture(
            GameMode::EngineVsEngine,
            &PlayerConfig::BuiltIn { depth: 12 },
            &PlayerConfig::External {
                path: "/tmp/sf".to_owned(),
                protocol: crate::app_core::uci_process::ExternalEngineProtocol::Uci,
            },
            mujrim_study::opening::START_FEN,
            &["e2e4".to_owned(), "e7e5".to_owned()],
            true,
            false,
        );
        assert_eq!(checkpoint.parsed_mode(), GameMode::EngineVsEngine);
        assert_eq!(
            checkpoint.parsed_white(),
            PlayerConfig::BuiltIn { depth: 12 }
        );
        assert_eq!(
            checkpoint.parsed_black(),
            PlayerConfig::External {
                path: "/tmp/sf".to_owned(),
                protocol: crate::app_core::uci_process::ExternalEngineProtocol::Uci,
            }
        );
        assert_eq!(checkpoint.moves, ["e2e4", "e7e5"]);
        assert!(checkpoint.is_resumable());
        assert!(checkpoint.flipped);
    }
}
