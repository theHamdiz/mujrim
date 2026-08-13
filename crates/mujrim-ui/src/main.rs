#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod app_core;

#[cfg(all(feature = "floem-ui", feature = "iced-ui"))]
compile_error!("enable exactly one GUI backend: floem-ui or iced-ui");

#[cfg(not(any(feature = "floem-ui", feature = "iced-ui")))]
compile_error!("enable a GUI backend: floem-ui (default) or iced-ui");

#[cfg(feature = "floem-ui")]
mod floem_ui;
#[cfg(feature = "iced-ui")]
mod iced_ui;

fn main() {
    #[cfg(feature = "floem-ui")]
    floem_ui::run();

    #[cfg(feature = "iced-ui")]
    if let Err(error) = iced_ui::run() {
        eprintln!("mujrim-ui (iced) failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod feature_tests {
    #[test]
    fn gui_backends_are_mutually_exclusive() {
        const {
            assert!(
                cfg!(feature = "floem-ui") ^ cfg!(feature = "iced-ui"),
                "exactly one of floem-ui or iced-ui must be enabled"
            );
        }
    }

    #[cfg(feature = "floem-ui")]
    #[test]
    fn default_backend_is_floem() {
        const {
            assert!(cfg!(feature = "floem-ui"));
            assert!(!cfg!(feature = "iced-ui"));
        }
    }
}
