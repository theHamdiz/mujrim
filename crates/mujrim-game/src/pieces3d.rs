use crate::state::ChessGame;
use bevy::prelude::*;

#[derive(Component)]
pub struct Piece3d {
    pub piece: types::Piece,
    pub color: types::Color,
    pub square: types::Square,
}

#[derive(Component)]
pub struct Piece3dAnimation {
    pub start: Vec3,
    pub end: Vec3,
    pub elapsed: f32,
    pub duration: f32,
}

/// Reusable GPU assets shared by every procedural chess piece.
#[derive(Resource)]
pub struct Piece3dAssets {
    white: Handle<StandardMaterial>,
    black: Handle<StandardMaterial>,
    base: Handle<Mesh>,
    stem: Handle<Mesh>,
    pawn_head: Handle<Mesh>,
    royal_head: Handle<Mesh>,
    bishop_head: Handle<Mesh>,
    rook_head: Handle<Mesh>,
    knight_head: Handle<Mesh>,
    crown: Handle<Mesh>,
    cross_vertical: Handle<Mesh>,
    cross_horizontal: Handle<Mesh>,
}

impl FromWorld for Piece3dAssets {
    fn from_world(world: &mut World) -> Self {
        let (
            base,
            stem,
            pawn_head,
            royal_head,
            bishop_head,
            rook_head,
            knight_head,
            crown,
            cross_vertical,
            cross_horizontal,
        ) = {
            let mut meshes = world.resource_mut::<Assets<Mesh>>();
            (
                meshes.add(Cylinder {
                    half_height: 0.06,
                    radius: 0.25,
                }),
                meshes.add(Cylinder {
                    half_height: 0.18,
                    radius: 0.12,
                }),
                meshes.add(Sphere { radius: 0.15 }),
                meshes.add(Sphere { radius: 0.16 }),
                meshes.add(Cone {
                    radius: 0.18,
                    height: 0.34,
                }),
                meshes.add(Cuboid::new(0.38, 0.18, 0.38)),
                meshes.add(Cuboid::new(0.22, 0.34, 0.18)),
                meshes.add(Cone {
                    radius: 0.14,
                    height: 0.22,
                }),
                meshes.add(Cuboid::new(0.07, 0.30, 0.07)),
                meshes.add(Cuboid::new(0.24, 0.07, 0.07)),
            )
        };
        let (white, black) = {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            (
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.92, 0.85),
                    perceptual_roughness: 0.42,
                    metallic: 0.08,
                    ..default()
                }),
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.15, 0.12, 0.10),
                    perceptual_roughness: 0.38,
                    metallic: 0.12,
                    ..default()
                }),
            )
        };
        Self {
            white,
            black,
            base,
            stem,
            pawn_head,
            royal_head,
            bishop_head,
            rook_head,
            knight_head,
            crown,
            cross_vertical,
            cross_horizontal,
        }
    }
}

const SQ_SIZE: f32 = 1.0;
const BOARD_SURFACE_Y: f32 = 0.1;

pub fn square_to_world_3d(file: u8, rank: u8, flipped: bool) -> Vec3 {
    let (f, r) = if flipped {
        (7 - file, 7 - rank)
    } else {
        (file, rank)
    };
    Vec3::new(
        (f as f32 - 3.5) * SQ_SIZE,
        BOARD_SURFACE_Y,
        (r as f32 - 3.5) * SQ_SIZE,
    )
}

pub fn spawn_pieces_3d(mut commands: Commands, assets: Res<Piece3dAssets>, game: Res<ChessGame>) {
    for sq in types::Square::ALL {
        if let Some((piece, color)) = game.board.piece_on(sq) {
            spawn_piece(
                &mut commands,
                &assets,
                piece,
                color,
                sq,
                square_to_world_3d(sq.file(), sq.rank(), game.flipped),
                None,
            );
        }
    }
}

pub fn sync_piece_positions_3d(
    mut commands: Commands,
    assets: Res<Piece3dAssets>,
    game: Res<ChessGame>,
    pieces: Query<(Entity, &Piece3d, &Transform)>,
) {
    if !game.is_changed() {
        return;
    }

    let mut to_despawn = Vec::new();
    let mut matched = [false; 64];
    for (entity, piece3d, transform) in pieces.iter() {
        let sq = piece3d.square;
        if let Some((bp, bc)) = game.board.piece_on(sq)
            && bp == piece3d.piece
            && bc == piece3d.color
        {
            let target = square_to_world_3d(sq.file(), sq.rank(), game.flipped);
            if transform.translation.distance(target) > 0.1 {
                commands.entity(entity).insert(Piece3dAnimation {
                    start: transform.translation,
                    end: target,
                    elapsed: 0.0,
                    duration: 0.2,
                });
            }
            matched[sq.index()] = true;
            continue;
        }
        to_despawn.push(entity);
    }
    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    for sq in types::Square::ALL {
        if matched[sq.index()] {
            continue;
        }
        if let Some((piece, color)) = game.board.piece_on(sq) {
            let final_pos = square_to_world_3d(sq.file(), sq.rank(), game.flipped);
            let start_pos = game
                .last_move
                .filter(|mv| mv.to == sq)
                .map(|mv| square_to_world_3d(mv.from.file(), mv.from.rank(), game.flipped));
            let animation = start_pos
                .filter(|start| start.distance(final_pos) > 0.1)
                .map(|start| Piece3dAnimation {
                    start,
                    end: final_pos,
                    elapsed: 0.0,
                    duration: 0.2,
                });
            spawn_piece(
                &mut commands,
                &assets,
                piece,
                color,
                sq,
                start_pos.unwrap_or(final_pos),
                animation,
            );
        }
    }
}

fn spawn_piece(
    commands: &mut Commands,
    assets: &Piece3dAssets,
    piece: types::Piece,
    color: types::Color,
    square: types::Square,
    position: Vec3,
    animation: Option<Piece3dAnimation>,
) {
    let material = match color {
        types::Color::White => assets.white.clone(),
        types::Color::Black => assets.black.clone(),
    };
    let mut entity = commands.spawn((
        Transform::from_translation(position),
        Visibility::Inherited,
        Piece3d {
            piece,
            color,
            square,
        },
    ));
    entity.with_children(|parent| {
        parent.spawn((
            Mesh3d(assets.base.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, 0.06, 0.0),
        ));
        parent.spawn((
            Mesh3d(assets.stem.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, stem_height(piece), 0.0).with_scale(stem_scale(piece)),
        ));
        match piece {
            types::Piece::Pawn => {
                parent.spawn((
                    Mesh3d(assets.pawn_head.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 0.48, 0.0),
                ));
            }
            types::Piece::Knight => {
                parent.spawn((
                    Mesh3d(assets.knight_head.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 0.56, -0.04)
                        .with_rotation(Quat::from_rotation_z(-0.42)),
                ));
            }
            types::Piece::Bishop => {
                parent.spawn((
                    Mesh3d(assets.bishop_head.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 0.61, 0.0),
                ));
            }
            types::Piece::Rook => {
                parent.spawn((
                    Mesh3d(assets.rook_head.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 0.52, 0.0),
                ));
            }
            types::Piece::Queen => {
                parent.spawn((
                    Mesh3d(assets.royal_head.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, 0.69, 0.0),
                ));
                parent.spawn((
                    Mesh3d(assets.crown.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 0.88, 0.0),
                ));
            }
            types::Piece::King => {
                parent.spawn((
                    Mesh3d(assets.royal_head.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, 0.72, 0.0),
                ));
                parent.spawn((
                    Mesh3d(assets.cross_vertical.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, 0.96, 0.0),
                ));
                parent.spawn((
                    Mesh3d(assets.cross_horizontal.clone()),
                    MeshMaterial3d(material),
                    Transform::from_xyz(0.0, 1.00, 0.0),
                ));
            }
        }
    });
    if let Some(animation) = animation {
        entity.insert(animation);
    }
}

fn stem_height(piece: types::Piece) -> f32 {
    match piece {
        types::Piece::Pawn => 0.28,
        types::Piece::Knight | types::Piece::Rook => 0.30,
        types::Piece::Bishop => 0.36,
        types::Piece::Queen => 0.43,
        types::Piece::King => 0.46,
    }
}

fn stem_scale(piece: types::Piece) -> Vec3 {
    match piece {
        types::Piece::Pawn => Vec3::new(0.78, 0.72, 0.78),
        types::Piece::Knight => Vec3::new(0.92, 0.88, 0.92),
        types::Piece::Bishop => Vec3::new(0.86, 1.12, 0.86),
        types::Piece::Rook => Vec3::new(1.20, 0.82, 1.20),
        types::Piece::Queen => Vec3::new(1.02, 1.42, 1.02),
        types::Piece::King => Vec3::new(1.08, 1.55, 1.08),
    }
}

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
        transform.translation.y += 0.5 * (1.0 - (2.0 * t - 1.0).powi(2));
        if t >= 1.0 {
            transform.translation = anim.end;
            commands.entity(entity).remove::<Piece3dAnimation>();
        }
    }
}

pub fn despawn_pieces_3d(mut commands: Commands, entities: Query<Entity, With<Piece3d>>) {
    for entity in entities.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flipped_coordinates_are_exactly_reversed() {
        for file in 0..8 {
            for rank in 0..8 {
                assert_eq!(
                    square_to_world_3d(file, rank, true),
                    square_to_world_3d(7 - file, 7 - rank, false)
                );
            }
        }
    }

    #[test]
    fn royal_pieces_have_taller_stems_than_minor_pieces() {
        assert!(stem_height(types::Piece::Queen) > stem_height(types::Piece::Bishop));
        assert!(stem_height(types::Piece::King) > stem_height(types::Piece::Queen));
    }

    #[test]
    fn piece_asset_cache_is_independent_of_piece_count() {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Piece3dAssets>();

        assert_eq!(world.resource::<Assets<Mesh>>().len(), 10);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 2);
    }
}
