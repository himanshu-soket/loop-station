//! Metric availability model.
//!
//! Every system metric is either measured or honestly unavailable — this
//! dashboard never fabricates a value. [`Metric`] carries the value, its unit,
//! a short human explanation, and (when missing) the reason it cannot be read.

use serde::Serialize;

/// A possibly-unavailable metric reading.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric<T> {
    /// `true` when `value` holds a real measurement.
    pub available: bool,
    /// The measurement, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    /// Unit label for display (e.g. `"%"`, `"GB"`, `"MB/s"`).
    pub unit: &'static str,
    /// One-line explanation of what this metric means.
    pub description: &'static str,
    /// Why the metric could not be read; present only when unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<&'static str>,
}

impl<T> Metric<T> {
    /// A successful reading.
    pub const fn ok(value: T, unit: &'static str, description: &'static str) -> Self {
        Self {
            available: true,
            value: Some(value),
            unit,
            description,
            unavailable_reason: None,
        }
    }

    /// An unavailable reading with an honest reason.
    pub const fn unavailable(
        unit: &'static str,
        description: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            available: false,
            value: None,
            unit,
            description,
            unavailable_reason: Some(reason),
        }
    }

    /// Read the value if present.
    #[allow(dead_code)] // convenience accessor used by tests
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Map the inner value while preserving availability metadata.
    #[allow(dead_code)] // used by tests; handy for unit conversion later
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Metric<U> {
        match self.value {
            Some(v) => Metric {
                available: self.available,
                value: Some(f(v)),
                unit: self.unit,
                description: self.description,
                unavailable_reason: self.unavailable_reason,
            },
            None => Metric {
                available: false,
                value: None,
                unit: self.unit,
                description: self.description,
                unavailable_reason: self.unavailable_reason,
            },
        }
    }
}

impl<T: Copy> Metric<T> {
    /// Copy out the value if present.
    pub fn copied(&self) -> Option<T> {
        self.value
    }
}

impl<T: Default> Metric<T> {
    /// Value or the type's default (for chart series where gaps are acceptable).
    #[allow(dead_code)] // reserved for chart rendering of unavailable points
    pub fn unwrap_or_default(self) -> T {
        self.value.unwrap_or_default()
    }
}

/// Extracts `"Key"=<number>` or `"Key" = <number>` from ioreg text.
///
/// Handles both shapes seen in real output:
/// * nested dict on one line: `"Device Utilization %"=37,...`
/// * standalone entry:       `"Device Utilization %" = 37`
pub fn parse_ioreg_number_any(text: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\"");
    for line in text.lines() {
        let Some(pos) = line.find(&needle) else {
            continue;
        };
        let after_key = &line[pos + needle.len()..];
        // skip whitespace and the '='
        let rest = after_key.trim_start().strip_prefix('=')?.trim_start();
        // take the leading numeric token (stop at , ; } or ")
        let token: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
            .collect();
        if token.is_empty() {
            continue;
        }
        if let Ok(v) = token.parse::<f64>() {
            if v.is_finite() {
                return Some(v);
            }
        }
    }
    None
}

/// Extracts `"Key" = "Value"` string pairs.
pub fn parse_ioreg_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in text.lines() {
        let Some(pos) = line.find(&needle) else {
            continue;
        };
        let after_key = &line[pos + needle.len()..];
        let rest = after_key.trim_start().strip_prefix('=')?.trim_start();
        let inner = rest.strip_prefix('"')?;
        let end = inner.find('"')?;
        return Some(inner[..end].to_string());
    }
    None
}

/// Per-core CPU usage derived from cumulative tick counters.
///
/// Pure function over two snapshots of Mach `processor_cpu_load_info`; unit
/// tested independently of the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreDelta {
    /// Aggregate user+system+nice percentage across all cores (0–100).
    pub total_percent: f64,
    /// User-mode percentage (0–100).
    pub user_percent: f64,
    /// Kernel/system-mode percentage (0–100).
    pub system_percent: f64,
    /// Idle percentage (0–100).
    pub idle_percent: f64,
}

/// Compute per-core percentages between two tick snapshots.
///
/// Each snapshot is `[user, system, idle, nice]` cumulative ticks. Returns
/// `None` when the counter delta is zero (first sample, or suspend/resume).
pub fn core_delta(prev: [u64; 4], cur: [u64; 4]) -> Option<CoreDelta> {
    let d: Vec<i128> = prev
        .iter()
        .zip(cur.iter())
        .map(|(a, b)| i128::from(*b) - i128::from(*a))
        .collect();
    let total: i128 = d.iter().sum::<i128>().max(0);
    if total == 0 {
        return None;
    }
    let pct = |v: i128| (v.max(0) as f64 / total as f64) * 100.0;
    let user = pct(d[0] + d[3]); // nice is user-mode by definition
    let system = pct(d[1]);
    let idle = pct(d[2]);
    Some(CoreDelta {
        total_percent: (user + system).min(100.0),
        user_percent: user,
        system_percent: system,
        idle_percent: idle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_ok_roundtrip() {
        let m = Metric::ok(42.5, "%", "CPU usage");
        assert!(m.available);
        assert_eq!(m.copied(), Some(42.5));
        assert_eq!(m.unavailable_reason, None);
    }

    #[test]
    fn ioreg_number_standalone_format() {
        assert_eq!(
            parse_ioreg_number_any("\"Device Utilization %\" = 37", "Device Utilization %"),
            Some(37.0)
        );
        assert_eq!(
            parse_ioreg_number_any("   \"Cycle Count\" = 129,", "Cycle Count"),
            Some(129.0)
        );
    }

    #[test]
    fn ioreg_number_inline_dict_format() {
        // Real AGXAccelerator output: everything on one line inside a dict.
        let line = r#"  |   "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=10466148352,"Tiler Utilization %"=19,"Renderer Utilization %"=37,"Device Utilization %"=37}"#;
        assert_eq!(
            parse_ioreg_number_any(line, "Device Utilization %"),
            Some(37.0)
        );
        assert_eq!(
            parse_ioreg_number_any(line, "Tiler Utilization %"),
            Some(19.0)
        );
        assert_eq!(
            parse_ioreg_number_any(line, "In use system memory"),
            None // key absent (only "(driver)" variant present)
        );
        // The same key with a suffix must not be confused with the plain key:
        assert_eq!(
            parse_ioreg_number_any(line, "In use system memory (driver)"),
            Some(0.0)
        );
    }

    #[test]
    fn ioreg_number_rejects_garbage() {
        assert_eq!(parse_ioreg_number_any("\"x\" = yes", "x"), None);
        assert_eq!(parse_ioreg_number_any("no key here", "x"), None);
        assert_eq!(parse_ioreg_number_any("\"x\" =", "x"), None);
    }

    #[test]
    fn ioreg_string_extracts_model() {
        assert_eq!(
            parse_ioreg_string("  |   \"model\" = \"Apple M5\"", "model"),
            Some("Apple M5".to_string())
        );
        assert_eq!(parse_ioreg_string("nothing", "model"), None);
    }

    #[test]
    fn metric_unavailable_has_reason() {
        let m: Metric<f64> = Metric::unavailable("%", "GPU temperature", "requires root");
        assert!(!m.available);
        assert_eq!(m.value, None);
        assert_eq!(m.unavailable_reason, Some("requires root"));
    }

    #[test]
    fn metric_map_preserves_metadata() {
        let m = Metric::ok(1024u64, "B", "bytes").map(|b| b as f64 / 1024.0);
        assert!(m.available);
        assert_eq!(m.value, Some(1.0));
        let u: Metric<u64> = Metric::unavailable("B", "bytes", "n/a");
        let u2 = u.map(|b| b + 1);
        assert!(!u2.available && u2.value.is_none());
    }

    #[test]
    fn core_delta_half_busy_core() {
        // 100 user ticks, 0 system, 100 idle → 50% busy
        let d = core_delta([0; 4], [100, 0, 100, 0]).unwrap();
        assert!((d.total_percent - 50.0).abs() < 1e-9);
        assert!((d.user_percent - 50.0).abs() < 1e-9);
        assert!((d.system_percent - 0.0).abs() < 1e-9);
        assert!((d.idle_percent - 50.0).abs() < 1e-9);
    }

    #[test]
    fn core_delta_counts_nice_as_user() {
        let d = core_delta([0; 4], [25, 25, 0, 50]).unwrap();
        assert!((d.user_percent - 75.0).abs() < 1e-9); // 25 user + 50 nice
        assert!((d.system_percent - 25.0).abs() < 1e-9);
        assert!((d.total_percent - 100.0).abs() < 1e-9);
    }

    #[test]
    fn core_delta_zero_total_is_none() {
        assert!(core_delta([5, 5, 5, 5], [5, 5, 5, 5]).is_none());
        assert!(core_delta([5, 5, 5, 5], [4, 5, 5, 5]).is_none()); // clamped to 0
    }

    #[test]
    fn core_delta_never_exceeds_100() {
        // pathological counters must not produce >100%
        let d = core_delta([0; 4], [u64::MAX / 2, u64::MAX / 2, 0, 0]).unwrap();
        assert!(d.total_percent <= 100.0);
    }

    #[test]
    fn core_delta_sums_to_100() {
        let d = core_delta([10, 20, 30, 40], [110, 120, 130, 140]).unwrap();
        let sum = d.user_percent + d.system_percent + d.idle_percent;
        assert!((sum - 100.0).abs() < 1e-9);
    }
}
