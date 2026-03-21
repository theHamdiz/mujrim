use bevy::prelude::*;

fn main() {
    types::init();

    App::new()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "KishMat Chess".into(),
                    resolution: bevy::window::WindowResolution::new(1200, 800),
                    ..default()
                }),
                ..default()
            }),
        )
        .insert_resource(ClearColor(Color::srgb(0.10, 0.10, 0.13)))
        .add_plugins(kishmat_game::KishmatGamePlugin)
        .run();
}
