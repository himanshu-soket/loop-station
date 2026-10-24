//! HTTP API: read-only JSON endpoints over the trace store.
//!
//! Every handler loads a fresh snapshot (cheap at session scale), applies
//! filters in memory, and returns serde-serializable structs. Errors map to
//! JSON `{ "error": "…" }` bodies with proper status codes.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::analytics::{
    overview, session_summaries, timeline, DataWindow, SessionSummary, TimeRange, TurnStatus,
};
use crate::demo::DemoStore;
use crate::payload::EntryPayload;
use crate::store::{EntryRow, RawEntry, SessionRow, StoreError, TraceStore};
use crate::sysmon::SysMonitor;
use crate::web::web_router;

/// Options for building the full application.
pub struct AppOptions {
    /// Serve synthetic traces (demo mode).
    pub demo: bool,
    /// Explicit sessions database path.
    pub db: Option<std::path::PathBuf>,
    /// Serve synthetic system metrics.
    pub sys_demo: bool,
    /// System-monitor sampling interval.
    pub sys_interval: std::time::Duration,
}

/// Unified data source: the real SQLite store or the clearly-labeled demo set.
pub enum Source {
    /// Real session database (read-only).
    Real(TraceStore),
    /// Synthetic data for demos; never mixed with real traces.
    Demo(DemoStore),
}

impl Source {
    fn counts(&self) -> Result<(u64, u64), StoreError> {
        match self {
            Self::Real(s) => s.counts(),
            Self::Demo(d) => d.counts(),
        }
    }

    fn path_label(&self) -> String {
        match self {
            Self::Real(s) => s.path().display().to_string(),
            Self::Demo(d) => d.label.display().to_string(),
        }
    }

    fn is_demo(&self) -> bool {
        matches!(self, Self::Demo(_))
    }
}

fn load_snapshot(source: Option<&Source>) -> Result<(Vec<SessionRow>, Vec<EntryRow>), StoreError> {
    match source {
        Some(Source::Real(store)) => {
            let sessions = store.list_sessions()?;
            let raw = store.list_all_entries()?;
            Ok((sessions, parse_raw(raw)))
        }
        Some(Source::Demo(demo)) => {
            let sessions = demo.list_sessions()?;
            let raw = demo.list_all_entries()?;
            Ok((sessions, parse_raw(raw)))
        }
        None => Ok((Vec::new(), Vec::new())),
    }
}

fn parse_raw(raw: Vec<RawEntry>) -> Vec<EntryRow> {
    raw.into_iter()
        .map(|r| EntryRow {
            session_id: r.session_id,
            entry_id: r.entry_id,
            parent_id: r.parent_id,
            entry_seq: r.entry_seq,
            payload: EntryPayload::parse(&r.payload),
        })
        .collect()
}

fn get_session(source: Option<&Source>, id: &str) -> Result<Option<SessionRow>, StoreError> {
    match source {
        Some(Source::Real(s)) => s.get_session(id),
        Some(Source::Demo(d)) => d.get_session(id),
        None => Ok(None),
    }
}

fn list_entries(source: Option<&Source>, id: &str) -> Result<Vec<EntryRow>, StoreError> {
    match source {
        Some(Source::Real(s)) => s.list_entries(id),
        Some(Source::Demo(d)) => d.list_entries(id),
        None => Ok(Vec::new()),
    }
}

/// Shared application state.
pub struct AppState {
    /// Trace data source (real, demo, or absent when the DB is missing).
    pub source: Option<Source>,
    /// Why traces are unavailable, when they are.
    pub traces_unavailable_reason: Option<String>,
}

/// JSON error body.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    let body = Json(serde_json::json!({ "error": message.into() }));
    (status, body).into_response()
}

impl IntoResponse for StoreError {
    fn into_response(self) -> Response {
        match self {
            Self::Missing(path) => api_error(
                StatusCode::NOT_FOUND,
                format!(
                    "session database not found at {}. Start the loop agent once to create it.",
                    path.display()
                ),
            ),
            Self::Sqlite(e) => api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("database unavailable: {e}"),
            ),
        }
    }
}

/// `GET /api/health`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthBody {
    ok: bool,
    demo: bool,
    db_path: String,
    sessions: u64,
    entries: u64,
    /// False when the trace database could not be opened.
    traces_available: bool,
    /// Human reason traces are unavailable.
    traces_unavailable_reason: Option<String>,
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let body = match &state.source {
        Some(source) => match source.counts() {
            Ok((sessions, entries)) => HealthBody {
                ok: true,
                demo: source.is_demo(),
                db_path: source.path_label(),
                sessions,
                entries,
                traces_available: true,
                traces_unavailable_reason: None,
            },
            Err(err) => HealthBody {
                ok: false,
                demo: source.is_demo(),
                db_path: source.path_label(),
                sessions: 0,
                entries: 0,
                traces_available: false,
                traces_unavailable_reason: Some(err.to_string()),
            },
        },
        None => HealthBody {
            ok: true,
            demo: false,
            db_path: "not found".into(),
            sessions: 0,
            entries: 0,
            traces_available: false,
            traces_unavailable_reason: state.traces_unavailable_reason.clone(),
        },
    };
    Json(body).into_response()
}

/// Query params shared by overview + sessions.
#[derive(Debug, Default, Deserialize)]
pub struct FilterParams {
    /// `all` (default) | `24h` | `7d` | `30d`.
    range: Option<String>,
    /// Model label filter.
    model: Option<String>,
}

/// `GET /api/overview`
async fn overview_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FilterParams>,
) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let result = tokio::task::spawn_blocking(move || {
        let (sessions, entries) = load_snapshot(state.source.as_ref())?;
        let range = TimeRange::parse(params.range.as_deref());
        Ok::<_, StoreError>(overview(
            &sessions,
            &entries,
            range,
            params.model.as_deref(),
            now,
        ))
    })
    .await;
    match result {
        Ok(Ok(data)) => Json(data).into_response(),
        Ok(Err(err)) => err.into_response(),
        Err(join) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// Sortable/paginated session list query.
#[derive(Debug, Default, Deserialize)]
struct SessionsQuery {
    #[serde(flatten)]
    filter: FilterParams,
    /// Free-text search over title, cwd, id, models.
    q: Option<String>,
    /// `all` | `success` | `failed` | `incomplete`.
    status: Option<String>,
    /// `lastActivity` (default) | `created` | `turns` | `tokens` | `latency` | `title`.
    sort: Option<String>,
    /// `asc` | `desc` (default).
    dir: Option<String>,
    /// 1-based page number.
    page: Option<u32>,
    /// Page size (capped at 100).
    per_page: Option<u32>,
}

/// `GET /api/sessions`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsBody {
    total: u64,
    page: u32,
    per_page: u32,
    pages: u32,
    items: Vec<SessionSummary>,
}

const MAX_PER_PAGE: u32 = 100;

async fn sessions_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionsQuery>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let (sessions, entries) = load_snapshot(state.source.as_ref())?;
        let mut summaries = session_summaries(&sessions, &entries);

        // time-range filter: keep sessions whose last activity is in range
        let now = chrono::Utc::now().timestamp_millis();
        let range = TimeRange::parse(query.filter.range.as_deref());
        if let Some(start) = range.start_ms(now) {
            summaries.retain(|s| s.last_activity >= start);
        }

        // model filter
        if let Some(model) = query
            .filter
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != "all")
        {
            summaries.retain(|s| s.models.iter().any(|m| m == model));
        }

        // status filter
        if let Some(status) = query
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let want = match status {
                "success" => Some(TurnStatus::Success),
                "failed" => Some(TurnStatus::Failed),
                "incomplete" => Some(TurnStatus::Incomplete),
                _ => None,
            };
            if let Some(want) = want {
                summaries.retain(|s| s.status == want);
            }
        }

        // free-text search
        if let Some(q) = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let needle = q.to_lowercase();
            summaries.retain(|s| {
                s.title.to_lowercase().contains(&needle)
                    || s.id.to_lowercase().contains(&needle)
                    || s.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
                    || s.models.iter().any(|m| m.to_lowercase().contains(&needle))
            });
        }

        // sort
        let dir_asc = query.dir.as_deref().map(str::trim) == Some("asc");
        let key = query
            .sort
            .as_deref()
            .map(str::trim)
            .unwrap_or("lastActivity");
        summaries.sort_by(|a, b| {
            let ord = match key {
                "created" => a.created_at.cmp(&b.created_at),
                "turns" => a.turns.cmp(&b.turns),
                "tokens" => a.usage.total_or_sum().cmp(&b.usage.total_or_sum()),
                "latency" => a
                    .avg_latency_ms
                    .partial_cmp(&b.avg_latency_ms)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "title" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                _ => a.last_activity.cmp(&b.last_activity),
            };
            if dir_asc {
                ord
            } else {
                ord.reverse()
            }
        });

        let total = summaries.len() as u64;
        let per_page = query.per_page.unwrap_or(20).clamp(1, MAX_PER_PAGE);
        let pages = u32::try_from(total.div_ceil(per_page as u64))
            .unwrap_or(u32::MAX)
            .max(1);
        let page = query.page.unwrap_or(1).clamp(1, pages);
        let skip = ((page - 1) * per_page) as usize;
        let items: Vec<SessionSummary> = summaries
            .into_iter()
            .skip(skip)
            .take(per_page as usize)
            .collect();

        Ok::<_, StoreError>(SessionsBody {
            total,
            page,
            per_page,
            pages,
            items,
        })
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(err)) => err.into_response(),
        Err(join) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/filters` — available models + data window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FiltersBody {
    models: Vec<String>,
    window: DataWindow,
}

async fn filters_handler(State(state): State<Arc<AppState>>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let (_sessions, entries) = load_snapshot(state.source.as_ref())?;
        let mut models: Vec<String> = Vec::new();
        let mut window = DataWindow::default();
        for e in &entries {
            let Some(p) = &e.payload else { continue };
            if let Some(ts) = p.timestamp {
                window.earliest = Some(window.earliest.map_or(ts, |c| c.min(ts)));
                window.latest = Some(window.latest.map_or(ts, |c| c.max(ts)));
            }
            let Some(msg) = &p.message else { continue };
            if let crate::payload::MessageBody::Known(crate::payload::KnownMessage::Assistant(a)) =
                msg
            {
                let label = a
                    .model
                    .clone()
                    .or_else(|| a.response_model.clone())
                    .filter(|m| !m.trim().is_empty());
                if let Some(label) = label {
                    if !models.contains(&label) {
                        models.push(label);
                    }
                }
            }
        }
        models.sort();
        Ok::<_, StoreError>(FiltersBody { models, window })
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(err)) => err.into_response(),
        Err(join) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/sessions/:id` — full detail with timeline.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDetailBody {
    id: String,
    title: String,
    cwd: Option<String>,
    name: Option<String>,
    created_at: i64,
    last_activity: i64,
    summary: SessionSummary,
    timeline: Vec<crate::analytics::TimelineEvent>,
}

async fn session_detail_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let not_found_id = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let Some(meta) = get_session(state.source.as_ref(), &id)? else {
            return Ok::<Option<SessionDetailBody>, StoreError>(None);
        };
        let entries = list_entries(state.source.as_ref(), &id)?;
        let single = vec![SessionRow {
            id: meta.id.clone(),
            cwd: meta.cwd.clone(),
            name: meta.name.clone(),
            parent_session_id: meta.parent_session_id.clone(),
            created_at: meta.created_at,
        }];
        let mut summaries = session_summaries(&single, &entries);
        let summary = summaries.pop().unwrap_or_else(|| SessionSummary {
            id: meta.id.clone(),
            title: crate::analytics::session_title_public(&meta),
            cwd: meta.cwd.clone(),
            created_at: meta.created_at,
            last_activity: meta.created_at,
            turns: 0,
            failed: 0,
            incomplete: 0,
            user_messages: 0,
            tool_calls: 0,
            tool_errors: 0,
            usage: Default::default(),
            models: Vec::new(),
            status: TurnStatus::Incomplete,
            avg_latency_ms: None,
        });
        Ok(Some(SessionDetailBody {
            id: meta.id.clone(),
            title: summary.title.clone(),
            cwd: meta.cwd.clone(),
            name: meta.name.clone(),
            created_at: meta.created_at,
            last_activity: summary.last_activity,
            summary,
            timeline: timeline(&entries),
        }))
    })
    .await;
    match result {
        Ok(Ok(Some(body))) => Json(body).into_response(),
        Ok(Ok(None)) => api_error(
            StatusCode::NOT_FOUND,
            format!("session {not_found_id} not found"),
        ),
        Ok(Err(err)) => err.into_response(),
        Err(join) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `POST /api/shutdown` — intentionally disabled (no remote control surface).
async fn shutdown_handler() -> Response {
    api_error(StatusCode::FORBIDDEN, "shutdown via API is disabled")
}

/// Build the full application router (API + static web).
/// Build the full application router (traces + system monitor + static web).
///
/// A missing trace database no longer aborts startup: traces degrade to an
/// honest "unavailable" state while the system monitor runs normally.
/// Returns the router plus the live monitor handle for spawning the sampler.
pub fn build_app(options: AppOptions) -> anyhow::Result<(Router, SysMonitor)> {
    let (source, unavailable_reason) = if options.demo {
        (Some(Source::Demo(DemoStore::new())), None)
    } else {
        let path = match options.db {
            Some(p) => p,
            None => {
                let home = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("$HOME not set; pass --db explicitly"))?;
                home.join(".loop").join("agent").join("sessions.db")
            }
        };
        match TraceStore::open(&path) {
            Ok(store) => (Some(Source::Real(store)), None),
            Err(StoreError::Missing(p)) => (
                None,
                Some(format!(
                    "session database not found at {}. Start the loop agent once to create it; \
                     the system monitor below works without it.",
                    p.display()
                )),
            ),
            Err(err) => (None, Some(format!("traces unavailable: {err}"))),
        }
    };

    // System monitor: seeded immediately so /system has data on first paint.
    let monitor = SysMonitor::new(options.sys_interval);
    if options.sys_demo {
        let mut demo_source = crate::sysmon::SampleSource::demo();
        monitor.sample_with(&mut demo_source);
    }

    let sys_state = Arc::new(crate::sysmon::api::SysState {
        monitor: monitor.clone(),
        demo: options.sys_demo,
    });

    let state = Arc::new(AppState {
        source,
        traces_unavailable_reason: unavailable_reason,
    });

    let router = Router::new()
        .route("/api/health", get(health))
        .route("/api/overview", get(overview_handler))
        .route("/api/sessions", get(sessions_handler))
        .route("/api/sessions/{id}", get(session_detail_handler))
        .route("/api/filters", get(filters_handler))
        .route("/api/shutdown", post(shutdown_handler))
        .with_state(state)
        .merge(crate::sysmon::api::sys_router(sys_state))
        .merge(web_router());
    Ok((router, monitor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // oneshot

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    fn test_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
               id TEXT PRIMARY KEY, cwd TEXT, name TEXT, parent_session_id TEXT,
               active_leaf_id TEXT, created_at INTEGER NOT NULL);
             CREATE TABLE session_entries (
               session_id TEXT NOT NULL, entry_id TEXT NOT NULL, parent_id TEXT,
               entry_seq INTEGER NOT NULL, payload TEXT NOT NULL,
               PRIMARY KEY (session_id, entry_id));
             INSERT INTO sessions VALUES ('s1','/w/proj',NULL,NULL,NULL,1000);
             INSERT INTO sessions VALUES ('s2','/w/other','named-session',NULL,NULL,2000);
             INSERT INTO session_entries VALUES ('s1','e1',NULL,1,
               '{\"type\":\"message\",\"timestamp\":1100,\"message\":{\"role\":\"user\",\"content\":\"hello\",\"timestamp\":1100}}');
             INSERT INTO session_entries VALUES ('s1','e2','e1',2,
               '{\"type\":\"message\",\"timestamp\":2500,\"message\":{\"role\":\"assistant\",\"model\":\"m1\",\"provider\":\"soket\",\"stopReason\":\"stop\",\"usage\":{\"input\":100,\"output\":20,\"totalTokens\":120},\"content\":[{\"type\":\"text\",\"text\":\"hi there\"}],\"timestamp\":2500}}');
             INSERT INTO session_entries VALUES ('s1','e3','e2',3,
               '{\"type\":\"message\",\"timestamp\":2600,\"message\":{\"role\":\"toolResult\",\"toolCallId\":\"c1\",\"toolName\":\"bash\",\"isError\":true,\"content\":[{\"type\":\"text\",\"text\":\"boom\"}]}}');
             INSERT INTO session_entries VALUES ('s2','e4',NULL,1,
               '{\"type\":\"message\",\"timestamp\":2100,\"message\":{\"role\":\"assistant\",\"model\":\"m2\",\"stopReason\":\"error\",\"errorMessage\":\"provider exploded\",\"content\":[],\"usage\":{\"input\":10,\"output\":0},\"timestamp\":2100}}');",
        )
        .unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn health_reports_counts() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (status, body) = get_json(&app, "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["sessions"], 2);
        assert_eq!(body["entries"], 4);
        assert_eq!(body["demo"], false);
    }

    #[tokio::test]
    async fn overview_aggregates_real_payloads() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (status, body) = get_json(&app, "/api/overview?range=all").await;
        assert_eq!(status, StatusCode::OK);
        // two assistant turns: one success, one failed
        assert_eq!(body["kpis"]["turns"], 2);
        assert_eq!(body["kpis"]["successful"], 1);
        assert_eq!(body["kpis"]["failed"], 1);
        // token totals from stored usage
        assert_eq!(body["kpis"]["usage"]["input"], 110);
        // derived latency only for s1 (user → assistant): 1400ms
        assert_eq!(body["kpis"]["latencySamples"], 1);
        assert_eq!(body["kpis"]["avgLatencyMs"], 1400.0);
        // one tool call flagged as error
        assert_eq!(body["kpis"]["toolErrors"], 1);
        // models seen
        assert_eq!(body["kpis"]["distinctModels"], 2);
        // series buckets exist
        assert!(!body["series"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn model_filter_narrows_overview() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (_, body) = get_json(&app, "/api/overview?model=m1").await;
        assert_eq!(body["kpis"]["turns"], 1);
        assert_eq!(body["kpis"]["successful"], 1);
        let (_, body2) = get_json(&app, "/api/overview?model=nonexistent").await;
        assert_eq!(body2["kpis"]["turns"], 0);
    }

    #[tokio::test]
    async fn sessions_endpoint_filters_sorts_paginates() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();

        // default listing
        let (status, body) = get_json(&app, "/api/sessions").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["total"], 2);
        assert_eq!(body["items"].as_array().unwrap().len(), 2);

        // status filter: only s2 failed
        let (_, failed) = get_json(&app, "/api/sessions?status=failed").await;
        assert_eq!(failed["total"], 1);
        assert_eq!(failed["items"][0]["id"], "s2");

        // search by title text
        let (_, hit) = get_json(&app, "/api/sessions?q=named").await;
        assert_eq!(hit["total"], 1);
        assert_eq!(hit["items"][0]["title"], "named-session");

        // search by cwd fragment
        let (_, hit2) = get_json(&app, "/api/sessions?q=proj").await;
        assert_eq!(hit2["total"], 1);
        assert_eq!(hit2["items"][0]["id"], "s1");

        // sorting asc by created
        let (_, sorted) = get_json(&app, "/api/sessions?sort=created&dir=asc").await;
        assert_eq!(sorted["items"][0]["id"], "s1");

        // pagination: 1 per page → page 2 exists
        let (_, paged) = get_json(&app, "/api/sessions?per_page=1&page=2").await;
        assert_eq!(paged["pages"], 2);
        assert_eq!(paged["items"].as_array().unwrap().len(), 1);

        // out-of-range page clamps
        let (_, clamped) = get_json(&app, "/api/sessions?per_page=1&page=99").await;
        assert_eq!(clamped["page"], 2);
    }

    #[tokio::test]
    async fn session_detail_returns_timeline() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (status, body) = get_json(&app, "/api/sessions/s1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["summary"]["turns"], 1);
        let tl = body["timeline"].as_array().unwrap();
        // user prompt + assistant turn + tool result
        assert_eq!(tl.len(), 3);
        assert_eq!(tl[0]["kind"], "userPrompt");
        assert_eq!(tl[1]["kind"], "assistantTurn");
        assert_eq!(tl[1]["latencyMs"], 1400);
        assert_eq!(tl[2]["kind"], "toolResult");
        assert_eq!(tl[2]["isError"], true);
    }

    #[tokio::test]
    async fn unknown_session_is_404_with_json_error() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (status, body) = get_json(&app, "/api/sessions/does-not-exist").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn missing_db_yields_clean_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.db");
        // build_app should fail loudly rather than serve empty data
        // Missing DB no longer aborts: traces degrade, sysmon still works.
        let (app, _) = build_app(AppOptions {
            demo: false,
            db: Some(missing),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (_, health) = get_json(&app, "/api/health").await;
        assert_eq!(health["tracesAvailable"], false);
        assert!(health["tracesUnavailableReason"]
            .as_str()
            .unwrap()
            .contains("not found"));
        let (_, sys) = get_json(&app, "/api/sys/overview").await;
        assert!(sys["memoryTotalBytes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn filters_endpoint_lists_models() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (status, body) = get_json(&app, "/api/filters").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["models"], serde_json::json!(["m1", "m2"]));
    }

    #[tokio::test]
    async fn demo_mode_is_labeled_everywhere() {
        let (app, _monitor) = build_app(AppOptions {
            demo: true,
            db: None,
            sys_demo: true,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let (_, health) = get_json(&app, "/api/health").await;
        assert_eq!(health["demo"], true);
        let (_, ov) = get_json(&app, "/api/overview").await;
        assert!(ov["kpis"]["turns"].as_u64().unwrap() > 0, "demo has data");
    }

    #[tokio::test]
    async fn shutdown_is_forbidden() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/shutdown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn index_page_serves_html() {
        let (_dir, db) = test_db();
        let (app, _monitor) = build_app(AppOptions {
            demo: false,
            db: Some(db),
            sys_demo: false,
            sys_interval: std::time::Duration::from_secs(2),
        })
        .unwrap();
        let res = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.starts_with("text/html"));
    }
}
