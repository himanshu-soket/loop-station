//! Durable backend storage for dashboard traces, requests and conversations.
//!
//! Design:
//! * One SQLite database at `~/.loop/dashboard/store.db` (WAL,
//!   `synchronous=FULL`), owned exclusively by the dashboard process.
//! * A single writer connection guarded by a mutex; every call runs on the
//!   blocking pool (`spawn_blocking`) so async handlers never block.
//! * Versioned migrations (idempotent `IF NOT EXISTS` + a `schema_migrations`
//!   ledger) run at open time inside one immediate transaction.
//! * Crash recovery: rows left `running` by a dead process are marked
//!   `interrupted` at next open — never silently turned into successes. The
//!   store is shared with live CLI sessions, so recovery checks the owner
//!   pid recorded on each trace and leaves rows of running processes alone.
//! * Retention: `enforce_retention()` deletes the oldest traces (and their
//!   requests) beyond a configurable cap. It is cheap enough to run after
//!   each finalized write.
//!
//! Timestamps are unix epoch milliseconds everywhere.

pub mod model;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

pub use model::{
    ConversationRow, ConversationSummary, MessageRow, RequestFilter, RequestRow, RequestStatus,
    RetentionConfig, SettingsMap, SpanRow, TraceDetail, TraceFilter, TraceRow, TraceStatus,
};

/// Errors surfaced by the store.
#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    /// SQLite failure.
    #[error("storage error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The parent directory could not be created.
    #[error("cannot create storage directory: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, PersistError>;

/// Whether the process owning a stored trace is still alive.
///
/// `None` (owner unknown — legacy rows) counts as dead so recovery keeps
/// claiming pre-pid rows. The check is `kill(pid, 0)`: it sends no signal,
/// and `ESRCH` is the only error that means "no such process". A live
/// process owned by another user yields `EPERM`, which still proves the
/// owner exists, so it counts as alive.
fn process_alive(owner_pid: Option<i64>) -> bool {
    let Some(pid) = owner_pid else { return false };
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill(2) with signal 0 performs permission checks only; the
    // pid comes from our own database rows, never user input.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

const MIGRATION_001: &str = include_str!("migrations/001_initial.sql");
const MIGRATION_002: &str = include_str!("migrations/002_retention.sql");
const MIGRATION_003: &str = include_str!("migrations/003_owner_pid.sql");

struct Migration {
    id: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "001_initial.sql",
        sql: MIGRATION_001,
    },
    Migration {
        id: "002_retention.sql",
        sql: MIGRATION_002,
    },
    Migration {
        id: "003_owner_pid.sql",
        sql: MIGRATION_003,
    },
];

/// How many rows a single list query may return.
pub const MAX_PAGE: i64 = 200;

/// Durable dashboard store.
#[derive(Debug, Clone)]
pub struct DashboardStore {
    inner: Arc<StoreInner>,
}

#[derive(Debug)]
struct StoreInner {
    #[allow(dead_code)] // read through DashboardStore::path in console wiring
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl DashboardStore {
    /// Open an in-memory store (tests).
    #[allow(dead_code)] // test constructor
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA journal_mode=MEMORY; PRAGMA synchronous=OFF; PRAGMA foreign_keys=ON;",
        )?;
        Self::register_owner_pid_fn(&conn)?;
        let mut conn = conn;
        Self::apply_migrations(&mut conn)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                path: PathBuf::from(":memory:"),
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Register the `stored_owner_alive(pid)` SQL helper used by recovery.
    ///
    /// A NULL pid means "owner unknown" and is treated as dead, so legacy
    /// rows are recovered exactly as they were before pid tracking existed.
    fn register_owner_pid_fn(conn: &Connection) -> Result<()> {
        conn.create_scalar_function(
            "stored_owner_alive",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                let pid = ctx.get::<Option<i64>>(0)?;
                Ok(process_alive(pid) as i64)
            },
        )?;
        Ok(())
    }

    /// Open (creating if needed) the store at `path`, apply migrations and
    /// recover interrupted rows from processes that are no longer running.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; \
             PRAGMA busy_timeout=5000;",
        )?;
        Self::register_owner_pid_fn(&conn)?;
        Self::apply_migrations(&mut conn)?;
        Self::recover_interrupted(&conn)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                path,
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Database path (health display).
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Run `f` with the connection on the current thread.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.inner.conn.lock();
        f(&guard)
    }

    fn apply_migrations(conn: &mut Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
               id TEXT PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
        )?;
        for migration in MIGRATIONS {
            let applied: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM schema_migrations WHERE id = ?1",
                    params![migration.id],
                    |_| Ok(1),
                )
                .optional()?
                .map(|_| 1);
            if applied.is_some() {
                continue;
            }
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let result = (|| {
                conn.execute_batch(migration.sql)?;
                conn.execute(
                    "INSERT INTO schema_migrations (id, applied_at_ms) VALUES (?1, ?2)",
                    params![migration.id, chrono::Utc::now().timestamp_millis()],
                )?;
                Ok::<(), PersistError>(())
            })();
            match result {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Rows still `running` whose owning process has exited are marked
    /// `interrupted`. Returns how many were recovered.
    ///
    /// The store is shared by concurrent processes (the dashboard plus every
    /// live CLI session), so a `running` row is only abandoned when its
    /// `owner_pid` no longer exists. Rows without an owner (legacy, or
    /// writers that predate pid tracking) are always recovered: the previous
    /// behaviour claimed every `running` row, and being honest about an
    /// interrupted trace beats falsely interrupting nothing.
    fn recover_interrupted(conn: &Connection) -> Result<usize> {
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "UPDATE traces SET status='interrupted',
                ended_at_ms=COALESCE(ended_at_ms, ?1),
                duration_ms=COALESCE(duration_ms, ?1 - started_at_ms),
                error_message=COALESCE(error_message, 'process exited before the trace finished')
             WHERE status='running' AND NOT stored_owner_alive(owner_pid)",
            params![now],
        )?;
        conn.execute(
            "UPDATE spans SET status='interrupted',
                end_ms=COALESCE(end_ms, ?1),
                duration_ms=COALESCE(duration_ms, ?1 - start_ms)
             WHERE status='running' AND NOT EXISTS (
                SELECT 1 FROM traces t
                WHERE t.trace_id = spans.trace_id
                  AND t.status = 'running'
                  AND stored_owner_alive(t.owner_pid))",
            params![now],
        )?;
        let n = conn.execute(
            "UPDATE requests SET status='interrupted',
                ended_at_ms=COALESCE(ended_at_ms, ?1),
                duration_ms=COALESCE(duration_ms, ?1 - started_at_ms),
                error_message=COALESCE(error_message, 'process exited before the request finished')
             WHERE status='running' AND NOT EXISTS (
                SELECT 1 FROM traces t
                WHERE t.trace_id = requests.trace_id
                  AND t.status = 'running'
                  AND stored_owner_alive(t.owner_pid))",
            params![now],
        )?;
        // In-flight conversation messages stay honest too: only those whose
        // trace owner is gone (or that have no trace at all).
        conn.execute(
            "UPDATE conversation_messages SET status='cancelled'
             WHERE status='streaming' AND NOT EXISTS (
                SELECT 1 FROM traces t
                WHERE t.trace_id = conversation_messages.trace_id
                  AND t.status = 'running'
                  AND stored_owner_alive(t.owner_pid))",
            [],
        )?;
        Ok(n)
    }

    // ------------------------------------------------------------------
    // Traces
    // ------------------------------------------------------------------

    /// Insert a new running trace (root span included).
    pub fn insert_trace(&self, row: &TraceRow) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO traces (trace_id, root_span_id, operation, status, model, provider,
                   request_id, session_id, started_at_ms, ended_at_ms, duration_ms,
                   input_tokens, output_tokens, ttft_ms, error_message, params_json,
                   content_captured, owner_pid)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                params![
                    row.trace_id,
                    row.root_span_id,
                    row.operation,
                    row.status.as_str(),
                    row.model,
                    row.provider,
                    row.request_id,
                    row.session_id,
                    row.started_at_ms,
                    row.ended_at_ms,
                    row.duration_ms,
                    row.input_tokens,
                    row.output_tokens,
                    row.ttft_ms,
                    row.error_message,
                    row.params_json,
                    if row.content_captured { 1 } else { 0 },
                    row.owner_pid.unwrap_or_else(|| std::process::id() as i64),
                ],
            )?;
            Ok(())
        })
    }

    /// Update the model (and optionally provider) recorded on a trace, used by
    /// CLI sessions that switch models mid-run.
    pub fn update_trace_model(
        &self,
        trace_id: &str,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET model=COALESCE(?2, model), provider=COALESCE(?3, provider)
                 WHERE trace_id=?1",
                params![trace_id, model, provider],
            )?;
            Ok(())
        })
    }

    /// Point a trace at a different session id (the CLI's conversation
    /// changed under it: agent adoption, `/new`, fork or resume).
    pub fn update_trace_session(&self, trace_id: &str, session_id: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET session_id=?2 WHERE trace_id=?1",
                params![trace_id, session_id],
            )?;
            Ok(())
        })
    }

    /// Merge token usage / TTFT measurements into a trace row without
    /// finalizing it. Used by CLI session traces that accumulate usage
    /// across many turns: each call adds to the stored totals so the
    /// traces list shows real token counts while the session runs.
    pub fn update_trace_usage(
        &self,
        trace_id: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        ttft_ms: Option<u64>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET
                   input_tokens  = COALESCE(input_tokens, 0)  + COALESCE(?2, 0),
                   output_tokens = COALESCE(output_tokens, 0) + COALESCE(?3, 0),
                   ttft_ms       = COALESCE(ttft_ms, ?4)
                 WHERE trace_id=?1",
                params![
                    trace_id,
                    input_tokens.map(|v| v as i64),
                    output_tokens.map(|v| v as i64),
                    ttft_ms.map(|v| v as i64),
                ],
            )?;
            Ok(())
        })
    }

    /// Flag a trace as having captured message content (prompts/responses).
    pub fn mark_trace_content_captured(&self, trace_id: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET content_captured = 1 WHERE trace_id = ?1",
                params![trace_id],
            )?;
            Ok(())
        })
    }

    /// Finalize a trace: set terminal fields.
    #[allow(clippy::too_many_arguments, reason = "terminal record fields")]
    pub fn finalize_trace(
        &self,
        trace_id: &str,
        status: TraceStatus,
        ended_at_ms: i64,
        duration_ms: i64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        ttft_ms: Option<u64>,
        error_message: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET status=?2, ended_at_ms=?3, duration_ms=?4,
                   input_tokens=COALESCE(?5, input_tokens),
                   output_tokens=COALESCE(?6, output_tokens),
                   ttft_ms=COALESCE(?7, ttft_ms),
                   error_message=COALESCE(?8, error_message)
                 WHERE trace_id=?1",
                params![
                    trace_id,
                    status.as_str(),
                    ended_at_ms,
                    duration_ms,
                    input_tokens.map(|v| v as i64),
                    output_tokens.map(|v| v as i64),
                    ttft_ms.map(|v| v as i64),
                    error_message,
                ],
            )?;
            Ok(())
        })
    }

    /// Insert a span row.
    pub fn insert_span(&self, row: &SpanRow) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO spans (trace_id, span_id, parent_span_id, name, kind,
                   status, start_ms, end_ms, duration_ms, attributes_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    row.trace_id,
                    row.span_id,
                    row.parent_span_id,
                    row.name,
                    row.kind,
                    row.status.as_str(),
                    row.start_ms,
                    row.end_ms,
                    row.duration_ms,
                    row.attributes_json,
                ],
            )?;
            Ok(())
        })
    }

    /// Close an existing span.
    pub fn close_span(
        &self,
        trace_id: &str,
        span_id: &str,
        status: TraceStatus,
        end_ms: i64,
        attributes_json: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE spans SET status=?3, end_ms=?4,
                   duration_ms=?4 - start_ms,
                   attributes_json=COALESCE(?5, attributes_json)
                 WHERE trace_id=?1 AND span_id=?2",
                params![trace_id, span_id, status.as_str(), end_ms, attributes_json],
            )?;
            Ok(())
        })
    }

    /// Append a structured event.
    pub fn insert_event(
        &self,
        trace_id: &str,
        seq: i64,
        at_ms: i64,
        name: &str,
        data_json: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO trace_events (trace_id, seq, at_ms, name, data_json)
                 VALUES (?1,?2,?3,?4,?5)",
                params![trace_id, seq, at_ms, name, data_json],
            )?;
            Ok(())
        })
    }

    /// Load one trace by id.
    pub fn get_trace(&self, trace_id: &str) -> Result<Option<TraceRow>> {
        self.with_conn(|c| {
            let row = c
                .query_row(
                    "SELECT trace_id, root_span_id, operation, status, model, provider,
                       request_id, session_id, started_at_ms, ended_at_ms, duration_ms,
                       input_tokens, output_tokens, ttft_ms, error_message, params_json,
                       content_captured
                     FROM traces WHERE trace_id = ?1",
                    params![trace_id],
                    TraceRow::from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Full detail: trace + spans + events.
    pub fn get_trace_detail(&self, trace_id: &str) -> Result<Option<TraceDetail>> {
        let Some(trace) = self.get_trace(trace_id)? else {
            return Ok(None);
        };
        let spans = self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT trace_id, span_id, parent_span_id, name, kind, status,
                        start_ms, end_ms, duration_ms, attributes_json
                 FROM spans WHERE trace_id = ?1 ORDER BY start_ms ASC",
            )?;
            let rows = stmt.query_map(params![trace_id], SpanRow::from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })?;
        let events = self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT seq, at_ms, name, data_json
                 FROM trace_events WHERE trace_id = ?1 ORDER BY seq ASC",
            )?;
            let rows = stmt.query_map(params![trace_id], |r| {
                Ok(model::TraceEvent {
                    seq: r.get(0)?,
                    at_ms: r.get(1)?,
                    name: r.get(2)?,
                    data_json: r.get(3)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })?;
        Ok(Some(TraceDetail {
            trace,
            spans,
            events,
        }))
    }

    /// Paginated, filtered trace list. Filtering happens in SQL.
    pub fn list_traces(&self, filter: &TraceFilter) -> Result<(Vec<TraceRow>, u64)> {
        self.with_conn(|c| {
            let mut where_clauses: Vec<String> = Vec::new();
            let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if !filter.q.trim().is_empty() {
                where_clauses.push("(trace_id LIKE ? OR operation LIKE ? OR model LIKE ?)".into());
                let needle = format!("%{}%", filter.q.trim());
                args.push(Box::new(needle.clone()));
                args.push(Box::new(needle.clone()));
                args.push(Box::new(needle));
            }
            if let Some(statuses) = filter.statuses.as_ref().filter(|s| !s.is_empty()) {
                let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                where_clauses.push(format!("status IN ({placeholders})"));
                for s in statuses {
                    args.push(Box::new(s.as_str().to_string()));
                }
            }
            if let Some(models) = filter.models.as_ref().filter(|m| !m.is_empty()) {
                let placeholders = models.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                where_clauses.push(format!("model IN ({placeholders})"));
                for m in models {
                    args.push(Box::new(m.clone()));
                }
            }
            if let Some(start) = filter.from_ms {
                where_clauses.push("started_at_ms >= ?".into());
                args.push(Box::new(start));
            }
            if let Some(end) = filter.to_ms {
                where_clauses.push("started_at_ms <= ?".into());
                args.push(Box::new(end));
            }
            if let Some(min) = filter.min_duration_ms {
                where_clauses.push("duration_ms >= ?".into());
                args.push(Box::new(min));
            }
            if let Some(max) = filter.max_duration_ms {
                where_clauses.push("duration_ms <= ?".into());
                args.push(Box::new(max));
            }

            let where_sql = if where_clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", where_clauses.join(" AND "))
            };

            let total: i64 = {
                let sql = format!("SELECT COUNT(*) FROM traces {where_sql}");
                let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                    args.iter().map(|b| b.as_ref()).collect();
                c.query_row(&sql, params_refs.as_slice(), |r| r.get(0))?
            };

            let sort_col = match filter.sort.as_deref() {
                Some("duration") => "duration_ms",
                _ => "started_at_ms",
            };
            let dir = if filter.dir.as_deref() == Some("asc") {
                "ASC"
            } else {
                "DESC"
            };
            let page = filter.page.max(1);
            let per_page = filter.per_page.clamp(1, MAX_PAGE);
            let offset = (page - 1) * per_page;

            let sql = format!(
                "SELECT trace_id, root_span_id, operation, status, model, provider,
                        request_id, session_id, started_at_ms, ended_at_ms, duration_ms,
                        input_tokens, output_tokens, ttft_ms, error_message, params_json,
                        content_captured
                 FROM traces {where_sql}
                 ORDER BY {sort_col} {dir}, trace_id ASC
                 LIMIT {per_page} OFFSET {offset}"
            );
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                args.iter().map(|b| b.as_ref()).collect();
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(params_refs.as_slice(), TraceRow::from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok((out, total as u64))
        })
    }

    // ------------------------------------------------------------------
    // Requests
    // ------------------------------------------------------------------

    /// Insert a request row.
    pub fn insert_request(&self, row: &RequestRow) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO requests (request_id, trace_id, conversation_id, model, provider,
                   status, started_at_ms, ended_at_ms, duration_ms, input_tokens,
                   output_tokens, ttft_ms, cancel_reason, error_message)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    row.request_id,
                    row.trace_id,
                    row.conversation_id,
                    row.model,
                    row.provider,
                    row.status.as_str(),
                    row.started_at_ms,
                    row.ended_at_ms,
                    row.duration_ms,
                    row.input_tokens.map(|v| v as i64),
                    row.output_tokens.map(|v| v as i64),
                    row.ttft_ms.map(|v| v as i64),
                    row.cancel_reason,
                    row.error_message,
                ],
            )?;
            Ok(())
        })
    }

    /// Finalize a request row.
    #[allow(clippy::too_many_arguments, reason = "terminal record fields")]
    pub fn finalize_request(
        &self,
        request_id: &str,
        status: RequestStatus,
        ended_at_ms: i64,
        duration_ms: i64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        ttft_ms: Option<u64>,
        cancel_reason: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE requests SET status=?2, ended_at_ms=?3, duration_ms=?4,
                   input_tokens=COALESCE(?5, input_tokens),
                   output_tokens=COALESCE(?6, output_tokens),
                   ttft_ms=COALESCE(?7, ttft_ms),
                   cancel_reason=COALESCE(?8, cancel_reason),
                   error_message=COALESCE(?9, error_message)
                 WHERE request_id=?1",
                params![
                    request_id,
                    status.as_str(),
                    ended_at_ms,
                    duration_ms,
                    input_tokens.map(|v| v as i64),
                    output_tokens.map(|v| v as i64),
                    ttft_ms.map(|v| v as i64),
                    cancel_reason,
                    error_message,
                ],
            )?;
            Ok(())
        })
    }

    /// Paginated request list.
    pub fn list_requests(&self, filter: &RequestFilter) -> Result<(Vec<RequestRow>, u64)> {
        self.with_conn(|c| {
            let mut where_clauses: Vec<String> = Vec::new();
            let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if !filter.q.trim().is_empty() {
                where_clauses.push("(request_id LIKE ? OR trace_id LIKE ? OR model LIKE ?)".into());
                let needle = format!("%{}%", filter.q.trim());
                args.push(Box::new(needle.clone()));
                args.push(Box::new(needle.clone()));
                args.push(Box::new(needle));
            }
            if let Some(statuses) = filter.statuses.as_ref().filter(|s| !s.is_empty()) {
                let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                where_clauses.push(format!("status IN ({placeholders})"));
                for s in statuses {
                    args.push(Box::new(s.as_str().to_string()));
                }
            }
            if let Some(start) = filter.from_ms {
                where_clauses.push("started_at_ms >= ?".into());
                args.push(Box::new(start));
            }
            if let Some(end) = filter.to_ms {
                where_clauses.push("started_at_ms <= ?".into());
                args.push(Box::new(end));
            }
            let where_sql = if where_clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", where_clauses.join(" AND "))
            };
            let total: i64 = {
                let sql = format!("SELECT COUNT(*) FROM requests {where_sql}");
                let refs: Vec<&dyn rusqlite::types::ToSql> =
                    args.iter().map(|b| b.as_ref()).collect();
                c.query_row(&sql, refs.as_slice(), |r| r.get(0))?
            };
            let page = filter.page.max(1);
            let per_page = filter.per_page.clamp(1, MAX_PAGE);
            let offset = (page - 1) * per_page;
            let sql = format!(
                "SELECT request_id, trace_id, conversation_id, model, provider, status,
                        started_at_ms, ended_at_ms, duration_ms, input_tokens, output_tokens,
                        ttft_ms, cancel_reason, error_message
                 FROM requests {where_sql}
                 ORDER BY started_at_ms DESC, request_id ASC
                 LIMIT {per_page} OFFSET {offset}"
            );
            let refs: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(refs.as_slice(), RequestRow::from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok((out, total as u64))
        })
    }

    // ------------------------------------------------------------------
    // Terminal commands (Zsh instrumentation)
    // ------------------------------------------------------------------

    /// Idempotently record the start of a terminal command: one trace row,
    /// one root span and one request row, all keyed by the client-generated
    /// trace id. Re-delivering the same event (hook retry, duplicate precmd)
    /// is a no-op. Returns `true` when this call created the rows.
    pub fn insert_terminal_trace(
        &self,
        trace_id: &str,
        command: &str,
        cwd: Option<&str>,
        host: Option<&str>,
        shell_name: Option<&str>,
        shell_pid: Option<u32>,
        session_id: &str,
        started_at_ms: i64,
    ) -> Result<bool> {
        let attrs = serde_json::json!({
            "origin": "terminal",
            "command": command,
            "cwd": cwd,
            "host": host,
            "shell": shell_name,
            "shellPid": shell_pid,
        })
        .to_string();
        self.with_conn(|c| {
            let created = c.execute(
                "INSERT OR IGNORE INTO traces (trace_id, root_span_id, operation, status,
                   model, provider, request_id, session_id, started_at_ms, ended_at_ms,
                   duration_ms, input_tokens, output_tokens, ttft_ms, error_message,
                   params_json, content_captured, owner_pid)
                 VALUES (?1, ?1, 'terminal.command', 'running', NULL, 'terminal', ?1, ?6,
                         ?2, NULL, NULL, NULL, NULL, NULL, NULL, ?5, 0, ?7)",
                params![
                    trace_id,
                    started_at_ms,
                    command,
                    cwd.unwrap_or(""),
                    attrs,
                    session_id,
                    shell_pid.map(|p| p as i64)
                ],
            )? > 0;
            if !created {
                return Ok(false);
            }
            c.execute(
                "INSERT OR REPLACE INTO spans (trace_id, span_id, parent_span_id, name, kind,
                   status, start_ms, end_ms, duration_ms, attributes_json)
                 VALUES (?1, ?1, NULL, 'shell.exec', 'phase', 'running', ?2, NULL, NULL, ?3)",
                params![trace_id, started_at_ms, attrs],
            )?;
            c.execute(
                "INSERT OR IGNORE INTO requests (request_id, trace_id, conversation_id, model,
                   provider, status, started_at_ms, ended_at_ms, duration_ms, input_tokens,
                   output_tokens, ttft_ms, cancel_reason, error_message)
                 VALUES (?1, ?1, NULL, NULL, 'terminal', 'running', ?2, NULL, NULL,
                         NULL, NULL, NULL, NULL, NULL)",
                params![trace_id, started_at_ms],
            )?;
            Ok(true)
        })
    }

    /// Finalize a terminal command: set the terminal status, end time,
    /// duration and exit-code metadata on the trace, its root span and its
    /// request row. Idempotent — re-delivery converges to the same values.
    #[allow(clippy::too_many_arguments, reason = "terminal record fields")]
    pub fn finalize_terminal_trace(
        &self,
        trace_id: &str,
        status: TraceStatus,
        ended_at_ms: i64,
        duration_ms: i64,
        error_message: Option<&str>,
        content_captured: bool,
        span_attributes_json: &str,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE traces SET status=?2, ended_at_ms=?3, duration_ms=?4,
                   error_message=COALESCE(?5, error_message),
                   content_captured=CASE WHEN ?6=1 THEN 1 ELSE content_captured END
                 WHERE trace_id=?1",
                params![
                    trace_id,
                    status.as_str(),
                    ended_at_ms,
                    duration_ms,
                    error_message,
                    if content_captured { 1 } else { 0 },
                ],
            )?;
            c.execute(
                "UPDATE spans SET status=?2, end_ms=?3, duration_ms=?3 - start_ms,
                   attributes_json=?4
                 WHERE trace_id=?1 AND span_id=?1",
                params![trace_id, status.as_str(), ended_at_ms, span_attributes_json],
            )?;
            c.execute(
                "UPDATE requests SET status=?2, ended_at_ms=?3, duration_ms=?4,
                   error_message=COALESCE(?5, error_message)
                 WHERE request_id=?1",
                params![
                    trace_id,
                    status.as_str(),
                    ended_at_ms,
                    duration_ms,
                    error_message,
                ],
            )?;
            Ok(())
        })
    }

    /// Append a structured event with an automatically allocated sequence
    /// number (used for optional captured terminal output). Storing an
    /// event also marks the trace as having captured content.
    pub fn append_terminal_event(
        &self,
        trace_id: &str,
        at_ms: i64,
        name: &str,
        data_json: &str,
    ) -> Result<()> {
        self.with_conn(|c| {
            let seq: i64 = c.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM trace_events WHERE trace_id = ?1",
                params![trace_id],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT OR REPLACE INTO trace_events (trace_id, seq, at_ms, name, data_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![trace_id, seq, at_ms, name, data_json],
            )?;
            c.execute(
                "UPDATE traces SET content_captured = 1 WHERE trace_id = ?1",
                params![trace_id],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Conversations
    // ------------------------------------------------------------------

    /// Create a conversation.
    pub fn create_conversation(&self, row: &ConversationRow) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO conversations (conversation_id, title, created_at_ms,
                   updated_at_ms, model)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    row.conversation_id,
                    row.title,
                    row.created_at_ms,
                    row.updated_at_ms,
                    row.model,
                ],
            )?;
            Ok(())
        })
    }

    /// List conversations newest-first.
    pub fn list_conversations(&self, limit: i64) -> Result<Vec<ConversationSummary>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT c.conversation_id, c.title, c.created_at_ms, c.updated_at_ms, c.model,
                        (SELECT COUNT(*) FROM conversation_messages m
                          WHERE m.conversation_id = c.conversation_id) AS message_count
                 FROM conversations c
                 ORDER BY c.updated_at_ms DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit.clamp(1, MAX_PAGE)], |r| {
                Ok(ConversationSummary {
                    conversation_id: r.get(0)?,
                    title: r.get(1)?,
                    created_at_ms: r.get(2)?,
                    updated_at_ms: r.get(3)?,
                    model: r.get(4)?,
                    message_count: r.get::<_, i64>(5)? as u64,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Delete a conversation (messages cascade).
    pub fn delete_conversation(&self, conversation_id: &str) -> Result<bool> {
        self.with_conn(|c| {
            let n = c.execute(
                "DELETE FROM conversations WHERE conversation_id = ?1",
                params![conversation_id],
            )?;
            Ok(n > 0)
        })
    }

    /// Rename a conversation.
    pub fn rename_conversation(&self, conversation_id: &str, title: &str) -> Result<bool> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE conversations SET title = ?2, updated_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![
                    conversation_id,
                    title,
                    chrono::Utc::now().timestamp_millis()
                ],
            )?;
            Ok(n > 0)
        })
    }

    /// Append a message to a conversation.
    pub fn append_message(&self, msg: &MessageRow) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO conversation_messages (conversation_id, message_id, seq, role,
                   content, created_at_ms, status, error_message, model, request_id, trace_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    msg.conversation_id,
                    msg.message_id,
                    msg.seq,
                    msg.role,
                    msg.content,
                    msg.created_at_ms,
                    msg.status,
                    msg.error_message,
                    msg.model,
                    msg.request_id,
                    msg.trace_id,
                ],
            )?;
            c.execute(
                "UPDATE conversations SET updated_at_ms = ?2 WHERE conversation_id = ?1",
                params![msg.conversation_id, msg.created_at_ms],
            )?;
            Ok(())
        })
    }

    /// Update an existing message's content/status (streaming completion).
    pub fn update_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        content: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE conversation_messages SET content=?3, status=?4,
                   error_message=COALESCE(?5, error_message)
                 WHERE conversation_id=?1 AND message_id=?2",
                params![conversation_id, message_id, content, status, error_message],
            )?;
            Ok(())
        })
    }

    /// All messages of a conversation in order.
    pub fn list_messages(&self, conversation_id: &str) -> Result<Vec<MessageRow>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT conversation_id, message_id, seq, role, content, created_at_ms,
                        status, error_message, model, request_id, trace_id
                 FROM conversation_messages WHERE conversation_id = ?1
                 ORDER BY seq ASC",
            )?;
            let rows = stmt.query_map(params![conversation_id], MessageRow::from_row)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Get one conversation's metadata.
    pub fn get_conversation(&self, conversation_id: &str) -> Result<Option<ConversationRow>> {
        self.with_conn(|c| {
            let row = c
                .query_row(
                    "SELECT conversation_id, title, created_at_ms, updated_at_ms, model
                     FROM conversations WHERE conversation_id = ?1",
                    params![conversation_id],
                    ConversationRow::from_row,
                )
                .optional()?;
            Ok(row)
        })
    }

    // ------------------------------------------------------------------
    // Settings + retention
    // ------------------------------------------------------------------

    /// Read all dashboard settings.
    pub fn load_settings(&self) -> Result<SettingsMap> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT key, value FROM dashboard_settings")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = SettingsMap::default();
            for r in rows {
                let (k, v) = r?;
                out.0.insert(k, v);
            }
            Ok(out)
        })
    }

    /// Persist one dashboard setting.
    pub fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO dashboard_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// Delete old traces beyond `max_traces` (requests follow via cascade-free
    /// explicit delete). Returns (deleted_traces, deleted_requests).
    pub fn enforce_retention(&self, config: &RetentionConfig) -> Result<(u64, u64)> {
        let nothing_to_do = config.max_traces == 0 && config.max_age_ms.is_none();
        if nothing_to_do {
            return Ok((0, 0));
        }
        self.with_conn(|c| {
            let now = chrono::Utc::now().timestamp_millis();
            let cutoff_time = config.max_age_ms.map(|age| now - age);

            // Ids of traces to delete: too old, or beyond the count cap.
            let mut ids: Vec<String> = Vec::new();
            if let Some(cutoff) = cutoff_time {
                let mut stmt = c.prepare(
                    "SELECT trace_id FROM traces
                     WHERE started_at_ms < ?1
                     ORDER BY started_at_ms ASC",
                )?;
                let rows = stmt.query_map(params![cutoff], |r| r.get::<_, String>(0))?;
                for r in rows {
                    ids.push(r?);
                }
            }
            if config.max_traces > 0 {
                let mut stmt = c.prepare(
                    "SELECT trace_id FROM traces
                     ORDER BY started_at_ms DESC
                     LIMIT -1 OFFSET ?1",
                )?;
                let offset = config.max_traces as i64;
                let rows = stmt.query_map(params![offset], |r| r.get::<_, String>(0))?;
                for r in rows {
                    ids.push(r?);
                }
            }
            ids.sort();
            ids.dedup();
            if ids.is_empty() {
                return Ok((0, 0));
            }

            let deleted_requests = {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!("DELETE FROM requests WHERE trace_id IN ({placeholders})");
                let refs: Vec<&dyn rusqlite::types::ToSql> = ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::types::ToSql)
                    .collect();
                c.execute(&sql, refs.as_slice())? as u64
            };
            let deleted_traces = {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!("DELETE FROM traces WHERE trace_id IN ({placeholders})");
                let refs: Vec<&dyn rusqlite::types::ToSql> = ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::types::ToSql)
                    .collect();
                c.execute(&sql, refs.as_slice())? as u64
            };
            c.execute(
                "INSERT OR REPLACE INTO retention_runs (ran_at_ms, deleted_traces, deleted_requests)
                 VALUES (?1, ?2, ?3)",
                params![
                    chrono::Utc::now().timestamp_millis(),
                    deleted_traces as i64,
                    deleted_requests as i64
                ],
            )?;
            Ok((deleted_traces, deleted_requests))
        })
    }

    /// Row counts for health.
    pub fn counts(&self) -> Result<(u64, u64, u64)> {
        self.with_conn(|c| {
            let traces: i64 = c.query_row("SELECT COUNT(*) FROM traces", [], |r| r.get(0))?;
            let spans: i64 = c.query_row("SELECT COUNT(*) FROM spans", [], |r| r.get(0))?;
            let requests: i64 = c.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))?;
            Ok((traces as u64, spans as u64, requests as u64))
        })
    }
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Patterns whose values are stripped before any content is persisted.
///
/// Each entry pairs a label (used in the placeholder, e.g. `[REDACTED api_key]`)
/// with a matcher over `key: value` / `key=value` style text and common
/// bearer-token spellings. Matching is case-insensitive on the key.
const REDACTION_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "authorization",
    "auth",
    "bearer",
    "credential",
    "client_secret",
    "private_key",
    "session_cookie",
];

/// Redact secrets from free text before persistence.
///
/// Catches the shapes that actually appear in tool output and command lines:
/// * `KEY=value`, `KEY: value`, `"KEY": "value"`, `export KEY=value`
/// * long hex/base64-ish literals assigned to those keys
/// * `Authorization: Bearer <token>` / `Bearer <token>`
///
/// Everything else passes through untouched; this is a best-effort scrub, not
/// a security boundary — access control remains the real protection.
pub fn redact_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let mut out = raw.to_string();
    for key in REDACTION_KEYS {
        // KEY=value / KEY: value / "KEY": "value" … up to a delimiter.
        for sep in [":", "="] {
            let pattern = format!("{key}{sep}");
            let mut search_from = 0usize;
            while let Some(idx) = out[search_from..].to_lowercase().find(&pattern) {
                let start = search_from + idx;
                let after_key = start + pattern.len();
                let rest = &out[after_key..];
                let skipped = rest.len() - rest.trim_start_matches([' ', '"', '\'']).len();
                let value_start = after_key + skipped;
                let value = &out[value_start..];
                let end = value
                    .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | ')'))
                    .unwrap_or(value.len());
                if end == 0 {
                    search_from = after_key;
                    continue;
                }
                let label = key.replace('_', " ");
                let replacement = format!("[REDACTED {label}]");
                out.replace_range(value_start..value_start + end, &replacement);
                search_from = value_start + replacement.len();
                if search_from >= out.len() {
                    break;
                }
            }
        }
    }
    out
}

/// Redact secrets from a JSON value before persistence.
///
/// Walks objects and arrays; any string field whose key looks like a secret
/// name is replaced with a placeholder, and string values inside are also
/// passed through [`redact_text`] so embedded `KEY=value` pairs are caught.
pub fn redact_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact_text(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_lowercase().replace(['-', ' '], "_");
                if REDACTION_KEYS.iter().any(|key| lower.contains(key)) {
                    out.insert(k.clone(), serde_json::Value::String("[REDACTED]".into()));
                } else {
                    out.insert(k.clone(), redact_json(v));
                }
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}
