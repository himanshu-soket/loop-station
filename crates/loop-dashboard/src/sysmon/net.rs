//! Network metrics per interface.
//!
//! sysinfo exposes cumulative received/transmitted counters and derives
//! per-second rates on refresh. We surface every interface with a flag marking
//! which are "active" (have carried traffic), so the UI can default to the
//! interesting ones without hiding the rest.

use serde::Serialize;
use sysinfo::Networks;

/// One interface's reading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceInfo {
    /// Interface name (e.g. "en0", "lo0").
    pub name: String,
    /// Total bytes received since boot (interface lifetime).
    pub total_received: u64,
    /// Total bytes transmitted since boot.
    pub total_transmitted: u64,
    /// Download rate over the last window (bytes/sec).
    pub received_per_sec: f64,
    /// Upload rate over the last window (bytes/sec).
    pub transmitted_per_sec: f64,
    /// MAC address, when the interface has one.
    pub mac_address: Option<String>,
    /// True when the interface has carried any traffic.
    pub active: bool,
}

/// Aggregate across all physical (non-loopback) interfaces.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkTotals {
    /// Summed download rate (bytes/sec).
    pub received_per_sec: f64,
    /// Summed upload rate (bytes/sec).
    pub transmitted_per_sec: f64,
    /// Summed lifetime received bytes (physical interfaces only).
    pub total_received: u64,
    /// Summed lifetime transmitted bytes (physical interfaces only).
    pub total_transmitted: u64,
}

impl NetworkTotals {
    /// True when at least one physical interface is moving data.
    #[allow(dead_code)] // used by tests; UI uses it to highlight live traffic
    pub fn any_activity(&self) -> bool {
        self.received_per_sec > 0.5 || self.transmitted_per_sec > 0.5
    }
}

/// Loopback names excluded from aggregate bandwidth (they never leave the Mac).
const LOOPBACK: [&str; 2] = ["lo0", "lo"];

/// Stateful network collector.
pub struct NetworkCollector {
    networks: Networks,
}

impl Default for NetworkCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkCollector {
    /// New collector; call [`Self::sample`] twice before rates are meaningful.
    pub fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
        }
    }

    /// Take a sample. Rates describe the window since the previous call.
    pub fn sample(&mut self) -> Vec<InterfaceInfo> {
        self.networks.refresh(true);
        self.networks
            .iter()
            .map(|(name, data)| {
                let rx = data.received() as f64;
                let tx = data.transmitted() as f64;
                InterfaceInfo {
                    name: name.clone(),
                    total_received: data.total_received(),
                    total_transmitted: data.total_transmitted(),
                    received_per_sec: rx,
                    transmitted_per_sec: tx,
                    mac_address: Some(data.mac_address().to_string()),
                    active: data.total_received() > 0 || data.total_transmitted() > 0,
                }
            })
            .collect()
    }
}

/// Aggregate physical-interface totals (loopback excluded).
///
/// Pure function over a snapshot — unit tested independently of live sockets.
pub fn aggregate(interfaces: &[InterfaceInfo]) -> NetworkTotals {
    let mut t = NetworkTotals::default();
    for i in interfaces {
        if LOOPBACK.contains(&i.name.as_str()) {
            continue;
        }
        t.received_per_sec += i.received_per_sec;
        t.transmitted_per_sec += i.transmitted_per_sec;
        t.total_received = t.total_received.saturating_add(i.total_received);
        t.total_transmitted = t.total_transmitted.saturating_add(i.total_transmitted);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn iface(name: &str, rx: u64, tx: u64, rxs: f64, txs: f64) -> InterfaceInfo {
        InterfaceInfo {
            name: name.into(),
            total_received: rx,
            total_transmitted: tx,
            received_per_sec: rxs,
            transmitted_per_sec: txs,
            mac_address: None,
            active: true,
        }
    }

    #[test]
    fn aggregate_excludes_loopback() {
        let list = vec![
            iface("lo0", 1_000_000, 1_000_000, 500.0, 500.0),
            iface("en0", 10_000, 2_000, 100.0, 40.0),
            iface("en1", 0, 0, 0.0, 0.0),
        ];
        let t = aggregate(&list);
        assert!((t.received_per_sec - 100.0).abs() < 1e-9);
        assert!((t.transmitted_per_sec - 40.0).abs() < 1e-9);
        assert_eq!(t.total_received, 10_000);
        assert_eq!(t.total_transmitted, 2_000);
        assert!(t.any_activity());
    }

    #[test]
    fn idle_interfaces_report_no_activity() {
        let list = vec![iface("en0", 100, 100, 0.0, 0.0)];
        assert!(!aggregate(&list).any_activity());
    }

    #[test]
    fn aggregate_saturates_instead_of_overflow() {
        let list = vec![
            iface("a", u64::MAX, u64::MAX, 0.0, 0.0),
            iface("b", 10, 10, 0.0, 0.0),
        ];
        let t = aggregate(&list);
        assert_eq!(t.total_received, u64::MAX);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_sample_has_interfaces() {
        let mut c = NetworkCollector::new();
        let first = c.sample(); // seeds counters
        assert!(!first.is_empty());
        std::thread::sleep(Duration::from_millis(400));
        let second = c.sample();
        assert_eq!(first.len(), second.len());
        // loopback always exists on macOS
        assert!(second.iter().any(|i| i.name == "lo0"));
    }
}
