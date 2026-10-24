//! CPU metrics via Mach `host_processor_info` + sysinfo.
//!
//! The user/system/idle breakdown is not exposed by sysinfo, so we read the
//! kernel's cumulative per-core tick counters directly through libc and derive
//! percentages from deltas between two snapshots. Everything else (brand,
//! core count, load average) comes from sysinfo.

use serde::Serialize;
use sysinfo::{CpuRefreshKind, System};

/// 1/5/15-minute load averages (own type: sysinfo's is not `Serialize`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAverages {
    /// 1-minute average.
    pub one: f64,
    /// 5-minute average.
    pub five: f64,
    /// 15-minute average.
    pub fifteen: f64,
}

use super::metrics::core_delta;

/// One CPU core reading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreUsage {
    /// Core index (0-based, matches `hw.ncpu` ordering).
    pub index: usize,
    /// Busy percentage over the last sample window.
    pub total_percent: f64,
    /// User-mode share.
    pub user_percent: f64,
    /// Kernel-mode share.
    pub system_percent: f64,
    /// Idle share.
    pub idle_percent: f64,
}

/// Full CPU snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuSnapshot {
    /// Aggregate busy percentage across all cores.
    pub usage_percent: f64,
    /// User / system / idle aggregate percentages.
    pub user_percent: f64,
    pub system_percent: f64,
    pub idle_percent: f64,
    /// Per-core readings; empty before the first delta is available.
    pub cores: Vec<CoreUsage>,
    /// Logical processor count.
    pub logical_cores: Option<u32>,
    /// Physical core count.
    pub physical_cores: Option<u32>,
    /// CPU brand string (e.g. "Apple M5").
    pub brand: Option<String>,
    /// 1/5/15-minute load averages.
    pub load_average: Option<LoadAverages>,
    /// Minimum sensible interval between CPU samples (sysinfo contract).
    pub min_interval_ms: u64,
}

/// Reads cumulative per-core `[user, system, idle, nice]` ticks from Mach.
///
/// Returns one array per logical core. `None` when the kernel call fails —
/// surfaced honestly rather than fabricated.
///
/// `libc`'s mach port getters are deprecated in favor of the `mach2` crate,
/// which this workspace does not depend on; the functions remain correct.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn mach_core_ticks() -> Option<Vec<[u64; 4]>> {
    use std::ptr;

    let mut count: libc::natural_t = 0;
    let mut ticks_ptr: libc::processor_info_array_t = ptr::null_mut();
    let mut info_count: libc::mach_msg_type_number_t = 0;
    let safety = unsafe {
        libc::host_processor_info(
            libc::mach_host_self(),
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut ticks_ptr,
            &mut info_count,
        )
    };
    if safety != libc::KERN_SUCCESS || count == 0 {
        return None;
    }
    // Each core contributes CPU_STATE_MAX (=4) c_uint entries.
    let per_core = libc::CPU_STATE_MAX as usize;
    let len = count as usize * per_core;
    let result = unsafe { std::slice::from_raw_parts(ticks_ptr.cast::<u32>(), len) }
        .chunks_exact(per_core)
        .map(|c| [c[0] as u64, c[1] as u64, c[2] as u64, c[3] as u64]) // user,system,idle,nice
        .collect::<Vec<_>>();
    unsafe {
        let size = len * std::mem::size_of::<u32>();
        libc::vm_deallocate(
            libc::mach_task_self(),
            ticks_ptr as libc::vm_address_t,
            size as libc::vm_size_t,
        );
    }
    Some(result)
}

/// Non-macOS platforms have no Mach counters; the breakdown is unavailable.
#[cfg(not(target_os = "macos"))]
fn mach_core_ticks() -> Option<Vec<[u64; 4]>> {
    None
}

/// Stateful CPU collector: keeps the previous tick snapshot to compute deltas.
pub struct CpuCollector {
    system: System,
    prev_ticks: Option<Vec<[u64; 4]>>,
}

impl CpuCollector {
    /// New collector. Call [`Self::sample`] at least twice (≥200 ms apart)
    /// before percentages are meaningful.
    pub fn new() -> Self {
        let mut system = System::new();
        // `refresh_cpu_specifics` alone does not populate the CPU list on
        // macOS; the list variant does, and frequency is cheap context.
        system.refresh_cpu_list(CpuRefreshKind::everything());
        Self {
            system,
            prev_ticks: None,
        }
    }

    /// Take a sample. Percentages describe the window since the previous call.
    pub fn sample(&mut self) -> CpuSnapshot {
        self.system
            .refresh_cpu_specifics(CpuRefreshKind::everything());

        let cur = mach_core_ticks();
        let (usage_percent, user, system_pct, idle_pct, cores) = match (&self.prev_ticks, &cur) {
            (Some(prev), Some(cur)) if prev.len() == cur.len() && !prev.is_empty() => {
                let mut cores_out = Vec::with_capacity(cur.len());
                let mut agg_user = 0.0;
                let mut agg_system = 0.0;
                let mut agg_idle = 0.0;
                for (i, (p, c)) in prev.iter().zip(cur.iter()).enumerate() {
                    if let Some(d) = core_delta(*p, *c) {
                        agg_user += d.user_percent;
                        agg_system += d.system_percent;
                        agg_idle += d.idle_percent;
                        cores_out.push(CoreUsage {
                            index: i,
                            total_percent: d.total_percent,
                            user_percent: d.user_percent,
                            system_percent: d.system_percent,
                            idle_percent: d.idle_percent,
                        });
                    }
                }
                let n = cores_out.len().max(1) as f64;
                (
                    ((agg_user + agg_system) / n).min(100.0),
                    agg_user / n,
                    agg_system / n,
                    agg_idle / n,
                    cores_out,
                )
            }
            _ => (0.0, 0.0, 0.0, 0.0, Vec::new()),
        };

        self.prev_ticks = cur;

        let brand = self
            .system
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty());

        CpuSnapshot {
            usage_percent,
            user_percent: user,
            system_percent: system_pct,
            idle_percent: idle_pct,
            cores,
            logical_cores: Some(self.system.cpus().len() as u32),
            physical_cores: sysinfo::System::physical_core_count().map(|n| n as u32),
            brand,
            load_average: Some(sysinfo::System::load_average()).map(|l| LoadAverages {
                one: l.one,
                five: l.five,
                fifteen: l.fifteen,
            }),
            min_interval_ms: sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Live collector test: only meaningful where Mach counters exist.
    #[cfg(target_os = "macos")]
    #[test]
    fn collector_produces_two_sample_percentages() {
        let mut c = CpuCollector::new();
        let first = c.sample();
        // First sample has no delta window yet → cores empty by design.
        assert_eq!(first.cores.len(), 0);
        std::thread::sleep(Duration::from_millis(600));
        let second = c.sample();
        let n = second.logical_cores.unwrap_or(0) as usize;
        assert_eq!(second.cores.len(), n);
        for core in &second.cores {
            assert!((0.0..=100.0).contains(&core.total_percent));
        }
        assert!((0.0..=100.0).contains(&second.usage_percent));
        assert!(second.physical_cores.unwrap_or(0) > 0);
        assert!(second.brand.is_some());
    }

    #[test]
    fn empty_state_before_first_delta() {
        // A collector that never saw ticks reports zeros, not fabrications.
        let mut c = CpuCollector::new();
        let s = c.sample();
        if s.cores.is_empty() {
            assert_eq!(s.usage_percent, 0.0);
        }
    }
}
