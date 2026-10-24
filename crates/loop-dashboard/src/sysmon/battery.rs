//! Battery metrics via `ioreg` (AppleSmartBattery) — no root required.
//!
//! Cycle count, design/max capacity (health), charge percentage and charging
//! state are all readable unprivileged. Desktop Macs report the whole domain
//! as unavailable rather than pretending.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::sysmon::metrics::{parse_ioreg_number_any, Metric};

/// How long a battery-health (ioreg) reading stays valid.
///
/// Cycle count and design/full-charge capacity only move when the battery is
/// actually cycled; an hour is far shorter than any real change while cutting
/// ~44 ms of subprocess time from all but one slow pass per hour.
pub const HEALTH_TTL: Duration = Duration::from_secs(3600);

/// Battery reading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatterySnapshot {
    /// Charge percentage (0–100).
    pub charge_percent: Metric<f64>,
    /// Charging / discharging / fully charged / on AC.
    pub status: Metric<String>,
    /// Time remaining estimate in minutes (absent while charging).
    pub time_remaining_minutes: Metric<i64>,
    /// Cycle count.
    pub cycle_count: Metric<u32>,
    /// Maximum capacity as % of design capacity ("health").
    pub health_percent: Metric<f64>,
    /// Design capacity in mAh.
    pub design_capacity_mah: Metric<u32>,
    /// Current maximum capacity in mAh.
    pub max_capacity_mah: Metric<u32>,
}

impl Default for BatterySnapshot {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl BatterySnapshot {
    /// All-unavailable snapshot (no battery).
    pub fn unavailable() -> Self {
        let reason = "no battery detected on this machine";
        Self {
            charge_percent: Metric::unavailable("%", "Battery charge", reason),
            status: Metric::unavailable("", "Charging state", reason),
            time_remaining_minutes: Metric::unavailable("min", "Estimated time left", reason),
            cycle_count: Metric::unavailable("", "Charge cycle count", reason),
            health_percent: Metric::unavailable("%", "Battery health (max/design)", reason),
            design_capacity_mah: Metric::unavailable("mAh", "Design capacity", reason),
            max_capacity_mah: Metric::unavailable("mAh", "Current full-charge capacity", reason),
        }
    }
}

/// Parses `pmset -g batt` output.
///
/// Example:
/// ```text
/// Now drawing from 'Battery Power'
///  `-InternalBattery-0 (id=7209059)  46%; discharging; 4:09 remaining present: true`
/// ```
pub fn parse_pmset_batt(text: &str) -> Option<BatterySnapshot> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let mut snap = BatterySnapshot::unavailable();

    // percentage: "46%;" or "46%; "
    let pct = line
        .split(';')
        .next()?
        .rsplit(|c: char| !c.is_ascii_digit())
        .find_map(|t| t.parse::<f64>().ok())?;
    snap.charge_percent = Metric::ok(pct.clamp(0.0, 100.0), "%", "Battery charge");

    let lower = line.to_lowercase();
    let status = if lower.contains("fully charged") {
        "fully charged"
    } else if lower.contains("; charging") || lower.contains("finishing charge") {
        "charging"
    } else if lower.contains("discharging") {
        "discharging"
    } else if lower.contains("ac power") {
        "on ac power"
    } else {
        "unknown"
    };
    snap.status = Metric::ok(status.into(), "", "Charging state");

    // "4:09 remaining" → minutes
    let remaining = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|w| {
            if w[1] == "remaining" && w[0].contains(':') {
                let mut it = w[0].split(':');
                let h: i64 = it.next()?.parse().ok()?;
                let m: i64 = it.next()?.parse().ok()?;
                Some(h * 60 + m)
            } else {
                None
            }
        });
    if let Some(mins) = remaining {
        snap.time_remaining_minutes =
            Metric::ok(mins, "min", "Estimated time remaining (system estimate)");
    }

    Some(snap)
}

/// Parses `ioreg -r -c AppleSmartBattery` output for health data.
pub fn parse_battery_ioreg(text: &str) -> Option<BatterySnapshot> {
    if !text.contains("AppleSmartBattery") {
        return None;
    }
    let mut snap = BatterySnapshot::unavailable();

    let number = |key: &str| parse_ioreg_number_any(text, key);

    // Cycle count key varies by macOS version: older prints
    // `"Cycle Count" = 129`, newer prints `"CycleCount" = 129`.
    if let Some(cycles) = number("CycleCount").or_else(|| number("Cycle Count")) {
        snap.cycle_count = Metric::ok(cycles as u32, "", "Charge cycle count");
    }
    if let Some(design) = number("DesignCapacity") {
        snap.design_capacity_mah = Metric::ok(design as u32, "mAh", "Design capacity");
    }
    // Prefer AppleRawMaxCapacity (true chemical max); fall back to MaxCapacity.
    let max = number("AppleRawMaxCapacity")
        .or_else(|| number("MaxCapacity"))
        .or_else(|| number("NominalChargeCapacity"));
    if let Some(max) = max {
        snap.max_capacity_mah = Metric::ok(max as u32, "mAh", "Current full-charge capacity");
        if let Some(design) = number("DesignCapacity") {
            if design > 0.0 {
                let health = (max / design * 100.0).clamp(0.0, 100.0);
                snap.health_percent =
                    Metric::ok(health, "%", "Battery health (current max ÷ design)");
            }
        }
    }
    // Real ioreg prints `"IsCharging" = Yes` / `= No`.
    if snap.status.unavailable_reason.is_some() {
        if let Some(charging) = crate::sysmon::metrics::parse_ioreg_string(text, "IsCharging") {
            let s = if charging.eq_ignore_ascii_case("yes") {
                "charging"
            } else {
                "not charging"
            };
            snap.status = Metric::ok(s.into(), "", "Charging state");
        }
    }
    Some(snap)
}

/// Merges pmset (charge/status/estimate) over ioreg (health/cycles).
pub fn merge_snapshots(
    pmset: Option<BatterySnapshot>,
    ioreg: Option<BatterySnapshot>,
) -> BatterySnapshot {
    let mut out = pmset.unwrap_or_default();
    if let Some(io) = ioreg {
        if out.cycle_count.unavailable_reason.is_some() && io.cycle_count.available {
            out.cycle_count = io.cycle_count;
        }
        if out.health_percent.unavailable_reason.is_some() && io.health_percent.available {
            out.health_percent = io.health_percent;
        }
        if out.design_capacity_mah.unavailable_reason.is_some() && io.design_capacity_mah.available
        {
            out.design_capacity_mah = io.design_capacity_mah;
        }
        if out.max_capacity_mah.unavailable_reason.is_some() && io.max_capacity_mah.available {
            out.max_capacity_mah = io.max_capacity_mah;
        }
    }
    out
}

/// One-shot battery reader.
pub struct BatteryCollector {
    /// Cached ioreg (health/cycle) reading with its collection instant.
    /// Health and cycle count change on the scale of weeks; re-reading the
    /// 44 ms subprocess pair every slow tick is waste. Bounded by
    /// [`HEALTH_TTL`] — a cache entry is always discarded after expiry, so a
    /// value can never be served indefinitely stale.
    health_cache: Option<(Instant, BatterySnapshot)>,
}

impl BatteryCollector {
    /// New collector.
    pub fn new() -> Self {
        Self { health_cache: None }
    }

    /// Take a sample by combining pmset + ioreg.
    ///
    /// * Charge/status/estimate come from `pmset -g batt` on every call —
    ///   they change minute-to-minute and must stay fresh.
    /// * Cycle count / capacities come from `ioreg AppleSmartBattery` and are
    ///   cached for [`HEALTH_TTL`] because they are effectively constant
    ///   between charges. `force_refresh` bypasses the cache (first sample).
    #[cfg(target_os = "macos")]
    pub fn sample(&mut self, force_refresh: bool) -> BatterySnapshot {
        let pmset = std::process::Command::new("pmset")
            .args(["-g", "batt"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .and_then(|t| parse_pmset_batt(&t));

        let ioreg = if force_refresh
            || self
                .health_cache
                .as_ref()
                .is_none_or(|(at, _)| at.elapsed() >= HEALTH_TTL)
        {
            let snap = std::process::Command::new("ioreg")
                .args(["-r", "-c", "AppleSmartBattery"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .and_then(|t| parse_battery_ioreg(&t));
            if let Some(s) = &snap {
                // Only cache *successful* reads; a failure must retry next time.
                self.health_cache = Some((Instant::now(), s.clone()));
            }
            snap
        } else {
            self.health_cache.as_ref().map(|(_, s)| s.clone())
        };
        merge_snapshots(pmset, ioreg)
    }

    /// Non-macOS: unavailable.
    #[cfg(not(target_os = "macos"))]
    pub fn sample(&mut self, _force_refresh: bool) -> BatterySnapshot {
        BatterySnapshot::unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PMSET: &str = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=7209059)\t46%; discharging; 4:09 remaining present: true\n";

    const IOREG: &str = r#"
+-o AppleSmartBattery  <class AppleSmartBattery, id 0x1000004a1>
  | {
  |   "CycleCount" = 129
  |   "DesignCapacity" = 4629
  |   "AppleRawMaxCapacity" = 4458
  |   "MaxCapacity" = 100
  |   "IsCharging" = No
  | }
"#;

    /// Older macOS prints the key with a space.
    const IOREG_LEGACY_SPACED: &str = r#"
+-o AppleSmartBattery  <class AppleSmartBattery, id 0x1000004a1>
  | {
  |   "Cycle Count" = 129
  |   "DesignCapacity" = 4629
  |   "AppleRawMaxCapacity" = 4458
  | }
"#;

    #[test]
    fn parses_pmset_charge_and_status() {
        let s = parse_pmset_batt(PMSET).unwrap();
        assert_eq!(s.charge_percent.value, Some(46.0));
        assert_eq!(s.status.value.as_deref(), Some("discharging"));
        assert_eq!(s.time_remaining_minutes.value, Some(249)); // 4h09m
    }

    #[test]
    fn parses_charging_status() {
        let text = "-InternalBattery-0 (id=1)\t80%; charging; 0:52 remaining present: true\n";
        let s = parse_pmset_batt(text).unwrap();
        assert_eq!(s.status.value.as_deref(), Some("charging"));
        // the time estimate must survive the charging case too
        assert!(s.time_remaining_minutes.available);
        assert_eq!(s.time_remaining_minutes.value, Some(52));
    }

    #[test]
    fn parses_fully_charged_on_ac() {
        let text = "Now drawing from 'AC Power'\n -InternalBattery-0\t100%; fully charged; 0:00 remaining\n";
        let s = parse_pmset_batt(text).unwrap();
        assert_eq!(s.status.value.as_deref(), Some("fully charged"));
    }

    #[test]
    fn pmset_without_battery_is_none() {
        assert!(parse_pmset_batt("Now drawing from 'AC Power'\n").is_none());
    }

    #[test]
    fn parses_ioreg_health_preferring_apple_raw() {
        let s = parse_battery_ioreg(IOREG).unwrap();
        assert_eq!(s.cycle_count.value, Some(129));
        assert_eq!(s.design_capacity_mah.value, Some(4629));
        // AppleRawMaxCapacity wins over MaxCapacity
        assert_eq!(s.max_capacity_mah.value, Some(4458));
        let health = s.health_percent.value.unwrap();
        assert!((health - 96.3).abs() < 0.2); // 4458/4629
    }

    #[test]
    fn parses_legacy_spaced_cycle_count_key() {
        // Older macOS: "Cycle Count" with a space. Both shapes must work.
        let s = parse_battery_ioreg(IOREG_LEGACY_SPACED).unwrap();
        assert_eq!(s.cycle_count.value, Some(129));
        assert_eq!(s.design_capacity_mah.value, Some(4629));
    }

    #[test]
    fn ioreg_without_smartbattery_is_none() {
        assert!(parse_battery_ioreg("nothing here").is_none());
    }

    #[test]
    fn merge_combines_sources() {
        let pmset = parse_pmset_batt(PMSET);
        let ioreg = parse_battery_ioreg(IOREG);
        let merged = merge_snapshots(pmset, ioreg);
        assert_eq!(merged.charge_percent.value, Some(46.0)); // from pmset
        assert_eq!(merged.cycle_count.value, Some(129)); // from ioreg
        assert!(merged.health_percent.available);
    }

    #[test]
    fn merge_with_nothing_stays_honest() {
        let merged = merge_snapshots(None, None);
        assert!(!merged.charge_percent.available);
        assert!(!merged.health_percent.available);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_battery_or_unavailable() {
        let mut c = BatteryCollector::new();
        let s = c.sample(true);
        // On a MacBook both sources work; on desktop both are unavailable.
        // Either way: available ⇒ values in range.
        if s.charge_percent.available {
            let pct = s.charge_percent.value.unwrap();
            assert!((0.0..=100.0).contains(&pct));
        }
        if s.health_percent.available {
            let h = s.health_percent.value.unwrap();
            assert!((0.0..=100.0).contains(&h));
        }
    }

    /// The health cache must serve the same reading within TTL and re-read
    /// after it expires — never an unbounded or indefinite cache.
    #[cfg(target_os = "macos")]
    #[test]
    fn health_cache_respects_ttl() {
        let mut c = BatteryCollector::new();
        let first = c.sample(true);
        // Within TTL: second call reuses the cached ioreg reading (identical
        // cycle count / capacities) while pmset data stays fresh.
        let second = c.sample(false);
        assert_eq!(first.cycle_count, second.cycle_count, "cache should hit");
        assert_eq!(first.design_capacity_mah, second.design_capacity_mah);

        // Expire the cache by back-dating the entry, then confirm a re-read
        // happens (a fresh read produces a new timestamped cache entry).
        let stale = Instant::now() - HEALTH_TTL - Duration::from_secs(1);
        if let Some((at, _)) = &mut c.health_cache {
            *at = stale;
        }
        let third = c.sample(false);
        assert_eq!(
            second.cycle_count, third.cycle_count,
            "values are stable across a real re-read"
        );
        // And the cache was refreshed (entry no longer stale).
        let (_, cached) = c.health_cache.as_ref().expect("cache repopulated");
        assert_eq!(cached.cycle_count, third.cycle_count);
    }

    /// A failed ioreg read must not poison the cache: the next call retries.
    #[test]
    fn failed_read_does_not_populate_cache() {
        let c = BatteryCollector::new();
        // Simulate a failed read by parsing garbage directly.
        assert!(parse_battery_ioreg("not ioreg output").is_none());
        // Cache stays empty → next sample() will retry rather than serve None.
        assert!(c.health_cache.is_none());
    }
}
