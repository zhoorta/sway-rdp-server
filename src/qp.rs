//! The quantiser of the GPU encoders (VAAPI, NVENC): both run at a constant quantiser and let this move it with the link.

use std::time::Instant;

/// Picks the quantiser from how many bits the last frames took against the bitrate the link allows. A bucket fills with the bits of each
/// frame and drains at the target rate (it cannot hold credit: a quiet second does not pay for a busy one).
///
/// The quantiser moves in tiers, from `floor` (the quality wanted, where a desktop stays almost always) to `ceiling` (what a slow link may
/// cost), and rarely: a change is only a number in the next slice header (no new parameter sets, no keyframe), but a client that is slow to follow is better served by few changes. One tier up after a second with the bucket
/// full, one down after five with it empty, and never twice within two seconds.
pub struct QpControl {
    floor: u32,
    ceiling: u32,
    pub target_bps: f64,
    bucket: f64,
    tier: u32,
    last: Instant,
    changed: Instant,
    full_since: Option<Instant>,
    empty_since: Option<Instant>,
}

const TIERS: u32 = 3;
const FULL_AFTER: std::time::Duration = std::time::Duration::from_secs(1);
const EMPTY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);
const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

impl QpControl {
    pub fn new(floor: u32, ceiling: u32, target_bps: u32) -> Self {
        let now = Instant::now();
        Self {
            floor,
            ceiling: ceiling.max(floor),
            target_bps: f64::from(target_bps.max(1)),
            bucket: 0.0,
            tier: 0,
            last: now,
            changed: now - HOLD,
            full_since: None,
            empty_since: None,
        }
    }

    /// The quantiser for the next frame, after one of `bytes` bytes.
    pub fn after_frame(&mut self, bytes: usize) -> u32 {
        self.after_frame_at(Instant::now(), bytes)
    }

    pub fn after_frame_at(&mut self, now: Instant, bytes: usize) -> u32 {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.bucket = (self.bucket + bytes as f64 * 8.0 - self.target_bps * dt).max(0.0);
        // Full: half a second of target bits or more behind. Empty: less than a twentieth of a second.
        let (full, empty) = (self.bucket >= self.target_bps * 0.5, self.bucket <= self.target_bps * 0.05);
        self.full_since = if full { self.full_since.or(Some(now)) } else { None };
        self.empty_since = if empty { self.empty_since.or(Some(now)) } else { None };
        if now.saturating_duration_since(self.changed) >= HOLD {
            if self.full_since.is_some_and(|t| now.saturating_duration_since(t) >= FULL_AFTER) && self.tier < TIERS {
                self.tier += 1;
                self.changed = now;
                self.full_since = None;
            } else if self.empty_since.is_some_and(|t| now.saturating_duration_since(t) >= EMPTY_AFTER) && self.tier > 0 {
                self.tier -= 1;
                self.changed = now;
                self.empty_since = None;
            }
        }
        self.qp()
    }

    pub fn qp(&self) -> u32 {
        self.floor + self.tier * (self.ceiling - self.floor) / TIERS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn quantiser_steps_up_under_sustained_overload_and_back_down() {
        let t0 = Instant::now();
        let mut c = QpControl::new(18, 42, 8_000_000);
        assert_eq!(c.qp(), 18);
        // 100 KB every 20 ms is 40 Mbit/s against 8: the bucket is full within a second, and a second later the quantiser steps up.
        let mut now = t0;
        let mut seen = vec![];
        for _ in 0..300 {
            now += Duration::from_millis(20);
            seen.push(c.after_frame_at(now, 100_000));
        }
        assert_eq!(seen[0], 18, "one big frame is not a reason");
        assert!(seen.contains(&26), "first step: {seen:?}");
        assert_eq!(*seen.last().unwrap(), 42, "six seconds of overload reach the ceiling");
        // Quiet: tiny frames, the bucket drains, and after five seconds of it the quantiser steps down, one tier at a time.
        let mut down = vec![];
        for _ in 0..800 {
            now += Duration::from_millis(50);
            down.push(c.after_frame_at(now, 200));
        }
        assert_eq!(*down.last().unwrap(), 18, "back at the floor after a long quiet time");
        let changes = down.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes <= 3, "one tier at a time: {changes} changes");
    }

    #[test]
    fn a_keyframe_burst_does_not_move_the_quantiser() {
        let t0 = Instant::now();
        let mut c = QpControl::new(18, 42, 8_000_000);
        let mut now = t0;
        c.after_frame_at(now, 600_000);
        for _ in 0..100 {
            now += Duration::from_millis(50);
            assert_eq!(c.after_frame_at(now, 5_000), 18);
        }
    }

    #[test]
    fn ceiling_is_never_under_floor() {
        let c = QpControl::new(30, 20, 1_000_000);
        assert_eq!(c.qp(), 30);
    }
}
