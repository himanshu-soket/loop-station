//! HTTP API for the system monitor: read-only JSON over `/api/sys/*`.
//!
//! Handlers read the shared snapshot; collection never happens on the
//! request path. Process filtering/sorting is delegated to the pure helper so
//! it stays unit-testable.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::sysmon::processes::{filter_sort_limit, SortKey};
use crate::sysmon::{History, SysMonitor};

/// Shared state for sysmon routes.
pub struct SysState {
    /// Live monitor handle.
    pub monitor: SysMonitor,
    /// True when serving clearly-labeled synthetic data.
    pub demo: bool,
}

/// Query params for the process list.
#[derive(Debug, Default, Deserialize)]
pub struct ProcessesQuery {
    /// Free-text search over name or PID.
    q: Option<String>,
    /// `cpu` (default) | `memory` | `name` | `pid`.
    sort: Option<String>,
    /// `asc` | `desc` (default).
    dir: Option<String>,
    /// Max rows (capped at 200).
    limit: Option<u32>,
}

/// `GET /api/sys/overview` — one-shot KPI payload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OverviewBody {
    demo: bool,
    sampled_at_ms: i64,
    interval_ms: u64,
    sample_count: u64,
    cpu_percent: f64,
    cpu_user_percent: f64,
    cpu_system_percent: f64,
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    swap_used_bytes: Option<u64>,
    disk_read_per_sec: Option<f64>,
    disk_write_per_sec: Option<f64>,
    net_recv_per_sec: f64,
    net_sent_per_sec: f64,
    battery_percent: Option<f64>,
    battery_status: Option<String>,
    gpu_utilization_percent: Option<f64>,
    process_count: usize,
}

async fn overview(State(state): State<Arc<SysState>>) -> Response {
    let s = state.monitor.snapshot();
    Json(OverviewBody {
        demo: state.demo,
        sampled_at_ms: s.sampled_at_ms,
        interval_ms: s.interval_ms,
        sample_count: s.sample_count,
        cpu_percent: s.cpu.usage_percent,
        cpu_user_percent: s.cpu.user_percent,
        cpu_system_percent: s.cpu.system_percent,
        memory_used_bytes: s.memory.used_bytes,
        memory_total_bytes: s.memory.total_bytes,
        swap_used_bytes: s.memory.swap_used_bytes,
        disk_read_per_sec: s.disk_io.valid.then_some(s.disk_io.read_bytes_per_sec),
        disk_write_per_sec: s.disk_io.valid.then_some(s.disk_io.written_bytes_per_sec),
        net_recv_per_sec: s.network_totals.received_per_sec,
        net_sent_per_sec: s.network_totals.transmitted_per_sec,
        battery_percent: s.battery.charge_percent.copied(),
        battery_status: s.battery.status.value.clone(),
        gpu_utilization_percent: s.gpu.utilization_percent.copied(),
        process_count: s.processes.len(),
    })
    .into_response()
}

/// `GET /api/sys/snapshot` — full detail (all domains).
///
/// The process table is large (~500 rows) and only needed on the Processes
/// page, so it is excluded by default. Pass `?processes=1` to include it;
/// `/api/sys/processes` remains the dedicated endpoint for the table.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotBody {
    demo: bool,
    /// Number of processes in the snapshot (count only, rows omitted).
    process_count: usize,
    #[serde(flatten)]
    snapshot: SysSnapshotNoProcesses,
}

/// [`SysSnapshot`] without the heavyweight process vector.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SysSnapshotNoProcesses {
    sampled_at_ms: i64,
    interval_ms: u64,
    cpu: crate::sysmon::cpu::CpuSnapshot,
    memory: crate::sysmon::memory::MemorySnapshot,
    volumes: Vec<crate::sysmon::disk::VolumeInfo>,
    disk_io: crate::sysmon::disk::DiskIo,
    interfaces: Vec<crate::sysmon::net::InterfaceInfo>,
    network_totals: crate::sysmon::net::NetworkTotals,
    gpu: crate::sysmon::gpu::GpuSnapshot,
    battery: crate::sysmon::battery::BatterySnapshot,
    sample_count: u64,
    uptime_secs: Option<u64>,
    sample_duration_us: u64,
}

async fn snapshot(State(state): State<Arc<SysState>>) -> Response {
    let s = state.monitor.snapshot();
    Json(SnapshotBody {
        demo: state.demo,
        process_count: s.processes.len(),
        snapshot: SysSnapshotNoProcesses {
            sampled_at_ms: s.sampled_at_ms,
            interval_ms: s.interval_ms,
            cpu: s.cpu,
            memory: s.memory,
            volumes: s.volumes,
            disk_io: s.disk_io,
            interfaces: s.interfaces,
            network_totals: s.network_totals,
            gpu: s.gpu,
            battery: s.battery,
            sample_count: s.sample_count,
            uptime_secs: s.uptime_secs,
            sample_duration_us: s.sample_duration_us,
        },
    })
    .into_response()
}

/// `GET /api/sys/history` — chart series.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryBody {
    demo: bool,
    history: History,
}

async fn history(State(state): State<Arc<SysState>>) -> Response {
    Json(HistoryBody {
        demo: state.demo,
        history: state.monitor.history(),
    })
    .into_response()
}

/// `GET /api/sys/processes` — filtered/sorted process table.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessesBody {
    demo: bool,
    /// Total processes in the snapshot (before filtering).
    total: usize,
    /// Processes matching the current search (before limiting).
    matched: usize,
    items: Vec<serde_json::Value>,
}

const MAX_PROCESS_ROWS: usize = 200;

async fn processes(
    State(state): State<Arc<SysState>>,
    Query(q): Query<ProcessesQuery>,
) -> Response {
    let s = state.monitor.snapshot();
    let total = s.processes.len();
    let sort = SortKey::parse(q.sort.as_deref());
    let descending = q.dir.as_deref().map(str::trim) != Some("asc");
    let limit = q.limit.unwrap_or(50).clamp(1, MAX_PROCESS_ROWS as u32) as usize;
    let rows = filter_sort_limit(s.processes.clone(), q.q.as_deref(), sort, descending, limit);
    // `rows` is already limited; recompute the matched count without the cap
    // so the UI can say "N of M matching" honestly.
    let matched = match q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(needle) => s
            .processes
            .iter()
            .filter(|r| {
                let n = needle.to_lowercase();
                r.name.to_lowercase().contains(&n) || r.pid.to_string().contains(&n)
            })
            .count(),
        None => total,
    };
    Json(ProcessesBody {
        demo: state.demo,
        total,
        matched,
        items: rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "pid": r.pid,
                    "name": r.name,
                    "cpuPercent": r.cpu_percent,
                    // null = unreadable (other user's process), never a fake 0
                    "memoryBytes": r.memory_bytes,
                    "memoryReadable": r.memory_bytes.is_some(),
                })
            })
            .collect(),
    })
    .into_response()
}

/// `POST /api/sys/interval` — adjust sampling cadence (localhost UI only).
#[derive(Deserialize, Default)]
struct IntervalBody {
    ms: Option<u64>,
}

async fn set_interval(
    State(state): State<Arc<SysState>>,
    Json(body): Json<IntervalBody>,
) -> Response {
    let Some(ms) = body.ms else {
        return error_response("missing 'ms' field");
    };
    if !(500..=60_000).contains(&ms) {
        return error_response("interval must be between 500 and 60000 ms");
    }
    // The running loop picks this up on its next tick via the shared snapshot.
    state.monitor.set_interval(Duration::from_millis(ms));
    Json(serde_json::json!({ "ok": true, "intervalMs": ms })).into_response()
}

fn error_response(message: &str) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// Build the sysmon router.
pub fn sys_router(state: Arc<SysState>) -> Router {
    Router::new()
        .route("/api/sys/overview", get(overview))
        .route("/api/sys/snapshot", get(snapshot))
        .route("/api/sys/history", get(history))
        .route("/api/sys/processes", get(processes))
        .route("/api/sys/interval", post(set_interval))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysmon::SampleSource;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<SysState> {
        let monitor = SysMonitor::with_empty_snapshot(Duration::from_secs(2));
        let mut c = SampleSource::real();
        monitor.sample_with(&mut c);
        Arc::new(SysState {
            monitor,
            demo: false,
        })
    }

    async fn get_json(router: &Router, uri: &str) -> (axum::http::StatusCode, serde_json::Value) {
        let res = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn overview_reports_live_shape() {
        let router = sys_router(test_state());
        let (status, body) = get_json(&router, "/api/sys/overview").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body["demo"], false);
        assert!(body["sampleCount"].as_u64().unwrap() >= 1);
        assert!(body["memoryTotalBytes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn snapshot_has_all_domains() {
        let router = sys_router(test_state());
        let (_, body) = get_json(&router, "/api/sys/snapshot").await;
        for key in ["cpu", "memory", "volumes", "interfaces", "gpu", "battery"] {
            assert!(body.get(key).is_some(), "missing {key}");
        }
        // Process rows are excluded by default (payload size); the count is
        // present so the UI can show "N processes" without the table.
        assert!(
            body.get("processes").is_none(),
            "process rows must not bloat the default snapshot"
        );
        assert!(body["processCount"].as_u64().unwrap() > 0);
        // GPU temperature must carry an unavailable reason, never a value.
        let temp = &body["gpu"]["temperatureCelsius"];
        assert_eq!(temp["available"], false);
        assert!(temp["unavailableReason"].is_string());
        // Sampling self-measurement is surfaced.
        assert!(body["sampleDurationUs"].is_u64());
    }

    #[tokio::test]
    async fn processes_support_search_sort_limit() {
        let router = sys_router(test_state());
        let (_, body) = get_json(&router, "/api/sys/processes?limit=5").await;
        assert!(body["total"].as_u64().unwrap() > 10);
        // no search → matched equals total
        assert_eq!(body["matched"], body["total"]);
        assert_eq!(body["items"].as_array().unwrap().len(), 5);

        let (_, sorted) =
            get_json(&router, "/api/sys/processes?sort=memory&dir=desc&limit=3").await;
        let mems: Vec<u64> = sorted["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["memoryBytes"].as_u64().unwrap())
            .collect();
        assert!(
            mems.windows(2).all(|w| w[0] >= w[1]),
            "memory desc: {mems:?}"
        );

        let (_, hit) = get_json(&router, "/api/sys/processes?q=kernel").await;
        // matched reflects the search, not the whole table
        let matched = hit["matched"].as_u64().unwrap();
        assert!(matched > 0 && matched < hit["total"].as_u64().unwrap());
        assert!(hit["items"].as_array().unwrap().iter().all(|i| i["name"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("kernel")));
    }

    #[tokio::test]
    async fn interval_validation() {
        let router = sys_router(test_state());
        // too fast
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sys/interval")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ms": 10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::BAD_REQUEST);
        // valid
        let res = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sys/interval")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ms": 3000}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn history_starts_populated() {
        let router = sys_router(test_state());
        let (_, body) = get_json(&router, "/api/sys/history").await;
        assert_eq!(body["history"]["cpuPercent"].as_array().unwrap().len(), 1);
    }
}
