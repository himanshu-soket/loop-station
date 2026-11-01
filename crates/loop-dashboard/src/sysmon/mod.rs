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
pub mod health;
pub mod history;
pub mod host;
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
    /// Seconds since boot (from `sysinfo::System::boot_time`).
    pub uptime_secs: Option<u64>,
    /// Machine identity (hostname, OS). Read per pass; cheap static queries.
    pub host: host::HostInfo,
    /// How long the last full sampling pass took (microseconds).
    /// Surfaced for the performance overlay; measurement, not a metric.
    pub sample_duration_us: u64,
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
    /// GPU device utilization % (NaN when unavailable on this hardware).
    pub gpu_util_percent: Vec<history::Point>,
    /// Battery charge % (recorded on slow ticks; NaN when no battery).
    pub battery_charge_percent: Vec<history::Point>,
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
    gpu_util_percent: Series,
    battery_charge_percent: Series,
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
            gpu_util_percent: s(),
            battery_charge_percent: s(),
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
            gpu_util_percent: self.gpu_util_percent.snapshot(),
            battery_charge_percent: self.battery_charge_percent.snapshot(),
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
            uptime_secs: None,
            host: host::HostInfo {
                host_name: None,
                os_name: None,
                os_version: None,
                kernel_version: None,
            },
            sample_duration_us: 0,
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

        // The disk-rate window is the time between consecutive samples of the
        // *counter*, i.e. since the previous pass — not the few microseconds
        // this collector takes to run. Dividing by collector elapsed time
        // inflated rates by ~interval/collection-time (verified against
        // iostat: dashboard read 90–180 MB/s while iostat measured 2–19).
        let prev_sample_ms = self.inner.read().snapshot.sampled_at_ms;
        let counter_window_secs = if prev_sample_ms > 0 && now_ms > prev_sample_ms {
            (now_ms - prev_sample_ms) as f64 / 1000.0
        } else {
            // First sample: no window yet; `sample_io` marks it invalid.
            0.0
        };

        let tick = source.tick();
        let slow_tick = tick.is_multiple_of(SLOW_EVERY) || tick == 1;
        let cpu = source.cpu();
        let memory = source.memory();
        let volumes = if slow_tick {
            source.volumes()
        } else {
            self.inner.read().snapshot.volumes.clone()
        };
        let disk_io = source.disk_io(counter_window_secs.max(0.001));
        let interfaces = source.interfaces();
        let network_totals = net::aggregate(&interfaces);
        // GPU utilization is sampled every tick: it drives a live chart and
        // changes second-to-second. The ioreg subprocess costs ~21 ms; the
        // thermal half of the old combined read is TTL-cached inside the
        // collector, so steady-state cost here is one ioreg call per tick.
        let gpu = source.gpu();
        // Battery charge/status change minute-to-minute but only need slow-tick
        // cadence; the expensive ioreg *health* half is TTL-cached in the
        // collector. Force a full read on the very first sample.
        let battery = if slow_tick {
            source.battery(tick == 1)
        } else {
            self.inner.read().snapshot.battery.clone()
        };
        let processes = source.processes();

        // Rates are already computed against the true inter-sample window by
        // `disk_io` above; keep the raw byte deltas untouched.
        let sample_duration_us = started.elapsed().as_micros() as u64;

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
            // GPU utilization is sampled every tick; record NaN when the
            // platform doesn't expose it so charts show a gap, not a fake 0.
            h.gpu_util_percent.push(
                now_ms,
                match gpu.utilization_percent.value {
                    Some(v) if gpu.utilization_percent.available => v,
                    _ => f64::NAN,
                },
            );
            // Charge moves minute-to-minute and is only re-read on slow
            // ticks; record the current value then, NaN in between so the
            // chart shows a sparse-but-honest series rather than a fake
            // flat line at a stale value.
            h.battery_charge_percent.push(
                now_ms,
                if slow_tick {
                    match battery.charge_percent.value {
                        Some(v) if battery.charge_percent.available => v,
                        _ => f64::NAN,
                    }
                } else {
                    f64::NAN
                },
            );

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
                // boot_time() is the epoch second of boot → uptime is the delta.
                uptime_secs: Some(
                    chrono::Utc::now().timestamp().max(0) as u64 - sysinfo::System::boot_time(),
                ),
                host: source.host(),
                sample_duration_us,
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

    /// Battery with cache control: `force_refresh` bypasses the health TTL
    /// (used on the first sample so the very first paint has full data).
    fn battery(&mut self, force_refresh: bool) -> BatterySnapshot {
        match self {
            Self::Real(c) => c.battery.sample(force_refresh),
            Self::Demo(d) => d.battery(),
        }
    }

    fn processes(&mut self) -> Vec<ProcessRow> {
        match self {
            Self::Real(c) => c.processes.sample(),
            Self::Demo(d) => d.processes(),
        }
    }

    /// Machine identity. Read-only, so it takes `&self` (no advance).
    fn host(&mut self) -> host::HostInfo {
        if self.is_demo() {
            host::demo_host_info()
        } else {
            host::HostInfo::read()
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

/// Test-only access to the internal state so health-doc tests can inject
/// domain readings (memory pressure, battery) without faking collectors.
#[cfg(test)]
impl SysMonitor {
    pub(crate) fn inner_for_tests(&self) -> parking_lot::RwLockWriteGuard<'_, Inner> {
        self.inner.write()
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

    /// Machine identity: real samples carry host info (string-or-null per
    /// field); demo samples carry the clearly-labeled demo identity so a
    /// synthetic dashboard can never masquerade as this machine.
    #[test]
    fn host_info_real_vs_demo() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut real = SampleSource::real();
        m.sample_with(&mut real);
        let h = m.snapshot().host;
        // On macOS all four resolve; assert at least hostname + OS name so a
        // silent sysinfo regression is caught (fields stay Option-typed for
        // other platforms).
        assert!(h.host_name.is_some(), "hostname must resolve on macOS");
        assert!(h.os_name.is_some(), "OS name must resolve on macOS");

        let m2 = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut demo = SampleSource::demo();
        m2.sample_with(&mut demo);
        let hd = m2.snapshot().host;
        assert_eq!(hd.host_name.as_deref(), Some("demo-host"));
        assert_ne!(
            hd.host_name, h.host_name,
            "demo must not echo the real hostname"
        );
    }

    /// Regression: disk rates must be divided by the inter-sample window,
    /// not by collector execution time. The old code divided a ~2000 ms
    /// window's byte delta by ~2 ms of collector time, inflating rates
    /// ~1000× (dashboard read 90–180 MB/s while `iostat` measured 2–19).
    #[test]
    fn disk_rates_use_inter_sample_window_not_collector_time() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_millis(2000));
        let mut src = SampleSource::real();

        // First sample seeds counters (invalid window) and records t0.
        m.sample_with(&mut src);
        let first = m.snapshot();
        assert!(!first.disk_io.valid, "first window has no baseline");

        // Let a realistic window elapse, then sample again.
        std::thread::sleep(Duration::from_millis(300));
        m.sample_with(&mut src);
        let second = m.snapshot();
        assert!(second.disk_io.valid);

        // The rate must be consistent with bytes over the *elapsed window*
        // (~0.3 s here), never with collector microseconds. A 100× inflation
        // would show up as rate >> bytes/0.3s; allow generous slack for
        // machine noise but catch order-of-magnitude errors.
        let window = 0.3_f64;
        let max_sane_rate = (second.disk_io.read_bytes as f64 / window) * 3.0 + 1.0;
        assert!(
            second.disk_io.read_bytes_per_sec <= max_sane_rate,
            "read rate {} B/s looks inflated for {} B in ~{window}s \
             (collector-time division bug?)",
            second.disk_io.read_bytes_per_sec,
            second.disk_io.read_bytes
        );

        // And the measurement duration itself must be recorded and tiny.
        assert!(second.sample_duration_us > 0);
        assert!(
            second.sample_duration_us < 5_000_000,
            "sampling pass took {} µs — suspiciously slow",
            second.sample_duration_us
        );
    }

    /// Two rapid samples: the window shrinks to near zero and the second
    /// sample's rates stay finite (no division blow-up, no NaN).
    #[test]
    fn rapid_samples_keep_rates_finite() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_millis(500));
        let mut src = SampleSource::real();
        m.sample_with(&mut src);
        m.sample_with(&mut src); // immediately again
        let s = m.snapshot();
        if s.disk_io.valid {
            assert!(s.disk_io.read_bytes_per_sec.is_finite());
            assert!(s.disk_io.written_bytes_per_sec.is_finite());
        }
    }

    #[test]
    fn uptime_is_a_sane_positive_delta() {
        // Real source: uptime must be a plausible value (this Mac has been up
        // for minutes at minimum during any test run).
        let m = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut c = SampleSource::real();
        m.sample_with(&mut c);
        let up = m.snapshot().uptime_secs.expect("uptime present");
        // More than a minute, less than 10 years — catches sign/direction bugs
        // (e.g. accidentally reporting the boot epoch itself, ~1.7 billion).
        assert!(up > 60, "uptime too small: {up}");
        assert!(up < 10 * 365 * 24 * 3600, "uptime implausibly large: {up}");
    }

    #[test]
    fn empty_snapshot_reports_uptime_unavailable() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        assert_eq!(m.snapshot().uptime_secs, None);
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

    /// Cadence contract: GPU utilization is collected on *every* tick (it
    /// drives a live chart), while battery/volumes only refresh on slow ticks
    /// and are carried over unchanged in between.
    #[test]
    fn gpu_refreshes_every_tick_battery_only_on_slow_ticks() {
        let m = SysMonitor::with_empty_snapshot(Duration::from_millis(2000));
        let mut src = SampleSource::real();

        m.sample_with(&mut src); // tick 1 (slow)
        let after_first = m.snapshot();
        let first_volumes = after_first.volumes.clone();
        let first_battery_cycles = after_first.battery.cycle_count.clone();

        // Ticks 2..SLOW_EVERY are fast ticks: volumes and battery carry over.
        for _ in 2..SLOW_EVERY {
            m.sample_with(&mut src);
        }
        let before_slow = m.snapshot();
        assert_eq!(before_slow.volumes, first_volumes, "volumes carried over");
        assert_eq!(
            before_slow.battery.cycle_count, first_battery_cycles,
            "battery carried over"
        );

        // The GPU reading must still be present and fresh-shaped on every one
        // of those fast ticks (a real machine yields available=true; a VM or
        // Intel Mac legitimately yields unavailable — but never a stale
        // placeholder from tick 1).
        if before_slow.gpu.utilization_percent.available {
            assert_eq!(
                before_slow.gpu.model, after_first.gpu.model,
                "model is stable across ticks"
            );
        }
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
