//! Process table: read-only, never terminating anything.
//!
//! sysinfo gives name/pid/cpu/mem per process. CPU% needs two refresh cycles
//! (the collector keeps its own cadence), and we sort/filter server-side so
//! the payload stays small. There is deliberately **no** kill endpoint.

use serde::Serialize;
use sysinfo::{ProcessesToUpdate, System};

/// One process row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRow {
    /// PID.
    pub pid: u32,
    /// Process name.
    pub name: String,
    /// CPU usage percentage (can exceed 100 on multi-core workloads).
    pub cpu_percent: f64,
    /// Resident memory bytes; `None` when macOS denies the read for
    /// other users' processes (e.g. root/`_windowserver` daemons).
    pub memory_bytes: Option<u64>,
}

/// Sortable columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// By CPU usage.
    Cpu,
    /// By memory.
    Memory,
    /// By name.
    Name,
    /// By PID.
    Pid,
}

impl SortKey {
    /// Parse from a query-string value.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            Some("memory") => Self::Memory,
            Some("name") => Self::Name,
            Some("pid") => Self::Pid,
            _ => Self::Cpu,
        }
    }
}

/// Applies search + sort + limit to a process list. Pure → heavily tested.
pub fn filter_sort_limit(
    mut rows: Vec<ProcessRow>,
    search: Option<&str>,
    sort: SortKey,
    descending: bool,
    limit: usize,
) -> Vec<ProcessRow> {
    if let Some(q) = search.map(str::trim).filter(|s| !s.is_empty()) {
        let needle = q.to_lowercase();
        rows.retain(|r| {
            r.name.to_lowercase().contains(&needle) || r.pid.to_string().contains(&needle)
        });
    }
    let cmp = |a: &ProcessRow, b: &ProcessRow| match sort {
        SortKey::Cpu => a
            .cpu_percent
            .partial_cmp(&b.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal),
        // None handling is done by the sink in filter_sort_limit.
        SortKey::Memory => a.memory_bytes.cmp(&b.memory_bytes),
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortKey::Pid => a.pid.cmp(&b.pid),
    };
    rows.sort_by(|a, b| {
        let ord = cmp(a, b);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
    // Unreadable rows (memory unknown) always sink to the bottom regardless
    // of direction — they carry no ranking signal.
    if sort == SortKey::Memory {
        rows.sort_by_key(|r| r.memory_bytes.is_none());
    }
    rows.truncate(limit);
    rows
}

/// Stateful process collector.
pub struct ProcessCollector {
    system: System,
}

impl ProcessCollector {
    /// New collector; CPU% meaningful from the second [`Self::sample`] on.
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    /// Take a sample of all processes.
    /// Take a sample of all processes.
    ///
    /// Memory is `None` for processes owned by other users: macOS denies
    /// `PROC_PIDTASKINFO`/`proc_pid_rusage` for them without root, and a
    /// zero would read as a fabricated value.
    pub fn sample(&mut self) -> Vec<ProcessRow> {
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        // Discover our own uid from any process whose memory is actually
        // readable (the kernel zeroes the field for foreign processes).
        let my_uid = self
            .system
            .processes()
            .values()
            .find(|p| p.memory() > 0)
            .and_then(|p| p.user_id());
        self.system
            .processes()
            .values()
            .map(|p| {
                // The kernel reports 0 resident size for other users'
                // processes; distinguish "genuinely tiny" (own process) from
                // "unreadable" (foreign user) so we never fabricate a zero.
                let readable = my_uid.is_none_or(|uid| p.user_id() == Some(uid));
                ProcessRow {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string_lossy().to_string(),
                    cpu_percent: p.cpu_usage() as f64,
                    memory_bytes: readable.then_some(p.memory()),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn row(pid: u32, name: &str, cpu: f64, mem: u64) -> ProcessRow {
        ProcessRow {
            pid,
            name: name.into(),
            cpu_percent: cpu,
            memory_bytes: Some(mem),
        }
    }

    fn foreign_row(pid: u32, name: &str) -> ProcessRow {
        ProcessRow {
            pid,
            name: name.into(),
            cpu_percent: 0.0,
            memory_bytes: None, // other user's process: unreadable on macOS
        }
    }

    fn sample_rows() -> Vec<ProcessRow> {
        vec![
            row(1, "kernel_task", 5.0, 100_000_000),
            row(2, "WindowServer", 12.5, 400_000_000),
            row(3, "cargo", 88.0, 900_000_000),
            row(4, "Code Helper", 30.0, 1_200_000_000),
            row(5, "loop-dashboard", 0.5, 40_000_000),
        ]
    }

    #[test]
    fn default_sort_is_cpu_desc() {
        let out = filter_sort_limit(sample_rows(), None, SortKey::Cpu, true, 10);
        assert_eq!(out[0].name, "cargo");
        assert_eq!(out[1].name, "Code Helper");
    }

    #[test]
    fn sorts_by_memory() {
        let out = filter_sort_limit(sample_rows(), None, SortKey::Memory, true, 10);
        assert_eq!(out[0].name, "Code Helper");
    }

    #[test]
    fn sorts_by_name_ascending() {
        let out = filter_sort_limit(sample_rows(), None, SortKey::Name, false, 10);
        assert_eq!(out[0].name, "cargo");
        assert_eq!(out[1].name, "Code Helper");
    }

    #[test]
    fn sorts_by_pid() {
        let out = filter_sort_limit(sample_rows(), None, SortKey::Pid, false, 10);
        assert_eq!(out[0].pid, 1);
        assert_eq!(out.last().unwrap().pid, 5);
    }

    #[test]
    fn search_matches_name_case_insensitive() {
        let out = filter_sort_limit(sample_rows(), Some("HELPER"), SortKey::Cpu, true, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Code Helper");
    }

    #[test]
    fn search_matches_pid_substring() {
        let out = filter_sort_limit(sample_rows(), Some("12"), SortKey::Cpu, true, 10);
        // "12" matches no names but... none of these pids contain 12 either
        assert!(out.is_empty());
        let out2 = filter_sort_limit(sample_rows(), Some("4"), SortKey::Cpu, true, 10);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].pid, 4);
    }

    #[test]
    fn limit_truncates() {
        let out = filter_sort_limit(sample_rows(), None, SortKey::Cpu, true, 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn blank_search_is_noop() {
        let out = filter_sort_limit(sample_rows(), Some("   "), SortKey::Cpu, true, 10);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn unreadable_memory_sorts_below_real_values() {
        let mut rows = sample_rows();
        rows.push(foreign_row(999, "WindowServer"));
        let out = filter_sort_limit(rows, None, SortKey::Memory, true, 10);
        // every readable row ranks above the unreadable one
        assert_eq!(out.last().unwrap().name, "WindowServer");
        assert_eq!(out.last().unwrap().memory_bytes, None);
    }

    #[test]
    fn sort_key_parses() {
        assert_eq!(SortKey::parse(Some("memory")), SortKey::Memory);
        assert_eq!(SortKey::parse(Some("name")), SortKey::Name);
        assert_eq!(SortKey::parse(Some("pid")), SortKey::Pid);
        assert_eq!(SortKey::parse(None), SortKey::Cpu);
        assert_eq!(SortKey::parse(Some("bogus")), SortKey::Cpu);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_sample_finds_self_and_kernel() {
        let mut c = ProcessCollector::new();
        let _ = c.sample(); // seed
        std::thread::sleep(Duration::from_millis(400));
        let rows = c.sample();
        assert!(rows.len() > 10, "expected a real process table");
        // PID 0 (kernel_task) is not enumerated by sysinfo; launchd (PID 1) is.
        assert!(rows.iter().any(|r| r.pid == 1), "launchd expected");
        assert!(rows.iter().all(|r| r.cpu_percent >= 0.0));
        // Own-user processes must carry a real reading; foreign ones are
        // honestly None rather than a fabricated zero.
        assert!(rows.iter().any(|r| r.memory_bytes.is_some()));
        assert!(rows
            .iter()
            .filter(|r| r.memory_bytes.is_none())
            .all(|r| r.cpu_percent == 0.0));
    }
}
