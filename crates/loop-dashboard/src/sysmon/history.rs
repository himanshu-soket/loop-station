//! Bounded in-memory history for live charts.
//!
//! A fixed-capacity ring of timestamped samples. The dashboard process keeps
//! only its own runtime's worth of data (no persistence, no disk writes), so
//! "historical" means since the monitor started — stated plainly in the UI.

use std::collections::VecDeque;

use serde::Serialize;

/// One point in a series.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Point {
    /// Unix epoch milliseconds.
    pub t: i64,
    /// Value at that time.
    pub v: f64,
}

impl Point {
    /// Construct a point.
    pub const fn new(t: i64, v: f64) -> Self {
        Self { t, v }
    }
}

/// Fixed-capacity time series.
#[derive(Debug, Clone)]
pub struct Series {
    points: VecDeque<Point>,
    capacity: usize,
}

impl Series {
    /// New series holding at most `capacity` points.
    pub fn new(capacity: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
        }
    }

    /// Append a point; drops the oldest when full.
    ///
    /// Timestamps must be strictly increasing: duplicates and out-of-order
    /// inserts are rejected so charts never see two samples claiming the
    /// same instant or history that moves backwards. (The eviction above is
    /// intentionally skipped for rejected points — a rejected sample must
    /// not cost us the oldest real one.)
    pub fn push(&mut self, t: i64, v: f64) {
        if let Some(last) = self.points.back() {
            if t <= last.t {
                return;
            }
        }
        if self.points.len() == self.capacity {
            self.points.pop_front();
        }
        self.points.push_back(Point::new(t, v));
    }

    /// Current length.
    #[allow(dead_code)] // used by tests; snapshot() covers production reads
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// True when no points are stored.
    #[allow(dead_code)] // used by tests
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Snapshot as a vector (oldest first).
    pub fn snapshot(&self) -> Vec<Point> {
        self.points.iter().copied().collect()
    }

    /// Most recent value.
    #[allow(dead_code)] // used by tests
    pub fn last(&self) -> Option<Point> {
        self.points.back().copied()
    }
}

/*
 * Default depth.
 *
 * 1800 points ≈ 1 hour at the default 2-second sampling interval, which is
 * what the UI's 1h range button needs. Memory cost per series:
 *   1800 × sizeof(Point) = 1800 × 16 B = 28.8 KB
 * The monitor keeps 10 series (cpu×3, mem×2, disk×2, net×2, gpu, battery →
 * 11), so the full history costs ~320 KB of process memory — negligible
 * next to the metrics themselves.
 */
pub const DEFAULT_CAPACITY: usize = 1800;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_snapshot_order() {
        let mut s = Series::new(3);
        s.push(1, 10.0);
        s.push(2, 20.0);
        s.push(3, 30.0);
        assert_eq!(
            s.snapshot(),
            vec![
                Point::new(1, 10.0),
                Point::new(2, 20.0),
                Point::new(3, 30.0)
            ]
        );
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut s = Series::new(2);
        s.push(1, 1.0);
        s.push(2, 2.0);
        s.push(3, 3.0);
        assert_eq!(s.len(), 2);
        assert_eq!(s.snapshot()[0].t, 2); // oldest dropped
        assert_eq!(s.last().unwrap().v, 3.0);
    }

    #[test]
    fn non_monotonic_timestamps_dropped() {
        let mut s = Series::new(5);
        s.push(100, 1.0);
        s.push(50, 99.0); // clock went backwards → ignored
        assert_eq!(s.len(), 1);
        assert_eq!(s.last().unwrap().t, 100);
    }

    #[test]
    fn zero_capacity_still_holds_one() {
        let mut s = Series::new(0);
        s.push(1, 5.0);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn default_capacity_is_one_hour_at_2s() {
        // 1800 samples × 2 s = 3600 s = 1 h of retained history.
        assert_eq!(DEFAULT_CAPACITY * 2, 3600);
        let mut s = Series::new(DEFAULT_CAPACITY);
        for i in 0..DEFAULT_CAPACITY + 100 {
            s.push(i as i64 * 2000, i as f64);
        }
        assert_eq!(s.len(), DEFAULT_CAPACITY);
    }

    #[test]
    fn duplicate_timestamps_are_rejected() {
        let mut s = Series::new(4);
        s.push(100, 1.0);
        s.push(100, 2.0); // same instant → ignored, not appended
        assert_eq!(s.len(), 1);
        assert_eq!(s.last().unwrap().v, 1.0);
    }

    #[test]
    fn out_of_order_inserts_are_rejected() {
        let mut s = Series::new(4);
        s.push(200, 2.0);
        s.push(100, 1.0); // older than the newest → ignored
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn equal_timestamp_replaces_nothing_and_keeps_bound() {
        // A long run at the same timestamp must not grow the buffer.
        let mut s = Series::new(3);
        for _ in 0..10 {
            s.push(500, 7.0);
        }
        assert_eq!(s.len(), 1);
    }
}
