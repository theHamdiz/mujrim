#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let buffer = std::env::args().skip(1).collect();
    mujrim_native_v60::run(buffer);
}
