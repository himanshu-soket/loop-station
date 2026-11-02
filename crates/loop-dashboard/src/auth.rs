//! User authentication for the dashboard: a single local owner account with
//! cookie sessions.
//!
//! Design notes
//! ------------
//! * **Single-owner model.** The dashboard is a localhost tool bound to
//!   loopback with per-machine stores; there is no user-ownership concept in
//!   any existing table. Rather than retrofit multi-user semantics onto
//!   schemas that never had them, the first successful sign-up creates *the*
//!   owner account and further sign-ups are refused. This keeps every
//!   existing row unambiguously owned by the person at the machine.
//! * **Passwords** are hashed with Argon2id (19 MiB memory, 2 iterations,
//!   degree-1 parallelism — OWASP-aligned for a local service) with a unique
//!   random salt per password. Plaintext never touches the store or logs.
//! * **Sessions** use opaque 256-bit tokens from the OS CSPRNG. The cookie
//!   carries the raw token (HttpOnly); the database stores only its SHA-256
//!   digest, so a leaked database cannot be replayed against the live app.
//! * **Storage** lives in its own `auth.db` next to the console store — the
//!   trace/conversation schema is deliberately untouched.

use std::sync::Arc;
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};

/// How long a signed-in session stays valid absent activity.
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 7); // 7 days

/// Cookie name carrying the session token.
pub const SESSION_COOKIE: &str = "soket_session";

/// Minimum acceptable password length.
const MIN_PASSWORD_LEN: usize = 8;

/// Maximum identifier length (defensive bound).
const MAX_IDENTIFIER_LEN: usize = 64;

/// Sign-in attempts allowed per window before throttling kicks in.
///
/// Public alongside [`SIGNIN_WINDOW`] so tests drive the limiter to its
/// real boundary instead of restating a magic number that could drift.
pub const SIGNIN_ATTEMPT_LIMIT: u32 = 8;

/// Window over which sign-in attempts are counted.
///
/// Public so the HTTP layer can advertise the same window in the
/// `Retry-After` header of a 429 — one constant, no drift between the
/// limiter and what users are told to wait.
pub const SIGNIN_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Registration attempts accepted per rolling window (Phase 4E).
///
/// Deliberately generous for a small-team local service — enough for a
/// batch of legitimate colleagues to register in one sitting, low enough
/// that a flood produces at most this many pending rows per window that an
/// administrator must review. See [`AuthStore::allow_registration_attempt`].
pub const REGISTRATION_CAPACITY: usize = 20;

/// Window over which registration attempts are counted.
pub const REGISTRATION_WINDOW: Duration = Duration::from_secs(60 * 60);

/// How long a failure record survives after the last failed attempt before
/// cleanup drops it. Long enough that a patient attacker's streak persists
/// across restarts and reloads; short enough that the table stays tiny
/// (one row per recently-failing identity, pruned opportunistically).
pub const SIGNIN_FAILURE_RETENTION: Duration = Duration::from_secs(60 * 60 * 24);

/// The only account role with administrative authority.
///
/// Multiple accounts may hold this role (administrators can promote other
/// accounts). The invariant that matters — **never zero administrators** —
/// is enforced in code by [`AuthStore::set_account_role`], which refuses to
/// demote the last remaining administrator inside the same transaction as
/// the change itself, so concurrent requests cannot race past the check.
pub const ADMIN_ROLE: &str = "admin";

/// Role for ordinary accounts.
pub const USER_ROLE: &str = "user";

/// Registration requested; an administrator must approve it.
pub const STATUS_PENDING: &str = "pending";

/// Approved by an administrator; sign-in is allowed.
pub const STATUS_APPROVED: &str = "approved";

/// Registration request refused by an administrator.
pub const STATUS_REJECTED: &str = "rejected";

/// Approved earlier, but access has been withdrawn by an administrator.
pub const STATUS_SUSPENDED: &str = "suspended";

/// How long an administrator-triggered password-reset token stays valid.
pub const PASSWORD_RESET_TTL: Duration = Duration::from_secs(30 * 60);

/// Minimum delay between two admin-triggered resets for the same account.
pub const PASSWORD_RESET_COOLDOWN: Duration = Duration::from_secs(60);

/// Wrong submissions allowed per reset token before it dies.
pub const PASSWORD_RESET_MAX_ATTEMPTS: u32 = 5;

/// How long spent (consumed/expired) reset tokens linger before cleanup.
const PASSWORD_RESET_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;

/// Audit event kinds recorded by administrative actions. The string forms
/// are stable wire identifiers persisted in the audit log and returned by
/// the activity API; never rename one casually.
pub mod audit_event {
    /// A successful sign-in (recorded on every approved authentication).
    pub const SIGNIN_SUCCESS: &str = "signin_success";
    /// A failed sign-in (wrong password or unknown identifier).
    pub const SIGNIN_FAILED: &str = "signin_failed";
    /// An administrator issued a password-reset email.
    pub const PASSWORD_RESET_ISSUED: &str = "password_reset_issued";
    /// A password was changed by completing a reset token.
    pub const PASSWORD_RESET_COMPLETED: &str = "password_reset_completed";
    /// An account's role was changed (promoted or demoted).
    pub const ROLE_CHANGED: &str = "role_changed";
    /// An account was suspended.
    pub const ACCOUNT_SUSPENDED: &str = "account_suspended";
    /// A suspended account was reactivated.
    pub const ACCOUNT_ACTIVATED: &str = "account_activated";
    /// All of an account's sessions were revoked by an administrator.
    pub const SESSIONS_REVOKED: &str = "sessions_revoked";
    /// An account was permanently deleted.
    pub const ACCOUNT_DELETED: &str = "account_deleted";
    /// An account was approved (pending → approved).
    pub const ACCOUNT_APPROVED: &str = "account_approved";
    /// A registration request was rejected.
    pub const ACCOUNT_REJECTED: &str = "account_rejected";
}

/// The grandfathered administrator identifier. Its account record, Argon2id
/// password hash and live sessions are preserved verbatim by the migration.
pub const LEGACY_ADMIN_IDENTIFIER: &str = "himanshusudan771@gmail.com";

/// Errors surfaced to clients. Deliberately coarse on the sign-in path so
/// the response cannot be used to enumerate accounts.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Credentials did not match (sign-in path: never says which part).
    #[error("invalid credentials")]
    InvalidCredentials,
    /// The request was malformed or failed validation.
    #[error("{0}")]
    Validation(String),
    /// Sign-up attempted while an owner already exists.
    #[error("an owner account already exists on this machine")]
    OwnerExists,
    /// Too many attempts; the caller must wait.
    #[error("too many attempts; try again later")]
    RateLimited,
    /// Credentials were correct but the account has not been approved yet.
    ///
    /// Returned **only after** the password verifies, so it can never be
    /// used to probe which addresses hold pending accounts. The message is
    /// safe to show the legitimate registrant, who needs to know why their
    /// (correct) credentials are not enough.
    #[error("your account is awaiting administrator approval")]
    AccountPending,
    /// Credentials were correct but an administrator suspended the account.
    ///
    /// Like [`AuthError::AccountPending`], returned only after the password
    /// verifies — a caller with wrong credentials still gets the generic
    /// [`AuthError::InvalidCredentials`] and learns nothing. The person who
    /// legitimately owns the credentials deserves to know why access is
    /// refused so they can contact an administrator.
    #[error("your account has been suspended by an administrator")]
    AccountSuspended,
    /// Storage failure.
    #[error("authentication storage error")]
    Storage(#[from] rusqlite::Error),
}

impl AuthError {
    /// HTTP status appropriate for this error.
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            AuthError::InvalidCredentials => axum::http::StatusCode::UNAUTHORIZED,
            AuthError::Validation(_) => axum::http::StatusCode::BAD_REQUEST,
            AuthError::OwnerExists => axum::http::StatusCode::CONFLICT,
            AuthError::RateLimited => axum::http::StatusCode::TOO_MANY_REQUESTS,
            // Forbidden, not Unauthorized: the caller IS authenticated, the
            // account just lacks access.
            AuthError::AccountPending => axum::http::StatusCode::FORBIDDEN,
            // Same reasoning as AccountPending: correct credentials, refused
            // access. 403 with a truthful message for the real owner.
            AuthError::AccountSuspended => axum::http::StatusCode::FORBIDDEN,
            AuthError::Storage(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// A verified session, returned when a cookie token validates.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Display identifier of the owner.
    pub identifier: String,
    /// Absolute expiry (unix ms).
    pub expires_at_ms: i64,
}

/// A row of [`AuthStore::list_accounts`], projected to exactly the
/// fields an administrator may see. No password hash, no session
/// material — this struct cannot carry a secret because it never
/// selects one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedAccount {
    /// `auth_account.id`.
    pub id: i64,
    /// Normalized email / identifier.
    pub identifier: String,
    /// [`ADMIN_ROLE`] or [`USER_ROLE`].
    pub role: String,
    /// One of the four [`STATUS_*`](self) values.
    pub status: String,
    /// Registration time (unix ms).
    pub created_at_ms: i64,
    /// Most recent successful sign-in (unix ms), or `None` when the
    /// account has never signed in. Sourced from the `sessions` table's
    /// `last_seen_at_ms`, which is only ever written for a validated
    /// session, so it can only reflect a real sign-in.
    pub last_sign_in_at_ms: Option<i64>,
}

/// A live session row belonging to an account, projected to fields an
/// administrator may see. The token digest itself is never included —
/// this struct cannot leak it because it never selects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// When the session was minted (unix ms).
    pub created_at_ms: i64,
    /// Absolute expiry (unix ms).
    pub expires_at_ms: i64,
    /// Most recent validated request (unix ms).
    pub last_seen_at_ms: i64,
}

/// Full account detail for the administrator's "View details" panel.
///
/// Everything an administrator may legitimately see about one account:
/// identity, role, status, registration, last sign-in, last password
/// change and the count of live sessions. No password hash, no session
/// or reset token material — none is selected, so none can leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDetail {
    pub id: i64,
    pub identifier: String,
    pub role: String,
    pub status: String,
    pub created_at_ms: i64,
    /// Most recent successful sign-in, when one is recorded.
    pub last_sign_in_at_ms: Option<i64>,
    /// Last password change (registration or completed reset), when
    /// tracked. `None` for accounts that predate the column.
    pub last_password_change_ms: Option<i64>,
    /// Number of currently live sessions.
    pub live_session_count: i64,
}

/// A freshly minted administrator-triggered password-reset token. The raw
/// value is handed out exactly once (to the email sender); only its
/// SHA-256 digest is persisted.
#[derive(Debug)]
pub struct IssuedPasswordReset {
    /// Raw URL-safe token for the reset link. Never stored, never logged.
    pub token: String,
    pub account_id: i64,
    /// Normalized identifier of the account (the email recipient).
    pub identifier: String,
    pub expires_at_ms: i64,
}

/// One audit row, projected to the fields the activity view shows.
/// No secrets are ever selected — the table never stores any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub at_ms: i64,
    pub kind: String,
    pub actor_email: Option<String>,
    pub subject_email: Option<String>,
    pub detail: Option<String>,
}

/// Filters accepted by [`AuthStore::list_accounts`]. Every field is
/// optional; `None` means "do not filter on this".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AccountFilter {
    /// Case-insensitive substring match on `identifier`.
    pub search: Option<String>,
    /// Exact role match (`admin` | `user`).
    pub role: Option<String>,
    /// Exact status match (one of the four statuses).
    pub status: Option<String>,
    /// 1-based page number.
    pub page: i64,
    /// Rows per page.
    pub per_page: i64,
}

/// Upper bound on `per_page`. A single page can never exceed this, so a
/// hostile or buggy client cannot request an unbounded listing.
pub const ACCOUNT_LIST_MAX_PER_PAGE: i64 = 200;

/// The authenticator: owns the SQLite connection and the attempt counters.
pub struct AuthStore {
    conn: parking_lot::Mutex<Connection>,
    /// Sliding-window sign-in attempts (process-local).
    signin_attempts: parking_lot::Mutex<Vec<Instant>>,
    /// Sliding-window registration attempts (Phase 4E). Bounded by
    /// [`REGISTRATION_CAPACITY`] by construction; see
    /// [`AuthStore::allow_registration_attempt`].
    registration_attempts: parking_lot::Mutex<Vec<Instant>>,
}
/// Random salt for password hashing, sourced from the OS CSPRNG.
fn random_salt() -> SaltString {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS entropy source unavailable; refusing weak salts");
    SaltString::encode_b64(&bytes).expect("16 bytes always encode")
}

/// Normalized identity key for failure tracking (and any future
/// per-identifier bookkeeping).
///
/// Trim + Unicode-aware lowercase, so `" Him@Example.COM "`,
/// `"him@example.com"` and `"HIM@EXAMPLE.COM"` collapse to one key. This is
/// deliberately NOT email validation — it must accept the existing owner's
/// Gmail identifier unchanged in its normalized form, and it never rejects
/// input (rejection is the auth path's job; tracking must observe every
/// attempt regardless of validity).
pub fn normalize_identity(input: &str) -> String {
    input.trim().to_lowercase()
}

/// A stored account, as read back by the storage primitives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// Stable numeric id (`auth_account.id`).
    pub id: i64,
    /// Normalized email / identifier (unique).
    pub identifier: String,
    /// Role: [`ADMIN_ROLE`] or [`USER_ROLE`].
    pub role: String,
    /// Status: one of [`STATUS_PENDING`], [`STATUS_APPROVED`],
    /// [`STATUS_REJECTED`], [`STATUS_SUSPENDED`].
    pub status: String,
    /// Creation time (unix ms).
    pub created_at_ms: i64,
}

/// Errors raised by the Phase 4A storage primitives.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    /// The request was malformed or failed validation.
    #[error("{0}")]
    Validation(String),
    /// An account with that normalized identifier already exists.
    #[error("an account with that email already exists")]
    DuplicateEmail,
    /// No account matches the requested identifier/id.
    #[error("account not found")]
    NotFound,
    /// The requested status transition is not permitted.
    #[error("invalid status transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    /// The operation would leave the installation with no administrator.
    #[error("cannot demote the last remaining administrator")]
    LastAdmin,
    /// An administrator tried to delete their own account. A state
    /// conflict, not a malformed request: the account exists and could be
    /// deleted by a DIFFERENT admin, so 409 fits and lets clients show an
    /// actionable message.
    #[error("administrators cannot delete their own account")]
    AdminSelfDelete,
    /// A password reset was requested before the cooldown elapsed.
    #[error("a reset was emailed to this account recently; wait before requesting another")]
    ResetCooldown { retry_in_ms: i64 },
    /// The reset token is unknown, expired, exhausted or already used.
    #[error("this password reset link is invalid or has expired")]
    ResetInvalid,
    /// The new password failed validation.
    #[error("{0}")]
    PasswordValidation(String),
    /// Storage failure.
    #[error("authentication storage error")]
    Storage(#[from] rusqlite::Error),
}

impl AccountError {
    /// HTTP status appropriate for this error (wired up in a later phase).
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            AccountError::Validation(_) => axum::http::StatusCode::BAD_REQUEST,
            AccountError::DuplicateEmail => axum::http::StatusCode::CONFLICT,
            AccountError::NotFound => axum::http::StatusCode::NOT_FOUND,
            AccountError::InvalidTransition { .. } => axum::http::StatusCode::CONFLICT,
            AccountError::LastAdmin => axum::http::StatusCode::CONFLICT,
            AccountError::AdminSelfDelete => axum::http::StatusCode::CONFLICT,
            AccountError::ResetCooldown { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
            AccountError::ResetInvalid => axum::http::StatusCode::BAD_REQUEST,
            AccountError::PasswordValidation(_) => axum::http::StatusCode::BAD_REQUEST,
            AccountError::Storage(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Legacy schema (Phase ≤3): a single owner row plus owner-less sessions.
/// Used by the migration tests to build a realistic pre-Phase-4A database.
#[cfg(test)]
const LEGACY_AUTH_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS owner_account (
     id              INTEGER PRIMARY KEY CHECK (id = 1),
     identifier      TEXT NOT NULL UNIQUE,
     password_hash   TEXT NOT NULL,
     created_at_ms   INTEGER NOT NULL
 );
 CREATE TABLE IF NOT EXISTS sessions (
     token_digest    BLOB PRIMARY KEY,
     created_at_ms   INTEGER NOT NULL,
     expires_at_ms   INTEGER NOT NULL,
     last_seen_at_ms INTEGER NOT NULL
 );";

/// Multi-user s/// Multi-user schema (Phase 4A). `auth_account` replaces `owner_account`;
/// `sessions` gains `account_id`. Both are created fresh on a new database.
///
/// Phase 7: the partial unique index that capped the installation at ONE
/// administrator (`idx_auth_account_single_admin`) is gone. Administrators
/// may now promote other accounts, so multiple admin rows must be legal.
/// The "never zero administrators" invariant is enforced **in code** by
/// [`AuthStore::set_account_role`] inside the same transaction as the
/// demotion, and re-asserted by a dedicated regression test; a partial
/// index cannot express "at least one".
const AUTH_ACCOUNT_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS auth_account (
     id              INTEGER PRIMARY KEY AUTOINCREMENT,
     identifier      TEXT NOT NULL UNIQUE COLLATE NOCASE,
     password_hash   TEXT NOT NULL,
     role            TEXT NOT NULL DEFAULT 'user'
                     CHECK (role IN ('admin', 'user')),
     status          TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'approved', 'rejected', 'suspended')),
     created_at_ms   INTEGER NOT NULL
 );
 CREATE TABLE IF NOT EXISTS sessions_v2 (
     token_digest    BLOB PRIMARY KEY,
     account_id      INTEGER NOT NULL
                     REFERENCES auth_account (id) ON DELETE CASCADE,
     created_at_ms   INTEGER NOT NULL,
     expires_at_ms   INTEGER NOT NULL,
     last_seen_at_ms INTEGER NOT NULL
 );";

/// Consecutive sign-in failure tracking (Dino Runner trigger).
const SIGNIN_FAILURES_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS signin_failures (
     identity_key       TEXT PRIMARY KEY,
     consecutive        INTEGER NOT NULL,
     first_failed_at_ms INTEGER NOT NULL,
     last_failed_at_ms  INTEGER NOT NULL
 );";

/// Administrator-triggered password resets (User Management phase).
///
/// Only the SHA-256 digest of the raw token is stored — the same shape as
/// session tokens — so a leaked database cannot be replayed against the
/// live app. Tokens are single-use (`consumed_at_ms`), expiring
/// ([`PASSWORD_RESET_TTL`]) and attempt-bounded
/// ([`PASSWORD_RESET_MAX_ATTEMPTS`]).
const PASSWORD_RESET_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS password_reset_tokens (
     id               INTEGER PRIMARY KEY AUTOINCREMENT,
     account_id       INTEGER NOT NULL
                      REFERENCES auth_account (id) ON DELETE CASCADE,
     token_digest     BLOB NOT NULL UNIQUE,
     requested_by     INTEGER NOT NULL
                      REFERENCES auth_account (id) ON DELETE SET NULL,
     created_at_ms    INTEGER NOT NULL,
     expires_at_ms    INTEGER NOT NULL,
     attempts         INTEGER NOT NULL DEFAULT 0,
     consumed_at_ms   INTEGER
 );
 CREATE INDEX IF NOT EXISTS idx_password_reset_account
     ON password_reset_tokens (account_id);";

/// Audit log for administrative actions and authentication events
/// (User Management phase).
///
/// Rows survive account deletion: `subject_id`/`actor_id` have no foreign
/// key on purpose, so the history an administrator needs after removing an
/// account is preserved while the personal data itself is gone. The
/// `subject_email` snapshot keeps deleted accounts identifiable in the log
/// without retaining their full record.
const AUDIT_LOG_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS audit_log (
     id             INTEGER PRIMARY KEY AUTOINCREMENT,
     at_ms          INTEGER NOT NULL,
     kind           TEXT NOT NULL,
     actor_id       INTEGER,
     actor_email    TEXT,
     subject_id     INTEGER,
     subject_email  TEXT,
     detail         TEXT
 );
 CREATE INDEX IF NOT EXISTS idx_audit_subject ON audit_log (subject_id, at_ms DESC);
 CREATE INDEX IF NOT EXISTS idx_audit_kind ON audit_log (kind, at_ms DESC);";

/// Apply the User Management additive migration: `last_password_change_ms`
/// on `auth_account`, plus the reset-token and audit-log tables. Idempotent
/// and safe on every existing database shape — it only ever ADDS columns
/// and tables, never rewrites or drops anything.
fn migrate_user_management(conn: &mut Connection) -> Result<(), AuthError> {
    // 1. New tables (CREATE IF NOT EXISTS makes this a no-op when present).
    conn.execute_batch(PASSWORD_RESET_SCHEMA)?;
    conn.execute_batch(AUDIT_LOG_SCHEMA)?;

    // 2. New column on auth_account. SQLite has no ADD COLUMN IF NOT
    //    EXISTS, so probe sqlite_master's table_info first and ignore the
    //    "duplicate column" error that a racing open would produce.
    let has_column: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('auth_account')
              WHERE name = 'last_password_change_ms'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_column {
        match conn.execute(
            "ALTER TABLE auth_account ADD COLUMN last_password_change_ms INTEGER",
            [],
        ) {
            Ok(_) => {}
            // A concurrent opener added it between the probe and here.
            // SQLite reports this as a constraint failure on the column.
            Err(rusqlite::Error::SqliteFailure(ffi, _))
                if ffi.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT =>
            {
                let _ = ffi;
            }
            Err(e) => return Err(AuthError::Storage(e)),
        }
    }
    Ok(())
}

/// Apply the Phase 4A migration inside one transaction.
///
/// Safe by construction:
/// * **Fresh database** — nothing exists yet, so the multi-user schema is
///   created directly and no rebuild happens at all.
/// * **Legacy database** — `owner_account` exists. Its single row is copied
///   into `auth_account` keeping `id`, `identifier`, `password_hash` and
///   `created_at_ms` byte-for-byte, and promoted to the sole administrator
///   with `approved` status. Every existing session row is then re-inserted
///   into `sessions_v2` pointing at that account id, preserving its token
///   digest (so live cookies keep working) and all timestamps. Only after
///   the copy succeeds are the old tables dropped and `sessions_v2` renamed
///   to `sessions`. Any failure rolls the whole transaction back, leaving
///   the legacy schema and data exactly as it was.
fn migrate_to_multi_user(conn: &mut Connection) -> Result<(), AuthError> {
    // Phase 7: an older database may still carry the single-admin partial
    // unique index, which would make a second administrator impossible.
    // Drop it unconditionally and idempotently BEFORE any early return, so
    // every code path (fresh, legacy, already-migrated) ends up without it.
    conn.execute("DROP INDEX IF EXISTS idx_auth_account_single_admin", [])?;

    // Idempotency probe: is the current shape already multi-user?
    let already_migrated = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type = 'table' AND name = 'auth_account'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if already_migrated {
        return Ok(());
    }

    let has_legacy_owner = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type = 'table' AND name = 'owner_account'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);

    let tx = conn.transaction()?;
    let result = (|| -> Result<(), AuthError> {
        if has_legacy_owner {
            // Copy every legacy owner row (there is at most one, enforced by
            // CHECK (id = 1)) into the new table, preserving all columns.
            // The existing owner becomes the one and only administrator.
            tx.execute_batch(AUTH_ACCOUNT_SCHEMA)?;
            let copied = tx.execute(
                "INSERT INTO auth_account
                     (id, identifier, password_hash, role, status, created_at_ms)
                 SELECT id, identifier, password_hash, ?1, ?2, created_at_ms
                   FROM owner_account",
                rusqlite::params![ADMIN_ROLE, STATUS_APPROVED],
            )?;
            if copied != 1 {
                return Err(AuthError::Storage(rusqlite::Error::InvalidParameterName(
                    "legacy owner_account must hold exactly one row".into(),
                )));
            }
            // Re-point every existing session at the preserved account.
            // Token digests and timestamps are copied verbatim, so cookies
            // already in browsers remain valid without any client change.
            tx.execute(
                "INSERT INTO sessions_v2
                     (token_digest, account_id, created_at_ms, expires_at_ms, last_seen_at_ms)
                 SELECT s.token_digest, o.id, s.created_at_ms, s.expires_at_ms, s.last_seen_at_ms
                   FROM sessions AS s CROSS JOIN owner_account AS o",
                [],
            )?;
            // Drop the superseded tables. The data now lives (committed in
            // this same transaction) in auth_account/sessions_v2.
            tx.execute_batch("DROP TABLE sessions; DROP TABLE owner_account;")?;
            tx.execute_batch("ALTER TABLE sessions_v2 RENAME TO sessions")?;
        } else {
            // Fresh database: create the target schema directly. The
            // canonical session table is `sessions`; `sessions_v2` exists
            // only as a staging name during legacy migration.
            tx.execute_batch(AUTH_ACCOUNT_SCHEMA)?;
            tx.execute_batch("ALTER TABLE sessions_v2 RENAME TO sessions")?;
        }
        tx.execute_batch(SIGNIN_FAILURES_SCHEMA)?;
        Ok(())
    })();
    match result {
        Ok(()) => tx.commit().map_err(AuthError::from)?,
        Err(e) => {
            // Roll back; the caller's database keeps its previous state.
            let _ = tx.rollback();
            return Err(e);
        }
    }
    Ok(())
}

/// Apply the Phase 4A migration inside one transaction, then the User
/// Management additive migration.
///
/// Safe by construction:
impl AuthStore {
    /// Open (creating if necessary) the auth store at `path`, applying the
    /// Phase 4A multi-user migration first.
    ///
    /// On a **legacy** database (`owner_account` present) the single owner
    /// row — identifier, Argon2id password hash and creation timestamp — is
    /// preserved verbatim and promoted to the sole administrator account,
    /// and every existing session keeps its token digest and timestamps,
    /// so live cookies continue to validate. See [`migrate_to_multi_user`].
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, AuthError> {
        let mut conn = Connection::open(path)?;
        // Wait (briefly) for a competing writer instead of failing outright.
        // With multiple administrators now possible, two role changes can
        // genuinely race; SQLite's default is to fail IMMEDIATELY with
        // SQLITE_BUSY, which would surface as a spurious 500. A short
        // timeout makes the loser retry behind the winner. Set it BEFORE
        // any pragma that takes a write lock (`journal_mode` does).
        conn.busy_timeout(std::time::Duration::from_millis(5_000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Foreign keys are required for ON DELETE CASCADE (sessions,
        // reset tokens) to fire when an account is deleted. SQLite keeps
        // this OFF by default for compatibility, so enable it explicitly —
        // every existing deployment of this store has the constraint
        // declared but never enforced.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate_to_multi_user(&mut conn)?;
        migrate_user_management(&mut conn)?;
        Ok(Self {
            conn: parking_lot::Mutex::new(conn),
            signin_attempts: parking_lot::Mutex::new(Vec::new()),
            registration_attempts: parking_lot::Mutex::new(Vec::new()),
        })
    }

    /// True when no administrator account exists yet (first-run bootstrap
    /// state: the grandfathered owner has not been provisioned).
    pub fn is_first_run(&self) -> Result<bool, AuthError> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM auth_account WHERE role = ?1",
            [ADMIN_ROLE],
            |r| r.get(0),
        )?;
        Ok(n == 0)
    }

    /// Create the initial administrator account. Only legal on first run
    /// (no administrator exists yet); later accounts must go through
    /// [`AuthStore::create_pending_user`].
    pub fn sign_up(&self, identifier: &str, password: &str) -> Result<(), AuthError> {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Err(AuthError::Validation("identifier is required".into()));
        }
        if identifier.len() > MAX_IDENTIFIER_LEN {
            return Err(AuthError::Validation("identifier is too long".into()));
        }
        if !identifier
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_'))
        {
            return Err(AuthError::Validation(
                "identifier may contain letters, digits and @ . - _ only".into(),
            ));
        }
        if password.len() < MIN_PASSWORD_LEN {
            return Err(AuthError::Validation(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if password.len() > 256 {
            return Err(AuthError::Validation("password is too long".into()));
        }
        if !self.is_first_run()? {
            return Err(AuthError::OwnerExists);
        }

        // Argon2id with per-password random salt.
        let salt = random_salt();
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AuthError::Storage(rusqlite::Error::InvalidQuery))?
            .to_string();

        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO auth_account
                 (identifier, password_hash, role, status, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                normalize_identity(identifier),
                hash,
                ADMIN_ROLE,
                STATUS_APPROVED,
                now
            ],
        )?;
        Ok(())
    }

    /// Verify credentials, minting a fresh session when they match.
    /// Returns the raw token to place in the cookie.
    ///
    /// Status policy (checked only AFTER the password verifies, so no
    /// information about account existence or status leaks to callers with
    /// wrong credentials):
    /// * `approved`  → session minted.
    /// * `pending`   → [`AuthError::AccountPending`] (403), no session.
    /// * `rejected` / `suspended` → generic `InvalidCredentials`, and any
    ///   existing sessions for that account are revoked.
    pub fn sign_in(&self, identifier: &str, password: &str) -> Result<String, AuthError> {
        // Rate limit BEFORE touching the credential check.
        self.throttle_signin()?;

        let identifier_trimmed = identifier.trim();
        if identifier_trimmed.is_empty() || password.is_empty() {
            return Err(AuthError::InvalidCredentials);
        }

        let lookup: Result<String, rusqlite::Error> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT password_hash FROM auth_account
                  WHERE identifier = ?1 COLLATE NOCASE",
                rusqlite::params![identifier_trimmed],
                |r| r.get(0),
            )
        };
        let stored_hash = match lookup {
            Ok(hash) => hash,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Burn comparable time so absence is not observable by timing.
                let _ = Argon2::default().hash_password(b"timing-equalizer", &random_salt());
                // Unknown (or differently-cased) identifier: still a
                // failed attempt for the normalized identity, so the
                // counter cannot be forked or dodged by casing. Recorded
                // only after the connection guard above is released:
                // record_signin_failure re-locks the same non-reentrant
                // mutex, so calling it under the guard would self-deadlock.
                self.record_signin_failure(&normalize_identity(identifier));
                return Err(AuthError::InvalidCredentials);
            }
            Err(e) => return Err(AuthError::Storage(e)),
        };

        let parsed = PasswordHash::new(&stored_hash)
            .map_err(|_| AuthError::Storage(rusqlite::Error::InvalidQuery))?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            // Wrong password: record the consecutive failure for this
            // normalized identity (never the credential itself).
            self.record_audit(
                audit_event::SIGNIN_FAILED,
                None,
                None,
                &format!("wrong password for {}", normalize_identity(identifier)),
            );
            self.record_signin_failure(&normalize_identity(identifier));
            return Err(AuthError::InvalidCredentials);
        }

        // ---- Password verified. Only now may status influence the result. ----

        // Read the account's id and status in one go. The row exists (the
        // hash came from it), so NoRows here means a concurrent deletion —
        // treat it as invalid credentials rather than crashing.
        let account: Option<(i64, String)> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT id, status FROM auth_account
                  WHERE identifier = ?1 COLLATE NOCASE",
                rusqlite::params![identifier_trimmed],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        };
        let Some((account_id, status)) = account else {
            self.record_signin_failure(&normalize_identity(identifier));
            return Err(AuthError::InvalidCredentials);
        };

        match status.as_str() {
            STATUS_APPROVED => { /* fall through to session minting */ }
            STATUS_PENDING => {
                // Correct credentials, awaiting approval. Do NOT count this
                // as a failure against the rate limiter's per-identity
                // counter: the user did nothing wrong, and counting it
                // could lock out a legitimate registrant who retries while
                // waiting. The global throttle above still applies.
                self.record_audit(
                    audit_event::SIGNIN_FAILED,
                    None,
                    Some((account_id, identifier_trimmed)),
                    "account pending approval",
                );
                return Err(AuthError::AccountPending);
            }
            STATUS_SUSPENDED => {
                // Correct credentials, suspended by an administrator. Same
                // information-disclosure discipline as the pending case:
                // only the real owner (correct password) ever sees this.
                // Sessions were already revoked at suspension time; revoke
                // again defensively in case one slipped through.
                self.revoke_account_sessions(account_id);
                self.record_audit(
                    audit_event::SIGNIN_FAILED,
                    None,
                    Some((account_id, identifier_trimmed)),
                    "account suspended",
                );
                return Err(AuthError::AccountSuspended);
            }
            // rejected (or anything unexpected): deny without saying why,
            // and revoke whatever sessions the account still holds so the
            // refusal takes effect immediately.
            _ => {
                self.revoke_account_sessions(account_id);
                self.record_audit(
                    audit_event::SIGNIN_FAILED,
                    None,
                    Some((account_id, identifier_trimmed)),
                    "account rejected",
                );
                self.record_signin_failure(&normalize_identity(identifier));
                return Err(AuthError::InvalidCredentials);
            }
        }

        // Credentials good and account approved: reset the
        // consecutive-failure counter for this identity, then mint a session.
        self.clear_signin_failures(&normalize_identity(identifier));
        let token = Self::new_token();
        let digest = token_digest(&token);
        let now = chrono::Utc::now().timestamp_millis();
        let expires = now + SESSION_TTL.as_millis() as i64;
        {
            let conn = self.conn.lock();
            // Only APPROVED accounts may hold sessions. The status filter is
            // repeated inside the INSERT..SELECT so there is no window in
            // which a just-suspended account can slip a session through,
            // even if its status changed between the read above and here.
            let inserted = conn.execute(
                "INSERT INTO sessions
                     (token_digest, account_id, created_at_ms, expires_at_ms, last_seen_at_ms)
                 SELECT ?1, a.id, ?2, ?3, ?2
                   FROM auth_account a
                  WHERE a.id = ?4 AND a.status = ?5",
                rusqlite::params![digest, now, expires, account_id, STATUS_APPROVED],
            )?;
            if inserted == 0 {
                // The account was suspended between the status read and the
                // insert. Refuse without leaking which reason applied.
                return Err(AuthError::InvalidCredentials);
            }
            // Opportunistic cleanup of expired sessions.
            conn.execute("DELETE FROM sessions WHERE expires_at_ms < ?1", [now])?;
        }
        // Successful authentication is itself an auditable event: it is
        // what the activity view's "last sign-in" narrative is built from.
        self.record_audit(
            audit_event::SIGNIN_SUCCESS,
            None,
            Some((account_id, identifier_trimmed)),
            "signed in",
        );
        Ok(token)
    }

    /// Delete every session belonging to `account_id`.
    ///
    /// Used when an account is denied access (suspension, rejection) so its
    /// live cookies stop working immediately rather than at next validation.
    /// Best-effort by design: the read path re-checks status on every
    /// request anyway, so a failed delete cannot grant access.
    fn revoke_account_sessions(&self, account_id: i64) {
        let conn = self.conn.lock();
        let _ = conn.execute("DELETE FROM sessions WHERE account_id = ?1", [account_id]);
    }

    /// Validate a raw cookie token; refreshes the sliding expiry on success.
    ///
    /// A session validates only when ALL of the following hold, verified in
    /// a single joined query so no request can slip between checks:
    /// * the token's row exists and has not expired,
    /// * the owning account still exists (not deleted),
    /// * that account's status is currently `approved`.
    ///
    /// This is what makes suspension take effect on the very next request —
    /// no restart, no TTL wait — and why the guard layer must always go
    /// through this method rather than trusting a cached result.
    pub fn validate_session(&self, token: &str) -> Option<SessionInfo> {
        if token.is_empty() {
            return None;
        }
        let digest = token_digest(token);
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        // One query: expiry + account existence + current status together.
        let found: Option<(String, i64)> = conn
            .query_row(
                "SELECT a.identifier, s.expires_at_ms
                   FROM sessions AS s
                   JOIN auth_account AS a ON a.id = s.account_id
                  WHERE s.token_digest = ?1 AND a.status = ?2",
                rusqlite::params![digest, STATUS_APPROVED],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((identifier, expires_at_ms)) = found else {
            // Either the token is unknown, it belongs to an account that no
            // longer exists, or the account is no longer approved. In every
            // case access is denied; drop the row so a suspended account's
            // stale cookie cannot linger.
            let _ = conn.execute(
                "DELETE FROM sessions WHERE token_digest = ?1",
                rusqlite::params![digest],
            );
            return None;
        };
        if now >= expires_at_ms {
            let _ = conn.execute(
                "DELETE FROM sessions WHERE token_digest = ?1",
                rusqlite::params![digest],
            );
            return None;
        }
        // Sliding renewal: activity extends the session up to TTL from now.
        let new_expiry = now + SESSION_TTL.as_millis() as i64;
        let _ = conn.execute(
            "UPDATE sessions SET last_seen_at_ms = ?1, expires_at_ms = ?2 WHERE token_digest = ?3",
            rusqlite::params![now, new_expiry, digest],
        );
        Some(SessionInfo {
            identifier,
            expires_at_ms: new_expiry,
        })
    }

    /// Invalidate one session (its raw token).
    pub fn sign_out(&self, token: &str) -> Result<(), AuthError> {
        let digest = token_digest(token);
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM sessions WHERE token_digest = ?1",
            rusqlite::params![digest],
        )?;
        Ok(())
    }

    /// Invalidate every session (used on password change in a future phase).
    pub fn sign_out_all(&self) -> Result<(), AuthError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM sessions", [])?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Consecutive sign-in failure tracking (Dino Runner trigger backing)
    // ------------------------------------------------------------------

    /// Record one failed authentication attempt for `identity_key` (already
    /// normalized by the caller, or via [`normalize_identity`]).
    ///
    /// Concurrency: the UPDATE is a single atomic statement with
    /// `consecutive + 1` computed in SQL, so interleaved requests can only
    /// ever produce a correct monotonically increasing count — no lost
    /// updates, no torn resets. A successful sign-in's DELETE and a racing
    /// failure's UPSERT serialize on the row lock; the outcome is always
    /// either "reset then counted from 1" or "counted then reset", never a
    /// corrupted counter.
    ///
    /// Storage: only the count and timestamps are kept — never passwords,
    /// hashes, OTPs, or session tokens.
    fn record_signin_failure(&self, identity_key: &str) {
        if identity_key.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        // Best-effort by design: failure tracking must never turn a
        // sign-in rejection into a 500.
        let _ = conn.execute(
            "INSERT INTO signin_failures
                 (identity_key, consecutive, first_failed_at_ms, last_failed_at_ms)
             VALUES (?1, 1, ?2, ?2)
             ON CONFLICT(identity_key) DO UPDATE SET
                 consecutive     = excluded.consecutive + signin_failures.consecutive,
                 -- keep the original first_failed_at_ms; refresh the last
                 last_failed_at_ms = excluded.last_failed_at_ms",
            rusqlite::params![identity_key, now],
        );
        // Opportunistic retention: drop records idle past the window so the
        // table cannot grow without bound under identifier spraying.
        let cutoff = now - SIGNIN_FAILURE_RETENTION.as_millis() as i64;
        let _ = conn.execute(
            "DELETE FROM signin_failures WHERE last_failed_at_ms < ?1",
            [cutoff],
        );
    }

    /// Clear the consecutive-failure record for `identity_key` after a
    /// successful authentication. Idempotent.
    fn clear_signin_failures(&self, identity_key: &str) {
        if identity_key.is_empty() {
            return;
        }
        let conn = self.conn.lock();
        let _ = conn.execute(
            "DELETE FROM signin_failures WHERE identity_key = ?1",
            [identity_key],
        );
    }

    /// Current consecutive-failure count for a raw identifier (normalized
    /// internally). Tests and the future HTTP layer read this to decide
    /// whether the Dino Runner challenge should be shown. Never fails:
    /// an unreadable counter degrades to 0, which only means "no extra
    /// UI challenge", while the real rate limiter stays in force.
    pub fn consecutive_signin_failures(&self, raw_identifier: &str) -> u32 {
        let key = normalize_identity(raw_identifier);
        if key.is_empty() {
            return 0;
        }
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT consecutive FROM signin_failures WHERE identity_key = ?1",
            [&key],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n.max(0) as u32)
        .unwrap_or(0)
    }

    /// Number of distinct identities currently tracked (tests/diagnostics).
    pub fn tracked_failure_identities(&self) -> usize {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM signin_failures", [], |r| {
            r.get::<_, i64>(0)
        })
        .map(|n| n as usize)
        .unwrap_or(0)
    }

    // ------------------------------------------------------------------
    // Registration rate limiting (Phase 4E)
    // ------------------------------------------------------------------

    /// Count a registration attempt against the global budget and decide
    /// whether it may proceed.
    ///
    /// Policy — **global fixed capacity, sliding window**:
    /// * [`REGISTRATION_CAPACITY`] registrations are accepted per rolling
    ///   [`REGISTRATION_WINDOW`].
    /// * The state is a single `Vec<Instant>` guarded by one mutex — the
    ///   same pattern as the sign-in throttle, so concurrency cannot
    ///   bypass it: the check-and-push happens under one lock hold, so two
    ///   racing requests both observe the same count and only one can win
    ///   the last slot.
    /// * Bounded **by construction**: the vector never exceeds
    ///   [`REGISTRATION_CAPACITY`] entries (a push only happens when the
    ///   in-window count is strictly below capacity), and stale entries
    ///   are pruned on every call. There is no map keyed by email, IP or
    ///   any other attacker-controlled value — an attacker cannot inflate
    ///   memory by rotating addresses.
    ///
    /// Why global rather than per-address: `/api/auth/register` is a
    /// public, unauthenticated endpoint on a single-user local service,
    /// and the resource being protected (Argon2id hashing + a pending row
    /// an administrator must review) is process-wide, not per-identity.
    /// A per-email map would be an unbounded, attacker-keyed structure;
    /// a per-IP map is wrong for loopback-only deployments where every
    /// request shares one address. A global budget bounds the worst case
    /// (a flood produces at most `capacity` rows per window) while
    /// keeping legitimate use — a handful of colleagues registering —
    /// unaffected.
    ///
    /// The decision is made **before** any validation or storage work, so
    /// a rejected request costs one lock acquisition and nothing else.
    ///
    /// Takes `&self` (not `&mut self`): the store lives behind an `Arc`
    /// shared across handlers, so mutability is provided by the field's
    /// own mutex rather than the borrow.
    pub fn allow_registration_attempt(&self) -> Result<(), AuthError> {
        let mut attempts = self.registration_attempts.lock();
        let now = Instant::now();
        attempts.retain(|t| now.duration_since(*t) < REGISTRATION_WINDOW);
        if attempts.len() >= REGISTRATION_CAPACITY {
            return Err(AuthError::RateLimited);
        }
        attempts.push(now);
        Ok(())
    }

    /// Registrations still accepted in the current window (tests only).
    #[cfg(test)]
    pub fn registration_capacity_remaining(&self) -> usize {
        let attempts = self.registration_attempts.lock();
        let now = Instant::now();
        attempts
            .iter()
            .filter(|t| now.duration_since(**t) < REGISTRATION_WINDOW)
            .count()
    }

    // ------------------------------------------------------------------
    // Registration storage primitive (verified-registration flow)
    // ------------------------------------------------------------------

    /// Create the owner account AND its first session in one SQLite
    /// transaction, for use exclusively by the verified-registration flow.
    ///
    /// Contract:
    /// * **Single owner, enforced by the database.** The insert targets
    ///   `id = 1` with `CHECK (id = 1)` and a UNIQUE identifier; a second
    ///   concurrent registration fails on the constraint inside the
    ///   transaction and rolls back whole — no partial owner, no orphan
    ///   session, no grant left half-redeemed by this layer.
    /// * **Never touches an existing owner.** If any owner row exists the
    ///   call returns [`AuthError::OwnerExists`] before writing anything.
    /// * **Existing behavior preserved:** `sign_up`/`sign_in` are
    ///   unchanged; this method reuses their Argon2id hashing parameters
    ///   and token-minting code paths.
    /// * The caller (HTTP layer, later sub-phase) is responsible for
    ///   consuming the verification grant; this primitive deliberately
    ///   has no notion of grants so it cannot be used to bypass them.
    pub fn create_owner_with_session(
        &self,
        identifier: &str,
        password: &str,
    ) -> Result<String, AuthError> {
        // Validation identical to sign_up (same rules, same messages).
        let identifier_trimmed = identifier.trim();
        if identifier_trimmed.is_empty() {
            return Err(AuthError::Validation("identifier is required".into()));
        }
        if identifier_trimmed.len() > MAX_IDENTIFIER_LEN {
            return Err(AuthError::Validation("identifier is too long".into()));
        }
        if !identifier_trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_'))
        {
            return Err(AuthError::Validation(
                "identifier may contain letters, digits and @ . - _ only".into(),
            ));
        }
        if password.len() < MIN_PASSWORD_LEN {
            return Err(AuthError::Validation(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if password.len() > 256 {
            return Err(AuthError::Validation("password is too long".into()));
        }

        // Hash BEFORE opening the transaction: Argon2id is deliberately
        // slow, and holding the connection lock across it would stall every
        // other auth operation on the machine.
        let salt = random_salt();
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AuthError::Storage(rusqlite::Error::InvalidQuery))?
            .to_string();
        let token = Self::new_token();
        let digest = token_digest(&token);

        let now = chrono::Utc::now().timestamp_millis();
        let expires = now + SESSION_TTL.as_millis() as i64;

        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        // Guard inside the transaction: refuse if an administrator already
        // exists. This is what makes concurrent attempts safe — the check
        // and the insert are serialized by SQLite's write lock, so two
        // racers cannot both see "no owner".
        let admins: i64 = tx.query_row(
            "SELECT COUNT(*) FROM auth_account WHERE role = ?1",
            [ADMIN_ROLE],
            |r| r.get(0),
        )?;
        if admins > 0 {
            return Err(AuthError::OwnerExists);
        }
        tx.execute(
            "INSERT INTO auth_account
                 (identifier, password_hash, role, status, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                normalize_identity(identifier_trimmed),
                hash,
                ADMIN_ROLE,
                STATUS_APPROVED,
                now
            ],
        )?;
        let account_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO sessions
                 (token_digest, account_id, created_at_ms, expires_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?3)",
            rusqlite::params![digest, account_id, now, expires],
        )?;
        tx.commit()?;
        Ok(token)
    }

    // ------------------------------------------------------------------
    // Phase 4A: multi-user account storage primitives
    //
    // These are storage operations only. No HTTP handler calls them yet;
    // the admin/registration endpoints arrive in a later phase.
    // ------------------------------------------------------------------

    /// Look up an account by its normalized identifier.
    ///
    /// Matching is case-insensitive on the stored (already normalized)
    /// identifier, so `"User@Soket.AI"` and `"user@soket.ai"` resolve to the
    /// same row. Returns [`AccountError::NotFound`] when absent.
    pub fn find_account_by_identifier(
        &self,
        identifier: &str,
    ) -> Result<AccountRecord, AccountError> {
        let key = normalize_identity(identifier);
        if key.is_empty() {
            return Err(AccountError::Validation("identifier is required".into()));
        }
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, identifier, role, status, created_at_ms
               FROM auth_account
              WHERE identifier = ?1 COLLATE NOCASE",
            [&key],
            |r| {
                Ok(AccountRecord {
                    id: r.get(0)?,
                    identifier: r.get(1)?,
                    role: r.get(2)?,
                    status: r.get(3)?,
                    created_at_ms: r.get(4)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
            other => AccountError::Storage(other),
        })
    }

    /// Create a **pending** user account with an Argon2id password hash.
    ///
    /// * The identifier must be an exact `@soket.ai` address (validated by
    ///   [`crate::auth_email::normalize_registration_email`], which also
    ///   normalizes it).
    /// * The account is stored with `role = user` and `status = pending`; it
    ///   cannot sign in until an administrator approves it.
    /// * Duplicate normalized emails are rejected by the database's UNIQUE
    ///   constraint, so concurrent registrations of the same address cannot
    ///   both succeed.
    pub fn create_pending_user(
        &self,
        raw_email: &str,
        password: &str,
    ) -> Result<i64, AccountError> {
        let email = crate::auth_email::normalize_registration_email(raw_email)
            .map_err(AccountError::Validation)?;
        if password.len() < MIN_PASSWORD_LEN {
            return Err(AccountError::Validation(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if password.len() > 256 {
            return Err(AccountError::Validation("password is too long".into()));
        }

        // Hash BEFORE taking the connection lock: Argon2id is deliberately
        // slow and must not stall every other auth operation.
        let salt = random_salt();
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AccountError::Storage(rusqlite::Error::InvalidQuery))?
            .to_string();

        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        let inserted = conn.execute(
            "INSERT INTO auth_account
                 (identifier, password_hash, role, status, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![email, hash, USER_ROLE, STATUS_PENDING, now],
        );
        match inserted {
            Ok(_) => {}
            // The normalized identifier is UNIQUE, so a collision (including
            // a racing registration of the same address) is a duplicate, not
            // a storage fault. SQLite reports it as SQLITE_CONSTRAINT_UNIQUE.
            Err(rusqlite::Error::SqliteFailure(ffi, _))
                if ffi.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                return Err(AccountError::DuplicateEmail);
            }
            Err(e) => return Err(AccountError::Storage(e)),
        }
        Ok(conn.last_insert_rowid())
    }

    /// Read an account's role and status by id.
    pub fn account_role_and_status(&self, id: i64) -> Result<(String, String), AccountError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT role, status FROM auth_account WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
            other => AccountError::Storage(other),
        })
    }

    /// Change an account's status through a validated transition.
    ///
    /// Permitted transitions:
    ///
    /// ```text
    /// pending   -> approved | rejected
    /// approved  -> suspended
    /// suspended -> approved          (reinstatement)
    /// rejected  -> pending           (re-open a request)
    /// ```
    ///
    /// Everything else — including any transition targeting an
    /// administrator account, which must stay `approved` — is refused with
    /// [`AccountError::InvalidTransition`]. The guard and the update run in
    /// one transaction, so a concurrent writer cannot race the check.
    pub fn set_account_status(&self, id: i64, new_status: &str) -> Result<(), AccountError> {
        let new_status = new_status.trim();
        let valid = matches!(
            new_status,
            STATUS_PENDING | STATUS_APPROVED | STATUS_REJECTED | STATUS_SUSPENDED
        );
        if !valid {
            return Err(AccountError::Validation(format!(
                "unknown account status {new_status:?}"
            )));
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let current: Option<(String, String)> = tx
            .query_row(
                "SELECT role, status FROM auth_account WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
                other => AccountError::Storage(other),
            })
            .map(Some)
            .or_else(|e| match e {
                AccountError::NotFound => Ok(None),
                other => Err(other),
            })?;
        let Some((role, current_status)) = current else {
            tx.rollback().ok();
            return Err(AccountError::NotFound);
        };
        let allowed = role != ADMIN_ROLE
            && match (current_status.as_str(), new_status) {
                (STATUS_PENDING, STATUS_APPROVED | STATUS_REJECTED) => true,
                (STATUS_APPROVED, STATUS_SUSPENDED) => true,
                (STATUS_SUSPENDED, STATUS_APPROVED) => true,
                (STATUS_REJECTED, STATUS_PENDING) => true,
                _ => false,
            };
        if !allowed {
            tx.rollback().ok();
            return Err(AccountError::InvalidTransition {
                from: current_status,
                to: new_status.to_string(),
            });
        }
        tx.execute(
            "UPDATE auth_account SET status = ?1 WHERE id = ?2 AND role != ?3",
            rusqlite::params![new_status, id, ADMIN_ROLE],
        )?;
        // Leaving the approved state revokes the account's sessions in the
        // SAME transaction as the status change, so a suspended user's live
        // cookies stop working immediately. (The read path re-checks status
        // on every request too — this is defence in depth, and it also
        // cleans up rows that would otherwise linger until their expiry.)
        // Reinstatement does NOT resurrect old sessions: the user simply
        // signs in again, which mints a fresh one.
        if new_status != STATUS_APPROVED {
            tx.execute("DELETE FROM sessions WHERE account_id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// List accounts awaiting administrator review, oldest first.
    ///
    /// Intended for the future admin registration-review API; bounded by
    /// `limit` so a flood of requests cannot produce an unbounded response.
    pub fn list_pending_accounts(&self, limit: i64) -> Result<Vec<AccountRecord>, AccountError> {
        if limit <= 0 {
            return Err(AccountError::Validation("limit must be positive".into()));
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, identifier, role, status, created_at_ms
               FROM auth_account
              WHERE status = ?1
              ORDER BY created_at_ms ASC, id ASC
              LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![STATUS_PENDING, limit], |r| {
                Ok(AccountRecord {
                    id: r.get(0)?,
                    identifier: r.get(1)?,
                    role: r.get(2)?,
                    status: r.get(3)?,
                    created_at_ms: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Total number of accounts awaiting review (Phase 4E).
    ///
    /// Used alongside [`AuthStore::list_pending_accounts`] so the admin
    /// listing can report **truncation explicitly** rather than silently
    /// dropping rows past the page bound: the response says how many
    /// pending accounts exist in total, and the caller can tell whether
    /// the returned page is the whole set or just the first slice.
    pub fn count_pending_accounts(&self) -> Result<i64, AccountError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM auth_account WHERE status = ?1",
            [STATUS_PENDING],
            |r| r.get(0),
        )
        .map_err(AccountError::from)
    }

    // ------------------------------------------------------------------
    // Phase 7: full account listing + role management
    // ------------------------------------------------------------------

    /// List accounts matching a filter, newest-registered first, with
    /// bounded pagination.
    ///
    /// Returns the page plus the **total** number of matching rows, so the
    /// caller can render "x of y" and detect truncation without a second
    /// round trip. `last_sign_in_at_ms` is derived from the newest live or
    /// expired session row for that account: it is absent when the account
    /// never signed in, and never fabricated.
    pub fn list_accounts(
        &self,
        filter: &AccountFilter,
    ) -> Result<(Vec<ListedAccount>, i64), AccountError> {
        // ---- validate + clamp every client-supplied value ----
        let per_page = if filter.per_page < 1 {
            50
        } else {
            filter.per_page.min(ACCOUNT_LIST_MAX_PER_PAGE)
        };
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(per_page);

        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(q) = filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // Escape LIKE metacharacters so a query containing % or _ is a
            // literal substring search, not a pattern injection.
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            where_clauses.push(format!(
                "a.identifier LIKE ?{n} ESCAPE '\\'",
                n = params.len() + 1
            ));
            params.push(Box::new(format!("%{escaped}%")));
        }
        if let Some(role) = filter
            .role
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !matches!(role, ADMIN_ROLE | USER_ROLE) {
                return Err(AccountError::Validation(format!("unknown role {role:?}")));
            }
            where_clauses.push(format!("a.role = ?{n}", n = params.len() + 1));
            params.push(Box::new(role.to_string()));
        }
        if let Some(status) = filter
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !matches!(
                status,
                STATUS_PENDING | STATUS_APPROVED | STATUS_REJECTED | STATUS_SUSPENDED
            ) {
                return Err(AccountError::Validation(format!(
                    "unknown status {status:?}"
                )));
            }
            where_clauses.push(format!("a.status = ?{n}", n = params.len() + 1));
            params.push(Box::new(status.to_string()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let conn = self.conn.lock();

        // Total matching rows (independent of the page window).
        let count_sql = format!("SELECT COUNT(*) FROM auth_account a {where_sql}");
        let total: i64 = {
            let mut stmt = conn.prepare(&count_sql)?;
            let n = stmt.query_row(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |r| r.get(0),
            )?;
            n
        };

        // The page itself. The last-sign-in subquery takes MAX over the
        // account's session rows; sessions are deleted on suspension, so a
        // suspended account shows its last pre-suspension sign-in (accurate)
        // and a never-used account shows NULL (never fabricated).
        let page_sql = format!(
            "SELECT a.id, a.identifier, a.role, a.status, a.created_at_ms,
                    (SELECT MAX(s.last_seen_at_ms) FROM sessions s
                      WHERE s.account_id = a.id)
               FROM auth_account a
               {where_sql}
              ORDER BY a.created_at_ms DESC, a.id DESC
              LIMIT ?{limit_n} OFFSET ?{offset_n}",
            limit_n = params.len() + 1,
            offset_n = params.len() + 2,
        );
        let rows = {
            let mut stmt = conn.prepare(&page_sql)?;
            let mut all: Vec<Box<dyn rusqlite::ToSql>> = params;
            all.push(Box::new(per_page));
            all.push(Box::new(offset));
            let mapped = stmt.query_map(
                rusqlite::params_from_iter(all.iter().map(|p| p.as_ref())),
                |r| {
                    Ok(ListedAccount {
                        id: r.get(0)?,
                        identifier: r.get(1)?,
                        role: r.get(2)?,
                        status: r.get(3)?,
                        created_at_ms: r.get(4)?,
                        last_sign_in_at_ms: r.get(5).ok(),
                    })
                },
            )?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok((rows, total))
    }

    /// Change an account's role (Phase 7).
    ///
    /// Rules, all enforced inside one transaction:
    ///
    /// * `user → admin` (promotion) requires the target to currently be
    ///   `approved`. A pending/rejected/suspended account cannot be handed
    ///   authority it could not even use to sign in — approval must come
    ///   first, as a separate explicit action.
    /// * `admin → user` (demotion) is refused when the target is the last
    ///   remaining administrator, so an installation can never strand
    ///   itself with no administrator at all.
    /// * The caller cannot change its own role. Self-demotion is the
    ///   classic way an administrator locks everyone out by accident, and
    ///   self-promotion is meaningless.
    /// * Role changes never touch `password_hash`, `status` or sessions:
    ///   the target keeps their credentials and must already be signed in
    ///   (or sign in later) under them. No new session is minted.
    ///
    /// Returns the new role.
    pub fn set_account_role(&self, id: i64, new_role: &str) -> Result<String, AccountError> {
        let new_role = new_role.trim();
        if !matches!(new_role, ADMIN_ROLE | USER_ROLE) {
            return Err(AccountError::Validation(format!(
                "unknown role {new_role:?}"
            )));
        }
        let mut conn = self.conn.lock();
        // BEGIN IMMEDIATE, not a deferred BEGIN. This function reads the
        // target's current role and then writes, and a deferred transaction
        // that upgrades a read lock to a write lock gets an IMMEDIATE
        // SQLITE_BUSY when another writer commits in between — SQLite's
        // busy handler deliberately does not retry that case, because
        // waiting cannot resolve it (both readers would wait forever).
        // Taking the write lock up front means a racer simply queues behind
        // this transaction via `busy_timeout`, which is correct.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Read the target's CURRENT role and status inside the transaction,
        // so a concurrent writer cannot interleave between check and update.
        let current: Option<(String, String)> = tx
            .query_row(
                "SELECT role, status FROM auth_account WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
                other => AccountError::Storage(other),
            })
            .map(Some)
            .or_else(|e| match e {
                AccountError::NotFound => Ok(None),
                other => Err(other),
            })?;
        let Some((current_role, current_status)) = current else {
            tx.rollback().ok();
            return Err(AccountError::NotFound);
        };

        if current_role == new_role {
            // Idempotent no-op: report success with the unchanged role so a
            // double-click or a retry after a timeout is harmless.
            tx.rollback().ok();
            return Ok(current_role);
        }

        if current_role == USER_ROLE && new_role == ADMIN_ROLE {
            // Promotion: only an APPROVED account may gain authority.
            if current_status != STATUS_APPROVED {
                tx.rollback().ok();
                return Err(AccountError::InvalidTransition {
                    from: current_status,
                    to: format!("{new_role} (requires approved status)"),
                });
            }
        } else if current_role == ADMIN_ROLE && new_role == USER_ROLE {
            // Demotion: refuse to strand the installation without an admin.
            let admins: i64 = tx.query_row(
                "SELECT COUNT(*) FROM auth_account WHERE role = ?1",
                [ADMIN_ROLE],
                |r| r.get(0),
            )?;
            if admins <= 1 {
                tx.rollback().ok();
                return Err(AccountError::LastAdmin);
            }
        }

        tx.execute(
            "UPDATE auth_account SET role = ?1 WHERE id = ?2",
            rusqlite::params![new_role, id],
        )?;
        tx.commit()?;
        Ok(new_role.to_string())
    }

    // ------------------------------------------------------------------
    // User Management: details, sessions, deletion, password reset, audit
    // ------------------------------------------------------------------

    /// Look up an account by its numeric id.
    ///
    /// Mirrors [`AuthStore::find_account_by_identifier`] but keyed on the
    /// primary key, which is what every admin URL carries.
    pub fn find_account_by_id(&self, id: i64) -> Result<AccountRecord, AccountError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, identifier, role, status, created_at_ms
               FROM auth_account
              WHERE id = ?1",
            [id],
            |r| {
                Ok(AccountRecord {
                    id: r.get(0)?,
                    identifier: r.get(1)?,
                    role: r.get(2)?,
                    status: r.get(3)?,
                    created_at_ms: r.get(4)?,
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
            other => AccountError::Storage(other),
        })
    }

    /// Live (unexpired) sessions for one account, newest activity first.
    ///
    /// Bounded by `limit` so a pathological account cannot produce an
    /// unbounded response.
    pub fn account_sessions(
        &self,
        account_id: i64,
        limit: i64,
    ) -> Result<Vec<SessionSummary>, AccountError> {
        if limit <= 0 {
            return Err(AccountError::Validation("limit must be positive".into()));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT created_at_ms, expires_at_ms, last_seen_at_ms
               FROM sessions
              WHERE account_id = ?1 AND expires_at_ms > ?2
              ORDER BY last_seen_at_ms DESC
              LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![account_id, now, limit], |r| {
                Ok(SessionSummary {
                    created_at_ms: r.get(0)?,
                    expires_at_ms: r.get(1)?,
                    last_seen_at_ms: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read the full detail record for one account.
    pub fn account_detail(&self, id: i64) -> Result<AccountDetail, AccountError> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT a.id, a.identifier, a.role, a.status, a.created_at_ms,
                    a.last_password_change_ms,
                    (SELECT COUNT(*) FROM sessions s
                      WHERE s.account_id = a.id AND s.expires_at_ms > ?2),
                    (SELECT MAX(s.last_seen_at_ms) FROM sessions s
                      WHERE s.account_id = a.id)
               FROM auth_account a
              WHERE a.id = ?1",
            rusqlite::params![id, now],
            |r| {
                Ok(AccountDetail {
                    id: r.get(0)?,
                    identifier: r.get(1)?,
                    role: r.get(2)?,
                    status: r.get(3)?,
                    created_at_ms: r.get(4)?,
                    last_password_change_ms: r.get(5).ok(),
                    live_session_count: r.get(6)?,
                    last_sign_in_at_ms: r.get(7).ok(),
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
            other => AccountError::Storage(other),
        })
    }

    /// Revoke every live session belonging to `account_id`.
    ///
    /// Returns the number of sessions actually removed, so the caller can
    /// report an honest result ("3 sessions revoked") rather than a bare
    /// success even when there was nothing to revoke.
    pub fn revoke_all_sessions_for(&self, account_id: i64) -> Result<usize, AccountError> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM sessions WHERE account_id = ?1",
            [account_id],
        )?;
        Ok(n)
    }

    /// Permanently delete an account and every dependent record, in one
    /// transaction.
    ///
    /// Safety rules, all enforced inside the transaction:
    ///
    /// * The target must exist (404 otherwise).
    /// * The caller cannot delete itself: pass the acting administrator's
    ///   id as `acting_admin_id` and a self-delete is refused with
    ///   [`AccountError::Validation`]. This protects the signed-in
    ///   administrator from an accidental self-deletion that would strand
    ///   their own session mid-request.
    /// * The last administrator cannot be deleted: the admin count is read
    ///   and the delete executed under one IMMEDIATE transaction, so two
    ///   racing deletions cannot both pass the check.
    /// * Sessions, password-reset tokens and the per-identity failure
    ///   counter are removed in the same transaction — no orphan rows, no
    ///   window where a deleted account's cookie still validates.
    /// * The audit trail survives: audit rows reference ids without foreign
    ///   keys precisely so deletion cannot erase history.
    ///
    /// Returns the identifier of the deleted account (for the audit entry
    /// the caller records next).
    pub fn delete_account(
        &self,
        id: i64,
        acting_admin_id: i64,
    ) -> Result<String, AccountError> {
        let mut conn = self.conn.lock();
        // IMMEDIATE: the admin-count read and the deletes must not be
        // interleaved with another writer (see set_account_role).
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let target: Option<(String, String)> = tx
            .query_row(
                "SELECT identifier, role FROM auth_account WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
                other => AccountError::Storage(other),
            })
            .map(Some)
            .or_else(|e| match e {
                AccountError::NotFound => Ok(None),
                other => Err(other),
            })?;
        let Some((identifier, role)) = target else {
            tx.rollback().ok();
            return Err(AccountError::NotFound);
        };

        if id == acting_admin_id {
            tx.rollback().ok();
            return Err(AccountError::AdminSelfDelete);
        }
        if role == ADMIN_ROLE {
            let admins: i64 = tx.query_row(
                "SELECT COUNT(*) FROM auth_account WHERE role = ?1",
                [ADMIN_ROLE],
                |r| r.get(0),
            )?;
            if admins <= 1 {
                tx.rollback().ok();
                return Err(AccountError::LastAdmin);
            }
        }

        // Dependent rows first (explicitly, so behaviour does not depend on
        // the FK pragma being enabled): sessions, reset tokens, failures.
        tx.execute("DELETE FROM sessions WHERE account_id = ?1", [id])?;
        tx.execute("DELETE FROM password_reset_tokens WHERE account_id = ?1", [id])?;
        tx.execute(
            "DELETE FROM signin_failures WHERE identity_key = ?1",
            [&identifier],
        )?;
        tx.execute("DELETE FROM auth_account WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(identifier)
    }

    /// Issue a single-use, expiring password-reset token for an account.
    ///
    /// Returns the RAW token exactly once — it exists in cleartext only in
    /// this return value and the email the caller sends; the database
    /// stores only its SHA-256 digest. Any previous unconsumed token for
    /// the account is superseded, so at most one live reset link exists per
    /// mailbox at any time.
    pub fn issue_password_reset(
        &self,
        account_id: i64,
        requested_by: i64,
    ) -> Result<IssuedPasswordReset, AccountError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // The account must exist and be identifiable for the email.
        let identifier: String = tx
            .query_row(
                "SELECT identifier FROM auth_account WHERE id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AccountError::NotFound,
                other => AccountError::Storage(other),
            })?;

        // Cooldown against the most recent reset for this account, whatever
        // its state — prevents an administrator (or a compromised admin
        // session) from mail-bombing a mailbox.
        let last_created: Option<i64> = tx
            .query_row(
                "SELECT MAX(created_at_ms) FROM password_reset_tokens WHERE account_id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(t) = last_created {
            let ready_at = t + PASSWORD_RESET_COOLDOWN.as_millis() as i64;
            if now < ready_at {
                tx.rollback().ok();
                return Err(AccountError::ResetCooldown {
                    retry_in_ms: ready_at - now,
                });
            }
        }

        // Supersede every live token for this account.
        tx.execute(
            "UPDATE password_reset_tokens
                SET consumed_at_ms = ?1
              WHERE account_id = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![now, account_id],
        )?;

        let raw = Self::new_token();
        let digest = token_digest(&raw);
        let expires = now + PASSWORD_RESET_TTL.as_millis() as i64;
        tx.execute(
            "INSERT INTO password_reset_tokens
                 (account_id, token_digest, requested_by, created_at_ms, expires_at_ms, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            rusqlite::params![account_id, digest, requested_by, now, expires],
        )?;

        // Opportunistic cleanup of long-spent tokens.
        let cutoff = now - PASSWORD_RESET_RETENTION_MS;
        let _ = tx.execute(
            "DELETE FROM password_reset_tokens
              WHERE (expires_at_ms < ?1)
                 OR (consumed_at_ms IS NOT NULL AND consumed_at_ms < ?1)",
            [cutoff],
        );

        tx.commit()?;
        Ok(IssuedPasswordReset {
            token: raw,
            account_id,
            identifier,
            expires_at_ms: expires,
        })
    }

    /// Validate a raw reset token WITHOUT consuming it (for the reset page
    /// to decide whether to show the form). Never reveals why a token is
    /// invalid — one message for unknown/expired/consumed/exhausted.
    pub fn peek_password_reset(&self, token: &str) -> Result<bool, AccountError> {
        if token.is_empty() || token.len() > 128 {
            return Ok(false);
        }
        let digest = token_digest(token);
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        let ok = conn
            .query_row(
                "SELECT expires_at_ms FROM password_reset_tokens
                  WHERE token_digest = ?1 AND consumed_at_ms IS NULL AND expires_at_ms > ?2",
                rusqlite::params![digest, now],
                |r| r.get::<_, i64>(0),
            )
            .is_ok();
        Ok(ok)
    }

    /// Consume every live (unconsumed, unexpired) reset token for an
    /// account. Used when a delivery fails so an undelivered link can
    /// never later be used; also a safe manual kill-switch.
    pub fn revoke_live_reset_tokens(&self, account_id: i64) -> Result<(), AccountError> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE password_reset_tokens
                SET consumed_at_ms = ?1
              WHERE account_id = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![now, account_id],
        )?;
        Ok(())
    }

    /// Complete a password reset: consume the token and set the new
    /// password, atomically.
    ///
    /// Guarantees:
    /// * The token is single-use: two racing callers cannot both consume
    ///   it (the conditional UPDATE reports rows affected).
    /// * The new password is validated (8..=256 chars) and hashed with
    ///   Argon2id and a fresh random salt BEFORE any lock is taken;
    ///   plaintext never touches storage or logs.
    /// * Completing a reset revokes every existing session for the
    ///   account, so a stolen cookie cannot outlive a recovered password.
    /// * `last_password_change_ms` is stamped in the same transaction.
    ///
    /// Returns the account id + identifier so the caller can audit it.
    pub fn complete_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<(i64, String), AccountError> {
        if new_password.len() < MIN_PASSWORD_LEN {
            return Err(AccountError::PasswordValidation(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if new_password.len() > 256 {
            return Err(AccountError::PasswordValidation(
                "password is too long".into(),
            ));
        }
        if token.is_empty() || token.len() > 128 {
            return Err(AccountError::ResetInvalid);
        }

        // Hash BEFORE the transaction: Argon2id is deliberately slow and
        // must not hold the connection lock.
        let salt = random_salt();
        let hash = Argon2::default()
            .hash_password(new_password.as_bytes(), &salt)
            .map_err(|_| AccountError::Storage(rusqlite::Error::InvalidQuery))?
            .to_string();

        let digest = token_digest(token);
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Consume the token exactly once. The attempt counter guards
        // against brute-forcing the token itself through this endpoint.
        let row: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, attempts FROM password_reset_tokens
                  WHERE token_digest = ?1 AND consumed_at_ms IS NULL AND expires_at_ms > ?2",
                rusqlite::params![digest, now],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((reset_id, attempts)) = row else {
            // Unknown / expired / already used. One generic error for all.
            tx.commit().ok();
            return Err(AccountError::ResetInvalid);
        };
        if attempts >= PASSWORD_RESET_MAX_ATTEMPTS as i64 {
            // Exhausted: kill the token and refuse indistinguishably.
            tx.execute(
                "UPDATE password_reset_tokens SET consumed_at_ms = ?1 WHERE id = ?2",
                rusqlite::params![now, reset_id],
            )?;
            tx.commit().ok();
            return Err(AccountError::ResetInvalid);
        }
        let consumed = tx.execute(
            "UPDATE password_reset_tokens
                SET consumed_at_ms = ?1, attempts = attempts + 1
              WHERE id = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![now, reset_id],
        )?;
        if consumed == 0 {
            // Raced with another consumer that won.
            tx.commit().ok();
            return Err(AccountError::ResetInvalid);
        }

        let account: (i64, String) = tx
            .query_row(
                "SELECT a.id, a.identifier
                   FROM password_reset_tokens r
                   JOIN auth_account a ON a.id = r.account_id
                  WHERE r.id = ?1",
                [reset_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => AccountError::ResetInvalid,
                other => AccountError::Storage(other),
            })?;

        // Set the new hash and stamp the change time.
        tx.execute(
            "UPDATE auth_account
                SET password_hash = ?1, last_password_change_ms = ?2
              WHERE id = ?3",
            rusqlite::params![hash, now, account.0],
        )?;
        // A recovered password invalidates every prior session.
        tx.execute("DELETE FROM sessions WHERE account_id = ?1", [account.0])?;
        tx.commit()?;
        Ok(account)
    }

    // ------------------------------------------------------------------
    // Audit log
    // ------------------------------------------------------------------

    /// Record one audit event. Best-effort by design: an audit write must
    /// never turn a successful administrative action into a 500, and the
    /// caller decides whether a FAILED audit write should abort (the
    /// destructive paths use [`AuthStore::record_audit_checked`] instead).
    ///
    /// `actor` is `(id, email)` of the administrator who acted, or `None`
    /// for system events (sign-ins). `subject` is the affected account.
    /// `detail` is a short human-readable note; it must never contain a
    /// password, hash or token.
    pub fn record_audit(
        &self,
        kind: &str,
        actor: Option<(i64, &str)>,
        subject: Option<(i64, &str)>,
        detail: &str,
    ) {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        let _ = conn.execute(
            "INSERT INTO audit_log
                 (at_ms, kind, actor_id, actor_email, subject_id, subject_email, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                now,
                kind,
                actor.map(|a| a.0),
                actor.map(|a| a.1.to_string()),
                subject.map(|s| s.0),
                subject.map(|s| s.1.to_string()),
                detail
            ],
        );
    }

    /// Like [`AuthStore::record_audit`] but returns an error on failure, for
    /// callers that must not proceed silently when the audit trail cannot
    /// be written.
    pub fn record_audit_checked(
        &self,
        kind: &str,
        actor: Option<(i64, &str)>,
        subject: Option<(i64, &str)>,
        detail: &str,
    ) -> Result<(), AccountError> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO audit_log
                 (at_ms, kind, actor_id, actor_email, subject_id, subject_email, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                now,
                kind,
                actor.map(|a| a.0),
                actor.map(|a| a.1.to_string()),
                subject.map(|s| s.0),
                subject.map(|s| s.1.to_string()),
                detail
            ],
        )?;
        Ok(())
    }

    /// List audit events for one account (as subject), newest first, with
    /// bounded pagination. Returns the page plus the total matching count.
    ///
    /// Deleted accounts keep their history: the query matches on
    /// `subject_id`, which has no foreign key and therefore survives
    /// deletion.
    pub fn list_account_activity(
        &self,
        subject_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AuditEntry>, i64), AccountError> {
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE subject_id = ?1",
            [subject_id],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, at_ms, kind, actor_email, subject_email, detail
               FROM audit_log
              WHERE subject_id = ?1
              ORDER BY at_ms DESC, id DESC
              LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![subject_id, limit, offset], |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    at_ms: r.get(1)?,
                    kind: r.get(2)?,
                    actor_email: r.get(3)?,
                    subject_email: r.get(4)?,
                    detail: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    /// Bulk-create `n` pending accounts WITHOUT hashing passwords.
    ///
    /// **Test-only.** [`AuthStore::create_pending_user`] runs Argon2id per
    /// account, which is correct for production but makes a 500-row
    /// pagination/truncation test take minutes. This helper inserts rows
    /// with a placeholder hash (never verified by anything these tests
    /// exercise) inside one transaction. Production code must use
    /// [`AuthStore::create_pending_user`].
    #[cfg(test)]
    pub fn create_pending_users_unhashed(
        &self,
        prefix: &str,
        n: usize,
    ) -> Result<Vec<i64>, AccountError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            let identifier = format!("{prefix}{i}@soket.ai");
            // The placeholder must satisfy the NOT NULL constraint and be
            // obviously non-functional if ever accidentally used to verify.
            let placeholder = "!(test-placeholder-not-a-real-hash)";
            tx.execute(
                "INSERT INTO auth_account
                     (identifier, password_hash, role, status, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    identifier,
                    placeholder,
                    USER_ROLE,
                    STATUS_PENDING,
                    // Distinct, increasing timestamps so ordering is stable
                    // and matches the oldest-first contract under test.
                    1_700_000_000_000i64 + i as i64,
                ],
            )?;
            ids.push(tx.last_insert_rowid());
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Count of administrator accounts (tests/diagnostics). Must always
    /// be exactly one after bootstrap or migration.
    pub fn admin_count(&self) -> Result<i64, AuthError> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM auth_account WHERE role = ?1",
            [ADMIN_ROLE],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Count of live sessions (diagnostics/tests only).
    pub fn session_count(&self) -> Result<i64, AuthError> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp_millis();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE expires_at_ms >= ?1",
            [now],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// The stored password hash of the sole administrator (tests only —
    /// never sent over HTTP).
    #[cfg(test)]
    pub fn stored_password_hash(&self) -> String {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT password_hash FROM auth_account WHERE role = ?1",
            [ADMIN_ROLE],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// Raw connection access for test assertions that need SQL. Tests only:
    /// production code must go through the typed storage methods so every
    /// query stays parameterized and reviewed.
    #[cfg(test)]
    pub fn conn_for_tests(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }

    /// Mint a session for `account_id` WITHOUT any credential or status
    /// check. **Test-only**: it exists so tests can construct the "session
    /// created while the account was still approved, then the account was
    /// suspended" scenario, which the real sign-in path makes unreachable
    /// (sign_in refuses non-approved accounts). Production code must use
    /// [`AuthStore::sign_in`].
    #[cfg(test)]
    pub fn debug_mint_session_for(&self, account_id: i64) -> Result<String, AuthError> {
        let token = Self::new_token();
        let digest = token_digest(&token);
        let now = chrono::Utc::now().timestamp_millis();
        let expires = now + SESSION_TTL.as_millis() as i64;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions
                 (token_digest, account_id, created_at_ms, expires_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?3)",
            rusqlite::params![digest, account_id, now, expires],
        )?;
        Ok(token)
    }

    /// Reject the call when attempts within the window exceed the limit.
    fn throttle_signin(&self) -> Result<(), AuthError> {
        let mut attempts = self.signin_attempts.lock();
        let now = Instant::now();
        attempts.retain(|t| now.duration_since(*t) < SIGNIN_WINDOW);
        if attempts.len() as u32 >= SIGNIN_ATTEMPT_LIMIT {
            return Err(AuthError::RateLimited);
        }
        attempts.push(now);
        Ok(())
    }

    /// 256 bits of OS entropy, URL-safe base64 (no padding).
    fn new_token() -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .expect("OS entropy source unavailable; refusing to mint weak tokens");
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
    }
}

/// SHA-256 of the raw token; what the database stores.
fn token_digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Extract the raw session token from a Cookie header value.
pub fn session_token_from_cookie_header(header: &str) -> Option<&str> {
    for pair in header.split(';') {
        let mut it = pair.trim().splitn(2, '=');
        let name = it.next()?.trim();
        if name == SESSION_COOKIE {
            return it.next().map(|v| v.trim()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// Build a Set-Cookie header value for a session token.
///
/// `secure` is false on plain-HTTP localhost (the current deployment model:
/// loopback-only). If the app is ever served over HTTPS, pass true.
pub fn session_set_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_TTL.as_secs()
    ) + if secure { "; Secure" } else { "" }
}

/// Build a Clear-Cookie header value for sign-out.
pub fn session_clear_cookie(secure: bool) -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
        + if secure { "; Secure" } else { "" }
}

/// Shared auth state handed to routers.
pub type SharedAuthStore = Arc<AuthStore>;

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (AuthStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (AuthStore::open(dir.path().join("auth.db")).unwrap(), dir)
    }

    #[test]
    fn signup_creates_owner_and_refuses_second() {
        let (s, _d) = store();
        assert!(s.is_first_run().unwrap());
        s.sign_up("owner@local", "correct horse battery").unwrap();
        assert!(!s.is_first_run().unwrap());
        assert!(matches!(
            s.sign_up("other@local", "another password"),
            Err(AuthError::OwnerExists)
        ));
    }

    #[test]
    fn passwords_are_never_stored_plaintext() {
        let (s, _d) = store();
        let pw = "a very secret passphrase";
        s.sign_up("owner", pw).unwrap();
        let hash = s.stored_password_hash();
        assert_ne!(hash, pw);
        assert!(hash.starts_with("$argon2id$"), "must be Argon2id: {hash}");
        assert!(!hash.contains(pw));
    }

    #[test]
    fn signin_rejects_wrong_password_and_unknown_user_identically() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        let wrong_pw = s.sign_in("owner", "wrong password!");
        let unknown_user = s.sign_in("ghost", "correct horse battery");
        assert!(matches!(wrong_pw, Err(AuthError::InvalidCredentials)));
        assert!(matches!(unknown_user, Err(AuthError::InvalidCredentials)));
        // Identical error text — no account enumeration.
        assert_eq!(
            wrong_pw.unwrap_err().to_string(),
            unknown_user.unwrap_err().to_string()
        );
    }

    #[test]
    fn session_round_trip_and_signout() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        let token = s.sign_in("owner", "correct horse battery").unwrap();
        let info = s.validate_session(&token).unwrap();
        assert_eq!(info.identifier, "owner");
        assert_eq!(s.session_count().unwrap(), 1);

        s.sign_out(&token).unwrap();
        assert!(s.validate_session(&token).is_none());
        assert_eq!(s.session_count().unwrap(), 0);
    }

    #[test]
    fn invalid_and_garbage_tokens_rejected() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        assert!(s.validate_session("").is_none());
        assert!(s.validate_session("not-a-real-token").is_none());
        assert!(s.validate_session(&"A".repeat(43)).is_none());
    }

    #[test]
    fn signin_is_rate_limited() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        // Exhaust the attempt budget with bad passwords.
        for _ in 0..SIGNIN_ATTEMPT_LIMIT {
            let _ = s.sign_in("owner", "wrong");
        }
        // Even the CORRECT password is now throttled.
        assert!(matches!(
            s.sign_in("owner", "correct horse battery"),
            Err(AuthError::RateLimited)
        ));
    }

    #[test]
    fn validation_rejects_weak_input() {
        let (s, _d) = store();
        assert!(matches!(
            s.sign_up("", "long enough password"),
            Err(AuthError::Validation(_))
        ));
        assert!(matches!(
            s.sign_up("bad chars!", "long enough password"),
            Err(AuthError::Validation(_))
        ));
        assert!(matches!(
            s.sign_up("ok", "short"),
            Err(AuthError::Validation(_))
        ));
    }

    #[test]
    fn cookie_parsing_round_trips() {
        let header = "other=x; soket_session=abc123; more=y";
        assert_eq!(session_token_from_cookie_header(header), Some("abc123"));
        assert_eq!(session_token_from_cookie_header("nope=1"), None);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        AuthStore::open(&path).unwrap();
        AuthStore::open(&path).unwrap(); // second open must not fail
    }

    // ------------------------------------------------------------------
    // Phase 4A: consecutive sign-in failure tracking
    // ------------------------------------------------------------------

    #[test]
    fn failure_tracking_normalizes_identity() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        // Wrong password three times with casing/whitespace variants.
        for id in ["owner", "OWNER", "  Owner  ", "oWnEr"] {
            let _ = s.sign_in(id, "wrong password");
        }
        assert_eq!(s.consecutive_signin_failures("owner"), 4);
        assert_eq!(s.consecutive_signin_failures("  OWNER "), 4);
        // One row — the variants did not fork the counter.
        assert_eq!(s.tracked_failure_identities(), 1);
    }

    #[test]
    fn failures_increment_and_reset_on_success_only() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        for i in 1..=3 {
            let _ = s.sign_in("owner", "wrong password");
            assert_eq!(
                s.consecutive_signin_failures("owner"),
                i,
                "consecutive count after {i} failures"
            );
        }
        // Reading is side-effect free: a "reload" must not reset anything.
        let _ = s.consecutive_signin_failures("owner");
        assert_eq!(s.consecutive_signin_failures("owner"), 3);

        // A FAILED attempt with the right identifier but wrong password
        // still increments; only success resets.
        let _ = s.sign_in("owner", "still wrong");
        assert_eq!(s.consecutive_signin_failures("owner"), 4);

        // Successful authentication resets to zero.
        s.sign_in("owner", "correct horse battery").unwrap();
        assert_eq!(s.consecutive_signin_failures("owner"), 0);
        assert_eq!(s.tracked_failure_identities(), 0);
    }

    #[test]
    fn unknown_identifier_failures_are_tracked_but_harmless() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        // Spraying unknown identifiers creates rows (bounded by retention)
        // but never affects the real owner's counter…
        let _ = s.sign_in("ghost", "whatever");
        let _ = s.sign_in("GHOST", "whatever");
        assert_eq!(s.consecutive_signin_failures("ghost"), 2);
        assert_eq!(s.consecutive_signin_failures("owner"), 0);
        // …and never blocks the owner from signing in.
        assert!(s.sign_in("owner", "correct horse battery").is_ok());
    }

    #[test]
    fn stale_failure_records_are_cleaned_up() {
        let (s, _d) = store();
        // Seed a record whose last failure is older than the retention
        // window, plus one fresh record.
        let now = chrono::Utc::now().timestamp_millis();
        let stale_cutoff = now - SIGNIN_FAILURE_RETENTION.as_millis() as i64;
        let conn = s.conn.lock();
        conn.execute(
            "INSERT INTO signin_failures VALUES ('stale@example.com', 7, ?1, ?1)",
            [stale_cutoff - 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO signin_failures VALUES ('fresh@example.com', 2, ?1, ?1)",
            [now],
        )
        .unwrap();
        drop(conn);
        // Any subsequent failure triggers opportunistic cleanup.
        let _ = s.sign_in("owner", "nope"); // owner doesn't exist; still records
        let remaining = s.tracked_failure_identities();
        assert_eq!(remaining, 2, "stale row dropped, fresh + trigger kept");
        assert_eq!(s.consecutive_signin_failures("stale@example.com"), 0);
        assert_eq!(s.consecutive_signin_failures("fresh@example.com"), 2);
    }

    #[test]
    fn failure_tracking_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        {
            let s = AuthStore::open(&path).unwrap();
            s.sign_up("owner", "correct horse battery").unwrap();
            for _ in 0..3 {
                let _ = s.sign_in("owner", "nope");
            }
            assert_eq!(s.consecutive_signin_failures("owner"), 3);
        }
        // Reopen: persistence means restarts/reloads do not erase protection.
        let s = AuthStore::open(&path).unwrap();
        assert_eq!(s.consecutive_signin_failures("owner"), 3);
        // …and the next failure continues the streak rather than starting over.
        let _ = s.sign_in("owner", "nope");
        assert_eq!(s.consecutive_signin_failures("owner"), 4);
    }

    #[test]
    fn failure_table_stores_no_credentials() {
        let (s, _d) = store();
        s.sign_up("owner", "correct horse battery").unwrap();
        let _ = s.sign_in("owner", "super-secret-wrong-password");
        let conn = s.conn.lock();
        let mut stmt = conn
            .prepare("SELECT identity_key, consecutive FROM signin_failures")
            .unwrap();
        let rows: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(stmt);
        drop(conn);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "owner");
        assert_eq!(rows[0].1, 1);
        // The wrong password and the real hash appear nowhere in the table.
        let dump = {
            let conn = s.conn.lock();
            conn.query_row(
                "SELECT group_concat(identity_key || ':' || consecutive) FROM signin_failures",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_default()
        };
        assert!(!dump.contains("super-secret"), "password leaked: {dump}");
        assert!(
            !dump.contains("$argon2"),
            "hash material in tracking table: {dump}"
        );
    }

    // ------------------------------------------------------------------
    // Phase 4A: registration storage primitive
    // ------------------------------------------------------------------

    #[test]
    fn create_owner_with_session_returns_valid_token() {
        let (s, _d) = store();
        let token = s
            .create_owner_with_session("new-owner@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(!token.is_empty());
        assert!(!s.is_first_run().unwrap());
        // Session minted in the same transaction is immediately valid.
        let info = s.validate_session(&token).unwrap();
        assert_eq!(info.identifier, "new-owner@soket.ai");
        // Password was hashed with the same Argon2id scheme as sign_up.
        assert!(s.stored_password_hash().starts_with("$argon2id$"));
        // Regular sign-in works with the created credentials.
        assert!(s
            .sign_in("new-owner@soket.ai", "a strong passphrase")
            .is_ok());
    }

    #[test]
    fn create_owner_refuses_when_owner_exists() {
        let (s, _d) = store();
        s.sign_up("original-owner", "correct horse battery")
            .unwrap();
        let before = s.stored_password_hash();
        // Second creation attempt fails without touching the existing row.
        assert!(matches!(
            s.create_owner_with_session("attacker@soket.ai", "some password"),
            Err(AuthError::OwnerExists)
        ));
        assert_eq!(
            s.stored_password_hash(),
            before,
            "owner hash must not change"
        );
        // Still exactly one owner, still signed in by the old password.
        assert!(!s.is_first_run().unwrap());
        assert!(s.sign_in("original-owner", "correct horse battery").is_ok());
    }

    #[test]
    fn create_owner_validates_input_like_signup() {
        let (s, _d) = store();
        assert!(matches!(
            s.create_owner_with_session("", "long enough password"),
            Err(AuthError::Validation(_))
        ));
        assert!(matches!(
            s.create_owner_with_session("bad chars!", "long enough password"),
            Err(AuthError::Validation(_))
        ));
        assert!(matches!(
            s.create_owner_with_session("ok", "short"),
            Err(AuthError::Validation(_))
        ));
        // Nothing was created by the failed attempts.
        assert!(s.is_first_run().unwrap());
    }

    #[test]
    fn concurrent_owner_creation_yields_exactly_one_owner() {
        let (s, _d) = store();
        let s = std::sync::Arc::new(s);
        let mut handles = Vec::new();
        for i in 0..8 {
            let s = s.clone();
            handles.push(std::thread::spawn(move || {
                (
                    i,
                    s.create_owner_with_session(
                        &format!("racer-{i}@soket.ai"),
                        "correct horse battery",
                    ),
                )
            }));
        }
        let mut winners = 0;
        for (i, res) in handles.into_iter().map(|h| h.join().unwrap()) {
            match res {
                Ok(_) => winners += 1,
                Err(AuthError::OwnerExists) => {}
                Err(e) => panic!("racer {i} failed unexpectedly: {e:?}"),
            }
        }
        assert_eq!(winners, 1, "exactly one concurrent creator may win");
        assert!(!s.is_first_run().unwrap());
        // Winner's session is live; losers created nothing.
        assert_eq!(s.session_count().unwrap(), 1);
    }

    #[test]
    fn existing_owner_sessions_untouched_by_registration_primitive() {
        let (s, _d) = store();
        s.sign_up("himanshusudan771@gmail.com", "correct horse battery")
            .unwrap();
        let token = s
            .sign_in("himanshusudan771@gmail.com", "correct horse battery")
            .unwrap();
        // A later registration attempt must fail…
        assert!(matches!(
            s.create_owner_with_session("someone@soket.ai", "another password"),
            Err(AuthError::OwnerExists)
        ));
        // …and the existing owner's session stays perfectly valid.
        let info = s.validate_session(&token).unwrap();
        assert_eq!(info.identifier, "himanshusudan771@gmail.com");
        assert!(s
            .sign_in("himanshusudan771@gmail.com", "correct horse battery")
            .is_ok());
    }

    // ==================================================================
    // Phase 4A: multi-user schema migration and account storage
    //
    // Every test here builds its own throwaway database inside a
    // tempfile::TempDir. The real ~/.loop/dashboard/auth.db is never
    // opened, read or written by any test in this module.
    // ==================================================================

    /// Identifier of the grandfathered administrator.
    const LEGACY_ADMIN: &str = "himanshusudan771@gmail.com";

    /// Build a realistic LEGACY auth database (pre-Phase-4A shape): one
    /// owner row with an Argon2id hash and two live sessions, exactly as
    /// the previous release would have left it.
    fn legacy_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("auth.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(LEGACY_AUTH_SCHEMA).unwrap();
        let salt = random_salt();
        let hash = Argon2::default()
            .hash_password(b"correct horse battery".as_ref(), &salt)
            .unwrap()
            .to_string();
        conn.execute(
            "INSERT INTO owner_account (id, identifier, password_hash, created_at_ms)
             VALUES (1, ?1, ?2, ?3)",
            rusqlite::params![LEGACY_ADMIN, hash, 1_700_000_000_000i64],
        )
        .unwrap();
        for digest in [[1u8; 32], [2u8; 32]] {
            conn.execute(
                "INSERT INTO sessions
                     (token_digest, created_at_ms, expires_at_ms, last_seen_at_ms)
                 VALUES (?1, 1000, ?2, 1500)",
                rusqlite::params![
                    digest.to_vec(),
                    chrono::Utc::now().timestamp_millis() + 60_000
                ],
            )
            .unwrap();
        }
        path
    }

    /// Snapshot of a table's full contents, for before/after comparisons.
    fn dump_table(conn: &Connection, table: &str, order_by: &str) -> Vec<Vec<String>> {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order_by}"))
            .unwrap();
        let ncols = stmt.column_count();
        let mut rows = Vec::new();
        let mut result = stmt.query([]).unwrap();
        while let Some(row) = result.next().unwrap() {
            let mut cells = Vec::with_capacity(ncols);
            for i in 0..ncols {
                let v = match row.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Null => "NULL".to_string(),
                    rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                    rusqlite::types::ValueRef::Real(f) => f.to_string(),
                    rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                    rusqlite::types::ValueRef::Blob(b) => hex(b),
                };
                cells.push(v);
            }
            rows.push(cells);
        }
        rows
    }

    /// Lowercase hex (test helper).
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn fresh_database_initializes_multi_user_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        AuthStore::open(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for expected in ["auth_account", "sessions", "signin_failures"] {
            assert!(tables.iter().any(|t| t == expected), "missing {expected}");
        }
        assert!(
            !tables.iter().any(|t| t == "owner_account"),
            "legacy table must not be created on a fresh database"
        );
        // No administrator until bootstrap runs.
        let store = AuthStore::open(&path).unwrap();
        assert!(store.is_first_run().unwrap());
        assert_eq!(store.admin_count().unwrap(), 0);
    }

    #[test]
    fn migration_preserves_legacy_admin_record_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_db(&dir);

        // Snapshot the pre-migration data we must preserve.
        let (before_hash, before_created, before_id): (String, i64, i64) = {
            let conn = Connection::open(&path).unwrap();
            conn.query_row(
                "SELECT password_hash, created_at_ms, id FROM owner_account",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        let before_sessions = {
            let conn = Connection::open(&path).unwrap();
            dump_table(&conn, "sessions", "token_digest")
        };

        // Run the migration by opening the store.
        let store = AuthStore::open(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        // Legacy table is gone, replaced by auth_account.
        let owner_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='owner_account'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            owner_table, 0,
            "owner_account must be dropped after migration"
        );

        // Exactly one account: the grandfathered administrator, approved.
        let accounts = dump_table(&conn, "auth_account", "id");
        assert_eq!(accounts.len(), 1, "exactly one account after migration");
        assert_eq!(
            accounts[0][0],
            before_id.to_string(),
            "account id preserved"
        );
        assert_eq!(accounts[0][1], LEGACY_ADMIN, "identifier preserved");
        assert_eq!(
            accounts[0][2], before_hash,
            "Argon2id password hash preserved"
        );
        assert_eq!(accounts[0][3], ADMIN_ROLE, "promoted to administrator");
        assert_eq!(accounts[0][4], STATUS_APPROVED, "administrator is approved");
        assert_eq!(
            accounts[0][5],
            before_created.to_string(),
            "created_at_ms preserved"
        );

        // Sessions survived with identical digests/timestamps and are now
        // attached to the preserved account id. Note the column layouts
        // differ: legacy `sessions` has no account_id, so its timestamps
        // sit one index earlier than in the migrated table.
        let after_sessions = dump_table(&conn, "sessions", "token_digest");
        assert_eq!(after_sessions.len(), before_sessions.len());
        for (after, before) in after_sessions.iter().zip(before_sessions.iter()) {
            assert_eq!(after[0], before[0], "token digest preserved");
            assert_eq!(after[2], before[1], "created_at_ms preserved");
            assert_eq!(after[3], before[2], "expires_at_ms preserved");
            assert_eq!(after[4], before[3], "last_seen_at_ms preserved");
            assert_eq!(after[1], before_id.to_string(), "session bound to admin id");
        }

        // The stored hash still verifies the original password.
        assert!(store.sign_in(LEGACY_ADMIN, "correct horse battery").is_ok());
        assert_eq!(store.admin_count().unwrap(), 1);
        assert!(!store.is_first_run().unwrap());
    }

    #[test]
    fn migration_is_idempotent_and_reopenable() {
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_db(&dir);
        let first = AuthStore::open(&path).unwrap();
        let sessions_before = first.session_count().unwrap();
        drop(first);
        // Reopening must be a no-op that preserves everything again.
        let second = AuthStore::open(&path).unwrap();
        assert_eq!(second.admin_count().unwrap(), 1);
        assert_eq!(second.session_count().unwrap(), sessions_before);
        let rec = second.find_account_by_identifier(LEGACY_ADMIN).unwrap();
        assert_eq!(rec.role, ADMIN_ROLE);
        assert_eq!(rec.status, STATUS_APPROVED);
        // And a third open changes nothing either.
        drop(second);
        let third = AuthStore::open(&path).unwrap();
        assert_eq!(third.admin_count().unwrap(), 1);
    }

    #[test]
    fn migration_failure_leaves_legacy_database_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_db(&dir);
        // Sabotage: a SECOND owner row violates the migration's invariant,
        // so the copy aborts and the transaction must roll back entirely.
        // The legacy schema's CHECK (id = 1) would reject this insert under
        // normal rules, so the sabotage suspends CHECK enforcement for this
        // one statement — modelling a hand-edited or corrupted legacy
        // database that genuinely holds two owner rows.
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            conn.execute(
                "INSERT INTO owner_account (id, identifier, password_hash, created_at_ms)
                 VALUES (2, 'intruder@gmail.com', '$argon2id$fake', 1)",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        }
        let result = AuthStore::open(&path);
        assert!(
            result.is_err(),
            "two legacy owner rows cannot be migrated safely"
        );
        // The legacy schema and BOTH rows are still there, untouched.
        let conn = Connection::open(&path).unwrap();
        let owner_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM owner_account", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            owner_rows, 2,
            "legacy rows preserved after failed migration"
        );
        let sessions = dump_table(&conn, "sessions", "token_digest");
        assert_eq!(sessions.len(), 2, "legacy sessions preserved");
        let auth_account: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='auth_account'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            auth_account, 0,
            "no half-migrated table may survive a rollback"
        );
    }

    #[test]
    fn multiple_unique_user_accounts_are_supported() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        for email in ["alice@soket.ai", "bob.smith@soket.ai", "carol+tag@soket.ai"] {
            let id = s.create_pending_user(email, "a strong passphrase").unwrap();
            assert!(id > 0, "{email} got a row id");
        }
        // All three coexist alongside the administrator.
        let total: i64 = {
            let conn = s.conn.lock();
            conn.query_row("SELECT COUNT(*) FROM auth_account", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(total, 4, "admin + three users");
        assert_eq!(s.admin_count().unwrap(), 1);
    }

    #[test]
    fn duplicate_normalized_emails_are_rejected() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        s.create_pending_user("dave@soket.ai", "a strong passphrase")
            .unwrap();
        // Exact duplicate.
        assert!(matches!(
            s.create_pending_user("dave@soket.ai", "another passphrase"),
            Err(AccountError::DuplicateEmail)
        ));
        // Casing/whitespace variants normalize to the same identity.
        for variant in ["DAVE@soket.ai", "  Dave@Soket.AI ", "dave@SOKET.ai"] {
            assert!(
                matches!(
                    s.create_pending_user(variant, "another passphrase"),
                    Err(AccountError::DuplicateEmail)
                ),
                "variant {variant:?} must collide with dave@soket.ai"
            );
        }
        // Lookups find the same row regardless of casing.
        let rec = s.find_account_by_identifier(" DAVE@Soket.AI ").unwrap();
        assert_eq!(rec.identifier, "dave@soket.ai");
        // Still only one row for that address.
        let n: i64 = {
            let conn = s.conn.lock();
            conn.query_row(
                "SELECT COUNT(*) FROM auth_account WHERE identifier = 'dave@soket.ai'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(n, 1);
    }

    #[test]
    fn concurrent_duplicate_registrations_yield_one_row() {
        let (s, _d) = store();
        let s = std::sync::Arc::new(s);
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = s.clone();
            handles.push(std::thread::spawn(move || {
                s.create_pending_user("racer@soket.ai", "correct horse battery")
            }));
        }
        let mut ok = 0;
        for h in handles {
            match h.join().unwrap() {
                Ok(_) => ok += 1,
                Err(AccountError::DuplicateEmail) => {}
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one racer may create the account");
        let n: i64 = {
            let conn = s.conn.lock();
            conn.query_row(
                "SELECT COUNT(*) FROM auth_account WHERE identifier = 'racer@soket.ai'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(n, 1);
    }

    #[test]
    fn pending_signin_does_not_burn_the_failure_counter() {
        // A pending user retrying while awaiting approval must not be
        // rate-limited for it: their credentials are correct, so a pending
        // attempt never increments the per-identity failure counter.
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        s.create_pending_user("patient@soket.ai", "a strong passphrase")
            .unwrap();
        // Stay inside the GLOBAL sliding-window budget (8 per 15 min): the
        // point here is the per-identity counter, not the global throttle.
        for _ in 0..7 {
            assert!(matches!(
                s.sign_in("patient@soket.ai", "a strong passphrase"),
                Err(AuthError::AccountPending)
            ));
        }
        // No failure was ever recorded for that identity.
        assert_eq!(s.consecutive_signin_failures("patient@soket.ai"), 0);
        // And once approved, sign-in succeeds immediately — no lockout.
        let id = s.find_account_by_identifier("patient@soket.ai").unwrap().id;
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        assert!(s.sign_in("patient@soket.ai", "a strong passphrase").is_ok());
    }

    #[test]
    fn non_approved_statuses_cannot_sign_in_or_keep_sessions() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let id = s
            .create_pending_user("eve@soket.ai", "a strong passphrase")
            .unwrap();

        // Pending: correct credentials are refused with the explicit
        // pending-approval message (the password verified, so revealing
        // "awaiting approval" leaks nothing an attacker could use).
        assert!(matches!(
            s.sign_in("eve@soket.ai", "a strong passphrase"),
            Err(AuthError::AccountPending)
        ));
        // No session was minted for the pending account.
        assert_eq!(s.session_count().unwrap(), 0);
        // Approved: sign-in works.
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        let token = s.sign_in("eve@soket.ai", "a strong passphrase").unwrap();
        assert!(s.validate_session(&token).is_some());
        // Suspended: existing session dies immediately, sign-in refused.
        s.set_account_status(id, STATUS_SUSPENDED).unwrap();
        assert!(
            s.validate_session(&token).is_none(),
            "suspension must revoke live sessions"
        );
        // Correct credentials on a suspended account now yield the
        // explicit suspended message (User Management phase): the password
        // verified, so telling the real owner why access is refused leaks
        // nothing an attacker with wrong credentials could use — they still
        // get the generic InvalidCredentials below.
        assert!(matches!(
            s.sign_in("eve@soket.ai", "a strong passphrase"),
            Err(AuthError::AccountSuspended)
        ));
        // WRONG credentials on the same suspended account stay generic:
        // no status information for a caller who cannot prove ownership.
        assert!(matches!(
            s.sign_in("eve@soket.ai", "wrong password entirely"),
            Err(AuthError::InvalidCredentials)
        ));
        // Reinstated: access returns.
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        let token2 = s.sign_in("eve@soket.ai", "a strong passphrase").unwrap();
        assert!(s.validate_session(&token2).is_some());
        // Rejected (via pending): refused again.
        s.set_account_status(id, STATUS_SUSPENDED).unwrap();
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        s.set_account_status(id, STATUS_SUSPENDED).unwrap();
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        s.set_account_status(id, STATUS_SUSPENDED).unwrap();
        assert!(matches!(
            s.sign_in("eve@soket.ai", "a strong passphrase"),
            Err(AuthError::AccountSuspended)
        ));
    }

    #[test]
    fn status_transitions_are_validated() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let id = s
            .create_pending_user("frank@soket.ai", "a strong passphrase")
            .unwrap();

        // Supported transitions from pending.
        s.set_account_status(id, STATUS_REJECTED).unwrap();
        assert_eq!(s.account_role_and_status(id).unwrap().1, STATUS_REJECTED);
        s.set_account_status(id, STATUS_PENDING).unwrap();
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        assert_eq!(s.account_role_and_status(id).unwrap().1, STATUS_APPROVED);

        // Invalid transitions are refused and change nothing.
        assert!(matches!(
            s.set_account_status(id, STATUS_PENDING),
            Err(AccountError::InvalidTransition { .. })
        ));
        assert!(matches!(
            s.set_account_status(id, STATUS_REJECTED),
            Err(AccountError::InvalidTransition { .. })
        ));
        assert_eq!(s.account_role_and_status(id).unwrap().1, STATUS_APPROVED);

        // Unknown statuses are rejected outright.
        assert!(matches!(
            s.set_account_status(id, "superuser"),
            Err(AccountError::Validation(_))
        ));
        assert!(matches!(
            s.set_account_status(id, ""),
            Err(AccountError::Validation(_))
        ));

        // Unknown ids report NotFound.
        assert!(matches!(
            s.set_account_status(9999, STATUS_APPROVED),
            Err(AccountError::NotFound)
        ));

        // The administrator cannot be demoted through this API.
        let admin = s.find_account_by_identifier(LEGACY_ADMIN).unwrap();
        for target in [STATUS_SUSPENDED, STATUS_REJECTED, STATUS_PENDING] {
            assert!(
                matches!(
                    s.set_account_status(admin.id, target),
                    Err(AccountError::InvalidTransition { .. })
                ),
                "admin -> {target} must be refused"
            );
        }
        assert_eq!(
            s.account_role_and_status(admin.id).unwrap().1,
            STATUS_APPROVED
        );
    }

    #[test]
    fn multiple_administrators_may_exist_but_registration_never_creates_one() {
        // Phase 7 changed the invariant: administrators may promote other
        // accounts, so several admin rows are legal. What must NEVER happen
        // is registration (or any storage path other than an explicit,
        // guarded promotion) minting an administrator.
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        // New registrations are always ordinary pending users.
        let id = s
            .create_pending_user("grace@soket.ai", "a strong passphrase")
            .unwrap();
        let (role, status) = s.account_role_and_status(id).unwrap();
        assert_eq!(role, USER_ROLE);
        assert_eq!(status, STATUS_PENDING);
        // Even after approval they stay ordinary users.
        s.set_account_status(id, STATUS_APPROVED).unwrap();
        assert_eq!(s.account_role_and_status(id).unwrap().0, USER_ROLE);
        assert_eq!(s.admin_count().unwrap(), 1);

        // An explicit promotion through the guarded API DOES create a
        // second administrator — that is the supported path now.
        s.set_account_role(id, ADMIN_ROLE).unwrap();
        assert_eq!(s.admin_count().unwrap(), 2);
        let (role, status) = s.account_role_and_status(id).unwrap();
        assert_eq!(role, ADMIN_ROLE);
        assert_eq!(status, STATUS_APPROVED, "promotion leaves status alone");

        // Bootstrap (`sign_up`) still refuses to run once ANY admin exists,
        // so the unauthenticated first-run path cannot mint admins either.
        assert!(matches!(
            s.sign_up("attacker@soket.ai", "a strong passphrase"),
            Err(AuthError::OwnerExists)
        ));
        assert_eq!(s.admin_count().unwrap(), 2);
    }

    #[test]
    fn pending_accounts_are_listed_oldest_first() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        assert!(s.list_pending_accounts(50).unwrap().is_empty());
        let mut ids = Vec::new();
        for email in [
            "helen@soket.ai",
            "ivan@soket.ai",
            "judy@soket.ai",
            "karl@soket.ai",
        ] {
            ids.push(s.create_pending_user(email, "a strong passphrase").unwrap());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let pending = s.list_pending_accounts(50).unwrap();
        assert_eq!(pending.len(), 4);
        assert_eq!(
            pending
                .iter()
                .map(|p| p.identifier.clone())
                .collect::<Vec<_>>(),
            vec![
                "helen@soket.ai".to_string(),
                "ivan@soket.ai".to_string(),
                "judy@soket.ai".to_string(),
                "karl@soket.ai".to_string(),
            ]
        );
        assert!(pending.iter().all(|p| p.status == STATUS_PENDING));
        assert!(pending.iter().all(|p| p.role == USER_ROLE));
        // Approving one removes it from the queue.
        s.set_account_status(ids[1], STATUS_APPROVED).unwrap();
        let pending = s.list_pending_accounts(50).unwrap();
        assert_eq!(pending.len(), 3);
        assert!(!pending.iter().any(|p| p.id == ids[1]));
        // The limit bounds the response.
        assert_eq!(s.list_pending_accounts(2).unwrap().len(), 2);
        assert!(matches!(
            s.list_pending_accounts(0),
            Err(AccountError::Validation(_))
        ));
    }

    #[test]
    fn pending_user_password_hashes_are_never_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let s = AuthStore::open(&path).unwrap();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let password = "a very secret passphrase";
        s.create_pending_user("mallory@soket.ai", password).unwrap();

        let conn = Connection::open(&path).unwrap();
        let hash: String = conn
            .query_row(
                "SELECT password_hash FROM auth_account WHERE identifier = 'mallory@soket.ai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hash.starts_with("$argon2id$"), "must be Argon2id: {hash}");
        assert_ne!(hash, password);
        assert!(!hash.contains(password));

        // Scan the whole database file for the plaintext password.
        let db_bytes = std::fs::read(&path).unwrap();
        let needle = password.as_bytes();
        assert!(
            !db_bytes.windows(needle.len()).any(|w| w == needle),
            "plaintext password must never touch the database file"
        );
    }

    #[test]
    fn create_pending_user_validates_email_domain_and_password() {
        let (s, _d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        // Non-soket.ai domains are refused.
        for bad in ["user@gmail.com", "user@example.soket.ai", "user@soket.ai."] {
            assert!(
                matches!(
                    s.create_pending_user(bad, "a strong passphrase"),
                    Err(AccountError::Validation(_))
                ),
                "{bad} must be rejected"
            );
        }
        // Weak passwords are refused.
        assert!(matches!(
            s.create_pending_user("good@soket.ai", "short"),
            Err(AccountError::Validation(_))
        ));
        // Nothing was created by the failed attempts.
        assert!(matches!(
            s.find_account_by_identifier("good@soket.ai"),
            Err(AccountError::NotFound)
        ));
        assert_eq!(s.admin_count().unwrap(), 1);
    }

    #[test]
    fn find_account_by_identifier_reports_missing_accounts() {
        let (s, _d) = store();
        assert!(matches!(
            s.find_account_by_identifier("ghost@soket.ai"),
            Err(AccountError::NotFound)
        ));
        assert!(matches!(
            s.find_account_by_identifier("   "),
            Err(AccountError::Validation(_))
        ));
    }

    #[test]
    fn legacy_admin_identifier_is_documented_constant() {
        // Guards against accidentally changing the grandfathered identity.
        assert_eq!(LEGACY_ADMIN_IDENTIFIER, "himanshusudan771@gmail.com");
    }

    // ==================================================================
    // Phase 7: full account listing + role management
    // ==================================================================

    /// Helper: build a store with one admin plus accounts in the given
    /// (email, status) states, returning ids in the same order.
    fn store_with(accounts: &[(&str, &str)]) -> (AuthStore, tempfile::TempDir, Vec<i64>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let s = AuthStore::open(&path).unwrap();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let mut ids = Vec::new();
        for (email, status) in accounts {
            let id = match *status {
                STATUS_PENDING => s.create_pending_user(email, "a strong passphrase").unwrap(),
                STATUS_REJECTED | STATUS_SUSPENDED => {
                    // Stage through the only legal path: pending -> approved,
                    // then out of approved. (rejected is reached from
                    // pending directly, but the two-step keeps this branch
                    // uniform and exercises more transitions.)
                    let id = s.create_pending_user(email, "a strong passphrase").unwrap();
                    if *status == STATUS_REJECTED {
                        s.set_account_status(id, STATUS_REJECTED).unwrap();
                    } else {
                        s.set_account_status(id, STATUS_APPROVED).unwrap();
                        s.set_account_status(id, STATUS_SUSPENDED).unwrap();
                    }
                    id
                }
                _ => {
                    let id = s.create_pending_user(email, "a strong passphrase").unwrap();
                    if *status == STATUS_APPROVED {
                        s.set_account_status(id, STATUS_APPROVED).unwrap();
                    }
                    id
                }
            };
            ids.push(id);
        }
        (s, dir, ids)
    }

    #[test]
    fn list_accounts_returns_every_account_with_safe_fields() {
        let (s, _d, _) = store_with(&[
            ("a@soket.ai", STATUS_PENDING),
            ("b@soket.ai", STATUS_APPROVED),
            ("c@soket.ai", STATUS_REJECTED),
            ("d@soket.ai", STATUS_SUSPENDED),
        ]);
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 5, "admin + four users");
        assert_eq!(rows.len(), 5);
        // Every row carries exactly the safe projection.
        for r in &rows {
            assert!(!r.identifier.is_empty());
            assert!(matches!(r.role.as_str(), ADMIN_ROLE | USER_ROLE));
            assert!(matches!(
                r.status.as_str(),
                STATUS_PENDING | STATUS_APPROVED | STATUS_REJECTED | STATUS_SUSPENDED
            ));
        }
        // The admin is present and identifiable.
        assert!(rows
            .iter()
            .any(|r| r.role == ADMIN_ROLE && r.status == STATUS_APPROVED));
        // Never-signed-in accounts report no last sign-in rather than a
        // fabricated value.
        assert!(rows.iter().all(|r| r.last_sign_in_at_ms.is_none()));
    }

    #[test]
    fn list_accounts_reports_last_sign_in_when_recorded() {
        let (s, _d, ids) = store_with(&[("a@soket.ai", STATUS_APPROVED)]);
        let before = chrono::Utc::now().timestamp_millis();
        let token = s.sign_in("a@soket.ai", "a strong passphrase").unwrap();
        drop(token);
        let (rows, _) = s
            .list_accounts(&AccountFilter {
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        let a = rows.iter().find(|r| r.id == ids[0]).unwrap();
        let seen = a.last_sign_in_at_ms.expect("sign-in must be recorded");
        assert!(seen >= before, "last_seen must be at/after the sign-in");
    }

    #[test]
    fn list_accounts_search_filters_and_pagination() {
        let (s, _d, _) = store_with(&[
            ("alice@soket.ai", STATUS_APPROVED),
            ("alina@soket.ai", STATUS_PENDING),
            ("bob@soket.ai", STATUS_APPROVED),
        ]);

        // Search by substring, case-insensitive.
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                search: Some("ALI".into()),
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 2);
        assert!(rows.iter().all(|r| r.identifier.contains("li")));

        // LIKE metacharacters are literal, not wildcards.
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                search: Some("%".into()),
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 0, "a bare % must match nothing");
        assert!(rows.is_empty());

        // Filter by role.
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                role: Some(ADMIN_ROLE.into()),
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].role, ADMIN_ROLE);

        // Filter by status.
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                status: Some(STATUS_PENDING.into()),
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].identifier, "alina@soket.ai");

        // Combined filters narrow further.
        let (_, total) = s
            .list_accounts(&AccountFilter {
                search: Some("ali".into()),
                status: Some(STATUS_APPROVED.into()),
                per_page: 50,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 1, "only alice is both 'ali*' and approved");

        // Pagination: page size 2 over 4 accounts (admin + 3).
        let (p1, t) = s
            .list_accounts(&AccountFilter {
                per_page: 2,
                page: 1,
                ..AccountFilter::default()
            })
            .unwrap();
        let (p2, _) = s
            .list_accounts(&AccountFilter {
                per_page: 2,
                page: 2,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(t, 4);
        assert_eq!(p1.len(), 2);
        assert_eq!(p2.len(), 2);
        // No overlap between pages.
        let p1_ids: Vec<i64> = p1.iter().map(|r| r.id).collect();
        let p2_ids: Vec<i64> = p2.iter().map(|r| r.id).collect();
        assert!(!p1_ids.iter().any(|i| p2_ids.contains(i)));

        // per_page is clamped to the documented maximum.
        let (rows, _) = s
            .list_accounts(&AccountFilter {
                per_page: ACCOUNT_LIST_MAX_PER_PAGE * 10,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 4, "clamped but still returns all rows here");

        // Unknown role/status are rejected, not silently ignored.
        assert!(matches!(
            s.list_accounts(&AccountFilter {
                role: Some("superuser".into()),
                per_page: 10,
                ..AccountFilter::default()
            }),
            Err(AccountError::Validation(_))
        ));
        assert!(matches!(
            s.list_accounts(&AccountFilter {
                status: Some("deleted".into()),
                per_page: 10,
                ..AccountFilter::default()
            }),
            Err(AccountError::Validation(_))
        ));
    }

    #[test]
    fn promotion_requires_an_approved_account() {
        // A pending, a rejected and a suspended account may NOT be promoted:
        // approval is a separate, explicit step.
        let (s, _d, ids) = store_with(&[
            ("p@soket.ai", STATUS_PENDING),
            ("r@soket.ai", STATUS_REJECTED),
            ("u@soket.ai", STATUS_SUSPENDED),
            ("ok@soket.ai", STATUS_APPROVED),
        ]);
        for (i, expected) in [
            (0, STATUS_PENDING),
            (1, STATUS_REJECTED),
            (2, STATUS_SUSPENDED),
        ] {
            let err = s.set_account_role(ids[i], ADMIN_ROLE).unwrap_err();
            assert!(
                matches!(err, AccountError::InvalidTransition { .. }),
                "index {i} should refuse promotion, got {err:?}"
            );
            let (role, status) = s.account_role_and_status(ids[i]).unwrap();
            assert_eq!(role, USER_ROLE);
            assert_eq!(status, expected, "status must be untouched");
        }
        // An approved account promotes cleanly.
        assert_eq!(s.set_account_role(ids[3], ADMIN_ROLE).unwrap(), ADMIN_ROLE);
        let (role, status) = s.account_role_and_status(ids[3]).unwrap();
        assert_eq!(role, ADMIN_ROLE);
        assert_eq!(status, STATUS_APPROVED, "promotion must not alter status");
        // Two administrators now coexist.
        assert_eq!(s.admin_count().unwrap(), 2);
    }

    #[test]
    fn promotion_never_touches_password_or_sessions() {
        let (s, _d, ids) = store_with(&[("a@soket.ai", STATUS_APPROVED)]);
        let token = s.sign_in("a@soket.ai", "a strong passphrase").unwrap();
        let sessions_before = s.session_count().unwrap();

        s.set_account_role(ids[0], ADMIN_ROLE).unwrap();

        // Credentials still work; no new session was minted by the change.
        assert!(
            s.validate_session(&token).is_some(),
            "existing session survives"
        );
        assert_eq!(s.session_count().unwrap(), sessions_before);
        // The password is unchanged: sign-in with the SAME passphrase works.
        let again = s.sign_in("a@soket.ai", "a strong passphrase").unwrap();
        assert!(!again.is_empty());
    }

    #[test]
    fn last_administrator_cannot_be_demoted() {
        let (s, _d, ids) = store_with(&[]);
        let admin_id = ids
            .first()
            .copied()
            .unwrap_or_else(|| s.find_account_by_identifier(LEGACY_ADMIN).unwrap().id);
        // With only one administrator, demotion is refused.
        assert!(matches!(
            s.set_account_role(admin_id, USER_ROLE),
            Err(AccountError::LastAdmin)
        ));
        let (role, _) = s.account_role_and_status(admin_id).unwrap();
        assert_eq!(role, ADMIN_ROLE);

        // Promote a second admin, then demotion of EITHER becomes legal —
        // including the original.
        let helper = s
            .create_pending_user("h@soket.ai", "a strong passphrase")
            .unwrap();
        s.set_account_status(helper, STATUS_APPROVED).unwrap();
        s.set_account_role(helper, ADMIN_ROLE).unwrap();
        assert_eq!(s.admin_count().unwrap(), 2);
        assert_eq!(s.set_account_role(admin_id, USER_ROLE).unwrap(), USER_ROLE);
        assert_eq!(s.admin_count().unwrap(), 1);

        // And now the remaining admin is protected again.
        assert!(matches!(
            s.set_account_role(helper, USER_ROLE),
            Err(AccountError::LastAdmin)
        ));
    }

    #[test]
    fn role_change_is_idempotent_and_validated() {
        let (s, _d, ids) = store_with(&[("a@soket.ai", STATUS_APPROVED)]);
        // Same-role change is a harmless no-op reporting the current role.
        assert_eq!(s.set_account_role(ids[0], USER_ROLE).unwrap(), USER_ROLE);
        // Unknown roles are rejected outright.
        assert!(matches!(
            s.set_account_role(ids[0], "root"),
            Err(AccountError::Validation(_))
        ));
        // Unknown account ids are reported as missing.
        assert!(matches!(
            s.set_account_role(999_999, ADMIN_ROLE),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn concurrent_promotions_cannot_double_apply_or_orphan_admins() {
        // Two stores on ONE database file simulate concurrent writers.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let s1 = AuthStore::open(&path).unwrap();
        s1.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let target = s1
            .create_pending_user("t@soket.ai", "a strong passphrase")
            .unwrap();
        s1.set_account_status(target, STATUS_APPROVED).unwrap();
        let s2 = AuthStore::open(&path).unwrap();

        // Both promote the same account: both must succeed (idempotent) and
        // the result must be exactly one role value.
        let r1 = std::thread::spawn(move || s1.set_account_role(target, ADMIN_ROLE));
        let r2 = std::thread::spawn(move || s2.set_account_role(target, ADMIN_ROLE));
        let outcomes = [r1.join().unwrap(), r2.join().unwrap()];
        for o in outcomes {
            assert!(o.is_ok(), "concurrent promotion must not corrupt: {o:?}");
        }
        let s3 = AuthStore::open(&path).unwrap();
        let (role, status) = s3.account_role_and_status(target).unwrap();
        assert_eq!(role, ADMIN_ROLE);
        assert_eq!(status, STATUS_APPROVED);
        assert_eq!(s3.admin_count().unwrap(), 2);
    }

    #[test]
    fn concurrent_last_admin_demotions_leave_one_admin() {
        // Two admins; two concurrent attempts to demote BOTH. At least one
        // must fail so the installation is never stranded.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let s1 = AuthStore::open(&path).unwrap();
        s1.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let a = s1
            .create_pending_user("a@soket.ai", "a strong passphrase")
            .unwrap();
        s1.set_account_status(a, STATUS_APPROVED).unwrap();
        s1.set_account_role(a, ADMIN_ROLE).unwrap();
        let s2 = AuthStore::open(&path).unwrap();

        let admin_id = s1.find_account_by_identifier(LEGACY_ADMIN).unwrap().id;
        let r1 = std::thread::spawn(move || s1.set_account_role(admin_id, USER_ROLE));
        let r2 = std::thread::spawn(move || s2.set_account_role(a, USER_ROLE));
        let ok1 = r1.join().unwrap().is_ok();
        let ok2 = r2.join().unwrap().is_ok();
        assert!(
            !(ok1 && ok2),
            "both demotions succeeded: installation would have no admin"
        );
        let s3 = AuthStore::open(&path).unwrap();
        assert_eq!(
            s3.admin_count().unwrap(),
            1,
            "exactly one administrator must survive concurrent demotions"
        );
    }

    #[test]
    fn public_registration_never_creates_admins() {
        let (s, _d, _) = store_with(&[]);
        // Even after an admin exists, registration yields a pending USER.
        let id = s
            .create_pending_user("n@soket.ai", "a strong passphrase")
            .unwrap();
        let (role, status) = s.account_role_and_status(id).unwrap();
        assert_eq!(role, USER_ROLE, "signup must never grant admin");
        assert_eq!(status, STATUS_PENDING);
        assert_eq!(s.admin_count().unwrap(), 1);
    }

    #[test]
    fn phase7_migration_drops_single_admin_index_on_existing_db() {
        // A database created under the OLD schema still carries the partial
        // unique index; opening it must remove that index so a second
        // administrator becomes possible.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE auth_account (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     identifier TEXT NOT NULL UNIQUE COLLATE NOCASE,
                     password_hash TEXT NOT NULL,
                     role TEXT NOT NULL DEFAULT 'user'
                         CHECK (role IN ('admin','user')),
                     status TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending','approved','rejected','suspended')),
                     created_at_ms INTEGER NOT NULL
                 );
                 CREATE UNIQUE INDEX idx_auth_account_single_admin
                     ON auth_account (role) WHERE role = 'admin';
                 CREATE TABLE sessions (
                     token_digest BLOB PRIMARY KEY,
                     account_id INTEGER NOT NULL
                         REFERENCES auth_account (id) ON DELETE CASCADE,
                     created_at_ms INTEGER NOT NULL,
                     expires_at_ms INTEGER NOT NULL,
                     last_seen_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO auth_account
                     (identifier, password_hash, role, status, created_at_ms)
                 VALUES ('himanshusudan771@gmail.com', 'x', 'admin', 'approved', 1);",
            )
            .unwrap();
        }
        // Opening applies the Phase 7 migration.
        let s = AuthStore::open(&path).unwrap();
        let (rows, total) = s
            .list_accounts(&AccountFilter {
                per_page: 10,
                ..AccountFilter::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].role, ADMIN_ROLE);

        // The old index is gone, so promoting a second admin is now legal.
        let conn = Connection::open(&path).unwrap();
        let still_there: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type='index' AND name='idx_auth_account_single_admin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_there, 0, "old single-admin index must be dropped");

        let second = s
            .create_pending_user("two@soket.ai", "a strong passphrase")
            .unwrap();
        s.set_account_status(second, STATUS_APPROVED).unwrap();
        s.set_account_role(second, ADMIN_ROLE).unwrap();
        assert_eq!(s.admin_count().unwrap(), 2);
    }

    // ==================================================================
    // User Management: details, sessions, deletion, password reset, audit
    // ==================================================================

    /// Boot an admin + one approved ordinary user; returns (store, dir,
    /// admin_id, user_id).
    fn um_fixture() -> (AuthStore, tempfile::TempDir, i64, i64) {
        let (s, d) = store();
        s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        let admin = s.find_account_by_identifier(LEGACY_ADMIN).unwrap().id;
        let user = s
            .create_pending_user("victim@soket.ai", "a strong passphrase")
            .unwrap();
        s.set_account_status(user, STATUS_APPROVED).unwrap();
        (s, d, admin, user)
    }

    #[test]
    fn find_account_by_id_round_trips() {
        let (s, _d, _a, user) = um_fixture();
        let rec = s.find_account_by_id(user).unwrap();
        assert_eq!(rec.identifier, "victim@soket.ai");
        assert_eq!(rec.status, STATUS_APPROVED);
        assert!(matches!(
            s.find_account_by_id(999_999),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn account_detail_reports_every_safe_field() {
        let (s, _d, _admin, user) = um_fixture();
        // The user signs in twice: two live sessions.
        let t1 = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        let t2 = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        assert!(s.validate_session(&t1).is_some());
        assert!(s.validate_session(&t2).is_some());

        let d = s.account_detail(user).unwrap();
        assert_eq!(d.id, user);
        assert_eq!(d.identifier, "victim@soket.ai");
        assert_eq!(d.role, USER_ROLE);
        assert_eq!(d.status, STATUS_APPROVED);
        assert!(d.created_at_ms > 0);
        assert!(d.last_sign_in_at_ms.is_some(), "sign-in must be recorded");
        assert_eq!(d.live_session_count, 2);
        // No password-change tracking yet on a fresh account.
        assert!(d.last_password_change_ms.is_none());

        // Sessions list carries timestamps only — never digests.
        let sessions = s.account_sessions(user, 10).unwrap();
        assert_eq!(sessions.len(), 2);
        for sess in &sessions {
            assert!(sess.created_at_ms > 0);
            assert!(sess.expires_at_ms > sess.created_at_ms);
            assert!(sess.last_seen_at_ms >= sess.created_at_ms);
        }

        // Unknown id: NotFound, not a panic.
        assert!(matches!(
            s.account_detail(424_242),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn revoke_all_sessions_removes_only_that_accounts_sessions() {
        let (s, _d, admin, user) = um_fixture();
        let victim_tok = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        let admin_tok = s.sign_in(LEGACY_ADMIN, "correct horse battery").unwrap();

        let removed = s.revoke_all_sessions_for(user).unwrap();
        assert_eq!(removed, 1, "exactly the victim's session");
        assert!(s.validate_session(&victim_tok).is_none());
        // The admin's session is untouched.
        assert!(s.validate_session(&admin_tok).is_some());

        // Idempotent: nothing left to remove.
        assert_eq!(s.revoke_all_sessions_for(user).unwrap(), 0);
        let _ = admin;
    }

    #[test]
    fn delete_account_cleans_up_dependent_records() {
        let (s, _d, admin, user) = um_fixture();
        let tok = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        assert!(s.validate_session(&tok).is_some());
        // A pending reset token for the doomed account.
        s.issue_password_reset(user, admin).unwrap();
        // A failure-counter row for its identity.
        let _ = s.sign_in("victim@soket.ai", "wrong password");

        let deleted = s.delete_account(user, admin).unwrap();
        assert_eq!(deleted, "victim@soket.ai");

        // Account gone.
        assert!(matches!(
            s.find_account_by_id(user),
            Err(AccountError::NotFound)
        ));
        // Its session no longer validates (row gone AND join fails).
        assert!(s.validate_session(&tok).is_none());
        // Dependent rows are all gone.
        {
            let conn = s.conn_for_tests();
            let sessions: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE account_id = ?1",
                    [user],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sessions, 0, "sessions must be cleaned up");
            let resets: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM password_reset_tokens WHERE account_id = ?1",
                    [user],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(resets, 0, "reset tokens must be cleaned up");
            let failures: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM signin_failures WHERE identity_key = ?1",
                    ["victim@soket.ai"],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(failures, 0, "failure counter must be cleaned up");
        }
        // The admin survives.
        assert_eq!(s.admin_count().unwrap(), 1);
    }

    #[test]
    fn delete_account_refuses_self_and_last_admin() {
        let (s, _d, admin, _user) = um_fixture();
        // Self-delete refused with the specific conflict error (HTTP 409 on
        // the API): the account exists and could be deleted by a DIFFERENT
        // administrator, so this is a state conflict, not a bad request.
        assert!(matches!(
            s.delete_account(admin, admin),
            Err(AccountError::AdminSelfDelete)
        ));
        assert_eq!(s.admin_count().unwrap(), 1);

        // Last-admin delete refused even when the acting admin is someone
        // else (impossible today, but the storage layer must hold alone):
        // promote a peer, then delete BOTH would strand the install — the
        // second delete must fail.
        let peer = s
            .create_pending_user("peer@soket.ai", "a strong passphrase")
            .unwrap();
        s.set_account_status(peer, STATUS_APPROVED).unwrap();
        s.set_account_role(peer, ADMIN_ROLE).unwrap();
        assert_eq!(s.admin_count().unwrap(), 2);
        // Peer deletes the original admin: legal (one admin remains).
        assert!(s.delete_account(admin, peer).is_ok());
        assert_eq!(s.admin_count().unwrap(), 1);
        // Now the peer is the last admin: deleting them is refused (the
        // same self-delete conflict — the peer is both actor and target).
        assert!(matches!(
            s.delete_account(peer, peer),
            Err(AccountError::AdminSelfDelete)
        ));
        assert_eq!(s.admin_count().unwrap(), 1);
    }

    #[test]
    fn delete_account_requires_positive_id_that_exists() {
        let (s, _d, admin, _user) = um_fixture();
        assert!(matches!(
            s.delete_account(999_999, admin),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn password_reset_token_is_single_use_and_expiring() {
        let (s, _d, admin, user) = um_fixture();
        let issued = s.issue_password_reset(user, admin).unwrap();
        assert!(!issued.token.is_empty());
        assert_eq!(issued.identifier, "victim@soket.ai");

        // Peek says valid before use.
        assert!(s.peek_password_reset(&issued.token).unwrap());
        // A wrong token does not peek valid.
        assert!(!s.peek_password_reset("not-a-real-token").unwrap());

        // Complete the reset with a too-short password: refused, token still live.
        assert!(matches!(
            s.complete_password_reset(&issued.token, "short"),
            Err(AccountError::PasswordValidation(_))
        ));
        assert!(s.peek_password_reset(&issued.token).unwrap());

        // Complete properly.
        let (acct, ident) = s
            .complete_password_reset(&issued.token, "brand new passphrase")
            .unwrap();
        assert_eq!(acct, user);
        assert_eq!(ident, "victim@soket.ai");

        // The token is spent: peek false, replay refused.
        assert!(!s.peek_password_reset(&issued.token).unwrap());
        assert!(matches!(
            s.complete_password_reset(&issued.token, "another passphrase"),
            Err(AccountError::ResetInvalid)
        ));

        // The new password works; the old one does not.
        assert!(s.sign_in("victim@soket.ai", "brand new passphrase").is_ok());
        assert!(matches!(
            s.sign_in("victim@soket.ai", "a strong passphrase"),
            Err(AuthError::InvalidCredentials)
        ));

        // The change was tracked.
        let d = s.account_detail(user).unwrap();
        assert!(d.last_password_change_ms.is_some());
    }

    #[test]
    fn completing_a_reset_revokes_existing_sessions() {
        let (s, _d, admin, user) = um_fixture();
        let tok = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        assert!(s.validate_session(&tok).is_some());

        let issued = s.issue_password_reset(user, admin).unwrap();
        s.complete_password_reset(&issued.token, "replacement passphrase")
            .unwrap();

        // The pre-reset cookie is dead even though the account is approved.
        assert!(
            s.validate_session(&tok).is_none(),
            "a recovered password must invalidate prior sessions"
        );
        // And a fresh sign-in works with the new password.
        let fresh = s
            .sign_in("victim@soket.ai", "replacement passphrase")
            .unwrap();
        assert!(s.validate_session(&fresh).is_some());
    }

    #[test]
    fn issuing_a_new_reset_supersedes_the_previous_one() {
        let (s, _d, admin, user) = um_fixture();
        let first = s.issue_password_reset(user, admin).unwrap();
        // Cooldown blocks an immediate re-issue.
        match s.issue_password_reset(user, admin) {
            Err(AccountError::ResetCooldown { retry_in_ms }) => {
                assert!(retry_in_ms > 0);
            }
            other => panic!("expected cooldown, got {other:?}"),
        }
        // After the cooldown, the second issue supersedes the first.
        std::thread::sleep(std::time::Duration::from_millis(
            PASSWORD_RESET_COOLDOWN.as_millis() as u64 + 20,
        ));
        let second = s.issue_password_reset(user, admin).unwrap();
        assert_ne!(first.token, second.token);
        // The superseded token can no longer complete a reset.
        assert!(matches!(
            s.complete_password_reset(&first.token, "whatever passphrase"),
            Err(AccountError::ResetInvalid)
        ));
        // The fresh one can.
        assert!(s
            .complete_password_reset(&second.token, "chosen passphrase")
            .is_ok());
    }

    #[test]
    fn reset_tokens_are_never_stored_plaintext() {
        let (s, d, admin, user) = um_fixture();
        let issued = s.issue_password_reset(user, admin).unwrap();
        // Scan the whole DB file for the raw token's ASCII bytes.
        let db_bytes = std::fs::read(d.path().join("auth.db")).unwrap();
        let needle = issued.token.as_bytes();
        assert!(
            !db_bytes.windows(needle.len()).any(|w| w == needle),
            "plaintext reset token must never touch the database"
        );
    }

    #[test]
    fn reset_issuance_fails_cleanly_for_unknown_account() {
        let (s, _d, admin, _user) = um_fixture();
        assert!(matches!(
            s.issue_password_reset(313_131, admin),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn audit_log_records_and_lists_events() {
        let (s, _d, admin, user) = um_fixture();
        s.record_audit(
            audit_event::ROLE_CHANGED,
            Some((admin, LEGACY_ADMIN)),
            Some((user, "victim@soket.ai")),
            "role set to admin",
        );
        s.record_audit(
            audit_event::SIGNIN_SUCCESS,
            None,
            Some((user, "victim@soket.ai")),
            "signed in",
        );

        let (entries, total) = s.list_account_activity(user, 50, 0).unwrap();
        assert_eq!(total, 2);
        assert_eq!(entries.len(), 2);
        // Newest first.
        assert_eq!(entries[0].kind, audit_event::SIGNIN_SUCCESS);
        assert_eq!(entries[1].kind, audit_event::ROLE_CHANGED);
        assert_eq!(
            entries[1].actor_email.as_deref(),
            Some(LEGACY_ADMIN),
            "the acting administrator is recorded"
        );
        assert_eq!(entries[1].subject_email.as_deref(), Some("victim@soket.ai"));

        // record_audit_checked surfaces storage failures.
        assert!(s
            .record_audit_checked(
                audit_event::ACCOUNT_DELETED,
                Some((admin, LEGACY_ADMIN)),
                Some((user, "victim@soket.ai")),
                "x"
            )
            .is_ok());
    }

    #[test]
    fn audit_history_survives_account_deletion() {
        let (s, _d, admin, user) = um_fixture();
        s.record_audit(
            audit_event::ACCOUNT_SUSPENDED,
            Some((admin, LEGACY_ADMIN)),
            Some((user, "victim@soket.ai")),
            "status set to suspended",
        );
        let deleted = s.delete_account(user, admin).unwrap();
        assert_eq!(deleted, "victim@soket.ai");

        // The audit rows outlive the account they describe.
        let (entries, total) = s.list_account_activity(user, 50, 0).unwrap();
        assert_eq!(total, 1, "audit history must survive deletion");
        assert_eq!(entries[0].subject_email.as_deref(), Some("victim@soket.ai"));
    }

    #[test]
    fn suspended_signin_returns_distinct_error_after_verification() {
        let (s, _d, _admin, user) = um_fixture();
        s.set_account_status(user, STATUS_SUSPENDED).unwrap();
        // Correct credentials: the truthful suspended message.
        assert!(matches!(
            s.sign_in("victim@soket.ai", "a strong passphrase"),
            Err(AuthError::AccountSuspended)
        ));
        // Wrong credentials: still generic, no status leak.
        assert!(matches!(
            s.sign_in("victim@soket.ai", "wrong password entirely"),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn user_management_migration_is_idempotent_and_additive() {
        // Opening the same database twice must succeed and leave exactly
        // one of each new table / column.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        {
            let s = AuthStore::open(&path).unwrap();
            s.sign_up(LEGACY_ADMIN, "correct horse battery").unwrap();
        }
        AuthStore::open(&path).unwrap();
        AuthStore::open(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        for table in ["password_reset_tokens", "audit_log"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{table} must exist exactly once");
        }
        let cols: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('auth_account')
                  WHERE name = 'last_password_change_ms'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1, "the new column must exist exactly once");
        // Existing data untouched.
        let admins: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM auth_account WHERE role = 'admin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(admins, 1);
    }

    #[test]
    fn foreign_keys_are_enforced_on_delete() {
        // The store must enable PRAGMA foreign_keys so the schema's ON
        // DELETE CASCADE declarations actually fire.
        let (s, _d, admin, user) = um_fixture();
        let _ = s.sign_in("victim@soket.ai", "a strong passphrase").unwrap();
        s.issue_password_reset(user, admin).unwrap();
        let fk: i64 = {
            let conn = s.conn_for_tests();
            conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(fk, 1, "foreign_keys must be ON");
        // Cascade removes the dependent rows with the account.
        s.delete_account(user, admin).unwrap();
        let conn = Connection::open(_d.path().join("auth.db")).unwrap();
        let orphan_sessions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE account_id = ?1",
                [user],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphan_sessions, 0);
    }
}
