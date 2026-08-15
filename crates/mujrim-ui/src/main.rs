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
}
