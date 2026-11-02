//! Durable storage for support tickets.
//!
//! Design notes
//! ------------
//! * **Own database file** (`support.db`), beside the console store and
//!   `auth.db`. The trace/conversation schema is deliberately untouched —
//!   the same boundary the auth store already draws.
//! * **One writer connection guarded by a mutex**; every call runs on the
//!   blocking pool (`spawn_blocking`) so async handlers never block,
//!   matching [`crate::persist`].
//! * **Idempotent schema**: plain `CREATE TABLE IF NOT EXISTS` statements
//!   applied at open time inside one transaction. The schema is new, so
//!   there is no legacy shape to migrate.
//! * **Attachments are metadata + base64 bytes in one row**, bounded by
//!   [`MAX_ATTACHMENT_BYTES`]. A local single-machine dashboard has no
//!   object store; keeping the bytes with the row means a ticket can never
//!   lose its screenshot while remaining trivially backed up with the DB.
//! * **History is append-only**: every status change or reply inserts a
//!   `support_ticket_event` row, so "view ticket history" reads a log that
//!   cannot be rewritten after the fact.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

/// Errors surfaced by the support store.
#[derive(Debug, thiserror::Error)]
pub enum SupportStoreError {
    /// SQLite failure.
    #[error("support storage error")]
    Sqlite(#[from] rusqlite::Error),
    /// The parent directory could not be created.
    #[error("cannot create support storage directory: {0}")]
    Io(#[from] std::io::Error),
    /// No ticket matches the requested id.
    #[error("ticket not found")]
    NotFound,
    /// The optimistic-concurrency guard fired: the row changed between the
    /// caller's read and write.
    #[error("{0}")]
    Conflict(String),
}

impl SupportStoreError {
    /// HTTP status appropriate for this error.
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::Sqlite(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::Io(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound => axum::http::StatusCode::NOT_FOUND,
            Self::Conflict(_) => axum::http::StatusCode::CONFLICT,
        }
    }
}

/// Result alias.
type Result<T> = std::result::Result<T, SupportStoreError>;

// ---------------------------------------------------------------------------
// Vocabulary (stable wire identifiers; never rename casually)
// ---------------------------------------------------------------------------

/// Ticket categories, exactly as the submission form offers them.
pub const CATEGORIES: &[&str] = &[
    "technical_issue",
    "bug_report",
    "login_problem",
    "performance_issue",
    "data_pipeline",
    "model_evaluation",
    "feature_request",
    "general_inquiry",
    "other",
];

/// Ticket priorities.
pub const PRIORITIES: &[&str] = &["low", "medium", "high", "critical"];

/// Ticket lifecycle states.
pub const STATUSES: &[&str] = &[
    "pending",
    "in_progress",
    "waiting_user",
    "resolved",
    "closed",
];

/// Is `s` one of the known categories?
pub fn is_valid_category(s: &str) -> bool {
    CATEGORIES.contains(&s)
}

/// Is `s` one of the known priorities?
pub fn is_valid_priority(s: &str) -> bool {
    PRIORITIES.contains(&s)
}

/// Is `s` one of the known statuses?
pub fn is_valid_status(s: &str) -> bool {
    STATUSES.contains(&s)
}

// ---------------------------------------------------------------------------
// Bounds (defensive; enforced before anything touches the store)
// ---------------------------------------------------------------------------

/// Maximum subject length.
pub const MAX_SUBJECT_LEN: usize = 200;

/// Maximum description length.
pub const MAX_DESCRIPTION_LEN: usize = 20_000;

/// Maximum attachment size (5 MiB decoded).
pub const MAX_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;

/// Maximum attachments per ticket.
pub const MAX_ATTACHMENTS_PER_TICKET: usize = 5;

/// Maximum stored filename length.
pub const MAX_FILENAME_LEN: usize = 200;

/// Attachment content types accepted server-side. Anything else is refused
/// before the bytes are stored.
pub const ALLOWED_ATTACHMENT_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "application/pdf",
    "text/plain",
    "application/json",
    "text/csv",
    "application/zip",
    "text/markdown",
];

/// Is this MIME type acceptable for an attachment?
pub fn is_allowed_attachment_type(mime: &str) -> bool {
    // Normalize the parameter suffix ("image/png; charset=utf-8").
    let base = mime.split(';').next().unwrap_or("").trim().to_lowercase();
    ALLOWED_ATTACHMENT_TYPES.contains(&base.as_str())
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// One support ticket, projected to the fields a caller may see. Private
/// notes are NEVER part of this struct — they live only in the admin
/// detail projection ([`TicketDetail`]) and are filtered out for users.
#[derive(Debug, Clone, PartialEq)]
pub struct TicketRow {
    /// Human-readable unique id, e.g. `TKT-7F3K9Q`.
    pub ticket_id: String,
    /// Numeric account id of the submitter (`auth_account.id`).
    pub user_id: i64,
    /// Submitter's email (snapshot at submission time).
    pub user_email: String,
    /// Submitter's display name (snapshot at submission time).
    pub user_name: String,
    pub subject: String,
    pub category: String,
    pub priority: String,
    pub status: String,
    /// Full issue description.
    pub description: String,
    /// Creation time (unix ms).
    pub created_at_ms: i64,
    /// Last update time (unix ms).
    pub updated_at_ms: i64,
}

impl TicketRow {
    /// Serialize to the JSON shape the API returns (user-safe: no private
    /// notes by construction).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ticketId": self.ticket_id,
            "userId": self.user_id,
            "userName": self.user_name,
            "userEmail": self.user_email,
            "subject": self.subject,
            "category": self.category,
            "priority": self.priority,
            "status": self.status,
            "description": self.description,
            "createdAtMs": self.created_at_ms,
            "updatedAtMs": self.updated_at_ms,
        })
    }
}

/// One attachment, metadata only (the API never echoes the bytes back in a
/// listing; downloads go through the dedicated endpoint).
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentMeta {
    pub id: i64,
    pub ticket_id: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

impl AttachmentMeta {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "ticketId": self.ticket_id,
            "filename": self.filename,
            "contentType": self.content_type,
            "sizeBytes": self.size_bytes,
            "createdAtMs": self.created_at_ms,
        })
    }
}

/// One history event on a ticket.
#[derive(Debug, Clone, PartialEq)]
pub struct TicketEvent {
    pub id: i64,
    pub ticket_id: String,
    /// `created` | `status_changed` | `reply` | `internal_note`
    pub kind: String,
    /// Author's email snapshot (empty for system-generated events).
    pub author_email: String,
    /// True when the author was acting as an administrator.
    pub author_is_admin: bool,
    /// Event body: new status, reply text or note text.
    pub body: String,
    pub at_ms: i64,
}

impl TicketEvent {
    /// User-visible serialization: internal notes are excluded by the
    /// caller, never by omission here.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "ticketId": self.ticket_id,
            "kind": self.kind,
            "authorEmail": self.author_email,
            "authorIsAdmin": self.author_is_admin,
            "body": self.body,
            "atMs": self.at_ms,
        })
    }
}

/// Full ticket detail: the row plus its (already loaded) attachments and
/// history. Admin views receive everything; user views get the same struct
/// with internal-note events removed by the HTTP layer.
#[derive(Debug, Clone)]
pub struct TicketDetail {
    pub row: TicketRow,
    pub attachments: Vec<AttachmentMeta>,
    pub events: Vec<TicketEvent>,
}

/// Filter for the admin ticket listing. All fields optional.
#[derive(Debug, Default, Clone)]
pub struct TicketFilter {
    /// Free-text search across ticket id, subject and user email.
    pub search: Option<String>,
    /// Exact status match.
    pub status: Option<String>,
    /// Exact category match.
    pub category: Option<String>,
    /// Exact priority match.
    pub priority: Option<String>,
    /// Page number (1-based).
    pub page: i64,
    /// Rows per page.
    pub per_page: i64,
}

/// Summary counts for the admin dashboard cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TicketSummary {
    pub total: i64,
    pub pending: i64,
    pub in_progress: i64,
    pub resolved: i64,
    pub critical: i64,
}

impl TicketSummary {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total,
            "pending": self.pending,
            "inProgress": self.in_progress,
            "resolved": self.resolved,
            "critical": self.critical,
        })
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const TICKETS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS support_tickets (
     ticket_id     TEXT PRIMARY KEY,
     user_id       INTEGER NOT NULL,
     user_email    TEXT NOT NULL,
     user_name     TEXT NOT NULL,
     subject       TEXT NOT NULL,
     category      TEXT NOT NULL CHECK (category IN (
                     'technical_issue','bug_report','login_problem',
                     'performance_issue','data_pipeline','model_evaluation',
                     'feature_request','general_inquiry','other')),
     priority      TEXT NOT NULL CHECK (priority IN ('low','medium','high','critical')),
     status        TEXT NOT NULL CHECK (status IN (
                     'pending','in_progress','waiting_user','resolved','closed'))
                   DEFAULT 'pending',
     description   TEXT NOT NULL,
     created_at_ms INTEGER NOT NULL,
     updated_at_ms INTEGER NOT NULL
 );
 CREATE INDEX IF NOT EXISTS idx_support_tickets_user ON support_tickets(user_id, created_at_ms DESC);
 CREATE INDEX IF NOT EXISTS idx_support_tickets_status ON support_tickets(status);
 CREATE INDEX IF NOT EXISTS idx_support_tickets_priority ON support_tickets(priority);";

const ATTACHMENTS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS support_attachments (
     id            INTEGER PRIMARY KEY AUTOINCREMENT,
     ticket_id     TEXT NOT NULL REFERENCES support_tickets(ticket_id) ON DELETE CASCADE,
     filename      TEXT NOT NULL,
     content_type  TEXT NOT NULL,
     size_bytes    INTEGER NOT NULL,
     data          BLOB NOT NULL,
     created_at_ms INTEGER NOT NULL
 );
 CREATE INDEX IF NOT EXISTS idx_support_attachments_ticket ON support_attachments(ticket_id);";

const EVENTS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS support_ticket_events (
     id             INTEGER PRIMARY KEY AUTOINCREMENT,
     ticket_id      TEXT NOT NULL REFERENCES support_tickets(ticket_id) ON DELETE CASCADE,
     kind           TEXT NOT NULL CHECK (kind IN ('created','status_changed','reply','internal_note')),
     author_email   TEXT NOT NULL DEFAULT '',
     author_is_admin INTEGER NOT NULL DEFAULT 0,
     body           TEXT NOT NULL,
     at_ms          INTEGER NOT NULL
 );
 CREATE INDEX IF NOT EXISTS idx_support_events_ticket ON support_ticket_events(ticket_id, at_ms);";

/// How many rows a single list query may return.
pub const MAX_PAGE: i64 = 100;

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Durable support-ticket store.
#[derive(Clone)]
pub struct SupportStore {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    #[allow(dead_code)] // kept for parity with persist::DashboardStore
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl SupportStore {
    /// Open (creating if needed) the store at `path`, applying the schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; \
             PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        let tx = conn.transaction()?;
        tx.execute_batch(TICKETS_SCHEMA)?;
        tx.execute_batch(ATTACHMENTS_SCHEMA)?;
        tx.execute_batch(EVENTS_SCHEMA)?;
        tx.commit()?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                path,
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Open an in-memory store (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let mut conn = conn;
        let tx = conn.transaction()?;
        tx.execute_batch(TICKETS_SCHEMA)?;
        tx.execute_batch(ATTACHMENTS_SCHEMA)?;
        tx.execute_batch(EVENTS_SCHEMA)?;
        tx.commit()?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                path: PathBuf::from(":memory:"),
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Run `f` with the connection on the current thread.
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self.inner.conn.lock();
        f(&guard)
    }

    // -----------------------------------------------------------------
    // Creation
    // -----------------------------------------------------------------

    /// Create a ticket, generating its unique id, and record the `created`
    /// history event in the same transaction.
    ///
    /// `attachments` must already be validated (type, size, count); the
    /// store re-checks nothing except referential integrity.
    pub fn create_ticket(
        &self,
        user_id: i64,
        user_email: &str,
        user_name: &str,
        subject: &str,
        category: &str,
        priority: &str,
        description: &str,
        attachments: &[(String, String, Vec<u8>)], // (filename, content_type, bytes)
    ) -> Result<TicketRow> {
        debug_assert!(is_valid_category(category));
        debug_assert!(is_valid_priority(priority));
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.inner.conn.lock();
        let ticket_id = self.generate_ticket_id_locked(&conn);
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO support_tickets
                 (ticket_id, user_id, user_email, user_name, subject, category,
                  priority, status, description, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9, ?9)",
            params![
                ticket_id,
                user_id,
                user_email,
                user_name,
                subject,
                category,
                priority,
                description,
                now
            ],
        )?;
        for (filename, content_type, bytes) in attachments {
            tx.execute(
                "INSERT INTO support_attachments
                     (ticket_id, filename, content_type, size_bytes, data, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ticket_id, filename, content_type, bytes.len() as i64, bytes, now],
            )?;
        }
        tx.execute(
            "INSERT INTO support_ticket_events
                 (ticket_id, kind, author_email, author_is_admin, body, at_ms)
             VALUES (?1, 'created', ?2, 0, ?3, ?4)",
            params![ticket_id, user_email, subject, now],
        )?;
        tx.commit()?;
        drop(conn);
        // Read back through the public path so callers get the exact shape
        // every other reader produces.
        self.get_ticket(&ticket_id)?
            .ok_or(SupportStoreError::NotFound)
    }

    /// Generate a short, unambiguous, human-readable ticket id.
    ///
    /// Format: `TKT-` + 6 characters from the URL-safe base64 alphabet
    /// (letters and digits only, vowels and lookalikes removed to keep ids
    /// readable over the phone). 30 bits of OS entropy is plenty for a
    /// local dashboard's volume, and the PRIMARY KEY makes any collision a
    /// clean retry rather than corruption.
    ///
    /// Takes the ALREADY-HELD connection guard so callers inside a
    /// transaction do not re-enter the mutex (which would deadlock).
    fn generate_ticket_id_locked(&self, conn: &Connection) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
        loop {
            let mut bytes = [0u8; 6];
            getrandom::fill(&mut bytes)
                .expect("OS entropy source unavailable; refusing weak ticket ids");
            let suffix: String = bytes
                .iter()
                .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
                .collect();
            let candidate = format!("TKT-{suffix}");
            // Uniqueness check under the same lock the insert will take.
            let taken: bool = conn
                .query_row(
                    "SELECT 1 FROM support_tickets WHERE ticket_id = ?1",
                    params![candidate],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !taken {
                return candidate;
            }
        }
    }

    // -----------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------

    fn row_from_db(r: &rusqlite::Row<'_>) -> rusqlite::Result<TicketRow> {
        Ok(TicketRow {
            ticket_id: r.get(0)?,
            user_id: r.get(1)?,
            user_email: r.get(2)?,
            user_name: r.get(3)?,
            subject: r.get(4)?,
            category: r.get(5)?,
            priority: r.get(6)?,
            status: r.get(7)?,
            description: r.get(8)?,
            created_at_ms: r.get(9)?,
            updated_at_ms: r.get(10)?,
        })
    }

    const SELECT_ROW: &str =
        "SELECT ticket_id, user_id, user_email, user_name, subject, category, \
         priority, status, description, created_at_ms, updated_at_ms \
         FROM support_tickets";

    /// Fetch one ticket by id.
    pub fn get_ticket(&self, ticket_id: &str) -> Result<Option<TicketRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!("{} WHERE ticket_id = ?1", Self::SELECT_ROW),
                params![ticket_id],
                Self::row_from_db,
            )
            .optional()
            .map_err(SupportStoreError::from)
        })
    }

    /// List tickets, either all of them (admin) or one user's own.
    ///
    /// `owner` scopes the query to a single `user_id`; `None` lists every
    /// ticket. Filters are composed with AND; the search term matches
    /// ticket id, subject and user email as a literal substring
    /// (LIKE metacharacters escaped).
    pub fn list_tickets(
        &self,
        owner: Option<i64>,
        filter: &TicketFilter,
    ) -> Result<(Vec<TicketRow>, i64)> {
        let per_page = if filter.per_page < 1 {
            25
        } else {
            filter.per_page.min(MAX_PAGE)
        };
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(per_page);

        let mut where_clauses: Vec<String> = Vec::new();
        let mut sql_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(uid) = owner {
            where_clauses.push(format!("user_id = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(uid));
        }
        if let Some(q) = filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let n = sql_params.len() + 1;
            where_clauses.push(format!(
                "(ticket_id LIKE ?{n} ESCAPE '\\' OR subject LIKE ?{n} ESCAPE '\\' \
                  OR user_email LIKE ?{n} ESCAPE '\\')",
            ));
            sql_params.push(Box::new(format!("%{escaped}%")));
        }
        if let Some(status) = filter.status.as_deref().filter(|s| !s.is_empty()) {
            where_clauses.push(format!("status = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(status.to_string()));
        }
        if let Some(category) = filter.category.as_deref().filter(|s| !s.is_empty()) {
            where_clauses.push(format!("category = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(category.to_string()));
        }
        if let Some(priority) = filter.priority.as_deref().filter(|s| !s.is_empty()) {
            where_clauses.push(format!("priority = ?{}", sql_params.len() + 1));
            sql_params.push(Box::new(priority.to_string()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        self.with_conn(move |conn| {
            let count_sql = format!("SELECT COUNT(*) FROM support_tickets {where_sql}");
            let mut count_stmt = conn.prepare(&count_sql)?;
            let total: i64 = count_stmt.query_row(
                rusqlite::params_from_iter(sql_params.iter().map(|p| p.as_ref())),
                |r| r.get(0),
            )?;

            let list_sql = format!(
                "{} {} ORDER BY created_at_ms DESC LIMIT {} OFFSET {}",
                Self::SELECT_ROW,
                where_sql,
                per_page,
                offset
            );
            let mut stmt = conn.prepare(&list_sql)?;
            let rows = stmt
                .query_map(
                    rusqlite::params_from_iter(sql_params.iter().map(|p| p.as_ref())),
                    Self::row_from_db,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((rows, total))
        })
    }

    /// Summary counts for the admin dashboard cards.
    pub fn summary(&self) -> Result<TicketSummary> {
        self.with_conn(|conn| {
            let row = conn.query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(status = 'pending'), 0),
                    COALESCE(SUM(status = 'in_progress'), 0),
                    COALESCE(SUM(status = 'resolved'), 0),
                    COALESCE(SUM(priority = 'critical' AND status NOT IN ('resolved','closed')), 0)
                 FROM support_tickets",
                [],
                |r| {
                    Ok(TicketSummary {
                        total: r.get(0)?,
                        pending: r.get(1)?,
                        in_progress: r.get(2)?,
                        resolved: r.get(3)?,
                        critical: r.get(4)?,
                    })
                },
            )?;
            Ok(row)
        })
    }

    /// Count of open (not resolved/closed) tickets — the admin badge.
    pub fn open_count(&self) -> Result<i64> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM support_tickets
                  WHERE status NOT IN ('resolved', 'closed')",
                [],
                |r| r.get(0),
            )
            .map_err(SupportStoreError::from)
        })
    }

    /// Attachments of one ticket (metadata only).
    pub fn list_attachments(&self, ticket_id: &str) -> Result<Vec<AttachmentMeta>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, ticket_id, filename, content_type, size_bytes, created_at_ms
                   FROM support_attachments WHERE ticket_id = ?1 ORDER BY id",
            )?;
            let rows = stmt
                .query_map(params![ticket_id], |r| {
                    Ok(AttachmentMeta {
                        id: r.get(0)?,
                        ticket_id: r.get(1)?,
                        filename: r.get(2)?,
                        content_type: r.get(3)?,
                        size_bytes: r.get(4)?,
                        created_at_ms: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Download one attachment's bytes. Returns `(meta, data)`.
    pub fn get_attachment(
        &self,
        attachment_id: i64,
    ) -> Result<Option<(AttachmentMeta, Vec<u8>)>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, ticket_id, filename, content_type, size_bytes, data, created_at_ms
                   FROM support_attachments WHERE id = ?1",
                params![attachment_id],
                |r| {
                    Ok((
                        AttachmentMeta {
                            id: r.get(0)?,
                            ticket_id: r.get(1)?,
                            filename: r.get(2)?,
                            content_type: r.get(3)?,
                            size_bytes: r.get(4)?,
                            created_at_ms: r.get(6)?,
                        },
                        r.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(SupportStoreError::from)
        })
    }

    /// History events of one ticket, oldest first.
    pub fn list_events(&self, ticket_id: &str) -> Result<Vec<TicketEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, ticket_id, kind, author_email, author_is_admin, body, at_ms
                   FROM support_ticket_events WHERE ticket_id = ?1 ORDER BY at_ms, id",
            )?;
            let rows = stmt
                .query_map(params![ticket_id], |r| {
                    Ok(TicketEvent {
                        id: r.get(0)?,
                        ticket_id: r.get(1)?,
                        kind: r.get(2)?,
                        author_email: r.get(3)?,
                        author_is_admin: r.get::<_, i64>(4)? != 0,
                        body: r.get(5)?,
                        at_ms: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Full detail (row + attachments + events) for one ticket.
    pub fn ticket_detail(&self, ticket_id: &str) -> Result<Option<TicketDetail>> {
        let Some(row) = self.get_ticket(ticket_id)? else {
            return Ok(None);
        };
        let attachments = self.list_attachments(ticket_id)?;
        let events = self.list_events(ticket_id)?;
        Ok(Some(TicketDetail {
            row,
            attachments,
            events,
        }))
    }

    // -----------------------------------------------------------------
    // Mutation (admin)
    // -----------------------------------------------------------------

    /// Change a ticket's status, recording the transition in history.
    ///
    /// Returns the updated row, or [`SupportStoreError::NotFound`] when the
    /// ticket does not exist. The `expected_current` parameter, when given,
    /// makes the check-and-set atomic: a concurrent change between an
    /// admin's read and write surfaces as a conflict instead of silently
    /// overwriting.
    pub fn set_status(
        &self,
        ticket_id: &str,
        new_status: &str,
        actor_email: &str,
        expected_current: Option<&str>,
    ) -> Result<TicketRow> {
        debug_assert!(is_valid_status(new_status));
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.inner.conn.lock();
        let tx = conn.transaction()?;
        // Optimistic concurrency: when the caller states what status they
        // believe the ticket is in, the UPDATE only fires when that belief
        // still holds — a concurrent change surfaces as a conflict instead
        // of being silently overwritten.
        let changed = match expected_current {
            Some(expected) => tx.execute(
                "UPDATE support_tickets SET status = ?1, updated_at_ms = ?2
                  WHERE ticket_id = ?3 AND status = ?4",
                params![new_status, now, ticket_id, expected],
            )?,
            None => tx.execute(
                "UPDATE support_tickets SET status = ?1, updated_at_ms = ?2
                  WHERE ticket_id = ?3",
                params![new_status, now, ticket_id],
            )?,
        };
        if changed == 0 {
            // Either absent, or the optimistic-concurrency guard fired.
            let exists: Option<String> = tx
                .query_row(
                    "SELECT status FROM support_tickets WHERE ticket_id = ?1",
                    params![ticket_id],
                    |r| r.get(0),
                )
                .optional()?;
            return match (exists, expected_current) {
                (None, _) => Err(SupportStoreError::NotFound),
                (Some(_), Some(_)) => Err(SupportStoreError::Conflict(
                    "ticket was modified concurrently; reload and retry".into(),
                )),
                (Some(_), None) => Err(SupportStoreError::NotFound),
            };
        }
        tx.execute(
            "INSERT INTO support_ticket_events
                 (ticket_id, kind, author_email, author_is_admin, body, at_ms)
             VALUES (?1, 'status_changed', ?2, 1, ?3, ?4)",
            params![ticket_id, actor_email, new_status, now],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_ticket(ticket_id)?
            .ok_or(SupportStoreError::NotFound)
    }

    /// Append an administrator reply, recording it in history.
    pub fn add_reply(
        &self,
        ticket_id: &str,
        body: &str,
        actor_email: &str,
    ) -> Result<TicketRow> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.inner.conn.lock();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE support_tickets SET updated_at_ms = ?1 WHERE ticket_id = ?2",
            params![now, ticket_id],
        )?;
        if changed == 0 {
            return Err(SupportStoreError::NotFound);
        }
        tx.execute(
            "INSERT INTO support_ticket_events
                 (ticket_id, kind, author_email, author_is_admin, body, at_ms)
             VALUES (?1, 'reply', ?2, 1, ?3, ?4)",
            params![ticket_id, actor_email, body, now],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_ticket(ticket_id)?
            .ok_or(SupportStoreError::NotFound)
    }

    /// Append a private internal note (admin-only visibility).
    pub fn add_internal_note(
        &self,
        ticket_id: &str,
        body: &str,
        actor_email: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.inner.conn.lock();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE support_tickets SET updated_at_ms = ?1 WHERE ticket_id = ?2",
            params![now, ticket_id],
        )?;
        if changed == 0 {
            return Err(SupportStoreError::NotFound);
        }
        tx.execute(
            "INSERT INTO support_ticket_events
                 (ticket_id, kind, author_email, author_is_admin, body, at_ms)
             VALUES (?1, 'internal_note', ?2, 1, ?3, ?4)",
            params![ticket_id, actor_email, body, now],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SupportStore {
        SupportStore::open_in_memory().unwrap()
    }

    #[test]
    fn ticket_ids_are_short_unique_and_prefixed() {
        let s = store();
        let a = s
            .create_ticket(1, "a@soket.ai", "A", "s1", "other", "low", "d", &[])
            .unwrap();
        let b = s
            .create_ticket(2, "b@soket.ai", "B", "s2", "other", "low", "d", &[])
            .unwrap();
        assert!(a.ticket_id.starts_with("TKT-"));
        assert_eq!(a.ticket_id.len(), 10);
        assert_ne!(a.ticket_id, b.ticket_id, "ids must be unique");
    }

    #[test]
    fn create_records_the_created_event_atomically() {
        let s = store();
        let row = s
            .create_ticket(7, "u@soket.ai", "U", "subject", "bug_report", "high", "desc", &[])
            .unwrap();
        assert_eq!(row.status, "pending");
        let events = s.list_events(&row.ticket_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "created");
        assert_eq!(events[0].author_email, "u@soket.ai");
        assert!(!events[0].author_is_admin);
    }

    #[test]
    fn attachments_are_stored_and_downloadable() {
        let s = store();
        let bytes = b"fake-png-bytes".to_vec();
        let row = s
            .create_ticket(
                1,
                "u@soket.ai",
                "U",
                "s",
                "other",
                "low",
                "d",
                &[("shot.png".into(), "image/png".into(), bytes.clone())],
            )
            .unwrap();
        let atts = s.list_attachments(&row.ticket_id).unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].filename, "shot.png");
        assert_eq!(atts[0].size_bytes, bytes.len() as i64);

        let (meta, data) = s.get_attachment(atts[0].id).unwrap().unwrap();
        assert_eq!(meta.id, atts[0].id);
        assert_eq!(data, bytes);

        // Unknown id: None, not an error.
        assert!(s.get_attachment(99999).unwrap().is_none());
    }

    #[test]
    fn ownership_scoping_limits_listings() {
        let s = store();
        s.create_ticket(1, "one@soket.ai", "One", "a", "other", "low", "d", &[])
            .unwrap();
        s.create_ticket(1, "one@soket.ai", "One", "b", "other", "low", "d", &[])
            .unwrap();
        s.create_ticket(2, "two@soket.ai", "Two", "c", "other", "low", "d", &[])
            .unwrap();

        // Scoped to user 1: exactly their two rows.
        let (rows, total) = s.list_tickets(Some(1), &TicketFilter::default()).unwrap();
        assert_eq!(total, 2);
        assert!(rows.iter().all(|r| r.user_id == 1));

        // Unscoped (admin): all three.
        let (_, total) = s.list_tickets(None, &TicketFilter::default()).unwrap();
        assert_eq!(total, 3);
    }

    #[test]
    fn search_matches_id_subject_and_email_with_escaped_like() {
        let s = store();
        let row = s
            .create_ticket(1, "u@soket.ai", "U", "Pipeline 50% slower", "other", "low", "d", &[])
            .unwrap();

        // Subject substring with a LIKE metacharacter in the query.
        let f = TicketFilter {
            search: Some("50% slower".into()),
            ..Default::default()
        };
        let (_, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 1);

        // Ticket id.
        let f = TicketFilter {
            search: Some(row.ticket_id.clone()),
            ..Default::default()
        };
        let (_, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 1);

        // Email.
        let f = TicketFilter {
            search: Some("u@soket".into()),
            ..Default::default()
        };
        let (_, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 1);

        // No match.
        let f = TicketFilter {
            search: Some("zzz-nothing".into()),
            ..Default::default()
        };
        let (_, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 0);
    }

    #[test]
    fn filters_compose_with_and() {
        let s = store();
        s.create_ticket(1, "a@soket.ai", "A", "one", "bug_report", "high", "d", &[])
            .unwrap();
        s.create_ticket(1, "a@soket.ai", "A", "two", "bug_report", "low", "d", &[])
            .unwrap();
        s.create_ticket(1, "a@soket.ai", "A", "three", "other", "high", "d", &[])
            .unwrap();

        let f = TicketFilter {
            category: Some("bug_report".into()),
            priority: Some("high".into()),
            ..Default::default()
        };
        let (rows, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].subject, "one");
    }

    #[test]
    fn summary_counts_statuses_and_open_criticals() {
        let s = store();
        let t1 = s
            .create_ticket(1, "a@soket.ai", "A", "one", "other", "critical", "d", &[])
            .unwrap();
        let t2 = s
            .create_ticket(1, "a@soket.ai", "A", "two", "other", "low", "d", &[])
            .unwrap();
        let _t3 = s
            .create_ticket(1, "a@soket.ai", "A", "three", "other", "medium", "d", &[])
            .unwrap();

        let sum = s.summary().unwrap();
        assert_eq!(sum.total, 3);
        assert_eq!(sum.pending, 3);
        assert_eq!(sum.critical, 1, "the open critical counts");

        s.set_status(&t1.ticket_id, "in_progress", "admin@soket.ai", None)
            .unwrap();
        s.set_status(&t2.ticket_id, "resolved", "admin@soket.ai", None)
            .unwrap();

        let sum = s.summary().unwrap();
        assert_eq!(sum.in_progress, 1);
        assert_eq!(sum.resolved, 1);
        assert_eq!(sum.pending, 1);
        // t1 is critical but still OPEN (in_progress), so it keeps counting
        // as a critical issue; only resolved/closed criticals drop out.
        assert_eq!(sum.critical, 1);

        // Closing t1 removes it from the critical count.
        s.set_status(&t1.ticket_id, "closed", "admin@soket.ai", None)
            .unwrap();
        let sum = s.summary().unwrap();
        assert_eq!(sum.critical, 0);

        let open = s.open_count().unwrap();
        assert_eq!(open, 1, "resolved and closed tickets leave the open count");
    }

    #[test]
    fn status_transitions_are_recorded_in_history() {
        let s = store();
        let row = s
            .create_ticket(1, "u@soket.ai", "U", "s", "other", "low", "d", &[])
            .unwrap();
        let updated = s
            .set_status(&row.ticket_id, "waiting_user", "admin@soket.ai", Some("pending"))
            .unwrap();
        assert_eq!(updated.status, "waiting_user");
        assert!(updated.updated_at_ms >= row.created_at_ms);

        let events = s.list_events(&row.ticket_id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].kind, "status_changed");
        assert_eq!(events[1].body, "waiting_user");
        assert!(events[1].author_is_admin);
    }

    #[test]
    fn stale_expected_status_conflicts() {
        let s = store();
        let row = s
            .create_ticket(1, "u@soket.ai", "U", "s", "other", "low", "d", &[])
            .unwrap();
        s.set_status(&row.ticket_id, "in_progress", "a@soket.ai", Some("pending"))
            .unwrap();
        // Claiming it is still pending must now conflict.
        let err = s
            .set_status(&row.ticket_id, "resolved", "a@soket.ai", Some("pending"))
            .unwrap_err();
        assert!(matches!(err, SupportStoreError::Conflict(_)));
    }

    #[test]
    fn set_status_on_missing_ticket_is_not_found() {
        let s = store();
        let err = s.set_status("TKT-ZZZZZZ", "resolved", "a@soket.ai", None).unwrap_err();
        assert!(matches!(err, SupportStoreError::NotFound));
    }

    #[test]
    fn replies_and_notes_append_events() {
        let s = store();
        let row = s
            .create_ticket(1, "u@soket.ai", "U", "s", "other", "low", "d", &[])
            .unwrap();
        s.add_reply(&row.ticket_id, "here is the fix", "admin@soket.ai")
            .unwrap();
        s.add_internal_note(&row.ticket_id, "internal only", "admin@soket.ai")
            .unwrap();
        let events = s.list_events(&row.ticket_id).unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["created", "reply", "internal_note"]);

        // Missing ticket: NotFound, never a silent no-op.
        assert!(matches!(
            s.add_reply("TKT-ZZZZZZ", "x", "a@soket.ai").unwrap_err(),
            SupportStoreError::NotFound
        ));
        assert!(matches!(
            s.add_internal_note("TKT-ZZZZZZ", "x", "a@soket.ai").unwrap_err(),
            SupportStoreError::NotFound
        ));
    }

    #[test]
    fn ticket_detail_bundles_everything() {
        let s = store();
        let row = s
            .create_ticket(
                1,
                "u@soket.ai",
                "U",
                "s",
                "login_problem",
                "medium",
                "d",
                &[("a.txt".into(), "text/plain".into(), b"hello".to_vec())],
            )
            .unwrap();
        s.add_reply(&row.ticket_id, "r", "admin@soket.ai").unwrap();
        let detail = s.ticket_detail(&row.ticket_id).unwrap().unwrap();
        assert_eq!(detail.row.ticket_id, row.ticket_id);
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.events.len(), 2);
        assert!(s.ticket_detail("TKT-ZZZZZZ").unwrap().is_none());
    }

    #[test]
    fn pagination_clamps_to_bounds() {
        let s = store();
        for i in 0..30 {
            s.create_ticket(1, "u@soket.ai", "U", &format!("s{i}"), "other", "low", "d", &[])
                .unwrap();
        }
        // per_page above MAX_PAGE is clamped down…
        let f = TicketFilter {
            per_page: 10_000,
            page: 1,
            ..Default::default()
        };
        let (rows, total) = s.list_tickets(None, &f).unwrap();
        assert_eq!(total, 30);
        assert!(rows.len() <= MAX_PAGE as usize);
        // …and page 0 becomes page 1.
        let f = TicketFilter {
            per_page: 10,
            page: 0,
            ..Default::default()
        };
        let (rows, _) = s.list_tickets(None, &f).unwrap();
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn vocabulary_checks_reject_unknown_values() {
        assert!(is_valid_category("data_pipeline"));
        assert!(!is_valid_category("Data Pipeline"));
        assert!(!is_valid_category(""));
        assert!(is_valid_priority("critical"));
        assert!(!is_valid_priority("urgent"));
        assert!(is_valid_status("waiting_user"));
        assert!(!is_valid_status("waiting-user"));
    }

    #[test]
    fn attachment_type_allowlist_is_exact_and_parameter_tolerant() {
        assert!(is_allowed_attachment_type("image/png"));
        assert!(is_allowed_attachment_type("IMAGE/PNG"));
        assert!(is_allowed_attachment_type("image/png; charset=utf-8"));
        assert!(is_allowed_attachment_type("application/pdf"));
        assert!(is_allowed_attachment_type("text/csv"));
        // Executable and HTML-ish types must be refused.
        assert!(!is_allowed_attachment_type("text/html"));
        assert!(!is_allowed_attachment_type("application/x-executable"));
        assert!(!is_allowed_attachment_type("image/svg"));
        assert!(!is_allowed_attachment_type(""));
    }

    #[test]
    fn reopening_a_database_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("support.db");
        let s = SupportStore::open(&path).unwrap();
        s.create_ticket(1, "u@soket.ai", "U", "s", "other", "low", "d", &[])
            .unwrap();
        drop(s);
        // Reopen: schema application must be a no-op and the row survives.
        let s2 = SupportStore::open(&path).unwrap();
        let (_, total) = s2.list_tickets(None, &TicketFilter::default()).unwrap();
        assert_eq!(total, 1);
    }
}
