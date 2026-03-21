use bevy::prelude::*;
use crate::state::ChessGame;

/// Marker for a 3D piece entity.
#[derive(Component)]
pub struct Piece3d {
    pub piece: types::Piece,
    pub color: types::Color,
    pub square: types::Square,
}

/// Active animation for a 3D piece.
#[derive(Component)]
pub struct Piece3dAnimation {
    pub start: Vec3,
    pub end: Vec3,
    pub elapsed: f32,
    pub duration: f32,
}

const SQ_SIZE: f32 = 1.0;

/// Convert a square to 3D world position.
pub fn square_to_world_3d(file: u8, rank: u8, flipped: bool) -> Vec3 {
    let (f, r) = if flipped {
        (7 - file, 7 - rank)
    } else {
        (file, rank)
    };
    Vec3::new(
        (f as f32 - 3.5) * SQ_SIZE,
        0.1,
        (r as f32 - 3.5) * SQ_SIZE,
    )
}

/// Spawn 3D piece meshes (placeholder cylinders with spheres on top).
pub fn spawn_pieces_3d(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    game: Res<ChessGame>,
) {
    let white_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.92, 0.85),
        ..default()
    });
    let black_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.12, 0.10),
        ..default()
    });

    for sq in types::Square::ALL {
        if let Some((piece, color)) = game.board.piece_on(sq) {
            let mat = match color {
                types::Color::White => white_mat.clone(),
                types::Color::Black => black_mat.clone(),
            };
            let height = piece_height(piece);
            let radius = piece_radius(piece);

            let pos = square_to_world_3d(sq.file(), sq.rank(), game.flipped);
            let piece_pos = Vec3::new(pos.x, height / 2.0 + 0.05, pos.z);

            let cylinder = meshes.add(Cylinder {
                half_height: height / 2.0,
                radius,
            });
            let sphere = meshes.add(Sphere { radius: radius * 0.8 });

            commands
                .spawn((
                    Mesh3d(cylinder),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_translation(piece_pos),
                    Piece3d { piece, color, square: sq },
                ))
                .with_children(|parent| {
                    parent.spawn((
                        Mesh3d(sphere),
                        MeshMaterial3d(mat),
                        Transform::from_translation(Vec3::new(0.0, height / 2.0, 0.0)),
                    ));
                });
        }
    }
}

/// Sync 3D piece positions with the board state.
pub fn sync_piece_positions_3d(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    game: Res<ChessGame>,
    pieces: Query<(Entity, &Piece3d, &Transform)>,
) {
    if !game.is_changed() {
        return;
    }

    let mut to_despawn = Vec::new();
    let mut matched = vec![false; 64];

    for (entity, piece3d, transform) in pieces.iter() {
        let sq = piece3d.square;
        if let Some((bp, bc)) = game.board.piece_on(sq) {
            if bp == piece3d.piece && bc == piece3d.color {
                let target = square_to_world_3d(sq.file(), sq.rank(), game.flipped);
                let target_3d = Vec3::new(target.x, transform.translation.y, target.z);
                if transform.translation.distance(target_3d) > 0.1 {
                    commands.entity(entity).insert(Piece3dAnimation {
                        start: transform.translation,
                        end: target_3d,
                        elapsed: 0.0,
                        duration: 0.2,
                    });
                }
                matched[sq.index()] = true;
                continue;
            }
        }
        to_despawn.push(entity);
    }

    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    // Spawn missing pieces
    let white_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.92, 0.85),
        ..default()
    });
    let black_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.12, 0.10),
        ..default()
    });

    for sq in types::Square::ALL {
        if matched[sq.index()] {
            continue;
        }
        if let Some((piece, color)) = game.board.piece_on(sq) {
            let mat = match color {
                types::Color::White => white_mat.clone(),
                types::Color::Black => black_mat.clone(),
            };
            let height = piece_height(piece);
            let radius = piece_radius(piece);

            let pos = square_to_world_3d(sq.file(), sq.rank(), game.flipped);
            let final_pos = Vec3::new(pos.x, height / 2.0 + 0.05, pos.z);

            let start_pos = if let Some(mv) = game.last_move {
                if mv.to == sq {
                    let from = square_to_world_3d(mv.from.file(), mv.from.rank(), game.flipped);
                    Vec3::new(from.x, height / 2.0 + 0.05, from.z)
                } else {
                    final_pos
                }
            } else {
                final_pos
            };

            let cylinder = meshes.add(Cylinder {
                half_height: height / 2.0,
                radius,
            });
            let sphere = meshes.add(Sphere { radius: radius * 0.8 });

            let mut entity_cmds = commands.spawn((
                Mesh3d(cylinder),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(start_pos),
                Piece3d { piece, color, square: sq },
            ));

            entity_cmds.with_children(|parent| {
                parent.spawn((
                    Mesh3d(sphere),
                    MeshMaterial3d(mat),
                    Transform::from_translation(Vec3::new(0.0, height / 2.0, 0.0)),
                ));
            });

            if start_pos.distance(final_pos) > 0.1 {
                entity_cmds.insert(Piece3dAnimation {
                    start: start_pos,
                    end: final_pos,
                    elapsed: 0.0,
                    duration: 0.2,
                });
            }
        }
    }
}

/// Animate 3D piece movement.
pub fn animate_piece_movement_3d(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut Transform, &mut Piece3dAnimation)>,
) {
    for (entity, mut transform, mut anim) in pieces.iter_mut() {
        anim.elapsed += time.delta_secs();
        let t = (anim.elapsed / anim.duration).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        transform.translation = anim.start.lerp(anim.end, eased);

        // Slight hop in 3D
        let hop_height = 0.5 * (1.0 - (2.0 * t - 1.0).powi(2));
        transform.translation.y += hop_height;

        if t >= 1.0 {
            transform.translation = anim.end;
            commands.entity(entity).remove::<Piece3dAnimation>();
        }
    }
}

/// Despawn all 3D piece entities.
pub fn despawn_pieces_3d(mut commands: Commands, entities: Query<Entity, With<Piece3d>>) {
    for entity in entities.iter() {
        commands.entity(entity).despawn();
    }
}

fn piece_height(piece: types::Piece) -> f32 {
    match piece {
        types::Piece::Pawn => 0.4,
        types::Piece::Knight => 0.6,
        types::Piece::Bishop => 0.7,
        types::Piece::Rook => 0.5,
        types::Piece::Queen => 0.85,
        types::Piece::King => 0.95,
    }
}

fn piece_radius(piece: types::Piece) -> f32 {
    match piece {
        types::Piece::Pawn => 0.15,
        types::Piece::Knight => 0.18,
        types::Piece::Bishop => 0.17,
        types::Piece::Rook => 0.20,
        types::Piece::Queen => 0.22,
        types::Piece::King => 0.22,
    }
}
