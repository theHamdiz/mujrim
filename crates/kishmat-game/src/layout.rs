use bevy::prelude::*;

/// HUD panel width in pixels.
pub const HUD_WIDTH: f32 = 260.0;

/// Padding around the board.
const BOARD_PADDING: f32 = 40.0;

/// Dynamic board layout computed from window size.
/// The board is positioned left of the HUD panel.
#[derive(Resource, Clone)]
pub struct BoardLayout {
    pub board_size: f32,
    pub square_size: f32,
    /// Bottom-left corner in world space.
    pub board_origin: Vec2,
}

impl Default for BoardLayout {
    fn default() -> Self {
        Self::from_window(1200.0, 800.0)
    }
}

impl BoardLayout {
    /// Compute layout from window pixel dimensions.
    pub fn from_window(width: f32, height: f32) -> Self {
        let available_w = (width - HUD_WIDTH - BOARD_PADDING).max(200.0);
        let available_h = (height - BOARD_PADDING).max(200.0);
        let board_size = available_w.min(available_h);
        let square_size = board_size / 8.0;

        // Center the board in the area left of the HUD.
        let area_left = -(width / 2.0);
        let area_right = (width / 2.0) - HUD_WIDTH;
        let area_center_x = (area_left + area_right) / 2.0;
        let board_origin = Vec2::new(area_center_x - board_size / 2.0, -board_size / 2.0);

        Self {
            board_size,
            square_size,
            board_origin,
        }
    }

    /// Convert file/rank to world position (center of square).
    pub fn square_to_world(&self, file: u8, rank: u8, flipped: bool) -> Vec3 {
        let (f, r) = if flipped {
            (7 - file, 7 - rank)
        } else {
            (file, rank)
        };
        Vec3::new(
            self.board_origin.x + (f as f32 + 0.5) * self.square_size,
            self.board_origin.y + (r as f32 + 0.5) * self.square_size,
            0.0,
        )
    }

    /// Convert world position to file/rank.
    pub fn world_to_square(&self, pos: Vec2, flipped: bool) -> Option<(u8, u8)> {
        let local = pos - self.board_origin;
        let f = (local.x / self.square_size).floor() as i32;
        let r = (local.y / self.square_size).floor() as i32;
        if !(0..8).contains(&f) || !(0..8).contains(&r) {
            return None;
        }
        if flipped {
            Some((7 - f as u8, 7 - r as u8))
        } else {
            Some((f as u8, r as u8))
        }
    }
}

/// Recompute board layout when window size changes.
pub fn on_window_resize(mut layout: ResMut<BoardLayout>, windows: Query<&Window>) {
    let Ok(window) = windows.single() else { return };
    let new = BoardLayout::from_window(window.width(), window.height());
    if (new.board_size - layout.board_size).abs() > 0.5 {
        *layout = new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_default() {
        let l = BoardLayout::default();
        assert!(l.board_size > 100.0);
        assert!((l.square_size - l.board_size / 8.0).abs() < 0.01);
    }

    #[test]
    fn test_square_roundtrip() {
        let l = BoardLayout::from_window(1200.0, 800.0);
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let pos = l.square_to_world(file, rank, false);
                let (f, r) = l.world_to_square(pos.truncate(), false).unwrap();
                assert_eq!((f, r), (file, rank));
            }
        }
    }

    #[test]
    fn test_square_roundtrip_flipped() {
        let l = BoardLayout::from_window(1200.0, 800.0);
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let pos = l.square_to_world(file, rank, true);
                let (f, r) = l.world_to_square(pos.truncate(), true).unwrap();
                assert_eq!((f, r), (file, rank));
            }
        }
    }

    #[test]
    fn test_responsive_shrink() {
        let big = BoardLayout::from_window(1200.0, 800.0);
        let small = BoardLayout::from_window(600.0, 400.0);
        assert!(small.board_size < big.board_size);
    }
}
