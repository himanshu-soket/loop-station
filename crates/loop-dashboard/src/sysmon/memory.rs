//! Memory metrics with macOS-accurate semantics.
//!
//! `used` on macOS is *apparent* memory (wired + compressed + app memory),
//! not "total − free": file cache is reclaimable, so a near-zero free value is
//! normal and healthy. sysinfo ≥0.38.3 computes this correctly; we surface
//! both the numbers and the explanation so the UI can teach that distinction.

use serde::Serialize;
use sysinfo::System;

/// Full memory snapshot (all byte counts).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    /// Physical RAM installed.
    pub total_bytes: u64,
    /// Apparent used memory (macOS semantics: wired + compressed + app).
    pub used_bytes: u64,
    /// Reclaimable-by-pressure memory (what Activity Monitor calls available).
    pub available_bytes: u64,
    /// Truly unallocated pages.
    pub free_bytes: u64,
    /// Swap device size.
    pub swap_total_bytes: u64,
    /// Swap currently in use.
    pub swap_used_bytes: Option<u64>,
    /// Kernel memory-pressure level (macOS `kern.memorystatus_vm_pressure_level`).
    pub pressure_level: Option<u8>,
}

impl MemorySnapshot {
    /// Used fraction in 0..1.
    #[allow(dead_code)] // kept for the UI's progress bars + tests
    pub fn used_fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.used_bytes as f64 / self.total_bytes as f64
        }
    }

    /// Swap used fraction in 0..1 (swap may be disabled → None).
    #[allow(dead_code)] // kept for the UI's progress bars + tests
    pub fn swap_fraction(&self) -> Option<f64> {
        if self.swap_total_bytes == 0 {
            return None;
        }
        let used = self.swap_used_bytes?;
        Some(used as f64 / self.swap_total_bytes as f64)
    }
}

/// Reads `kern.memorystatus_vm_pressure_level` (1=NOMINAL 2=WARNING 3=CRITICAL).
///
/// Documented values are 1–4; anything above 3 is still reported so the UI can
/// show an honest "unknown" rather than clamping silently.
#[cfg(target_os = "macos")]
fn pressure_sysctl() -> Option<u8> {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>() as libc::size_t;
    let name = b"kern.memorystatus_vm_pressure_level\0";
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::addr_of_mut!(value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(value as u8)
}

#[cfg(not(target_os = "macos"))]
fn pressure_sysctl() -> Option<u8> {
    None
}

/// Stateful memory collector.
pub struct MemoryCollector {
    system: System,
}

impl MemoryCollector {
    /// New collector.
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    /// Take a sample.
    pub fn sample(&mut self) -> MemorySnapshot {
        self.system.refresh_memory();
        MemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes: self.system.used_memory(),
            available_bytes: self.system.available_memory(),
            free_bytes: self.system.free_memory(),
            swap_total_bytes: self.system.total_swap(),
            // sysinfo reports 0 when swap is absent rather than unknown; keep
            // the distinction so we never claim "0 bytes used of swap" wrongly.
            swap_used_bytes: match (self.system.total_swap(), self.system.used_swap()) {
                (0, 0) => None,
                (_, used) => Some(used),
            },
            pressure_level: pressure_sysctl(),
        }
    }
}

/// Human label for a pressure level value.
#[allow(dead_code)] // surfaced via the API payload in the UI build
pub fn pressure_label(level: Option<u8>) -> &'static str {
    match level {
        Some(1) => "nominal",
        Some(2) => "warning",
        Some(3) => "critical",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_are_bounded() {
        let m = MemorySnapshot {
            total_bytes: 1000,
            used_bytes: 400,
            available_bytes: 600,
            free_bytes: 100,
            swap_total_bytes: 500,
            swap_used_bytes: Some(250),
            pressure_level: Some(1),
        };
        assert!((m.used_fraction() - 0.4).abs() < 1e-9);
        assert!((m.swap_fraction().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn zero_total_is_safe() {
        let m = MemorySnapshot {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
            free_bytes: 0,
            swap_total_bytes: 0,
            swap_used_bytes: None,
            pressure_level: None,
        };
        assert_eq!(m.used_fraction(), 0.0);
        assert_eq!(m.swap_fraction(), None);
    }

    #[test]
    fn no_swap_means_unknown_not_zero() {
        let m = MemorySnapshot {
            total_bytes: 10,
            used_bytes: 5,
            available_bytes: 5,
            free_bytes: 1,
            swap_total_bytes: 0,
            swap_used_bytes: None,
            pressure_level: None,
        };
        // swap disabled → None, never a fabricated "0 used"
        assert_eq!(m.swap_fraction(), None);
    }

    #[test]
    fn pressure_labels() {
        assert_eq!(pressure_label(Some(1)), "nominal");
        assert_eq!(pressure_label(Some(2)), "warning");
        assert_eq!(pressure_label(Some(3)), "critical");
        assert_eq!(pressure_label(None), "unknown");
        assert_eq!(pressure_label(Some(99)), "unknown");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_sample_sane() {
        let mut c = MemoryCollector::new();
        let s = c.sample();
        assert!(s.total_bytes > 1_000_000_000);
        assert!(s.used_bytes <= s.total_bytes);
        assert!(s.available_bytes <= s.total_bytes);
        // free ≤ available on macOS (speculative counted in available)
        assert!(s.free_bytes <= s.available_bytes + 16_384 * 1024);
    }
}
