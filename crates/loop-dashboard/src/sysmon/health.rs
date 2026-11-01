//! Consolidated system-health document: the integration boundary for
//! consumers that need the machine's state as structured evidence rather
//! than scraped HTML.
//!
//! `GET /api/sys/health` serializes [`HealthDoc`]. Everything in it is
//! derived from the live sampler's snapshot and history buffers — never
//! fabricated, never hardcoded "healthy". Each domain reports its own
//! availability with the reason it could not be read, warnings carry the
//! observation that triggered them, and history metadata tells a consumer
//! exactly which time range it may honestly analyze.
//!
//! Status semantics (kept deliberately conservative):
//! * `unknown`  — the sampler has not produced a sample yet.
//! * `stale`    — the newest sample is older than the freshness threshold
//!                (max of 5 s and 3× the sampling interval).
//! * `degraded` — fresh data, but at least one evidence-based warning is
//!                present (kernel memory pressure, critically low battery).
//! * `healthy`  — fresh data and no warnings.
//!
//! A single high CPU reading is intentionally NOT a warning: builds and
//! compiles legitimately saturate cores. Only kernel- or OS-judged signals
//! (memory pressure level, battery state) promote the status, so consumers
//! can trust `degraded` means something.

use serde::Serialize;

use crate::sysmon::{History, SysSnapshot};

/// Freshness floor: below this cadence a sample can never be considered
/// stale, so short intervals don't produce flapping statuses.
const FRESHNESS_FLOOR_MS: i64 = 5_000;

/// Freshness multiplier: a sample is stale past 3× the expected cadence.
const FRESHNESS_MULTIPLE: i64 = 3;

/// Top-level health document served by `/api/sys/health`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthDoc {
    /// True when the sampler is producing clearly-labeled synthetic data.
    pub demo: bool,
    /// Aggregate status: `healthy` | `degraded` | `stale` | `unknown`.
    pub status: &'static str,
    /// Unix ms of the newest sample (0 before the first sample).
    pub sampled_at_ms: i64,
    /// Sampling cadence in ms.
    pub interval_ms: u64,
    /// Age of the newest sample in ms (absent before the first sample).
    pub age_ms: Option<i64>,
    /// Whether the newest sample is within the freshness threshold.
    pub fresh: bool,
    /// Samples taken since the monitor started.
    pub sample_count: u64,
    /// Machine identity.
    pub host: crate::sysmon::host::HostInfo,
    /// Per-domain availability and headline readings.
    pub metrics: MetricsDoc,
    /// Evidence-based warnings observed in the newest sample.
    pub warnings: Vec<Warning>,
    /// What historical series exist and the range they cover.
    pub history: HistoryMeta,
}

/// Per-domain availability summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsDoc {
    pub cpu: CpuHealth,
    pub memory: MemoryHealth,
    pub disk: DiskHealth,
    pub network: NetworkHealth,
    pub gpu: DomainHealth,
    pub battery: BatteryHealth,
    pub processes: ProcessesHealth,
}

/// CPU domain. Utilization is always computed once sampling runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuHealth {
    pub available: bool,
    pub usage_percent: f64,
    pub load_average_one: Option<f64>,
    pub logical_cores: Option<u32>,
}

/// Memory domain.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHealth {
    pub available: bool,
    pub used_bytes: u64,
    pub total_bytes: u64,
    /// Kernel memory-pressure level when reported (macOS: 1 nominal,
    /// 2 warning, 3 critical).
    pub pressure_level: Option<u8>,
}

/// Disk domain.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskHealth {
    pub available: bool,
    pub volume_count: usize,
    /// Whether the I/O rate window is valid (false on the first sample).
    pub io_valid: bool,
}

/// Network domain.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkHealth {
    pub available: bool,
    pub interface_count: usize,
}

/// Generic domain availability with an honest reason when unavailable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainHealth {
    pub available: bool,
    /// The reading when available (utilization % / charge %).
    pub value: Option<f64>,
    /// Why the domain could not be read; present only when unavailable.
    pub reason: Option<&'static str>,
}

/// Battery domain with charge/state when present.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHealth {
    pub available: bool,
    pub charge_percent: Option<f64>,
    pub status: Option<String>,
    pub reason: Option<&'static str>,
}

/// Process-table domain.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessesHealth {
    pub available: bool,
    pub count: usize,
}

/// One evidence-based warning from the newest sample.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// Stable machine-readable code (e.g. `memory-pressure`).
    pub code: &'static str,
    /// Human-readable statement of the observed evidence.
    pub message: String,
    /// Unix ms of the sample the warning was observed in.
    pub observed_at_ms: i64,
}

/// Metadata about the retained history: what a consumer may analyze.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMeta {
    /// Maximum points retained per series (ring-buffer capacity).
    pub capacity_per_series: usize,
    /// Per-series point counts and covered time ranges.
    pub series: Vec<SeriesMeta>,
}

/// One series' extent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMeta {
    /// Series name, matching `/api/sys/history` keys.
    pub name: &'static str,
    pub points: usize,
    /// Oldest retained timestamp (unix ms); absent when empty.
    pub oldest_ms: Option<i64>,
    /// Newest retained timestamp (unix ms); absent when empty.
    pub newest_ms: Option<i64>,
}

/// Compute the health document from a snapshot, its history, and the
/// current time. Pure: no I/O, fully deterministic given inputs.
pub fn compute(snapshot: &SysSnapshot, history: &History, now_ms: i64, demo: bool) -> HealthDoc {
    let age_ms = if snapshot.sampled_at_ms > 0 {
        Some((now_ms - snapshot.sampled_at_ms).max(0))
    } else {
        None
    };
    let threshold = freshness_threshold_ms(snapshot.interval_ms);
    let fresh = match age_ms {
        Some(age) => age <= threshold,
        None => false,
    };

    let warnings = collect_warnings(snapshot);

    let status = if snapshot.sample_count == 0 {
        "unknown"
    } else if !fresh {
        "stale"
    } else if !warnings.is_empty() {
        "degraded"
    } else {
        "healthy"
    };

    let m = &snapshot.memory;
    let c = &snapshot.cpu;
    let g = &snapshot.gpu;
    let b = &snapshot.battery;

    HealthDoc {
        demo,
        status,
        sampled_at_ms: snapshot.sampled_at_ms,
        interval_ms: snapshot.interval_ms,
        age_ms,
        fresh,
        sample_count: snapshot.sample_count,
        host: snapshot.host.clone(),
        metrics: MetricsDoc {
            cpu: CpuHealth {
                available: snapshot.sample_count > 0,
                usage_percent: c.usage_percent,
                load_average_one: c.load_average.as_ref().map(|l| l.one),
                logical_cores: c.logical_cores,
            },
            memory: MemoryHealth {
                available: m.total_bytes > 0,
                used_bytes: m.used_bytes,
                total_bytes: m.total_bytes,
                pressure_level: m.pressure_level,
            },
            disk: DiskHealth {
                available: !snapshot.volumes.is_empty(),
                volume_count: snapshot.volumes.len(),
                io_valid: snapshot.disk_io.valid,
            },
            network: NetworkHealth {
                available: !snapshot.interfaces.is_empty(),
                interface_count: snapshot.interfaces.len(),
            },
            gpu: DomainHealth {
                available: g.utilization_percent.available,
                value: g.utilization_percent.value,
                reason: unavailable(
                    &g.utilization_percent.available,
                    g.utilization_percent.unavailable_reason,
                ),
            },
            battery: BatteryHealth {
                available: b.charge_percent.available,
                charge_percent: b.charge_percent.value,
                status: b.status.value.clone(),
                reason: unavailable(
                    &b.charge_percent.available,
                    b.charge_percent.unavailable_reason,
                ),
            },
            processes: ProcessesHealth {
                available: snapshot.sample_count > 0 && !snapshot.processes.is_empty(),
                count: snapshot.processes.len(),
            },
        },
        warnings,
        history: history_meta(history),
    }
}

/// Freshness threshold for a given cadence.
fn freshness_threshold_ms(interval_ms: u64) -> i64 {
    (interval_ms as i64 * FRESHNESS_MULTIPLE).max(FRESHNESS_FLOOR_MS)
}

fn unavailable(available: &bool, reason: Option<&'static str>) -> Option<&'static str> {
    if *available {
        None
    } else {
        reason.or(Some("unavailable"))
    }
}

/// Evidence-based warnings from the newest sample. Conservative by design:
/// only OS-judged signals qualify (see module docs).
fn collect_warnings(s: &SysSnapshot) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let at = s.sampled_at_ms;

    // Kernel memory pressure (macOS kern.memorystatus_vm_pressure_level).
    match s.memory.pressure_level {
        Some(2) => warnings.push(Warning {
            code: "memory-pressure",
            message: "kernel memory pressure at warning level (2)".into(),
            observed_at_ms: at,
        }),
        Some(level @ 3..) => warnings.push(Warning {
            code: "memory-pressure",
            message: format!("kernel memory pressure at critical level ({level})"),
            observed_at_ms: at,
        }),
        _ => {}
    }

    // Battery: low charge while discharging is an objective power risk.
    if let (Some(charge), Some(status)) = (
        s.battery.charge_percent.value,
        s.battery.status.value.as_deref(),
    ) {
        let discharging = status.to_lowercase().contains("discharg");
        if discharging && charge < 20.0 {
            warnings.push(Warning {
                code: "battery-low",
                message: format!(
                    "battery at {:.0}% while discharging — connect power soon",
                    charge
                ),
                observed_at_ms: at,
            });
        }
    }

    warnings
}

/// Per-series metadata over the retained history buffers.
fn history_meta(h: &History) -> HistoryMeta {
    use crate::sysmon::history::DEFAULT_CAPACITY;

    let extent = |points: &[crate::sysmon::history::Point]| -> (usize, Option<i64>, Option<i64>) {
        if points.is_empty() {
            (0, None, None)
        } else {
            (
                points.len(),
                points.first().map(|p| p.t),
                points.last().map(|p| p.t),
            )
        }
    };

    let mut series = Vec::with_capacity(11);
    let (n, lo, hi) = extent(&h.cpu_percent);
    series.push(SeriesMeta {
        name: "cpuPercent",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.memory_used_bytes);
    series.push(SeriesMeta {
        name: "memoryUsedBytes",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.swap_used_bytes);
    series.push(SeriesMeta {
        name: "swapUsedBytes",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.disk_read_per_sec);
    series.push(SeriesMeta {
        name: "diskReadPerSec",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.disk_write_per_sec);
    series.push(SeriesMeta {
        name: "diskWritePerSec",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.net_recv_per_sec);
    series.push(SeriesMeta {
        name: "netRecvPerSec",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.net_sent_per_sec);
    series.push(SeriesMeta {
        name: "netSentPerSec",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.gpu_util_percent);
    series.push(SeriesMeta {
        name: "gpuUtilPercent",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });
    let (n, lo, hi) = extent(&h.battery_charge_percent);
    series.push(SeriesMeta {
        name: "batteryChargePercent",
        points: n,
        oldest_ms: lo,
        newest_ms: hi,
    });

    HistoryMeta {
        capacity_per_series: DEFAULT_CAPACITY,
        series,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysmon::battery::BatterySnapshot;
    use crate::sysmon::{History, SysMonitor};

    fn doc_from(monitor: &SysMonitor, now_ms: i64) -> HealthDoc {
        let snap = monitor.snapshot();
        let hist = monitor.history();
        compute(&snap, &hist, now_ms, false)
    }

    #[test]
    fn empty_monitor_is_unknown_not_healthy() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let d = doc_from(&m, 1_000_000);
        assert_eq!(d.status, "unknown");
        assert_eq!(d.sample_count, 0);
        assert!(!d.fresh);
        // Domains report unavailable honestly, never zero-as-data.
        assert!(!d.metrics.memory.available);
        assert!(!d.metrics.disk.available);
        assert!(!d.metrics.processes.available);
    }

    #[test]
    fn fresh_real_sample_is_healthy_with_history_extent() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let mut src = crate::sysmon::SampleSource::real();
        m.sample_with(&mut src);
        let now = chrono::Utc::now().timestamp_millis();
        let d = doc_from(&m, now);
        assert_eq!(d.status, "healthy", "warnings were: {:?}", d.warnings);
        assert!(d.fresh);
        assert!(d.metrics.cpu.available);
        assert!(d.metrics.memory.available);
        assert!(d.metrics.processes.available);
        // History metadata exposes real extents for the AI boundary.
        let cpu = d
            .history
            .series
            .iter()
            .find(|s| s.name == "cpuPercent")
            .unwrap();
        assert!(cpu.points >= 1);
        assert!(cpu.oldest_ms.is_some() && cpu.newest_ms.is_some());
        assert!(cpu.newest_ms.unwrap() >= cpu.oldest_ms.unwrap());
        assert!(d.history.capacity_per_series > 0);
    }

    #[test]
    fn aged_sample_is_stale() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let mut src = crate::sysmon::SampleSource::real();
        m.sample_with(&mut src);
        // 60 s later at a 2 s cadence: far past the 6 s threshold.
        let now = chrono::Utc::now().timestamp_millis() + 60_000;
        let d = doc_from(&m, now);
        assert_eq!(d.status, "stale");
        assert!(!d.fresh);
        assert!(d.age_ms.unwrap() >= 60_000);
    }

    #[test]
    fn memory_pressure_warning_degrades() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let mut src = crate::sysmon::SampleSource::real();
        m.sample_with(&mut src);
        {
            let mut inner = m.inner_for_tests();
            inner.snapshot.memory.pressure_level = Some(2);
        }
        let now = chrono::Utc::now().timestamp_millis();
        let d = doc_from(&m, now);
        assert_eq!(d.status, "degraded");
        assert_eq!(d.warnings.len(), 1);
        assert_eq!(d.warnings[0].code, "memory-pressure");
        assert!(d.warnings[0].message.contains("warning"));
        assert_eq!(d.warnings[0].observed_at_ms, d.sampled_at_ms);
    }

    #[test]
    fn low_discharging_battery_warns_but_full_does_not() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let mut src = crate::sysmon::SampleSource::real();
        m.sample_with(&mut src);
        {
            let mut inner = m.inner_for_tests();
            inner.snapshot.battery = BatterySnapshot {
                charge_percent: crate::sysmon::metrics::Metric::ok(12.0, "%", "battery charge"),
                status: crate::sysmon::metrics::Metric::ok(
                    "Discharging".to_string(),
                    "",
                    "power source",
                ),
                ..BatterySnapshot::default()
            };
        }
        let now = chrono::Utc::now().timestamp_millis();
        let d = doc_from(&m, now);
        assert_eq!(d.status, "degraded");
        assert!(d.warnings.iter().any(|w| w.code == "battery-low"));

        // Fully charged on AC: no warning.
        {
            let mut inner = m.inner_for_tests();
            inner.snapshot.battery = BatterySnapshot {
                charge_percent: crate::sysmon::metrics::Metric::ok(100.0, "%", "battery charge"),
                status: crate::sysmon::metrics::Metric::ok(
                    "Fully Charged".to_string(),
                    "",
                    "power source",
                ),
                ..BatterySnapshot::default()
            };
        }
        let d2 = doc_from(&m, now);
        assert_eq!(d2.status, "healthy", "{:?}", d2.warnings);
    }

    #[test]
    fn unavailable_domains_carry_reasons() {
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let mut src = crate::sysmon::SampleSource::real();
        m.sample_with(&mut src);
        let d = doc_from(&m, chrono::Utc::now().timestamp_millis());
        // Whatever the GPU state, availability and reason must agree.
        let g = &d.metrics.gpu;
        if g.available {
            assert!(g.value.is_some());
            assert!(g.reason.is_none());
        } else {
            assert!(g.reason.is_some(), "unavailable GPU must explain why");
        }
        // Same contract for battery.
        let b = &d.metrics.battery;
        if !b.available {
            assert!(b.reason.is_some(), "unavailable battery must explain why");
        }
    }

    /// The threshold math is the contract both the endpoint and any consumer
    /// rely on: floor 5 s, 3× cadence beyond that.
    #[test]
    fn freshness_threshold_math() {
        assert_eq!(freshness_threshold_ms(500), 5_000);
        assert_eq!(freshness_threshold_ms(2_000), 6_000);
        assert_eq!(freshness_threshold_ms(10_000), 30_000);
    }

    /// History metadata must enumerate every series the history API serves,
    /// so a consumer never discovers series by trial and error.
    #[test]
    fn history_meta_lists_all_series() {
        // An empty monitor yields empty (but well-formed) history buffers.
        let m = SysMonitor::with_empty_snapshot(std::time::Duration::from_secs(2));
        let h = m.history();
        let meta = history_meta(&h);
        assert_eq!(meta.series.len(), 9);
        assert!(meta.series.iter().all(|s| s.points == 0));
        assert!(meta.series.iter().all(|s| s.oldest_ms.is_none()));
    }
}
