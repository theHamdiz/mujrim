#![deny(warnings)]
#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod app_core;
mod floem_ui;

fn main() {
    floem_ui::run();
}

#[cfg(test)]
mod feature_tests {
    #[test]
    fn default_backend_is_floem() {
        const {
            assert!(cfg!(feature = "floem-ui"));
        }
    }

    #[test]
    fn gui_does_not_depend_on_in_process_engines() {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        assert!(!manifest.contains("mujrim-search"));
        assert!(!manifest.contains("mujrim-eval"));
        assert!(!manifest.contains("mujrim-gpu"));
        assert!(!manifest.contains("embedded-networks"));
    }
}
