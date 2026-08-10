use crate::layout::BoardLayout;
use crate::state::ChessGame;
use bevy::prelude::*;
use std::path::PathBuf;

const EMBEDDED_PIECE_ROOT: &str = "pieces/default";
const EMBEDDED_BGM: &[u8] = include_bytes!("../assets/music/Caves.mp3");

const PIECE_IMAGES: [(&str, &[u8]); 12] = [
    ("wK.png", include_bytes!("../assets/pieces/wK.png")),
    ("wQ.png", include_bytes!("../assets/pieces/wQ.png")),
    ("wR.png", include_bytes!("../assets/pieces/wR.png")),
    ("wB.png", include_bytes!("../assets/pieces/wB.png")),
    ("wN.png", include_bytes!("../assets/pieces/wN.png")),
    ("wP.png", include_bytes!("../assets/pieces/wP.png")),
    ("bK.png", include_bytes!("../assets/pieces/bK.png")),
    ("bQ.png", include_bytes!("../assets/pieces/bQ.png")),
    ("bR.png", include_bytes!("../assets/pieces/bR.png")),
    ("bB.png", include_bytes!("../assets/pieces/bB.png")),
    ("bN.png", include_bytes!("../assets/pieces/bN.png")),
    ("bP.png", include_bytes!("../assets/pieces/bP.png")),
];

/// Register the complete default piece set in Bevy's in-memory asset source.
/// The executable therefore renders pieces without a working-directory-dependent
/// `assets` folder next to it.
pub fn register_embedded_piece_assets(app: &mut App) {
    let registry = app
        .world()
        .resource::<bevy::asset::io::embedded::EmbeddedAssetRegistry>();
    for (file_name, bytes) in PIECE_IMAGES {
        let asset_path = PathBuf::from(EMBEDDED_PIECE_ROOT).join(file_name);
        registry.insert_asset(PathBuf::new(), &asset_path, bytes);
    }
    registry.insert_asset(
        PathBuf::new(),
        &PathBuf::from("music/Caves.mp3"),
        EMBEDDED_BGM,
    );
}

/// Marker for a piece sprite entity.
#[derive(Component)]
pub struct PieceSprite {
    pub piece: types::Piece,
    pub color: types::Color,
    pub square: types::Square,
}

/// Active animation: smoothly slide a piece from one position to another.
#[derive(Component)]
pub struct PieceAnimation {
    pub start: Vec3,
    pub end: Vec3,
    pub elapsed: f32,
    pub duration: f32,
}

/// Spawn piece sprites for the starting position.
pub fn spawn_pieces(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    game: Res<ChessGame>,
    layout: Res<BoardLayout>,
) {
    let piece_sz = layout.square_size * 0.82;
    for sq in types::Square::ALL {
        if let Some((piece, color)) = game.board.piece_on(sq) {
            let texture = load_piece_texture(&asset_server, piece, color);
            let pos = layout.square_to_world(sq.file(), sq.rank(), game.flipped);
            commands.spawn((
                Sprite {
                    image: texture,
                    custom_size: Some(Vec2::splat(piece_sz)),
                    ..default()
                },
                Transform::from_translation(pos.with_z(2.0)),
                PieceSprite {
                    piece,
                    color,
                    square: sq,
                },
            ));
        }
    }
}

/// Reconcile piece sprites with board state after mutations.
pub fn sync_piece_positions(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    game: Res<ChessGame>,
    layout: Res<BoardLayout>,
    pieces: Query<(Entity, &PieceSprite, &Transform)>,
) {
    if !game.is_changed() {
        return;
    }

    let piece_sz = layout.square_size * 0.82;
    let mut to_despawn = Vec::new();
    let mut matched = [false; 64];

    for (entity, piece_sprite, transform) in pieces.iter() {
        let sq = piece_sprite.square;
        if let Some((bp, bc)) = game.board.piece_on(sq)
            && bp == piece_sprite.piece
            && bc == piece_sprite.color
        {
            let target = layout
                .square_to_world(sq.file(), sq.rank(), game.flipped)
                .with_z(2.0);
            if transform.translation.distance(target) > 1.0 {
                commands.entity(entity).insert(PieceAnimation {
                    start: transform.translation,
                    end: target,
                    elapsed: 0.0,
                    duration: 0.15,
                });
            }
            // Update sprite size in case of resize
            commands.entity(entity).insert(Sprite {
                image: load_piece_texture(&asset_server, bp, bc),
                custom_size: Some(Vec2::splat(piece_sz)),
                ..default()
            });
            matched[sq.index()] = true;
            continue;
        }
        to_despawn.push(entity);
    }

    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    // Spawn missing pieces
    for sq in types::Square::ALL {
        if matched[sq.index()] {
            continue;
        }
        if let Some((piece, color)) = game.board.piece_on(sq) {
            let texture = load_piece_texture(&asset_server, piece, color);
            let final_pos = layout
                .square_to_world(sq.file(), sq.rank(), game.flipped)
                .with_z(2.0);

            let start_pos = if let Some(mv) = game.last_move {
                if mv.to == sq {
                    layout
                        .square_to_world(mv.from.file(), mv.from.rank(), game.flipped)
                        .with_z(2.0)
                } else {
                    final_pos
                }
            } else {
                final_pos
            };

            let mut entity_cmds = commands.spawn((
                Sprite {
                    image: texture,
                    custom_size: Some(Vec2::splat(piece_sz)),
                    ..default()
                },
                Transform::from_translation(start_pos),
                PieceSprite {
                    piece,
                    color,
                    square: sq,
                },
            ));

            if start_pos.distance(final_pos) > 1.0 {
                entity_cmds.insert(PieceAnimation {
                    start: start_pos,
                    end: final_pos,
                    elapsed: 0.0,
                    duration: 0.2,
                });
            }
        }
    }
}

/// Animate piece movement.
pub fn animate_piece_movement(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut Transform, &mut PieceAnimation)>,
) {
    for (entity, mut transform, mut anim) in pieces.iter_mut() {
        anim.elapsed += time.delta_secs();
        let t = (anim.elapsed / anim.duration).min(1.0);
        // Ease-out cubic
        let eased = 1.0 - (1.0 - t).powi(3);
        transform.translation = anim.start.lerp(anim.end, eased);

        // Slight arc
        let arc_height = 20.0 * (1.0 - (2.0 * t - 1.0).powi(2));
        transform.translation.y += arc_height;
        transform.translation.z = 3.0;

        if t >= 1.0 {
            transform.translation = anim.end;
            transform.translation.z = 2.0;
            commands.entity(entity).remove::<PieceAnimation>();
        }
    }
}

fn load_piece_texture(
    asset_server: &AssetServer,
    piece: types::Piece,
    color: types::Color,
) -> Handle<Image> {
    asset_server.load(piece_asset_path(piece, color))
}

fn piece_asset_path(piece: types::Piece, color: types::Color) -> &'static str {
    match (color, piece) {
        (types::Color::White, types::Piece::King) => "embedded://pieces/default/wK.png",
        (types::Color::White, types::Piece::Queen) => "embedded://pieces/default/wQ.png",
        (types::Color::White, types::Piece::Rook) => "embedded://pieces/default/wR.png",
        (types::Color::White, types::Piece::Bishop) => "embedded://pieces/default/wB.png",
        (types::Color::White, types::Piece::Knight) => "embedded://pieces/default/wN.png",
        (types::Color::White, types::Piece::Pawn) => "embedded://pieces/default/wP.png",
        (types::Color::Black, types::Piece::King) => "embedded://pieces/default/bK.png",
        (types::Color::Black, types::Piece::Queen) => "embedded://pieces/default/bQ.png",
        (types::Color::Black, types::Piece::Rook) => "embedded://pieces/default/bR.png",
        (types::Color::Black, types::Piece::Bishop) => "embedded://pieces/default/bB.png",
        (types::Color::Black, types::Piece::Knight) => "embedded://pieces/default/bN.png",
        (types::Color::Black, types::Piece::Pawn) => "embedded://pieces/default/bP.png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_piece_has_a_unique_embedded_asset() {
        let paths = [types::Color::White, types::Color::Black]
            .into_iter()
            .flat_map(|color| {
                types::Piece::ALL
                    .into_iter()
                    .map(move |piece| piece_asset_path(piece, color))
            })
            .collect::<HashSet<_>>();

        assert_eq!(paths.len(), PIECE_IMAGES.len());
        assert!(paths.iter().all(|path| path.starts_with("embedded://")));
        assert!(PIECE_IMAGES.iter().all(|(_, bytes)| !bytes.is_empty()));
    }

    #[test]
    fn background_music_is_embedded_with_the_game() {
        assert!(EMBEDDED_BGM.len() > 100_000);
    }
}
