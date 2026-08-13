//! Floem GUI backend for Mujrim.

mod actions;
mod board;
mod chrome;
mod engine;
mod eval_graph;
mod icons;
mod modals;
mod screens;
mod state;
mod theme;

use floem::kurbo::Size;
use floem::prelude::*;
use floem::window::{Theme, WindowConfig, WindowId};
use tokio::runtime::Runtime;

use self::state::AppState;

const CURIOUS_FONT: &[u8] = include_bytes!("../../assets/CuriousTrack.ttf");

pub fn run() {
    let runtime = Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        tokio::task::block_in_place(|| {
            #[cfg(target_os = "macos")]
            set_macos_dock_icon();
            let mut config = WindowConfig::default()
                .size(Size::new(1280.0, 850.0))
                .min_size(Size::new(800.0, 600.0))
                .show_titlebar(false)
                .undecorated(true)
                .undecorated_shadow(true)
                .title("Mujrim Chess")
                .resizable(true)
                .theme_override(Theme::Dark)
                .apply_default_theme(false);
            if let Some(icon) = load_window_icon() {
                config = config.window_icon(icon);
            }
            let _ = CURIOUS_FONT;
            floem::Application::new()
                .window(app_view, Some(config))
                .run();
        });
    });
}

fn load_window_icon() -> Option<floem::window::Icon> {
    let image = image::load_from_memory(include_bytes!(
        "../../../../assets/branding/mujrim-icon.png"
    ))
    .ok()?
    .into_rgba8();
    let (width, height) = image.dimensions();
    floem::window::RgbaIcon::new(image.into_raw(), width, height)
        .ok()
        .map(Into::into)
}

fn app_view(window_id: WindowId) -> impl IntoView {
    let (state, handles) = AppState::boot();
    tick_hub(state);
    let content = screens::root_content(state, handles.clone());
    let shell = chrome::shell(window_id, state, handles.clone(), content);
    let options_handles = handles.clone();
    let tournament_handles = handles.clone();
    Stack::new((
        shell,
        dyn_view(move || {
            if state.show_options.get() {
                modals::options_modal(state, options_handles.clone()).into_any()
            } else {
                Empty::new().into_any()
            }
        }),
        dyn_view(move || {
            if state.show_tournament_setup.get() {
                modals::tournament_setup_modal(state, tournament_handles.clone()).into_any()
            } else {
                Empty::new().into_any()
            }
        }),
    ))
    .style(|s| s.size_full())
    .window_title(|| "Mujrim Chess".to_owned())
}

fn tick_hub(state: AppState) {
    floem::action::exec_after(std::time::Duration::from_millis(16), move |_| {
        let next = (state.hub_progress.get_untracked() + 0.05).min(1.0);
        state.hub_progress.set(next);
        if next < 1.0 {
            tick_hub(state);
        }
    });
}

#[cfg(target_os = "macos")]
fn set_macos_dock_icon() {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let nsapp_class = Class::get("NSApplication").unwrap();
        let app: *mut Object = msg_send![nsapp_class, sharedApplication];
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let png_data: &[u8] = include_bytes!("../../../../assets/branding/mujrim-icon.png");
        let nsdata_class = Class::get("NSData").unwrap();
        let data: *mut Object = msg_send![nsdata_class, alloc];
        let data: *mut Object =
            msg_send![data, initWithBytes:png_data.as_ptr() length:png_data.len()];
        let nsimage_class = Class::get("NSImage").unwrap();
        let image: *mut Object = msg_send![nsimage_class, alloc];
        let image: *mut Object = msg_send![image, initWithData:data];
        if !image.is_null() {
            let _: () = msg_send![app, setApplicationIconImage:image];
        }
    }
}
