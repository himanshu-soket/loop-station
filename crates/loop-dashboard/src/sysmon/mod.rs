//! System monitor: collectors + sampling loop + shared snapshot.
//!
//! A single background task owns all collectors and samples them at the
//! configured interval. Fast domains (CPU, memory, network, disk I/O) sample
//! every tick; slow ones (battery, GPU/thermals, volume capacities) every
//! `SLOW_EVERY` ticks. Handlers read immutable snapshots — collection never
//! happens on the request path.

pub mod api;
pub mod battery;
pub mod cpu;
pub mod demo;
pub mod disk;
pub mod gpu;
pub mod history;
pub mod memory;
pub mod metrics;
pub mod net;
pub mod processes;

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::watch;

use battery::{BatteryCollector, BatterySnapshot};
use cpu::{CpuCollector, CpuSnapshot};
use disk::{DiskCollector, DiskIo, VolumeInfo};
use gpu::{GpuCollector, GpuSnapshot};
use history::{Series, DEFAULT_CAPACITY};
use memory::{MemoryCollector, MemorySnapshot};
use net::{NetworkCollector, NetworkTotals};
use processes::{ProcessCollector, ProcessRow};

/// Slow domains refresh every N fast ticks.
pub const SLOW_EVERY: u32 = 5;

/// Immutable view served to HTTP handlers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysSnapshot {
    /// When this snapshot was taken (unix ms).
    pub sampled_at_ms: i64,
    /// Sampling interval in milliseconds.
    pub interval_ms: u64,
    /// CPU reading.
    pub cpu: CpuSnapshot,
    /// Memory reading.
    pub memory: MemorySnapshot,
    /// Volumes with capacity.
    pub volumes: Vec<VolumeInfo>,
    /// Disk I/O over the last window.
    pub disk_io: DiskIo,
    /// Per-interface readings.
    pub interfaces: Vec<net::InterfaceInfo>,
    /// Physical-interface aggregate.
    pub network_totals: NetworkTotals,
    /// GPU + thermals.
    pub gpu: GpuSnapshot,
    /// Battery.
    pub battery: BatterySnapshot,
    /// Processes (full table; API filters).
    pub processes: Vec<ProcessRow>,
    /// Number of samples taken so far.
    pub sample_count: u64,
}

/// Chart series kept by the sampler.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// Aggregate CPU busy %.
    pub cpu_percent: Vec<history::Point>,
    /// CPU user %.
    pub cpu_user_percent: Vec<history::Point>,
    /// CPU system %.
    pub cpu_system_percent: Vec<history::Point>,
    /// Used memory bytes.
    pub memory_used_bytes: Vec<history::Point>,
    /// Swap used bytes.
    pub swap_used_bytes: Vec<history::Point>,
    /// Disk read bytes/sec.
    pub disk_read_per_sec: Vec<history::Point>,
    /// Disk write bytes/sec.
    pub disk_write_per_sec: Vec<history::Point>,
    /// Network download bytes/sec.
    pub net_recv_per_sec: Vec<history::Point>,
    /// Network upload bytes/sec.
    pub net_sent_per_sec: Vec<history::Point>,
}

/// Internal mutable state behind the RwLock.
struct Inner {
    snapshot: SysSnapshot,
    history: HistoryBuffers,
}

/// Ring buffers backing [`History`].
struct HistoryBuffers {
    cpu_percent: Series,
    cpu_user_percent: Series,
    cpu_system_percent: Series,
    memory_used_bytes: Series,
    swap_used_bytes: Series,
    disk_read_per_sec: Series,
    disk_write_per_sec: Series,
    net_recv_per_sec: Series,
    net_sent_per_sec: Series,
}

impl HistoryBuffers {
    fn new() -> Self {
        let s = || Series::new(DEFAULT_CAPACITY);
        Self {
            cpu_percent: s(),
            cpu_user_percent: s(),
            cpu_system_percent: s(),
            memory_used_bytes: s(),
            swap_used_bytes: s(),
            disk_read_per_sec: s(),
            disk_write_per_sec: s(),
            net_recv_per_sec: s(),
            net_sent_per_sec: s(),
        }
    }

    fn history(&self) -> History {
        History {
            cpu_percent: self.cpu_percent.snapshot(),
            cpu_user_percent: self.cpu_user_percent.snapshot(),
            cpu_system_percent: self.cpu_system_percent.snapshot(),
            memory_used_bytes: self.memory_used_bytes.snapshot(),
            swap_used_bytes: self.swap_used_bytes.snapshot(),
            disk_read_per_sec: self.disk_read_per_sec.snapshot(),
            disk_write_per_sec: self.disk_write_per_sec.snapshot(),
            net_recv_per_sec: self.net_recv_per_sec.snapshot(),
            net_sent_per_sec: self.net_sent_per_sec.snapshot(),
        }
    }
}

/// Shared handle for readers.
#[derive(Clone)]
pub struct SysMonitor {
    inner: Arc<RwLock<Inner>>,
    tx: watch::Sender<u64>,
}

impl SysMonitor {
    /// Build a monitor whose first sample is taken immediately.
    pub fn new(interval: Duration) -> Self {
        let m = Self::with_empty_snapshot(interval);
        m.sample_once(); // seed counters so the second sample has deltas
        m
    }

    /// Build a monitor without sampling (tests inject snapshots).
    pub fn with_empty_snapshot(interval: Duration) -> Self {
        let (tx, _rx) = watch::channel(0);
        let snapshot = SysSnapshot {
            sampled_at_ms: 0,
            interval_ms: interval.as_millis() as u64,
            cpu: CpuSnapshot {
                usage_percent: 0.0,
                user_percent: 0.0,
                system_percent: 0.0,
                idle_percent: 0.0,
                cores: Vec::new(),
                logical_cores: None,
                physical_cores: None,
                brand: None,
                load_average: None,
                min_interval_ms: 0,
            },
            memory: MemorySnapshot {
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
                free_bytes: 0,
                swap_total_bytes: 0,
                swap_used_bytes: None,
                pressure_level: None,
            },
            volumes: Vec::new(),
            disk_io: DiskIo::default(),
            interfaces: Vec::new(),
            network_totals: NetworkTotals::default(),
            gpu: GpuSnapshot::unavailable(),
            battery: BatterySnapshot::unavailable(),
            processes: Vec::new(),
            sample_count: 0,
        };
        Self {
            inner: Arc::new(RwLock::new(Inner {
                snapshot,
                history: HistoryBuffers::new(),
            })),
            tx,
        }
    }

    /// Take one sample using real collectors (used by the sampling loop).
    #[allow(clippy::too_many_lines)]
    pub fn sample_once(&self) {
        let mut source = SampleSource::real();
        self.sample_with(&mut source);
    }

    /// Take one sample from the given source (real or clearly-labeled demo).
    pub fn sample_with(&self, source: &mut SampleSource) {
        let started = Instant::now();
        let now_ms = chrono::Utc::now().timestamp_millis();

        let slow_tick = source.tick().is_multiple_of(SLOW_EVERY) || source.tick() == 1;
        let cpu = source.cpu();
        let memory = source.memory();
        let volumes = if slow_tick {
            source.volumes()
        } else {
            self.inner.read().snapshot.volumes.clone()
        };
        let disk_io = source.disk_io(started.elapsed().as_secs_f64().max(0.001));
        let interfaces = source.interfaces();
        let network_totals = net::aggregate(&interfaces);
        let gpu = if slow_tick {
            source.gpu()
        } else {
            self.inner.read().snapshot.gpu.clone()
        };
        let battery = if slow_tick {
            source.battery()
        } else {
            self.inner.read().snapshot.battery.clone()
        };
        let processes = source.processes();

        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        // Disk I/O rates were computed against collection time; recompute
        // against the caller-visible window for accuracy.
        let disk_io = DiskIo {
            read_bytes_per_sec: disk_io.read_bytes as f64 / elapsed,
            written_bytes_per_sec: disk_io.written_bytes as f64 / elapsed,
            ..disk_io
        };

        let sample_count = self.inner.read().snapshot.sample_count + 1;
        {
            let mut guard = self.inner.write();
            let h = &mut guard.history;
            h.cpu_percent.push(now_ms, cpu.usage_percent);
            h.cpu_user_percent.push(now_ms, cpu.user_percent);
            h.cpu_system_percent.push(now_ms, cpu.system_percent);
            h.memory_used_bytes.push(now_ms, memory.used_bytes as f64);
            h.swap_used_bytes
                .push(now_ms, memory.swap_used_bytes.unwrap_or(0) as f64);
            h.disk_read_per_sec.push(
                now_ms,
                if disk_io.valid {
                    disk_io.read_bytes_per_sec
                } else {
                    f64::NAN
                },
            );
            h.disk_write_per_sec.push(
                now_ms,
                if disk_io.valid {
                    disk_io.written_bytes_per_sec
                } else {
                    f64::NAN
                },
            );
            h.net_recv_per_sec
                .push(now_ms, network_totals.received_per_sec);
            h.net_sent_per_sec
                .push(now_ms, network_totals.transmitted_per_sec);

            guard.snapshot = SysSnapshot {
                sampled_at_ms: now_ms,
                interval_ms: guard.snapshot.interval_ms,
                cpu,
                memory,
                volumes,
                disk_io,
                interfaces,
                network_totals,
                gpu,
                battery,
                processes,
                sample_count,
            };
        }
        source.advance();
        self.tx.send_modify(|c| *c += 1);
    }

    /// Current snapshot (cheap clone of Arc'd data).
    pub fn snapshot(&self) -> SysSnapshot {
        self.inner.read().snapshot.clone()
    }

    /// Current chart history.
    pub fn history(&self) -> History {
        self.inner.read().history.history()
    }

    /// Update the sampling interval (persisted into snapshots).
    pub fn set_interval(&self, interval: Duration) {
        self.inner.write().snapshot.interval_ms = interval.as_millis() as u64;
    }

    /// Subscribe to sample notifications (each sample bumps the counter).
    #[allow(dead_code)] // reserved for a future SSE stream
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }
}

/// Where samples come from: the real machine, or clearly-labeled demo data.
///
/// Both implement the same sampling surface so the pipeline (history buffers,
/// snapshot publication) is shared and demo data can never silently mix with
/// real readings — the source is fixed at startup.
pub enum SampleSource {
    /// Real macOS collectors (boxed: the collector set is ~1 KB and the enum
    /// is moved around by the sampler loop).
    Real(Box<Collectors>),
    /// Synthetic waveforms for offline UI testing.
    Demo(demo::DemoCollectors),
}

impl SampleSource {
    /// Real collectors, tick starting at 1.
    pub fn real() -> Self {
        Self::Real(Box::default())
    }

    /// Demo collectors.
    pub fn demo() -> Self {
        Self::Demo(demo::DemoCollectors::new())
    }

    /// True when serving synthetic data.
    #[allow(dead_code)] // used by tests and future guards
    pub fn is_demo(&self) -> bool {
        matches!(self, Self::Demo(_))
    }

    fn tick(&self) -> u32 {
        match self {
            Self::Real(c) => c.tick,
            Self::Demo(d) => d.tick() as u32 + 1,
        }
    }

    fn advance(&mut self) {
        match self {
            Self::Real(c) => c.tick += 1,
            Self::Demo(d) => d.advance(),
        }
    }

    fn cpu(&mut self) -> CpuSnapshot {
        match self {
            Self::Real(c) => c.cpu.sample(),
            Self::Demo(d) => d.cpu(),
        }
    }

    fn memory(&mut self) -> MemorySnapshot {
        match self {
            Self::Real(c) => c.memory.sample(),
            Self::Demo(d) => d.memory(),
        }
    }

    fn volumes(&mut self) -> Vec<VolumeInfo> {
        match self {
            Self::Real(c) => c.disk.volumes(),
            Self::Demo(d) => d.volumes(),
        }
    }

    fn disk_io(&mut self, elapsed_secs: f64) -> DiskIo {
        match self {
            Self::Real(c) => c.disk.sample_io(elapsed_secs),
            Self::Demo(d) => d.disk_io(),
        }
    }

    fn interfaces(&mut self) -> Vec<net::InterfaceInfo> {
        match self {
            Self::Real(c) => c.net.sample(),
            Self::Demo(d) => d.interfaces(),
        }
    }

    fn gpu(&mut self) -> GpuSnapshot {
        match self {
            Self::Real(c) => c.gpu.sample(),
            Self::Demo(d) => d.gpu(),
        }
    }

    fn battery(&mut self) -> BatterySnapshot {
        match self {
            Self::Real(c) => c.battery.sample(),
            Self::Demo(d) => d.battery(),
        }
    }

    fn processes(&mut self) -> Vec<ProcessRow> {
        match self {
            Self::Real(c) => c.processes.sample(),
            Self::Demo(d) => d.processes(),
        }
    }
}

/// The real collector set owned by the sampling loop.
pub struct Collectors {
    /// CPU collector.
    pub cpu: CpuCollector,
    /// Memory collector.
    pub memory: MemoryCollector,
    /// Disk collector.
    pub disk: DiskCollector,
    /// Network collector.
    pub net: NetworkCollector,
    /// GPU collector.
    pub gpu: GpuCollector,
    /// Battery collector.
    pub battery: BatteryCollector,
    /// Process collector.
    pub processes: ProcessCollector,
    /// Tick counter driving slow-domain cadence.
    pub tick: u32,
}

impl Default for Collectors {
    fn default() -> Self {
        Self::new()
    }
}

impl Collectors {
    /// Fresh collectors.
    pub fn new() -> Self {
        Self {
            cpu: CpuCollector::new(),
            memory: MemoryCollector::new(),
            disk: DiskCollector::new(),
            net: NetworkCollector::new(),
            gpu: GpuCollector::new(),
            battery: BatteryCollector::new(),
            processes: ProcessCollector::new(),
            tick: 1,
        }
    }
}

/// Spawns the sampling loop. Runs until the process exits.
///
/// `interval` is read fresh each iteration so runtime interval changes take
/// effect without restarting.
pub async fn run_sampler(monitor: SysMonitor, demo: bool, base_interval: Duration) {
    let mut source = if demo {
        SampleSource::demo()
    } else {
        SampleSource::real()
    };
    // Warm-up sample so the very first UI paint has deltas.
    monitor.sample_with(&mut source);
    loop {
        let interval = Duration::from_millis(
            monitor
                .snapshot()
                .interval_ms
                .max(base_interval.as_millis() as u64),
        );
        tokio::time::sleep(interval).await;
        monitor.sample_with(&mut source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_history_grow_together() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut c = SampleSource::real();
        m.sample_with(&mut c);
        assert_eq!(m.snapshot().sample_count, 1);
        assert_eq!(m.history().cpu_percent.len(), 1);
        m.sample_with(&mut c);
        assert_eq!(m.snapshot().sample_count, 2);
        assert_eq!(m.history().cpu_percent.len(), 2);
    }

    #[test]
    fn demo_source_never_reports_real_shaped_zero_memory() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut d = SampleSource::demo();
        m.sample_with(&mut d);
        let s = m.snapshot();
        assert_eq!(s.memory.total_bytes, 16 * 1024 * 1024 * 1024);
        assert!(s.cpu.usage_percent > 0.0);
        // demo must not fake temperature either
        assert!(!s.gpu.temperature_celsius.available);
    }

    #[test]
    fn interval_is_reported() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_millis(5000));
        assert_eq!(m.snapshot().interval_ms, 5000);
    }

    #[tokio::test]
    async fn sampler_loop_produces_samples() {
        // Demo source: deterministic and cheap, so the test doesn't depend on
        // how fast the machine can enumerate processes.
        let m = SysMonitor::with_empty_snapshot(Duration::from_millis(20));
        let monitor_clone = m.clone();
        tokio::spawn(run_sampler(
            monitor_clone.clone(),
            true, // demo
            Duration::from_millis(20),
        ));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let count = monitor_clone.snapshot().sample_count;
        assert!(count >= 3, "expected repeated samples, got {count}");
        assert!(monitor_clone.history().cpu_percent.len() >= 3);
    }
}
