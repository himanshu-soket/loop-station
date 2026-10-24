//! Clearly-labeled synthetic system metrics for offline UI testing.
//!
//! Demo data is *never* mixed with real readings: the demo flag is threaded
//! through every API response and rendered as a persistent banner in the UI.
//! Values are deterministic-ish (smooth sine waves + noise) so charts look
//! alive without implying real measurements.

use super::battery::BatterySnapshot;
use super::cpu::CpuSnapshot;
use super::disk::{DiskIo, VolumeInfo};
use super::gpu::GpuSnapshot;
use super::memory::MemorySnapshot;
use super::metrics::Metric;
use super::net::InterfaceInfo;
use super::processes::ProcessRow;

/// Synthetic collector set.
pub struct DemoCollectors {
    tick: u64,
}

impl DemoCollectors {
    /// New synthetic collectors.
    pub fn new() -> Self {
        Self { tick: 0 }
    }

    fn wave(&self, base: f64, amplitude: f64, period_ticks: u64) -> f64 {
        let phase = (self.tick % period_ticks) as f64 / period_ticks as f64;
        base + amplitude * (2.0 * std::f64::consts::PI * phase).sin()
    }
}

impl Default for DemoCollectors {
    fn default() -> Self {
        Self::new()
    }
}

// The demo needs to slot into `Collectors`, whose fields are the real
// collector types. We model that by wrapping the real collectors and letting
// `sample_with` be intercepted: see `SysMonitor` demo wiring in api.rs —
// instead we expose standalone snapshot builders used by the demo sampler.
impl DemoCollectors {
    /// Synthetic CPU reading.
    pub fn cpu(&self) -> CpuSnapshot {
        let total = self.wave(35.0, 25.0, 45).clamp(1.0, 99.0);
        let user = total * 0.7;
        let system = total - user;
        let cores = (0..10)
            .map(|i| {
                let c = (total + ((i % 3) as f64 - 1.0) * 12.0).clamp(0.0, 100.0);
                super::cpu::CoreUsage {
                    index: i,
                    total_percent: c,
                    user_percent: c * 0.7,
                    system_percent: c * 0.3,
                    idle_percent: 100.0 - c,
                }
            })
            .collect();
        CpuSnapshot {
            usage_percent: total,
            user_percent: user,
            system_percent: system,
            idle_percent: 100.0 - total,
            cores,
            logical_cores: Some(10),
            physical_cores: Some(10),
            brand: Some("Demo CPU (synthetic)".into()),
            load_average: Some(crate::sysmon::cpu::LoadAverages {
                one: total / 25.0,
                five: total / 30.0,
                fifteen: total / 35.0,
            }),
            min_interval_ms: 200,
        }
    }

    /// Synthetic memory reading (16 GB machine shape).
    pub fn memory(&self) -> MemorySnapshot {
        let total = 16_u64 * 1024 * 1024 * 1024;
        let used_frac = self.wave(0.55, 0.15, 60).clamp(0.1, 0.95);
        let used = (total as f64 * used_frac) as u64;
        MemorySnapshot {
            total_bytes: total,
            used_bytes: used,
            available_bytes: total - used,
            free_bytes: (total as f64 * 0.02) as u64,
            swap_total_bytes: 3 * 1024 * 1024 * 1024,
            swap_used_bytes: Some(
                (self.wave(0.2, 0.1, 80).clamp(0.0, 0.9) * 3.0 * 1024.0 * 1024.0 * 1024.0) as u64,
            ),
            pressure_level: Some(if used_frac > 0.85 {
                3
            } else if used_frac > 0.7 {
                2
            } else {
                1
            }),
        }
    }

    /// Synthetic volumes.
    pub fn volumes(&self) -> Vec<VolumeInfo> {
        vec![VolumeInfo {
            mount_point: "/".into(),
            name: "Demo HD".into(),
            file_system: "apfs".into(),
            kind: "ssd".into(),
            removable: false,
            total_bytes: 494_332_366_848,
            available_bytes: 139_648_267_922,
        }]
    }

    /// Synthetic disk I/O.
    pub fn disk_io(&self) -> DiskIo {
        let read = (self.wave(40.0, 35.0, 30).max(0.0)) * 1024.0 * 1024.0;
        let write = (self.wave(20.0, 18.0, 37).max(0.0)) * 1024.0 * 1024.0;
        DiskIo {
            read_bytes: read as u64,
            written_bytes: write as u64,
            read_bytes_per_sec: read,
            written_bytes_per_sec: write,
            valid: true,
        }
    }

    /// Synthetic interfaces.
    pub fn interfaces(&self) -> Vec<InterfaceInfo> {
        let down = (self.wave(300.0, 280.0, 25).max(0.0)) * 1024.0;
        let up = (self.wave(80.0, 70.0, 33).max(0.0)) * 1024.0;
        vec![
            InterfaceInfo {
                name: "en0".into(),
                total_received: 103_516_342_234,
                total_transmitted: 10_134_108_611,
                received_per_sec: down,
                transmitted_per_sec: up,
                mac_address: Some("aa:bb:cc:dd:ee:ff".into()),
                active: true,
            },
            InterfaceInfo {
                name: "lo0".into(),
                total_received: 5_258_509_065,
                total_transmitted: 5_258_509_065,
                received_per_sec: 220.0,
                transmitted_per_sec: 220.0,
                mac_address: None,
                active: true,
            },
        ]
    }

    /// Synthetic GPU reading.
    pub fn gpu(&self) -> GpuSnapshot {
        GpuSnapshot {
            model: Metric::ok("Demo GPU (synthetic)".into(), "", "GPU chip"),
            utilization_percent: Metric::ok(
                self.wave(30.0, 25.0, 40).clamp(0.0, 100.0),
                "%",
                "GPU device utilization",
            ),
            renderer_utilization_percent: Metric::ok(
                self.wave(25.0, 20.0, 47).clamp(0.0, 100.0),
                "%",
                "GPU shader (renderer) utilization",
            ),
            tiler_utilization_percent: Metric::ok(
                self.wave(10.0, 8.0, 53).clamp(0.0, 100.0),
                "%",
                "GPU tiler utilization",
            ),
            memory_used_bytes: Metric::unavailable(
                "B",
                "GPU memory in use",
                "not exposed by macOS without root (also unavailable in demo)",
            ),
            temperature_celsius: Metric::unavailable(
                "°C",
                "GPU die temperature",
                "requires root (powermetrics); never estimated (also unavailable in demo)",
            ),
            thermal_pressure: Metric::ok("nominal".into(), "", "System thermal pressure"),
        }
    }

    /// Synthetic battery.
    pub fn battery(&self) -> BatterySnapshot {
        let pct = self.wave(55.0, 30.0, 400).clamp(5.0, 100.0);
        BatterySnapshot {
            charge_percent: Metric::ok(pct, "%", "Battery charge"),
            status: Metric::ok("discharging".into(), "", "Charging state"),
            time_remaining_minutes: Metric::ok(249, "min", "Estimated time remaining"),
            cycle_count: Metric::ok(129, "", "Charge cycle count"),
            health_percent: Metric::ok(96.3, "%", "Battery health (max/design)"),
            design_capacity_mah: Metric::ok(4629, "mAh", "Design capacity"),
            max_capacity_mah: Metric::ok(4458, "mAh", "Current full-charge capacity"),
        }
    }

    /// Synthetic process table.
    pub fn processes(&self) -> Vec<ProcessRow> {
        let names = [
            ("kernel_task", 5.0, 100_000_000u64),
            ("WindowServer", 12.5, 400_000_000),
            ("demo-renderer", 88.0, 900_000_000),
            ("Code Helper", 30.0, 1_200_000_000),
            ("loop-dashboard", 0.5, 40_000_000),
            ("mds_stores", 3.0, 150_000_000),
            ("coreaudiod", 1.2, 30_000_000),
            ("Safari", 8.0, 800_000_000),
        ];
        names
            .iter()
            .enumerate()
            .map(|(i, (name, cpu, mem))| ProcessRow {
                pid: 100 + i as u32 * 37,
                name: (*name).into(),
                cpu_percent: (cpu + self.wave(0.0, 3.0, 13 + i as u64)).max(0.0),
                memory_bytes: Some(*mem),
            })
            .collect()
    }

    /// Advance the internal clock.
    pub fn advance(&mut self) {
        self.tick += 1;
    }

    /// Current tick (0-based).
    pub fn tick(&self) -> u64 {
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_values_are_in_range() {
        for tick in 0..200 {
            let mut d = DemoCollectors { tick };
            d.advance();
            let cpu = d.cpu();
            assert!((0.0..=100.0).contains(&cpu.usage_percent));
            let mem = d.memory();
            assert!(mem.used_bytes <= mem.total_bytes);
            let io = d.disk_io();
            assert!(io.valid && io.read_bytes_per_sec >= 0.0);
            let gpu = d.gpu();
            assert!(
                !gpu.temperature_celsius.available,
                "demo must not fake temperature"
            );
        }
    }

    #[test]
    fn demo_processes_have_unique_pids() {
        let pids: Vec<u32> = DemoCollectors::new()
            .processes()
            .iter()
            .map(|p| p.pid)
            .collect();
        let unique: std::collections::HashSet<u32> = pids.iter().copied().collect();
        assert_eq!(pids.len(), unique.len());
    }
}
