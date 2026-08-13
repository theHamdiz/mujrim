use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::thread::ThreadData;

#[derive(Clone, Debug)]
pub enum Limits {
    Infinite,
    Depth(i32),
    Time(u64),
    Nodes(u64),
    Mate(u64),
    Fischer(u64, u64),
    Cyclic(u64, u64, u64),
}

/// Time retained for scheduler jitter and UCI transport latency even when the
/// GUI configures `MoveOverhead` to zero.
const MIN_CLOCK_RESERVE_MS: u64 = 50;
const MAX_CLOCK_RESERVE_MS: u64 = 250;
const MOVETIME_TRANSPORT_RESERVE_MS: u64 = 15;

const fn spendable_clock(main: u64, move_overhead: u64) -> u64 {
    let proportional_reserve = main / 100;
    let clock_reserve = if proportional_reserve < MIN_CLOCK_RESERVE_MS {
        MIN_CLOCK_RESERVE_MS
    } else if proportional_reserve > MAX_CLOCK_RESERVE_MS {
        MAX_CLOCK_RESERVE_MS
    } else {
        proportional_reserve
    };
    main.saturating_sub(move_overhead.saturating_add(clock_reserve))
}

#[derive(Clone)]
pub struct TimeManager {
    limits: Limits,
    start_time: Instant,
    soft_bound: Duration,
    hard_bound: Duration,
}

impl TimeManager {
    pub fn new(limits: Limits, fullmove_number: usize, move_overhead: u64) -> Self {
        let soft;
        let hard;

        match limits {
            Limits::Time(ms) => {
                let available = ms.saturating_sub(move_overhead.saturating_add(MOVETIME_TRANSPORT_RESERVE_MS));
                soft = available;
                hard = available;
            }
            Limits::Fischer(main, inc) => {
                let soft_scale = 0.0594 - 0.0492 * (-0.0386 * fullmove_number as f64).exp();
                let hard_scale = 0.7281;
                let available = spendable_clock(main, move_overhead);

                let soft_bound = (soft_scale * available as f64 + 0.75 * inc as f64) as u64;
                let hard_bound = (hard_scale * available as f64 + 0.75 * inc as f64) as u64;

                soft = soft_bound.min(available);
                hard = hard_bound.min(available);
            }
            Limits::Cyclic(main, inc, moves) => {
                let available = spendable_clock(main, move_overhead);
                let base = (available as f64 / moves.max(1) as f64) + 0.75 * inc as f64;

                soft = (base as u64).min(available);
                hard = ((5.0 * base) as u64).min(available);
            }
            _ => {
                soft = u64::MAX;
                hard = u64::MAX;
            }
        }

        Self {
            limits,
            start_time: Instant::now(),
            soft_bound: Duration::from_millis(soft),
            hard_bound: Duration::from_millis(hard),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn soft_limit(&self, td: &ThreadData, multiplier: impl Fn() -> f32) -> bool {
        match self.limits {
            Limits::Infinite | Limits::Depth(_) | Limits::Mate(_) => false,
            Limits::Nodes(maximum) => td.shared.nodes.aggregate() >= maximum,
            Limits::Time(_) => self.start_time.elapsed() >= self.soft_bound,
            _ => self.start_time.elapsed() >= Duration::from_secs_f32(self.soft_bound.as_secs_f32() * multiplier()),
        }
    }

    pub fn check_time(&self, td: &ThreadData) -> bool {
        if td.completed_depth == 0 {
            return false;
        }

        match self.limits {
            Limits::Infinite | Limits::Depth(_) | Limits::Mate(_) => false,
            Limits::Nodes(maximum) => td.shared.nodes.aggregate() > maximum,
            _ => td.nodes() & 2047 == 2047 && self.start_time.elapsed() >= self.hard_bound,
        }
    }

    pub fn limits(&self) -> Limits {
        self.limits.clone()
    }

    pub fn use_time_management(&self) -> bool {
        matches!(self.limits, Limits::Fischer(..) | Limits::Cyclic(..) | Limits::Time(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milliseconds(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap()
    }

    #[test]
    fn fischer_clock_keeps_an_emergency_reserve() {
        let manager = TimeManager::new(Limits::Fischer(1_000, 100), 30, 100);

        assert!(milliseconds(manager.hard_bound) <= 850);
        assert!(manager.soft_bound <= manager.hard_bound);
    }

    #[test]
    fn cyclic_clock_never_spends_unearned_increment() {
        let manager = TimeManager::new(Limits::Cyclic(500, 10_000, 1), 30, 100);

        assert!(milliseconds(manager.hard_bound) <= 350);
        assert!(manager.soft_bound <= manager.hard_bound);
    }

    #[test]
    fn explicit_movetime_respects_configured_and_transport_overhead() {
        let manager = TimeManager::new(Limits::Time(1_000), 1, 100);

        assert_eq!(milliseconds(manager.soft_bound), 885);
        assert_eq!(manager.soft_bound, manager.hard_bound);
    }

    #[test]
    fn exhausted_clock_requests_an_immediate_move() {
        let manager = TimeManager::new(Limits::Fischer(25, 1_000), 60, 0);

        assert_eq!(manager.soft_bound, Duration::ZERO);
        assert_eq!(manager.hard_bound, Duration::ZERO);
    }
}
