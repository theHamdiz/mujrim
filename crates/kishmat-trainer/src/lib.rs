//! KishMat NNUE Training Pipeline.
//!
//! Two-stage process:
//! 1. **Datagen**: Self-play games to generate training positions
//! 2. **Training**: Use bullet trainer to train NNUE weights
//!
//! Usage (via KishMat CLI):
//! ```bash
//! kishmat train datagen --games 100000 --depth 8 --output data.bin
//! kishmat train train --data data.bin --epochs 100 --output net.bin
//! kishmat train bench --net net.bin
//! ```

pub mod datagen;
pub mod config;
