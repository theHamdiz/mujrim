#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

use bevy::prelude::*;

fn main() {
    types::init();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(main_window()),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.10, 0.10, 0.13)))
        .add_plugins(mujrim_game::MujrimGamePlugin)
        .run();
}

fn main_window() -> Window {
    Window {
        title: "Mujrim Chess".into(),
        resolution: bevy::window::WindowResolution::new(1200, 800),
        resizable: true,
        decorations: true,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_uses_standard_resizable_chrome() {
        let window = main_window();
        assert!(window.resizable);
        assert!(window.decorations);
        assert_eq!(window.title, "Mujrim Chess");
    }

    #[test]
    fn windows_resource_embeds_the_mujrim_icon() {
        let resources = include_str!("../../../build/app.rc");
        assert!(resources.contains("mujrim.ico"));
    }
}
