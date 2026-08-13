//! Animation timing and system motion helpers for the GUI.

use std::time::Duration;

use super::settings::{CaptureAnimStyle, PieceAnimStyle};

/// Piece / capture animation pacing controlled from settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimPace {
    Fast,
    Normal,
    Slow,
}

impl AnimPace {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Normal, Self::Slow];

    pub fn from_setting(value: i32) -> Self {
        match value {
            0 => Self::Fast,
            2 => Self::Slow,
            _ => Self::Normal,
        }
    }

    pub const fn to_setting(self) -> i32 {
        match self {
            Self::Fast => 0,
            Self::Normal => 1,
            Self::Slow => 2,
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

    pub fn bounce(progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        if t < 0.7 {
            let u = t / 0.7;
            Self::ease(u) * 1.18
        } else {
            let u = (t - 0.7) / 0.3;
            1.18 - 0.18 * Self::ease(u)
        }
        .min(1.18)
    }
}

impl std::fmt::Display for AnimPace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
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

/// Interpolated piece flight in board-display row/col space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceFlight {
    pub row: f64,
    pub col: f64,
    pub scale: f64,
}

pub fn piece_flight(
    style: PieceAnimStyle,
    t: f32,
    from_row: f64,
    from_col: f64,
    to_row: f64,
    to_col: f64,
) -> PieceFlight {
    let t = t.clamp(0.0, 1.0);
    match style {
        PieceAnimStyle::Instant => PieceFlight {
            row: to_row,
            col: to_col,
            scale: 1.0,
        },
        PieceAnimStyle::Slide => {
            let e = AnimPace::ease(t) as f64;
            PieceFlight {
                row: from_row + (to_row - from_row) * e,
                col: from_col + (to_col - from_col) * e,
                scale: 1.0,
            }
        }
        PieceAnimStyle::Arc => {
            let e = AnimPace::ease(t) as f64;
            let hop = ((std::f64::consts::PI * e).sin()) * 0.55;
            PieceFlight {
                row: from_row + (to_row - from_row) * e - hop,
                col: from_col + (to_col - from_col) * e,
                scale: 1.0 + hop * 0.12,
            }
        }
        PieceAnimStyle::Bounce => {
            let e = AnimPace::bounce(t) as f64;
            PieceFlight {
                row: from_row + (to_row - from_row) * e.min(1.0),
                col: from_col + (to_col - from_col) * e.min(1.0),
                scale: 1.0 + (e - 1.0).max(0.0) * 0.35,
            }
        }
        PieceAnimStyle::Warp => {
            let scale = if t < 0.45 {
                1.0 - AnimPace::ease(t / 0.45) as f64
            } else if t > 0.55 {
                AnimPace::ease((t - 0.55) / 0.45) as f64
            } else {
                0.08
            };
            let (row, col) = if t < 0.5 {
                (from_row, from_col)
            } else {
                (to_row, to_col)
            };
            PieceFlight {
                row,
                col,
                scale: scale.max(0.08),
            }
        }
    }
}

/// Capture burst mark in unit-square space around the captured square center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurstMark {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    pub ring: bool,
}

pub fn capture_marks(style: CaptureAnimStyle, burst: f32) -> Vec<BurstMark> {
    let burst = burst.clamp(0.0, 1.0);
    if burst <= 0.0 || matches!(style, CaptureAnimStyle::Instant) {
        return Vec::new();
    }
    let p = 1.0 - burst as f64;
    let alpha = (burst * 220.0) as u8;
    match style {
        CaptureAnimStyle::Instant => Vec::new(),
        CaptureAnimStyle::Explosion => {
            let mut marks = vec![BurstMark {
                x: 0.0,
                y: 0.0,
                radius: 0.18 + p * 0.62,
                r: 255,
                g: 220,
                b: 80,
                a: alpha,
                ring: true,
            }];
            for i in 0..10 {
                let (dx, dy) = spark_dir(i, 2.399);
                let dist = p * (0.22 + f64::from(i % 4) * 0.08);
                marks.push(BurstMark {
                    x: dx * dist,
                    y: dy * dist,
                    radius: 0.05 * burst as f64,
                    r: 255,
                    g: 180,
                    b: 60,
                    a: alpha,
                    ring: false,
                });
            }
            marks
        }
        CaptureAnimStyle::Fire => {
            let mut marks = Vec::new();
            for i in 0..12 {
                let wobble = ((i as f64) * 0.7).sin() * 0.12;
                marks.push(BurstMark {
                    x: wobble * (0.4 + p),
                    y: -p * (0.15 + f64::from(i) * 0.05),
                    radius: (0.08 + f64::from(i % 3) * 0.02) * burst as f64,
                    r: 255,
                    g: (90 + i * 12).min(180) as u8,
                    b: 28,
                    a: alpha.saturating_sub(i as u8 * 8),
                    ring: false,
                });
            }
            marks
        }
        CaptureAnimStyle::Shatter => {
            let mut marks = Vec::new();
            for i in 0..8 {
                let (dx, dy) = spark_dir(i, 0.785);
                let dist = 0.12 + p * 0.55;
                marks.push(BurstMark {
                    x: dx * dist,
                    y: dy * dist,
                    radius: 0.07 * (1.0 - p * 0.4),
                    r: 230,
                    g: 230,
                    b: 240,
                    a: alpha,
                    ring: false,
                });
            }
            marks
        }
        CaptureAnimStyle::Vortex => {
            let mut marks = Vec::new();
            for i in 0..14 {
                let spin = p * std::f64::consts::TAU * 1.6 + f64::from(i) * 0.45;
                let radius = 0.12 + p * 0.42;
                marks.push(BurstMark {
                    x: spin.cos() * radius,
                    y: spin.sin() * radius * 0.72,
                    radius: 0.045 * burst as f64,
                    r: 120,
                    g: 210,
                    b: 255,
                    a: alpha,
                    ring: false,
                });
            }
            marks.push(BurstMark {
                x: 0.0,
                y: 0.0,
                radius: 0.16 + p * 0.2,
                r: 80,
                g: 160,
                b: 255,
                a: (burst * 120.0) as u8,
                ring: true,
            });
            marks
        }
        CaptureAnimStyle::Spark => {
            let mut marks = Vec::new();
            for i in 0..16 {
                let (dx, dy) = spark_dir(i, 0.393);
                let dist = p * (0.18 + f64::from(i % 5) * 0.07);
                marks.push(BurstMark {
                    x: dx * dist,
                    y: dy * dist,
                    radius: 0.03 + (1.0 - p) * 0.02,
                    r: 255,
                    g: 240,
                    b: 160,
                    a: alpha,
                    ring: false,
                });
            }
            marks
        }
    }
}

fn spark_dir(index: u32, step: f64) -> (f64, f64) {
    let angle = f64::from(index) * step;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::settings::{CaptureAnimStyle, PieceAnimStyle};

    #[test]
    fn pace_scales_duration() {
        assert!(AnimPace::Fast.quiet_move() < AnimPace::Normal.quiet_move());
        assert!(AnimPace::Slow.quiet_move() > AnimPace::Normal.quiet_move());
        assert_eq!(
            AnimPace::from_setting(AnimPace::Slow.to_setting()),
            AnimPace::Slow
        );
        assert_eq!(AnimPace::ALL.len(), 3);
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

    #[test]
    fn piece_flight_reaches_destination() {
        let flight = piece_flight(PieceAnimStyle::Slide, 1.0, 7.0, 4.0, 4.0, 4.0);
        assert!((flight.row - 4.0).abs() < 1e-6);
        assert!((flight.col - 4.0).abs() < 1e-6);
        let warp = piece_flight(PieceAnimStyle::Warp, 1.0, 7.0, 4.0, 4.0, 4.0);
        assert!((warp.row - 4.0).abs() < 1e-6);
        assert!((warp.scale - 1.0).abs() < 1e-6);
        let arc = piece_flight(PieceAnimStyle::Arc, 0.5, 7.0, 4.0, 4.0, 4.0);
        assert!(arc.row < 5.5);
    }

    #[test]
    fn capture_marks_are_empty_for_instant_and_populated_otherwise() {
        assert!(capture_marks(CaptureAnimStyle::Instant, 1.0).is_empty());
        assert!(!capture_marks(CaptureAnimStyle::Explosion, 1.0).is_empty());
        assert!(!capture_marks(CaptureAnimStyle::Fire, 0.8).is_empty());
        assert!(!capture_marks(CaptureAnimStyle::Shatter, 0.6).is_empty());
        assert!(!capture_marks(CaptureAnimStyle::Vortex, 0.5).is_empty());
        assert!(!capture_marks(CaptureAnimStyle::Spark, 0.4).is_empty());
        assert!(capture_marks(CaptureAnimStyle::Explosion, 0.0).is_empty());
    }
}
