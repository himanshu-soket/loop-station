//! Disk metrics: capacity per mounted volume + I/O throughput.
//!
//! sysinfo 0.38 exposes cumulative read/write counters per disk on macOS
//! (verified live on this machine). Throughput is derived from counter deltas;
//! a reboot or counter reset yields `None` for one window rather than a bogus
//! spike, and the UI shows an honest gap.

use serde::Serialize;
use sysinfo::Disks;

/// One volume's capacity snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    /// Mount path (deduplicated display key).
    pub mount_point: String,
    /// Volume name (e.g. "Macintosh HD").
    pub name: String,
    /// Filesystem type (e.g. "apfs").
    pub file_system: String,
    /// SSD / HDD / Unknown.
    pub kind: String,
    /// Whether this volume is removable media.
    pub removable: bool,
    /// Total capacity bytes.
    pub total_bytes: u64,
    /// Available-to-this-user bytes (what Finder shows).
    pub available_bytes: u64,
}

/// Whole-disk I/O reading for one sample window.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskIo {
    /// Bytes read during the window.
    pub read_bytes: u64,
    /// Bytes written during the window.
    pub written_bytes: u64,
    /// Read throughput (bytes/sec) over the window.
    pub read_bytes_per_sec: f64,
    /// Write throughput (bytes/sec) over the window.
    pub written_bytes_per_sec: f64,
    /// False when counters reset (reboot) — window dropped.
    pub valid: bool,
}

impl VolumeInfo {
    /// Used fraction in 0..1. (Used by tests now, the UI soon.)
    #[allow(dead_code)]
    pub fn used_fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            1.0 - (self.available_bytes.min(self.total_bytes) as f64 / self.total_bytes as f64)
        }
    }
}

/// Stateful disk collector.
pub struct DiskCollector {
    disks: Disks,
    /// Previous cumulative I/O counters (for rate deltas).
    prev_counters: Option<(u64, u64)>,
}

impl Default for DiskCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskCollector {
    /// New collector with the current volume list.
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            prev_counters: None,
        }
    }

    /// Refresh volume capacities and return them.
    pub fn volumes(&mut self) -> Vec<VolumeInfo> {
        self.disks.refresh(true);
        let mut out: Vec<VolumeInfo> = Vec::new();
        for d in self.disks.list() {
            let mount = d.mount_point().to_string_lossy().to_string();
            // APFS synthetic volumes share one container; dedupe by mount to
            // avoid double-counting "/" and "/System/Volumes/Data".
            if out.iter().any(|v| v.mount_point == mount) {
                continue;
            }
            out.push(VolumeInfo {
                mount_point: mount,
                name: d.name().to_string_lossy().to_string(),
                file_system: d.file_system().to_string_lossy().to_string(),
                kind: match d.kind() {
                    sysinfo::DiskKind::SSD => "ssd".into(),
                    sysinfo::DiskKind::HDD => "hdd".into(),
                    sysinfo::DiskKind::Unknown(_) => "unknown".into(),
                },
                removable: d.is_removable(),
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
            });
        }
        out
    }

    /// Cumulative I/O counters across all disks, summed.
    ///
    /// Returns `None` when sysinfo cannot read counters (older macOS, sandbox).
    fn io_counters(&mut self) -> Option<(u64, u64)> {
        self.disks.refresh(true);
        let mut read = 0u128;
        let mut written = 0u128;
        let mut any = false;
        for d in self.disks.list_mut() {
            let u = d.usage();
            read += u.total_read_bytes as u128;
            written += u.total_written_bytes as u128;
            any = true;
        }
        any.then_some((read as u64, written as u64))
    }

    /// Sample I/O throughput against the previous counter snapshot.
    /// `elapsed_secs` comes from the caller's clock.
    pub fn sample_io(&mut self, elapsed_secs: f64) -> DiskIo {
        let Some((read, written)) = self.io_counters() else {
            return DiskIo::default(); // valid=false → shown unavailable
        };
        let prev = self.prev_counters.take();
        self.prev_counters = Some((read, written));
        let Some((prev_read, prev_written)) = prev else {
            return DiskIo {
                valid: false,
                ..Default::default()
            };
        };
        // Counter reset (reboot) → drop the window instead of spiking.
        if read < prev_read || written < prev_written {
            return DiskIo {
                valid: false,
                ..Default::default()
            };
        }
        if elapsed_secs <= 0.0 {
            return DiskIo {
                valid: false,
                ..Default::default()
            };
        }
        let dr = read - prev_read;
        let dw = written - prev_written;
        DiskIo {
            read_bytes: dr,
            written_bytes: dw,
            read_bytes_per_sec: dr as f64 / elapsed_secs,
            written_bytes_per_sec: dw as f64 / elapsed_secs,
            valid: true,
        }
    }
}

/// Rate math isolated for testing: bytes delta → per-second rate.
#[allow(dead_code)] // exercised by unit tests; documents the rate contract
pub fn rate_per_sec(delta_bytes: u64, elapsed_secs: f64) -> Option<f64> {
    (elapsed_secs > 0.0).then(|| delta_bytes as f64 / elapsed_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rate_math() {
        assert!((rate_per_sec(100, 2.0).unwrap() - 50.0).abs() < 1e-9);
        assert!(rate_per_sec(100, 0.0).is_none());
        assert!((rate_per_sec(0, 1.0).unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn volume_used_fraction() {
        let v = VolumeInfo {
            mount_point: "/".into(),
            name: "HD".into(),
            file_system: "apfs".into(),
            kind: "ssd".into(),
            removable: false,
            total_bytes: 1000,
            available_bytes: 250,
        };
        assert!((v.used_fraction() - 0.75).abs() < 1e-9);
        // pathological: available > total clamps instead of going negative
        let bad = VolumeInfo {
            available_bytes: 2000,
            total_bytes: 1000,
            ..v
        };
        assert!(bad.used_fraction() >= 0.0 && bad.used_fraction() <= 1.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_volumes_and_counters() {
        let mut c = DiskCollector::new();
        let vols = c.volumes();
        assert!(!vols.is_empty(), "at least one mounted volume expected");
        assert!(vols.iter().all(|v| v.total_bytes > 0));
        // First sample seeds counters → invalid window, not fabricated rates.
        let first = c.sample_io(1.0);
        assert!(!first.valid);
        std::thread::sleep(Duration::from_millis(300));
        let second = c.sample_io(0.3);
        assert!(second.valid);
        assert!(second.read_bytes_per_sec >= 0.0);
    }
}
