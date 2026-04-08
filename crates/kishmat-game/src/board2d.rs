use crate::layout::BoardLayout;
use crate::state::ChessGame;
use bevy::prelude::*;

const LIGHT_SQUARE: Color = Color::srgb(0.93, 0.84, 0.71);
const DARK_SQUARE: Color = Color::srgb(0.71, 0.53, 0.39);
const SELECTED_COLOR: Color = Color::srgba(0.3, 0.7, 0.3, 0.6);
const LEGAL_MOVE_COLOR: Color = Color::srgba(0.2, 0.2, 0.2, 0.3);
const LAST_MOVE_COLOR: Color = Color::srgba(0.8, 0.8, 0.2, 0.35);
const CHECK_COLOR: Color = Color::srgba(0.9, 0.1, 0.1, 0.5);

/// Marker for the entire board entity.
#[derive(Component)]
pub struct BoardEntity;

/// Marker for an individual square sprite.
#[derive(Component)]
pub struct SquareEntity {
    pub file: u8,
    pub rank: u8,
}

/// Index-based overlay from a pre-spawned pool.
#[derive(Component)]
pub struct HighlightOverlay {
    pub slot: usize,
}

const MAX_HIGHLIGHTS: usize = 24;

/// Coordinate label markers.
#[derive(Component)]
pub struct CoordLabel;

/// Spawn the default 2D camera at startup.
pub fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Spawn the 8x8 board grid + highlight overlay pool.
pub fn spawn_board(mut commands: Commands, layout: Res<BoardLayout>) {
    for rank in 0..8u8 {
        for file in 0..8u8 {
            let is_light = (file + rank) % 2 == 0;
            let color = if is_light { DARK_SQUARE } else { LIGHT_SQUARE };
            let pos = layout.square_to_world(file, rank, false);

            commands.spawn((
                Sprite {
                    color,
                    custom_size: Some(Vec2::splat(layout.square_size)),
                    ..default()
                },
                Transform::from_translation(pos),
                SquareEntity { file, rank },
                BoardEntity,
            ));
        }
    }

    // Pre-spawn highlight overlay pool
    for slot in 0..MAX_HIGHLIGHTS {
        commands.spawn((
            Sprite {
                color: Color::NONE,
                custom_size: Some(Vec2::splat(layout.square_size)),
                ..default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, -10.0)),
            Visibility::Hidden,
            HighlightOverlay { slot },
            BoardEntity,
        ));
    }

    // Coordinate labels
    let font_size = 14.0;
    for file in 0..8u8 {
        let ch = (b'a' + file) as char;
        let pos = Vec3::new(
            layout.board_origin.x + (file as f32 + 0.5) * layout.square_size,
            layout.board_origin.y - 18.0,
            5.0,
        );
        commands.spawn((
            Text2d::new(ch.to_string()),
            TextFont {
                font_size,
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Transform::from_translation(pos),
            CoordLabel,
            BoardEntity,
        ));
    }
    for rank in 0..8u8 {
        let ch = (b'1' + rank) as char;
        let pos = Vec3::new(
            layout.board_origin.x - 18.0,
            layout.board_origin.y + (rank as f32 + 0.5) * layout.square_size,
            5.0,
        );
        commands.spawn((
            Text2d::new(ch.to_string()),
            TextFont {
                font_size,
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Transform::from_translation(pos),
            CoordLabel,
            BoardEntity,
        ));
    }
}

/// Reposition all board entities when layout changes (window resize / flip).
pub fn resize_board(
    layout: Res<BoardLayout>,
    game: Option<Res<ChessGame>>,
    mut squares: Query<(&SquareEntity, &mut Transform, &mut Sprite), Without<HighlightOverlay>>,
    mut coords: Query<
        (&CoordLabel, &mut Transform),
        (Without<SquareEntity>, Without<HighlightOverlay>),
    >,
) {
    if !layout.is_changed() {
        return;
    }

    let flipped = game.map_or(false, |g| g.flipped);

    for (sq, mut transform, mut sprite) in squares.iter_mut() {
        transform.translation = layout.square_to_world(sq.file, sq.rank, flipped);
        sprite.custom_size = Some(Vec2::splat(layout.square_size));
    }

    // Update coord positions
    let mut file_idx = 0u8;
    let mut rank_idx = 0u8;
    for (_coord, mut transform) in coords.iter_mut() {
        // Coordinate labels: first 8 are files (along bottom), next 8 are ranks (along left)
        if file_idx < 8 {
            transform.translation = Vec3::new(
                layout.board_origin.x + (file_idx as f32 + 0.5) * layout.square_size,
                layout.board_origin.y - 18.0,
                5.0,
            );
            file_idx += 1;
        } else {
            transform.translation = Vec3::new(
                layout.board_origin.x - 18.0,
                layout.board_origin.y + (rank_idx as f32 + 0.5) * layout.square_size,
                5.0,
            );
            rank_idx += 1;
        }
    }
}

/// Update highlight overlays based on game state.
pub fn update_square_highlights(
    game: Option<Res<ChessGame>>,
    layout: Res<BoardLayout>,
    mut overlays: Query<(
        &HighlightOverlay,
        &mut Sprite,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let Some(game) = game else {
        for (_, _, _, mut vis) in overlays.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    };

    let flipped = game.flipped;
    let sq_size = layout.square_size;

    let mut highlights: Vec<(Vec3, Color, f32, f32)> = Vec::with_capacity(MAX_HIGHLIGHTS);

    // Last move
    if let Some(mv) = game.last_move {
        for sq in [mv.from, mv.to] {
            let pos = layout.square_to_world(sq.file(), sq.rank(), flipped);
            highlights.push((pos, LAST_MOVE_COLOR, 1.0, sq_size));
        }
    }

    // Selected
    if let Some(sq) = game.selected_square {
        let pos = layout.square_to_world(sq.file(), sq.rank(), flipped);
        highlights.push((pos, SELECTED_COLOR, 1.5, sq_size));
    }

    // Legal move dots
    for mv in &game.legal_moves {
        let pos = layout.square_to_world(mv.to.file(), mv.to.rank(), flipped);
        highlights.push((pos, LEGAL_MOVE_COLOR, 1.5, sq_size * 0.35));
    }

    // Check
    if game.board.in_check() {
        let king_sq = game.board.king_square(game.board.side_to_move);
        let pos = layout.square_to_world(king_sq.file(), king_sq.rank(), flipped);
        highlights.push((pos, CHECK_COLOR, 1.2, sq_size));
    }

    for (overlay, mut sprite, mut transform, mut vis) in overlays.iter_mut() {
        let slot = overlay.slot;
        if slot < highlights.len() {
            let (pos, color, z, size) = highlights[slot];
            transform.translation = pos.with_z(z);
            sprite.color = color;
            sprite.custom_size = Some(Vec2::splat(size));
            *vis = Visibility::Visible;
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

/// Handle mouse clicks on the board.
pub fn handle_square_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform)>,
    layout: Res<BoardLayout>,
    mut game: Option<ResMut<ChessGame>>,
    mut move_messages: MessageWriter<crate::game_logic::MoveMessage>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(ref mut game) = game else { return };

    if !game.is_player_turn() {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };

    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        return;
    };

    let Some((file, rank)) = layout.world_to_square(world_pos, game.flipped) else {
        game.deselect();
        return;
    };

    let sq = types::Square::from_file_rank(file, rank);

    if game.selected_square.is_some() {
        if game.legal_moves.iter().any(|m| m.to == sq) {
            move_messages.write(crate::game_logic::MoveMessage { target: sq });
            return;
        }
    }

    if let Some((_piece, color)) = game.board.piece_on(sq) {
        if color == game.player_color {
            game.select_square(sq);
            return;
        }
    }

    game.deselect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::BoardLayout;

    #[test]
    fn test_square_world_conversion_roundtrip() {
        let layout = BoardLayout::from_window(1200.0, 800.0);
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let pos = layout.square_to_world(file, rank, false);
                let (f, r) = layout.world_to_square(pos.truncate(), false).unwrap();
                assert_eq!(
                    (f, r),
                    (file, rank),
                    "roundtrip failed for ({file}, {rank})"
                );
            }
        }
    }

    #[test]
    fn test_flipped_roundtrip() {
        let layout = BoardLayout::from_window(1200.0, 800.0);
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let pos = layout.square_to_world(file, rank, true);
                let (f, r) = layout.world_to_square(pos.truncate(), true).unwrap();
                assert_eq!((f, r), (file, rank));
            }
        }
    }

    #[test]
    fn test_out_of_bounds() {
        let layout = BoardLayout::from_window(1200.0, 800.0);
        assert!(
            layout
                .world_to_square(Vec2::new(-999.0, -999.0), false)
                .is_none()
        );
    }
}
