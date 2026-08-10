#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

fn main() -> iced::Result {
    mujrim_setup::run()
}
