#![allow(unsafe_op_in_unsafe_fn)]
#![warn(clippy::large_types_passed_by_value)]
#![warn(clippy::trivially_copy_pass_by_ref)]
#![warn(clippy::redundant_clone)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

mod board;
mod evaluation;
mod history;
mod lookup;
mod misc;
mod movepick;
mod nnue;
mod numa;
mod parameters;
mod search;
mod setwise;
mod stack;
mod thread;
mod threadpool;
mod time;
mod transposition;
mod types;

mod tools;

#[cfg(not(target_arch = "wasm32"))]
mod uci;

#[cfg(feature = "syzygy")]
mod tb;

#[cfg(feature = "syzygy")]
#[allow(warnings)]
mod bindings;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Statically composed native search backend. Implementations are selected at
/// compile time, so UCI search has no trait-object dispatch in its hot path.
pub trait NativeSearchAdapter {
    const ENGINE_NAME: &'static str;
    const NETWORK_ID: &'static str;
    const ENGINE_AUTHOR: &'static str;

    #[cfg(not(target_arch = "wasm32"))]
    fn run(buffer: std::collections::VecDeque<String>);
}

pub struct V60SearchAdapter;

impl NativeSearchAdapter for V60SearchAdapter {
    const ENGINE_NAME: &'static str = "Mujrim Native-v60";
    const NETWORK_ID: &'static str = "v60-7f587dfb";
    const ENGINE_AUTHOR: &'static str =
        "Ahmad Hamdi Emara (Egypt); Reckless contributors Arseniy Surkov, Shahin M. Shahin, and Styx";

    #[cfg(not(target_arch = "wasm32"))]
    fn run(buffer: std::collections::VecDeque<String>) {
        lookup::initialize();
        nnue::initialize();
        uci::message_loop(buffer);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(buffer: std::collections::VecDeque<String>) {
    V60SearchAdapter::run(buffer);
}

#[cfg(test)]
mod adapter_tests {
    use super::*;

    #[test]
    fn v60_adapter_metadata_is_stable() {
        assert_eq!(V60SearchAdapter::ENGINE_NAME, "Mujrim Native-v60");
        assert_eq!(V60SearchAdapter::NETWORK_ID, "v60-7f587dfb");
        assert!(V60SearchAdapter::ENGINE_AUTHOR.contains("Egypt"));
        assert!(V60SearchAdapter::ENGINE_AUTHOR.contains("Reckless contributors"));
    }
}
