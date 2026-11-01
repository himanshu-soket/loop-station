//! Machine identity: hostname and OS version.
//!
//! These values change only when the OS is upgraded or the machine is
//! renamed, so they are read once per sampling pass (not per request) via
//! sysinfo's static queries and carried in the snapshot. Every field is
//! optional: sysinfo returns `None` when the OS facility is unavailable,
//! and that absence is surfaced honestly rather than defaulted.

use serde::Serialize;

/// Static-ish machine description.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// The machine's network hostname (e.g. "macbook.local").
    pub host_name: Option<String>,
    /// OS name (e.g. "macOS", "Darwin" fallback).
    pub os_name: Option<String>,
    /// OS version (e.g. "15.4").
    pub os_version: Option<String>,
    /// Kernel version (e.g. "24.4.0").
    pub kernel_version: Option<String>,
}

impl HostInfo {
    /// Read the current values from sysinfo's static queries.
    ///
    /// Cheap: these resolve from cached system calls (`uname`, sysctl) and
    /// do not allocate collectors.
    pub fn read() -> Self {
        Self {
            host_name: sysinfo::System::host_name(),
            os_name: sysinfo::System::name(),
            os_version: sysinfo::System::os_version(),
            kernel_version: sysinfo::System::kernel_version(),
        }
    }
}

/// Demo-mode host info: clearly labeled, never mistaken for a real machine.
pub fn demo_host_info() -> HostInfo {
    HostInfo {
        host_name: Some("demo-host".into()),
        os_name: Some(sysinfo::System::name().unwrap_or_else(|| "Demo OS".into())),
        os_version: Some("0.0-demo".into()),
        kernel_version: None,
    }
}
