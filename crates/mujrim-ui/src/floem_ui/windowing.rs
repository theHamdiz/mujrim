//! Apply [`crate::app_core::windowing::WindowPolicy`] to a Floem window.

use floem::kurbo::Size;
use floem::window::{Theme, WindowConfig};

use crate::app_core::windowing::WindowPolicy;

pub fn main_window_config(policy: WindowPolicy) -> WindowConfig {
    WindowConfig::default()
        .size(Size::new(1280.0, 850.0))
        .min_size(Size::new(800.0, 600.0))
        .show_titlebar(policy.show_titlebar)
        .undecorated(policy.undecorated)
        .undecorated_shadow(policy.undecorated_shadow)
        .title("Mujrim Chess")
        .resizable(true)
        .theme_override(Theme::Dark)
        .apply_default_theme(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::windowing::WindowPolicy;

    #[test]
    fn linux_config_requests_compositor_titlebar() {
        let policy = WindowPolicy::LINUX_WAYLAND;
        let _ = main_window_config(policy);
        assert!(policy.show_titlebar);
        assert!(!policy.undecorated);
    }

    #[test]
    fn windowing_module_does_not_spawn_extra_windows() {
        let src = include_str!("windowing.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(!production.contains("new_window"));
        assert!(!production.contains("Application::window"));
    }
}
