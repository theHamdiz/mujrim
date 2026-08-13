//! Iced SVG handles wrapping shared piece bytes.

use iced::widget::svg::Handle;

pub use crate::app_core::pieces::PieceSet;

pub struct PieceAssets {
    inner: crate::app_core::pieces::PieceAssets,
}

impl PieceAssets {
    pub fn load() -> Self {
        Self {
            inner: crate::app_core::pieces::PieceAssets::load(),
        }
    }

    pub fn get(&self, set: PieceSet, piece: types::Piece, color: types::Color) -> Handle {
        Handle::from_memory(self.inner.get(set, piece, color).to_vec())
    }
}
