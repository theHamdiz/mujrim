use bevy::prelude::*;

use crate::state::{AppState, EngineConfig, RenderDimension, TurnState};

/// Trait marker for all Mujrim game plugins.
pub trait ChessGamePlugin: Plugin {}

/// Sets up game state resources and Bevy states.
pub struct GameStatePlugin;

impl Plugin for GameStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .init_state::<TurnState>()
            .insert_resource(RenderDimension::TwoD)
            .insert_resource(EngineConfig::default());
    }
}

impl ChessGamePlugin for GameStatePlugin {}

/// 2D board rendering.
pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::board3d::Board3dAssets>()
            .insert_resource(crate::layout::BoardLayout::default())
            .add_systems(Startup, crate::board2d::setup_camera)
            .add_systems(
                OnEnter(AppState::Playing),
                crate::board2d::spawn_board.after(crate::game_logic::start_new_game),
            )
            .add_systems(Update, crate::layout::on_window_resize)
            .add_systems(
                Update,
                crate::board2d::resize_board.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::board2d::update_square_highlights.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::board2d::handle_square_click.run_if(in_state(AppState::Playing)),
            );
    }
}

impl ChessGamePlugin for BoardPlugin {}

/// 2D piece rendering.
pub struct PiecePlugin;

impl Plugin for PiecePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::pieces3d::Piece3dAssets>()
            .add_systems(
                OnEnter(AppState::Playing),
                crate::pieces2d::spawn_pieces.after(crate::game_logic::start_new_game),
            )
            .add_systems(
                Update,
                crate::pieces2d::sync_piece_positions.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::pieces2d::animate_piece_movement.run_if(in_state(AppState::Playing)),
            )
            // 3D piece systems (run when in 3D render mode)
            .add_systems(
                Update,
                crate::pieces3d::sync_piece_positions_3d.run_if(
                    in_state(AppState::Playing)
                        .and(|dim: Res<RenderDimension>| *dim == RenderDimension::ThreeD),
                ),
            )
            .add_systems(
                Update,
                crate::pieces3d::animate_piece_movement_3d.run_if(
                    in_state(AppState::Playing)
                        .and(|dim: Res<RenderDimension>| *dim == RenderDimension::ThreeD),
                ),
            );
    }
}

impl ChessGamePlugin for PiecePlugin {}

/// Audio — SFX and background music.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<crate::audio::SoundMessage>()
            .add_systems(Startup, crate::audio::load_audio_assets)
            .add_systems(Update, crate::audio::play_background_music)
            .add_systems(Update, crate::audio::play_sound_effects);
    }
}

impl ChessGamePlugin for AudioPlugin {}

/// AI engine integration.
pub struct EnginePlugin;

impl Plugin for EnginePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(AppState::Playing),
            crate::engine::init_engine.after(crate::game_logic::start_new_game),
        )
        .add_systems(
            Update,
            crate::engine::start_engine_search.run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            crate::engine::poll_engine_result.run_if(in_state(AppState::Playing)),
        );
    }
}

impl ChessGamePlugin for EnginePlugin {}

/// Core game logic: move execution, game-over detection.
pub struct GameLogicPlugin;

impl Plugin for GameLogicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<crate::game_logic::MoveMessage>()
            .add_message::<crate::game_logic::UndoMessage>()
            .add_systems(
                OnEnter(AppState::Playing),
                crate::game_logic::start_new_game,
            )
            .add_systems(
                Update,
                crate::game_logic::execute_move.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::game_logic::handle_undo.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::game_logic::detect_game_over.run_if(in_state(AppState::Playing)),
            );
    }
}

impl ChessGamePlugin for GameLogicPlugin {}

/// HUD and menus.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Menu), crate::ui::spawn_menu)
            .add_systems(OnExit(AppState::Menu), crate::ui::despawn_menu)
            .add_systems(
                Update,
                crate::ui::menu_button_system.run_if(in_state(AppState::Menu)),
            )
            .add_systems(
                OnEnter(AppState::Playing),
                crate::ui::spawn_hud.after(crate::game_logic::start_new_game),
            )
            .add_systems(
                Update,
                crate::ui::update_hud.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::ui::hud_button_system.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::ui::hud_resign_system.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                crate::ui::update_depth_text.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                OnEnter(AppState::GameOver),
                crate::ui::spawn_game_over_overlay,
            )
            .add_systems(
                Update,
                crate::ui::game_over_button_system.run_if(in_state(AppState::GameOver)),
            );
    }
}

impl ChessGamePlugin for UiPlugin {}

/// Render mode switching (2D ↔ 3D).
pub struct RenderModePlugin;

impl Plugin for RenderModePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            crate::render_mode::toggle_render_mode.run_if(in_state(AppState::Playing)),
        );
    }
}

impl ChessGamePlugin for RenderModePlugin {}
