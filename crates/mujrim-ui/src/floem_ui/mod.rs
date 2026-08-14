//! Floem GUI backend for Mujrim.

mod actions;
mod board;
mod chrome;
mod clock;
mod dock;
mod engine;
mod eval_bar;
mod eval_graph;
mod icons;
mod modals;
mod screens;
mod state;
mod svg_cache;
mod theme;
mod widgets;
mod windowing;

use floem::prelude::*;
use floem::taffy::style::{Display, Overflow};
use floem::text::FONT_CONTEXT;
use floem::window::WindowId;
use tokio::runtime::Runtime;

use self::state::AppState;
use crate::app_core::windowing::WindowPolicy;

const CURIOUS_FONT: &[u8] = include_bytes!("../../assets/CuriousTrack.ttf");

pub fn run() {
    let runtime = Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        tokio::task::block_in_place(|| {
            #[cfg(target_os = "macos")]
            set_macos_dock_icon();
            {
                let mut font_cx = FONT_CONTEXT.lock();
                font_cx
                    .collection
                    .register_fonts(CURIOUS_FONT.to_vec().into(), None);
            }
            let mut config = windowing::main_window_config(WindowPolicy::current());
            if let Some(icon) = load_window_icon() {
                config = config.window_icon(icon);
            }
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
    tick_clocks(state, handles.clone());
    tick_eval_bar(state, handles.clone());
    let content = screens::root_content(state, handles.clone());
    let shell = chrome::shell(window_id, state, handles.clone(), content);
    let options = modals::options_modal(state, handles.clone())
        .style(move |s| overlay_host_style(s, state.show_options.get()));
    let tournament = modals::tournament_setup_modal(state, handles)
        .style(move |s| overlay_host_style(s, state.show_tournament_setup.get()));
    Stack::new((shell.style(|s| s.size_full()), options, tournament))
        .style(|s| {
            s.size_full()
                .min_width(0.0)
                .min_height(0.0)
                .overflow_x(Overflow::Clip)
                .overflow_y(Overflow::Clip)
        })
        .window_title(|| "Mujrim Chess".to_owned())
}

fn overlay_host_style(style: floem::style::Style, open: bool) -> floem::style::Style {
    if open {
        widgets::overlay_layer_style(style)
    } else {
        style.display(Display::None)
    }
}

fn tick_clocks(state: AppState, handles: state::AppHandles) {
    floem::action::exec_after(std::time::Duration::from_millis(100), move |_| {
        state
            .clock_now_ms
            .set(crate::app_core::tournament_live::now_unix_ms());
        actions::apply_play_thinking_overlays(state, &handles);
        tick_clocks(state, handles);
    });
}

fn tick_eval_bar(state: AppState, handles: state::AppHandles) {
    floem::action::exec_after(std::time::Duration::from_millis(400), move |_| {
        actions::refresh_eval_bar(state, &handles);
        tick_eval_bar(state, handles);
    });
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
#[allow(unexpected_cfgs)]
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

#[cfg(test)]
mod tests {
    #[test]
    fn objc_clippy_feature_cfg_is_declared() {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        assert!(
            manifest.contains(r#"cfg(feature, values("cargo-clippy"))"#),
            "objc 0.2 msg_send! expands feature=\"cargo-clippy\"; declare it so macOS clippy -D warnings passes"
        );
    }

    #[test]
    fn overlay_host_collapses_when_closed() {
        let src = include_str!("mod.rs");
        let production = src.split("#[cfg(test)]").next().expect("source");
        assert!(
            production.contains("overlay_host_style"),
            "Options and tournament setup must share a window-sized overlay host"
        );
        assert!(
            production.contains("Display::None"),
            "closed overlays must stay mounted and hidden instead of swapping Empty views"
        );
        assert!(
            !production.contains("Empty::new().into_any()"),
            "creating overlay widgets inside dyn_view leaves them without a window root"
        );
    }
}
