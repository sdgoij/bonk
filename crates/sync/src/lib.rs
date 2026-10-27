#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Synchronisation primitives: hybrid logical clocks and CRDT merge rules.
//!
//! M8 implements the oplog and relay. The HLC lands here first because it is
//! pure, testable, and every sync decision depends on its ordering being total
//! and monotonic.

use core::fmt;

/// A hybrid logical clock value: wall time, logical counter, device tiebreaker.
///
/// Ordered lexicographically by `(wall_ms, counter, device)`, which makes the
/// total order independent of clock skew between devices.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Hlc {
    wall_ms: u64,
    counter: u32,
    device: crux::DeviceId,
}

impl Hlc {
    /// Creates a clock reading.
    pub const fn new(wall_ms: u64, counter: u32, device: crux::DeviceId) -> Self {
        Self {
            wall_ms,
            counter,
            device,
        }
    }

    /// Wall-clock component in milliseconds since the Unix epoch.
    pub const fn wall_ms(self) -> u64 {
        self.wall_ms
    }

    /// Logical counter that breaks ties within the same millisecond.
    pub const fn counter(self) -> u32 {
        self.counter
    }

    /// Device that produced the value.
    pub const fn device(self) -> crux::DeviceId {
        self.device
    }

    /// Next value on this device at the given wall time.
    ///
    /// If the clock moved backwards, the previous wall time is kept and the
    /// counter advances, so ordering never regresses.
    pub fn tick(self, now_ms: u64) -> Self {
        if now_ms > self.wall_ms {
            Self::new(now_ms, 0, self.device)
        } else {
            Self::new(self.wall_ms, self.counter.saturating_add(1), self.device)
        }
    }

    /// Merge with a value observed from another device.
    ///
    /// The receiver's counter always exceeds both inputs, so a merged value is
    /// causally after everything it observed.
    pub fn merge(self, other: Self, now_ms: u64) -> Self {
        let wall = self.wall_ms.max(other.wall_ms).max(now_ms);
        let counter = if wall == self.wall_ms && wall == other.wall_ms {
            self.counter.max(other.counter).saturating_add(1)
        } else if wall == self.wall_ms {
            self.counter.saturating_add(1)
        } else if wall == other.wall_ms {
            other.counter.saturating_add(1)
        } else {
            0
        };
        Self::new(wall, counter, self.device)
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:013}-{:06}-{}", self.wall_ms, self.counter, self.device)
    }
}

/// How two concurrent field writes are resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MergeOutcome {
    /// The local value wins.
    KeepLocal,
    /// The remote value wins.
    TakeRemote,
    /// Values are identical; nothing to do.
    Identical,
}

/// Resolves a last-writer-wins field using HLC order.
pub fn resolve_lww<T: PartialEq>(local: &T, local_hlc: Hlc, remote: &T, remote_hlc: Hlc) -> MergeOutcome {
    if local == remote {
        return MergeOutcome::Identical;
    }
    if remote_hlc > local_hlc {
        MergeOutcome::TakeRemote
    } else {
        MergeOutcome::KeepLocal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> crux::DeviceId {
        crux::DeviceId::new()
    }

    #[test]
    fn ticks_are_monotonic() {
        let device = device();
        let start = Hlc::new(1_000, 0, device);
        let first = start.tick(1_000);
        let second = first.tick(1_000);
        assert!(second > first);
        assert!(second > start);
    }

    #[test]
    fn a_backwards_clock_never_regresses() {
        let device = device();
        let start = Hlc::new(2_000, 3, device);
        let ticked = start.tick(1_500);
        assert!(ticked > start);
        assert_eq!(ticked.wall_ms(), 2_000);
        assert_eq!(ticked.counter(), 4);
    }

    #[test]
    fn merge_exceeds_both_inputs() {
        let local_device = device();
        let remote_device = device();
        let local = Hlc::new(1_000, 2, local_device);
        let remote = Hlc::new(1_000, 5, remote_device);
        let merged = local.merge(remote, 1_000);
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_uses_the_new_wall_clock_when_time_moved_on() {
        let local = Hlc::new(1_000, 2, device());
        let remote = Hlc::new(1_500, 0, device());
        let merged = local.merge(remote, 2_000);
        assert_eq!(merged.wall_ms(), 2_000);
        assert_eq!(merged.counter(), 0);
    }

    #[test]
    fn lww_prefers_the_later_clock() {
        let a = Hlc::new(1_000, 0, device());
        let b = Hlc::new(1_001, 0, device());
        assert_eq!(resolve_lww(&"local", a, &"remote", b), MergeOutcome::TakeRemote);
        assert_eq!(resolve_lww(&"local", b, &"remote", a), MergeOutcome::KeepLocal);
        assert_eq!(resolve_lww(&"same", a, &"same", b), MergeOutcome::Identical);
    }

    #[test]
    fn sorting_is_total() {
        let mut values = [
            Hlc::new(2, 0, device()),
            Hlc::new(1, 9, device()),
            Hlc::new(1, 0, device()),
        ];
        values.sort();
        assert_eq!(values[0].wall_ms(), 1);
        assert_eq!(values[0].counter(), 0);
        assert_eq!(values[2].wall_ms(), 2);
    }
}
