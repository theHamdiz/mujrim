//! Mujrim NNUE Training Pipeline.
//!
//! Two-stage process:
//! 1. **Datagen**: Self-play games to generate training positions
//! 2. **Training**: Use bullet trainer to train NNUE weights
//!
//! Usage (via Mujrim CLI):
//! ```bash
//! mujrim train datagen --games 100000 --depth 8 --output data.bin
//! mujrim train train --data data.bin --epochs 100 --output net.bin
//! mujrim train bench --net net.bin
//! ```

pub mod ateed;
pub mod config;
pub mod datagen;
