//! Battery metrics via `ioreg` (AppleSmartBattery) — no root required.
//!
//! Cycle count, design/max capacity (health), charge percentage and charging
//! state are all readable unprivileged. Desktop Macs report the whole domain
//! as unavailable rather than pretending.

use serde::Serialize;

use crate::sysmon::metrics::{parse_ioreg_number_any, Metric};

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

    if let Some(cycles) = number("Cycle Count") {
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
pub struct BatteryCollector;

impl BatteryCollector {
    /// New collector.
    pub fn new() -> Self {
        Self
    }

    /// Take a sample by combining pmset + ioreg.
    #[cfg(target_os = "macos")]
    pub fn sample(&self) -> BatterySnapshot {
        let pmset = std::process::Command::new("pmset")
            .args(["-g", "batt"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .and_then(|t| parse_pmset_batt(&t));
        let ioreg = std::process::Command::new("ioreg")
            .args(["-r", "-c", "AppleSmartBattery"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .and_then(|t| parse_battery_ioreg(&t));
        merge_snapshots(pmset, ioreg)
    }

    /// Non-macOS: unavailable.
    #[cfg(not(target_os = "macos"))]
    pub fn sample(&self) -> BatterySnapshot {
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
  |   "Cycle Count" = 129
  |   "DesignCapacity" = 4629
  |   "AppleRawMaxCapacity" = 4458
  |   "MaxCapacity" = 100
  |   "IsCharging" = No
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
        let s = BatteryCollector::new().sample();
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
}
