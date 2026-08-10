pub mod audio;
pub mod board2d;
pub mod board3d;
pub mod engine;
pub mod game_logic;
pub mod layout;
pub mod pieces2d;
pub mod pieces3d;
pub mod plugins;
pub mod render_mode;
pub mod state;
pub mod ui;

use bevy::prelude::*;

/// Master plugin that bundles every sub-plugin for the Mujrim chess game.
pub struct MujrimGamePlugin;

impl Plugin for MujrimGamePlugin {
    fn build(&self, app: &mut App) {
        pieces2d::register_embedded_piece_assets(app);
        app.add_plugins((
            plugins::GameStatePlugin,
            plugins::BoardPlugin,
            plugins::PiecePlugin,
            plugins::AudioPlugin,
            plugins::EnginePlugin,
            plugins::GameLogicPlugin,
            plugins::UiPlugin,
            plugins::RenderModePlugin,
        ));
    }
}
