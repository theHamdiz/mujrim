//! Animation timing and system motion helpers for the GUI.

use std::time::Duration;

/// Piece / capture animation pacing controlled from settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimPace {
    Fast,
    Normal,
    Slow,
}

impl AnimPace {
    pub fn from_setting(value: i32) -> Self {
        match value {
            0 => Self::Fast,
            2 => Self::Slow,
            _ => Self::Normal,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Normal => "Normal",
            Self::Slow => "Slow",
        }
    }

    fn scale(self) -> f32 {
        match self {
            Self::Fast => 0.55,
            Self::Normal => 1.0,
            Self::Slow => 1.75,
        }
    }

    pub fn quiet_move(self) -> Duration {
        Duration::from_millis((150.0 * self.scale()) as u64)
    }

    pub fn capture_instant(self) -> Duration {
        Duration::from_millis((50.0 * self.scale()) as u64)
    }

    pub fn capture_explosion(self) -> Duration {
        Duration::from_millis((350.0 * self.scale()) as u64)
    }

    pub fn capture_fire(self) -> Duration {
        Duration::from_millis((400.0 * self.scale()) as u64)
    }

    /// Ease used for piece slide interpolation (smoothstep).
    pub fn ease(progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Landing / hub entrance fade progress from a start instant.
pub fn hub_entrance(progress_ms: u64, duration_ms: u64) -> f32 {
    if duration_ms == 0 {
        return 1.0;
    }
    AnimPace::ease(progress_ms as f32 / duration_ms as f32)
}

/// Staggered fade/slide progress for sequenced hub panels.
pub fn hub_stagger(progress_ms: u64, delay_ms: u64, duration_ms: u64) -> f32 {
    if progress_ms <= delay_ms {
        return 0.0;
    }
    hub_entrance(progress_ms - delay_ms, duration_ms)
}

/// Vertical slide offset (px) that settles to zero as a panel enters.
pub fn hub_slide_y(progress: f32, distance: f32) -> f32 {
    (1.0 - progress.clamp(0.0, 1.0)) * distance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_scales_duration() {
        assert!(AnimPace::Fast.quiet_move() < AnimPace::Normal.quiet_move());
        assert!(AnimPace::Slow.quiet_move() > AnimPace::Normal.quiet_move());
    }

    #[test]
    fn ease_is_smooth_and_clamped() {
        assert_eq!(AnimPace::ease(-1.0), 0.0);
        assert_eq!(AnimPace::ease(2.0), 1.0);
        assert!((AnimPace::ease(0.5) - 0.5).abs() < 0.01);
    }

    #[test]
    fn hub_entrance_completes() {
        assert_eq!(hub_entrance(0, 400), 0.0);
        assert_eq!(hub_entrance(400, 400), 1.0);
    }

    #[test]
    fn hub_stagger_waits_for_delay() {
        assert_eq!(hub_stagger(50, 100, 400), 0.0);
        assert!(hub_stagger(300, 100, 400) > 0.0);
        assert_eq!(hub_stagger(500, 100, 400), 1.0);
        assert_eq!(hub_slide_y(0.0, 24.0), 24.0);
        assert_eq!(hub_slide_y(1.0, 24.0), 0.0);
    }
}
