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
    pub fn push(&mut self, t: i64, v: f64) {
        if self.points.len() == self.capacity {
            self.points.pop_front();
        }
        // Monotonic timestamps keep charts sane across clock adjustments.
        if let Some(last) = self.points.back() {
            if t < last.t {
                return;
            }
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

/// Default depth: 900 points ≈ 30 minutes at a 2-second interval.
pub const DEFAULT_CAPACITY: usize = 900;

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
    fn default_capacity_is_30min_at_2s() {
        let mut s = Series::new(DEFAULT_CAPACITY);
        for i in 0..DEFAULT_CAPACITY + 100 {
            s.push(i as i64 * 2000, i as f64);
        }
        assert_eq!(s.len(), DEFAULT_CAPACITY);
    }
}
