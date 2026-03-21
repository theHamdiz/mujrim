use bevy::prelude::*;

/// Marker for 3D board entities.
#[derive(Component)]
pub struct Board3dEntity;

/// Marker for individual 3D square meshes.
#[derive(Component)]
pub struct Square3d {
    pub file: u8,
    pub rank: u8,
}

const SQ_SIZE: f32 = 1.0;

/// Spawn a procedural 3D chess board.
pub fn spawn_board_3d(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let light_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.93, 0.84, 0.71),
        perceptual_roughness: 0.8,
        ..default()
    });
    let dark_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.35, 0.22),
        perceptual_roughness: 0.8,
        ..default()
    });
    let sq_mesh = meshes.add(Cuboid::new(SQ_SIZE, 0.1, SQ_SIZE));

    for rank in 0..8u8 {
        for file in 0..8u8 {
            let is_light = (file + rank) % 2 != 0;
            let mat = if is_light {
                light_mat.clone()
            } else {
                dark_mat.clone()
            };
            let pos = Vec3::new(
                (file as f32 - 3.5) * SQ_SIZE,
                0.0,
                (rank as f32 - 3.5) * SQ_SIZE,
            );
            commands.spawn((
                Mesh3d(sq_mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(pos),
                Board3dEntity,
                Square3d { file, rank },
            ));
        }
    }

    // Board border/frame
    let border_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.12, 0.05),
        ..default()
    });
    let border_mesh = meshes.add(Cuboid::new(8.4 * SQ_SIZE, 0.05, 8.4 * SQ_SIZE));
    commands.spawn((
        Mesh3d(border_mesh),
        MeshMaterial3d(border_mat),
        Transform::from_translation(Vec3::new(0.0, -0.05, 0.0)),
        Board3dEntity,
    ));

    // Lighting
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, 0.4, 0.0)),
        Board3dEntity,
    ));

    // Camera
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 8.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        Board3dEntity,
    ));
}

/// Despawn all 3D board entities.
pub fn despawn_board_3d(mut commands: Commands, entities: Query<Entity, With<Board3dEntity>>) {
    for entity in entities.iter() {
        commands.entity(entity).despawn();
    }
}
