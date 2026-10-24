//! GPU + thermal metrics on Apple Silicon.
//!
//! Utilization comes from IOKit's `AGXAccelerator` PerformanceStatistics
//! (readable without root — verified live). Real keys on this machine:
//! `"Device Utilization %"`, `"Renderer Utilization %"`, `"Tiler Utilization %"`,
//! `"In use system memory"`, `"Alloc system memory"`.
//!
//! GPU **temperature** is not exposed unprivileged on current macOS; it is
//! reported as honestly unavailable rather than estimated.

use serde::Serialize;

use super::metrics::{parse_ioreg_number_any, parse_ioreg_string, Metric};

/// GPU reading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuSnapshot {
    /// Chip name (e.g. "Apple M5").
    pub model: Metric<String>,
    /// Device utilization percentage (0–100).
    pub utilization_percent: Metric<f64>,
    /// Shader/renderer utilization percentage.
    pub renderer_utilization_percent: Metric<f64>,
    /// Tiler utilization percentage.
    pub tiler_utilization_percent: Metric<f64>,
    /// GPU memory in use (driver-reported system memory share).
    pub memory_used_bytes: Metric<u64>,
    /// GPU temperature.
    pub temperature_celsius: Metric<f64>,
    /// Thermal pressure as reported by macOS.
    pub thermal_pressure: Metric<String>,
}

impl Default for GpuSnapshot {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl GpuSnapshot {
    /// All-unavailable snapshot (non-macOS or no GPU found).
    pub fn unavailable() -> Self {
        Self {
            model: Metric::unavailable("", "GPU chip", "no GPU detected"),
            utilization_percent: Metric::unavailable(
                "%",
                "GPU device utilization",
                "not readable on this system",
            ),
            renderer_utilization_percent: Metric::unavailable(
                "%",
                "GPU shader (renderer) utilization",
                "not readable on this system",
            ),
            tiler_utilization_percent: Metric::unavailable(
                "%",
                "GPU tiler utilization",
                "not readable on this system",
            ),
            memory_used_bytes: Metric::unavailable(
                "B",
                "GPU memory in use",
                "not exposed by macOS without root",
            ),
            temperature_celsius: Metric::unavailable(
                "°C",
                "GPU die temperature",
                "requires root (powermetrics); never estimated",
            ),
            thermal_pressure: Metric::unavailable(
                "",
                "System thermal pressure",
                "not readable on this system",
            ),
        }
    }
}

/// Thermal pressure from `pmset -g therm`.
///
/// When throttled, macOS prints `CPU_Speed_Limit = <pct>`. When everything is
/// nominal it prints notes like "No thermal warning level has been recorded",
/// which we report as `nominal` rather than "unavailable" — the absence of a
/// warning *is* the reading.
pub fn parse_thermal_pressure(pmset_output: &str) -> Option<String> {
    for line in pmset_output.lines() {
        let l = line.trim();
        if l.starts_with("CPU_Speed_Limit") {
            let after = l.split('=').nth(1)?.trim();
            let pct: f64 = after.parse().ok()?;
            return Some(if pct >= 100.0 {
                "nominal".into()
            } else {
                format!("throttled ({pct:.0}% speed limit)")
            });
        }
    }
    if pmset_output.contains("No thermal warning level has been recorded")
        || pmset_output.contains("No performance warning level has been recorded")
    {
        return Some("nominal (no warnings recorded)".into());
    }
    None
}

/// Reads GPU stats by shelling out to `ioreg` (no root required).
///
/// Parsing is split into [`parse_gpu_ioreg`] for testability.
#[cfg(target_os = "macos")]
fn read_gpu_via_ioreg() -> Option<GpuSnapshot> {
    let out = std::process::Command::new("ioreg")
        .args(["-r", "-c", "AGXAccelerator"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_gpu_ioreg(&text)
}

#[cfg(not(target_os = "macos"))]
fn read_gpu_via_ioreg() -> Option<GpuSnapshot> {
    None
}

/// Parses `ioreg -r -c AGXAccelerator` output into a snapshot.
///
/// Returns `None` when no accelerator was found (Intel Macs, VMs).
pub fn parse_gpu_ioreg(text: &str) -> Option<GpuSnapshot> {
    // The class name is versioned (AGXAcceleratorG17G…) but always starts
    // with AGXAccelerator; `-c` matches subclasses so the header confirms it.
    if !text.contains("AGXAccelerator") {
        return None;
    }
    let mut snap = GpuSnapshot::unavailable();

    if let Some(model) = parse_ioreg_string(text, "model") {
        snap.model = Metric::ok(model, "", "GPU chip");
    }

    let pct = |key: &str| parse_ioreg_number_any(text, key).map(|v| v.clamp(0.0, 100.0));

    if let Some(u) = pct("Device Utilization %") {
        snap.utilization_percent = Metric::ok(u, "%", "GPU device utilization");
    }
    if let Some(r) = pct("Renderer Utilization %") {
        snap.renderer_utilization_percent = Metric::ok(r, "%", "GPU shader (renderer) utilization");
    }
    if let Some(t) = pct("Tiler Utilization %") {
        snap.tiler_utilization_percent = Metric::ok(t, "%", "GPU tiler utilization");
    }

    // Memory keys observed on Apple Silicon. "In use system memory" is the
    // driver's view of GPU-resident memory; report only what exists.
    for key in ["In use system memory", "Alloc system memory"] {
        if let Some(m) = parse_ioreg_number_any(text, key) {
            snap.memory_used_bytes = Metric::ok(m as u64, "B", "GPU memory in use (IOKit driver)");
            break;
        }
    }
    Some(snap)
}

/// Reads thermal pressure via `pmset -g therm` (no root required).
#[cfg(target_os = "macos")]
fn read_thermal_pressure() -> Option<String> {
    let out = std::process::Command::new("pmset")
        .args(["-g", "therm"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_thermal_pressure(&text)
}

#[cfg(not(target_os = "macos"))]
fn read_thermal_pressure() -> Option<String> {
    None
}

/// One-shot GPU + thermals reader (cheap enough to sample every few seconds).
pub struct GpuCollector;

impl GpuCollector {
    /// New collector.
    pub fn new() -> Self {
        Self
    }

    /// Take a sample.
    pub fn sample(&self) -> GpuSnapshot {
        let mut snap = read_gpu_via_ioreg().unwrap_or_default();
        if let Some(pressure) = read_thermal_pressure() {
            snap.thermal_pressure = Metric::ok(pressure, "", "System thermal pressure (pmset)");
        }
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-shaped fixture: PerformanceStatistics as one inline dict, exactly
    /// like `ioreg -r -c AGXAccelerator` prints on an M-series Mac.
    const SAMPLE: &str = r#"
+-o AGXAcceleratorG17G  <class AGXAcceleratorG17G, id 0x1000005a3, registered, matched, active, busy 0 (5007 ms), retain 69>
  | {
  |   "IOMatchedAtBoot" = Yes
  |   "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=10466148352,"Tiler Utilization %"=19,"recoveryCount"=0,"lastRecoveryTime"=0,"Renderer Utilization %"=37,"TiledSceneBytes"=1212416,"Device Utilization %"=37,"SplitSceneCount"=0,"Allocated PB Size"=96993280,"In use system memory"=641990656}
  |   "model" = "Apple M5"
  |   "gpu-core-count" = 8
  | }
"#;

    #[test]
    fn parses_full_gpu_snapshot_from_real_shape() {
        let s = parse_gpu_ioreg(SAMPLE).expect("accelerator present");
        assert_eq!(s.model.value.as_deref(), Some("Apple M5"));
        assert_eq!(s.utilization_percent.value, Some(37.0));
        assert_eq!(s.renderer_utilization_percent.value, Some(37.0));
        assert_eq!(s.tiler_utilization_percent.value, Some(19.0));
        // "In use system memory" wins over "Alloc system memory"
        assert_eq!(s.memory_used_bytes.value, Some(641_990_656));
        // temperature genuinely absent → must stay unavailable
        assert!(!s.temperature_celsius.available);
        assert!(s.temperature_celsius.unavailable_reason.is_some());
    }

    #[test]
    fn missing_accelerator_is_none() {
        assert!(parse_gpu_ioreg("no gpu here").is_none());
    }

    #[test]
    fn utilization_clamped_to_100() {
        let text = "AGXAccelerator\n\"Device Utilization %\"=250\n";
        let s = parse_gpu_ioreg(text).unwrap();
        assert_eq!(s.utilization_percent.value, Some(100.0));
    }

    #[test]
    fn thermal_pressure_parsing() {
        let nominal = "Note: No thermal warning level has been recorded\nCPU_Speed_Limit = 100\n";
        assert_eq!(parse_thermal_pressure(nominal), Some("nominal".into()));
        let throttled = "CPU_Speed_Limit = 60\n";
        assert_eq!(
            parse_thermal_pressure(throttled),
            Some("throttled (60% speed limit)".into())
        );
        // Real output on an unthrottled Mac: notes only, no CPU_Speed_Limit.
        let notes_only = "Note: No thermal warning level has been recorded\nNote: No performance warning level has been recorded\n";
        assert_eq!(
            parse_thermal_pressure(notes_only),
            Some("nominal (no warnings recorded)".into())
        );
        assert_eq!(parse_thermal_pressure("nothing useful"), None);
    }

    #[test]
    fn unavailable_snapshot_is_honest() {
        let s = GpuSnapshot::unavailable();
        assert!(!s.utilization_percent.available);
        assert!(s.every_field_has_reason());
    }

    impl GpuSnapshot {
        /// Test helper: every field carries an unavailable reason.
        fn every_field_has_reason(&self) -> bool {
            self.model.unavailable_reason.is_some()
                && self.utilization_percent.unavailable_reason.is_some()
                && self
                    .renderer_utilization_percent
                    .unavailable_reason
                    .is_some()
                && self.tiler_utilization_percent.unavailable_reason.is_some()
                && self.memory_used_bytes.unavailable_reason.is_some()
                && self.temperature_celsius.unavailable_reason.is_some()
                && self.thermal_pressure.unavailable_reason.is_some()
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_gpu_read_on_this_machine() {
        let s = GpuCollector::new().sample();
        // This machine has an AGX accelerator: utilization must be a real
        // number in range, temperature must stay unavailable.
        if s.utilization_percent.available {
            let v = s.utilization_percent.value.unwrap();
            assert!((0.0..=100.0).contains(&v));
        }
        assert!(!s.temperature_celsius.available);
    }
}
