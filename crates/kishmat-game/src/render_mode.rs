use bevy::prelude::*;
use crate::board2d::BoardEntity;
use crate::board3d::Board3dEntity;
use crate::layout::BoardLayout;
use crate::pieces2d::PieceSprite;
use crate::pieces3d::Piece3d;
use crate::state::RenderDimension;

/// Toggle between 2D and 3D rendering mode with the Tab key.
///
/// Despawns all entities for the old mode and spawns the base entities for the
/// new mode. Pieces are reconciled automatically by the sync systems on the
/// next frame (they detect the `ChessGame` resource as changed because we
/// force a change-tick by calling `game.set_changed()` via `DerefMut`).
pub fn toggle_render_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut render_dim: ResMut<RenderDimension>,
    mut commands: Commands,
    layout: Res<BoardLayout>,
    mut game: Option<ResMut<crate::state::ChessGame>>,
    // 2D entities
    board2d: Query<Entity, With<BoardEntity>>,
    pieces2d: Query<Entity, With<PieceSprite>>,
    camera2d: Query<Entity, With<Camera2d>>,
    // 3D entities
    board3d: Query<Entity, With<Board3dEntity>>,
    pieces3d: Query<Entity, With<Piece3d>>,
    // For spawning 3D
    meshes: ResMut<Assets<Mesh>>,
    materials: ResMut<Assets<StandardMaterial>>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }

    match *render_dim {
        RenderDimension::TwoD => {
            // Switch to 3D: despawn 2D, spawn 3D board
            for entity in board2d.iter().chain(pieces2d.iter()).chain(camera2d.iter()) {
                commands.entity(entity).despawn();
            }
            crate::board3d::spawn_board_3d(commands.reborrow(), meshes, materials);
            *render_dim = RenderDimension::ThreeD;
            // Touch game resource so the 3D sync system picks up pieces
            if let Some(ref mut g) = game {
                g.deselect();
            }
        }
        RenderDimension::ThreeD => {
            // Switch to 2D: despawn 3D, respawn 2D board + camera
            for entity in board3d.iter().chain(pieces3d.iter()) {
                commands.entity(entity).despawn();
            }
            // Spawn 2D camera
            commands.spawn(Camera2d);

            // Spawn 2D board squares + overlays inline
            for rank in 0..8u8 {
                for file in 0..8u8 {
                    let is_light = (file + rank) % 2 == 0;
                    let color = if is_light {
                        Color::srgb(0.71, 0.53, 0.39)
                    } else {
                        Color::srgb(0.93, 0.84, 0.71)
                    };
                    let flipped = game.as_ref().map_or(false, |g| g.flipped);
                    let pos = layout.square_to_world(file, rank, flipped);
                    commands.spawn((
                        Sprite {
                            color,
                            custom_size: Some(Vec2::splat(layout.square_size)),
                            ..default()
                        },
                        Transform::from_translation(pos),
                        crate::board2d::SquareEntity { file, rank },
                        BoardEntity,
                    ));
                }
            }

            // Pre-spawn highlight pool
            for slot in 0..24usize {
                commands.spawn((
                    Sprite {
                        color: Color::NONE,
                        custom_size: Some(Vec2::splat(layout.square_size)),
                        ..default()
                    },
                    Transform::from_translation(Vec3::new(0.0, 0.0, -10.0)),
                    Visibility::Hidden,
                    crate::board2d::HighlightOverlay { slot },
                    BoardEntity,
                ));
            }

            *render_dim = RenderDimension::TwoD;
            // Touch game resource so the 2D piece sync system spawns pieces
            if let Some(ref mut g) = game {
                g.deselect();
            }
        }
    }
}
