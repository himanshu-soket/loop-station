//! One-time-password (OTP) challenges and verification grants for new owner
//! registration.
//!
//! Design notes
//! ------------
//! * **Additive schema.** Two new tables (`otp_challenges`,
//!   `verification_grants`) are created with `CREATE TABLE IF NOT EXISTS`
//!   alongside the existing `owner_account`/`sessions` tables in the same
//!   `auth.db`. No existing table, row, or session is touched; the current
//!   owner account is unaffected by anything here.
//! * **Never plaintext.** The six-digit code exists in cleartext only
//!   transiently while being handed to the caller (which will email it).
//!   At rest we keep `HMAC-SHA256(server_key, code)`; verification
//!   recomputes the MAC of the submitted code and compares through
//!   [`hmac::Mac::verify_slice`] (constant-time).
//! * **Single-use, atomically.** Every state transition is a conditional SQL
//!   update guarded by `rows_affected`, executed inside a transaction. Two
//!   racing verifiers cannot both consume one challenge: the second sees
//!   zero affected rows.
//! * **Supersession.** Issuing a new challenge for an email supersedes
//!   (marks consumed) every previous live challenge for that email, so at
//!   most one live challenge exists per mailbox at any time.
//! * **Grants.** Successful OTP verification mints an opaque 256-bit grant
//!   token; only its SHA-256 digest is stored. Grants are single-use and
//!   expire after ten minutes. They are the *only* way to assert "this
//!   email was verified" — no client-controlled field can do so.
//! * **Clock injection.** All time comes from [`OtpClock`], so tests are
//!   deterministic (no sleeps).
//! * **No mutex across await points.** The connection mutex is a
//!   synchronous `parking_lot` lock held only within these fully
//!   synchronous store methods; no async work (e.g., network sends) may
//!   happen while it is held. Email delivery must be performed outside the
//!   store, after [`OtpStore::issue_challenge`] returns.

use std::path::Path;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::auth_email::normalize_registration_email;

/// How long an OTP challenge remains valid.
pub const OTP_TTL: Duration = Duration::from_secs(5 * 60);

/// How long a verification grant remains valid after successful OTP entry.
pub const GRANT_TTL: Duration = Duration::from_secs(10 * 60);

/// Wrong submissions allowed per challenge before it dies.
pub const OTP_MAX_ATTEMPTS: u32 = 5;

/// Minimum delay between two issued challenges for the same email.
pub const OTP_RESEND_COOLDOWN: Duration = Duration::from_secs(60);

/// How long dead (expired/consumed) rows linger before cleanup removes them.
const RETENTION_AFTER_DEATH: i64 = 24 * 60 * 60 * 1000; // one day, in ms

type HmacSha256 = Hmac<Sha256>;

// `Duration` re-exported for doc links above.
#[allow(non_camel_case_types)]
type Duration = std::time::Duration;

/// Errors for the OTP subsystem. Messages are safe to show to clients
/// (no secrets, no enumeration signals).
#[derive(Debug, thiserror::Error)]
pub enum OtpError {
    /// The supplied email failed registration validation.
    #[error("{0}")]
    Validation(String),
    /// No live challenge for this email (never sent, expired, superseded,
    /// or exhausted).
    #[error("invalid or expired verification code")]
    NoActiveChallenge,
    /// The submitted code did not match, or the challenge is exhausted or
    /// expired. Deliberately one message for all three cases so responses
    /// cannot distinguish them.
    #[error("invalid or expired verification code")]
    WrongCode {
        /// Remaining submissions on this challenge, when known.
        attempts_left: Option<u32>,
    },
    /// A resend was requested before the cooldown elapsed.
    #[error("please wait before requesting another code")]
    Cooldown { retry_in_ms: i64 },
    /// The grant token is unknown, expired, or already used.
    #[error("verification session is invalid or expired")]
    GrantInvalid,
    /// Storage failure.
    #[error("authentication storage error")]
    Storage(#[from] rusqlite::Error),
    /// The signing key could not be initialized or loaded.
    #[error("failed to initialize otp signing key")]
    KeyInit(String),
}

impl OtpError {
    /// HTTP status appropriate for this error (wired up in a later phase).
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            OtpError::Validation(_) => axum::http::StatusCode::BAD_REQUEST,
            OtpError::NoActiveChallenge | OtpError::WrongCode { .. } => {
                axum::http::StatusCode::UNAUTHORIZED
            }
            OtpError::Cooldown { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
            OtpError::GrantInvalid => axum::http::StatusCode::UNAUTHORIZED,
            OtpError::Storage(_) | OtpError::KeyInit(_) => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

/// Injectable clock: production reads the wall clock; tests push time
/// forward deterministically.
pub trait OtpClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Wall-clock implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl OtpClock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// Test clock: a shared mutable millisecond counter.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct TestClock(pub Arc<parking_lot::Mutex<i64>>);

#[cfg(test)]
impl OtpClock for TestClock {
    fn now_ms(&self) -> i64 {
        *self.0.lock()
    }
}

#[cfg(test)]
impl TestClock {
    pub fn new(start_ms: i64) -> Self {
        Self(Arc::new(parking_lot::Mutex::new(start_ms)))
    }
    pub fn advance(&self, ms: i64) {
        *self.0.lock() += ms;
    }
}

/// Outcome of issuing a challenge: the raw code (to be emailed exactly once
/// and then dropped) plus timing metadata.
#[derive(Debug)]
pub struct IssuedChallenge {
    /// Database row id of the created challenge. Used by the delivery layer
    /// to invalidate exactly this row if the provider rejects the send.
    pub challenge_id: i64,
    /// Six decimal digits, e.g. `"042913"`. Never logged, never stored.
    pub code: String,
    /// Absolute expiry (unix ms) of this challenge.
    pub expires_at_ms: i64,
    /// Cooldown (ms) that must elapse before the next issue for this email.
    pub resend_cooldown_ms: i64,
}

/// Result of a successful OTP verification: the raw grant token (handed to
/// the client once) and its expiry.
#[derive(Debug)]
pub struct VerifiedGrant {
    /// Opaque URL-safe token; only its digest is persisted.
    pub token: String,
    /// Email the verification proved (normalized form).
    pub email: String,
    /// Absolute expiry (unix ms).
    pub expires_at_ms: i64,
}

/// A grant inspected without consuming it.
#[derive(Debug)]
pub struct GrantInfo {
    pub email: String,
    pub expires_at_ms: i64,
}

/// The OTP store: owns its SQLite connection, the signing key, and the clock.
pub struct OtpStore {
    conn: parking_lot::Mutex<Connection>,
    key: Vec<u8>,
    clock: Arc<dyn OtpClock>,
}

impl OtpStore {
    /// Open (creating if necessary) the OTP store at `path`, initializing or
    /// loading the signing key from `key_path`.
    ///
    /// Fails closed if the key cannot be created or loaded: an OTP system
    /// running on a lost key would silently invalidate every live challenge,
    /// which is worse than refusing to start.
    pub fn open(path: impl AsRef<Path>, key_path: impl AsRef<Path>) -> Result<Self, OtpError> {
        let key = load_or_create_key(key_path.as_ref())?;
        Self::open_with_key(path, key, Arc::new(SystemClock))
    }

    /// Constructor with an explicit key and clock (tests, embedding).
    pub fn open_with_key(
        path: impl AsRef<Path>,
        key: Vec<u8>,
        clock: Arc<dyn OtpClock>,
    ) -> Result<Self, OtpError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS otp_challenges (
                 id               INTEGER PRIMARY KEY AUTOINCREMENT,
                 email            TEXT    NOT NULL,
                 code_mac         BLOB    NOT NULL,
                 created_at_ms    INTEGER NOT NULL,
                 expires_at_ms    INTEGER NOT NULL,
                 attempts         INTEGER NOT NULL DEFAULT 0,
                 consumed_at_ms   INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_otp_email_live
                 ON otp_challenges (email) WHERE consumed_at_ms IS NULL;
             CREATE TABLE IF NOT EXISTS verification_grants (
                 token_digest     BLOB PRIMARY KEY,
                 email            TEXT    NOT NULL,
                 created_at_ms    INTEGER NOT NULL,
                 expires_at_ms    INTEGER NOT NULL,
                 consumed_at_ms   INTEGER
             );",
        )?;
        Ok(Self {
            conn: parking_lot::Mutex::new(conn),
            key,
            clock,
        })
    }

    /// Issue a new OTP challenge for `raw_email`.
    ///
    /// * Validates + normalizes the email (exact `@soket.ai` policy).
    /// * Enforces the resend cooldown against the most recent challenge
    ///   (live or dead) for that email.
    /// * Supersedes every live challenge for that email.
    /// * Returns the raw code exactly once; it is never persisted.
    pub fn issue_challenge(&self, raw_email: &str) -> Result<IssuedChallenge, OtpError> {
        let email = normalize_registration_email(raw_email).map_err(OtpError::Validation)?;
        let now = self.clock.now_ms();
        let code = generate_otp();
        let mac = self.mac_code(&code);
        let expires = now + OTP_TTL.as_millis() as i64;

        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        // Cooldown: newest challenge for this email, whatever its state.
        let last_created: Option<i64> = tx
            .query_row(
                "SELECT MAX(created_at_ms) FROM otp_challenges WHERE email = ?1",
                [&email],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(t) = last_created {
            let ready_at = t + OTP_RESEND_COOLDOWN.as_millis() as i64;
            if now < ready_at {
                return Err(OtpError::Cooldown {
                    retry_in_ms: ready_at - now,
                });
            }
        }

        // Supersede all live challenges for this email.
        tx.execute(
            "UPDATE otp_challenges
                SET consumed_at_ms = ?1
              WHERE email = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![now, email],
        )?;

        tx.execute(
            "INSERT INTO otp_challenges
                 (email, code_mac, created_at_ms, expires_at_ms, attempts)
             VALUES (?1, ?2, ?3, ?4, 0)",
            rusqlite::params![email, mac, now, expires],
        )?;
        let challenge_id = tx.last_insert_rowid();

        // Opportunistic cleanup of long-dead rows.
        cleanup_tx(&tx, now)?;

        tx.commit()?;
        Ok(IssuedChallenge {
            challenge_id,
            code,
            expires_at_ms: expires,
            resend_cooldown_ms: OTP_RESEND_COOLDOWN.as_millis() as i64,
        })
    }

    /// Invalidate one specific challenge row (by id) when its code could not
    /// be delivered. Idempotent; never touches any other row — a newer or
    /// older challenge for the same or another email is unaffected.
    ///
    /// Errors are swallowed deliberately: this runs on a best-effort failure
    /// path, and an already-consumed/expired/deleted row is exactly the
    /// desired end state.
    pub fn invalidate_challenge_by_id(&self, challenge_id: i64) {
        let conn = self.conn.lock();
        let _ = conn.execute(
            "UPDATE otp_challenges
                SET consumed_at_ms = ?1
              WHERE id = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![self.clock.now_ms(), challenge_id],
        );
    }

    /// Verify a submitted code against the live challenge for `raw_email`.
    ///
    /// On success the challenge is consumed and a fresh grant minted — both
    /// inside one transaction, so a crash can leave neither a half-consumed
    /// challenge nor an orphan grant. On the fifth wrong submission the
    /// challenge dies even if time remains.
    pub fn verify_otp(&self, raw_email: &str, code: &str) -> Result<VerifiedGrant, OtpError> {
        let email = normalize_registration_email(raw_email).map_err(OtpError::Validation)?;
        if !is_six_digits(code) {
            // Malformed input can never match a real code; reject without
            // burning an attempt or revealing state.
            return Err(OtpError::WrongCode {
                attempts_left: None,
            });
        }
        let now = self.clock.now_ms();

        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        // The single live challenge for this email (supersession guarantees
        // at most one).
        let row = tx
            .query_row(
                "SELECT id, code_mac, expires_at_ms, attempts
                   FROM otp_challenges
                  WHERE email = ?1 AND consumed_at_ms IS NULL
                  ORDER BY id DESC LIMIT 1",
                [&email],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .ok();

        let Some((id, stored_mac, expires_at_ms, attempts)) = row else {
            return Err(OtpError::NoActiveChallenge);
        };

        // Expired challenges fail exactly like wrong codes (identical
        // message ⇒ no oracle distinguishing mistyping from expiry).
        let expired = now >= expires_at_ms;
        let mac_ok = self.verify_code_mac(&stored_mac, code);
        if expired || (!mac_ok && attempts >= OTP_MAX_ATTEMPTS as i64) {
            tx.execute(
                "UPDATE otp_challenges SET consumed_at_ms = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
            tx.commit()?;
            return Err(OtpError::WrongCode {
                attempts_left: Some(0),
            });
        }
        if !mac_ok {
            let new_attempts = attempts + 1;
            let exhausted = new_attempts >= OTP_MAX_ATTEMPTS as i64;
            let updated = tx.execute(
                "UPDATE otp_challenges
                    SET attempts = ?1,
                        consumed_at_ms = CASE WHEN ?2 THEN ?3 ELSE consumed_at_ms END
                  WHERE id = ?4 AND consumed_at_ms IS NULL",
                rusqlite::params![new_attempts, exhausted, now, id],
            )?;
            let _ = updated; // single live row per email; guarded above
            tx.commit()?;
            return Err(OtpError::WrongCode {
                attempts_left: Some(OTP_MAX_ATTEMPTS.saturating_sub(new_attempts as u32)),
            });
        }

        // Correct code: consume the challenge exactly once…
        let consumed = tx.execute(
            "UPDATE otp_challenges
                SET consumed_at_ms = ?1
              WHERE id = ?2 AND consumed_at_ms IS NULL",
            rusqlite::params![now, id],
        )?;
        if consumed == 0 {
            // Raced with another verifier that won; indistinguishable from
            // "no live challenge".
            tx.commit()?;
            return Err(OtpError::NoActiveChallenge);
        }

        // …and mint the grant in the same transaction.
        let grant = insert_grant_tx(&tx, &email, now)?;

        tx.commit()?;
        Ok(grant)
    }

    /// Validate a grant token WITHOUT consuming it (pre-flight checks).
    pub fn peek_grant(&self, token: &str) -> Result<GrantInfo, OtpError> {
        let digest = token_digest(token);
        let now = self.clock.now_ms();
        let conn = self.conn.lock();
        match conn.query_row(
            "SELECT email, expires_at_ms FROM verification_grants
              WHERE token_digest = ?1 AND consumed_at_ms IS NULL",
            rusqlite::params![digest],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        ) {
            Ok((email, expires_at_ms)) if now < expires_at_ms => Ok(GrantInfo {
                email,
                expires_at_ms,
            }),
            _ => Err(OtpError::GrantInvalid),
        }
    }

    /// Validate and atomically consume a grant. Exactly one caller can ever
    /// succeed for a given token.
    pub fn consume_grant(&self, token: &str) -> Result<VerifiedGrant, OtpError> {
        let digest = token_digest(token);
        let now = self.clock.now_ms();
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE verification_grants
                SET consumed_at_ms = ?1
              WHERE token_digest = ?2 AND consumed_at_ms IS NULL AND expires_at_ms > ?1",
            rusqlite::params![now, digest],
        )?;
        if updated == 0 {
            return Err(OtpError::GrantInvalid);
        }
        let (email, expires_at_ms): (String, i64) = tx.query_row(
            "SELECT email, expires_at_ms FROM verification_grants WHERE token_digest = ?1",
            rusqlite::params![digest],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.commit()?;
        Ok(VerifiedGrant {
            token: token.to_string(),
            email,
            expires_at_ms,
        })
    }

    /// Delete challenges and grants that have been dead for over a day.
    /// Safe at any time; never touches live rows.
    /// Returns `(challenges_removed, grants_removed)`.
    pub fn cleanup_expired(&self) -> Result<(usize, usize), OtpError> {
        let now = self.clock.now_ms();
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let n = cleanup_tx(&tx, now)?;
        tx.commit()?;
        Ok(n)
    }

    /// Live (unconsumed, unexpired) challenge count — tests only.
    #[cfg(test)]
    pub fn live_challenge_count(&self) -> usize {
        let now = self.clock.now_ms();
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM otp_challenges
              WHERE consumed_at_ms IS NULL AND expires_at_ms > ?1",
            [now],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Unconsumed grant count — tests only.
    #[cfg(test)]
    pub fn unconsumed_grant_count(&self) -> usize {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM verification_grants WHERE consumed_at_ms IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// `HMAC-SHA256(server_key, code)` — what the database stores.
    fn mac_code(&self, code: &str) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(code.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// Constant-time verification of a submitted code against stored
    /// verification material ([`hmac::Mac::verify_slice`]).
    fn verify_code_mac(&self, stored: &[u8], code: &str) -> bool {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(code.as_bytes());
        mac.verify_slice(stored).is_ok()
    }
}

/// Six uniform random decimal digits via rejection sampling on OS entropy.
/// Bytes ≥ 250 are rejected so the modulo map stays unbiased.
fn generate_otp() -> String {
    loop {
        let mut bytes = [0u8; 6];
        getrandom::fill(&mut bytes).expect("OS entropy source unavailable; refusing weak OTPs");
        if bytes.iter().all(|b| *b < 250) {
            return bytes.iter().map(|b| (b % 10).to_string()).collect();
        }
    }
}

fn is_six_digits(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit())
}

fn token_digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Mint + persist a grant inside an open transaction.
fn insert_grant_tx(
    tx: &rusqlite::Transaction<'_>,
    email: &str,
    now: i64,
) -> Result<VerifiedGrant, OtpError> {
    let token = new_grant_token();
    let digest = token_digest(&token);
    let expires = now + GRANT_TTL.as_millis() as i64;
    tx.execute(
        "INSERT INTO verification_grants
             (token_digest, email, created_at_ms, expires_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![digest, email, now, expires],
    )?;
    Ok(VerifiedGrant {
        token,
        email: email.to_string(),
        expires_at_ms: expires,
    })
}

/// 256 bits of OS entropy, URL-safe base64 (matches session-token style).
fn new_grant_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS entropy source unavailable; refusing weak grants");
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// Delete rows that have been expired or consumed for over a day.
/// Live rows are never touched.
fn cleanup_tx(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<(usize, usize), OtpError> {
    let cutoff = now - RETENTION_AFTER_DEATH;
    let c = tx.execute(
        "DELETE FROM otp_challenges
          WHERE (expires_at_ms < ?1)
             OR (consumed_at_ms IS NOT NULL AND consumed_at_ms < ?1)",
        [cutoff],
    )?;
    let g = tx.execute(
        "DELETE FROM verification_grants
          WHERE (expires_at_ms < ?1)
             OR (consumed_at_ms IS NOT NULL AND consumed_at_ms < ?1)",
        [cutoff],
    )?;
    Ok((c, g))
}

// ---------------------------------------------------------------------------
// Signing-key management
// ---------------------------------------------------------------------------

/// Length of the OTP signing key, in bytes.
const KEY_LEN: usize = 32;

/// Load the OTP signing key from `path`, creating it on first use.
///
/// Behavior contract:
///
/// * **First run:** 32 bytes from the OS CSPRNG, written atomically
///   (fully-written temp file + `link`/`rename` into place) with owner-only
///   permissions (`0600` on Unix).
/// * **Later runs:** the existing key is loaded and **never replaced,
///   rotated, or overwritten**. Losing the key invalidates every live
///   challenge and grant MAC, so replacement is strictly avoided.
/// * **Concurrent initialization:** the final step is an atomic
///   create-if-absent (`hard_link`), so two racing processes yield one
///   winner; the loser reads the winner's key. No process ever observes a
///   partially written key.
/// * **Fail-closed:** if the file exists but cannot be read, or has the
///   wrong length, this returns an error rather than inventing a new key.
/// * **Lost key:** if the file is deleted, the next start generates a fresh
///   key. Live challenges issued under the old key become unverifiable
///   (users must request a new code); stored grants remain consumable only
///   if their digests predate the loss — in practice treat a lost key as
///   "restart registration".
/// * **Unsafe permissions:** on Unix, a world/group-accessible key file is
///   reported to stderr (the condition, never the key material).
///
/// The key material itself is never logged, returned in errors, or exposed
/// through any API.
pub fn load_or_create_key(path: &Path) -> Result<Vec<u8>, OtpError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.len() == KEY_LEN {
                warn_if_permissions_unsafe(path);
                return Ok(bytes);
            }
            return Err(OtpError::KeyInit(format!(
                "otp signing key at {} has invalid length {} (expected {KEY_LEN}); \
                 refusing to start with a corrupted key",
                path.display(),
                bytes.len()
            )));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_e) => {
            // Exists but unreadable: fail closed; never replace it.
            return Err(OtpError::KeyInit(format!(
                "cannot read otp signing key {}: {}",
                path.display(),
                std::fs::read(path)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            )));
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                OtpError::KeyInit(format!(
                    "cannot create directory for otp key {}: {e}",
                    path.display()
                ))
            })?;
        }
    }

    let mut key = [0u8; KEY_LEN];
    getrandom::fill(&mut key)
        .map_err(|e| OtpError::KeyInit(format!("OS entropy unavailable for otp key: {e}")))?;

    // Atomic publish: write a unique temp file, fsync, then link it into
    // place. `hard_link` fails if the destination already exists, which is
    // exactly the no-clobber guarantee we want; the temp is removed after.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp).map_err(|e| {
            OtpError::KeyInit(format!("cannot create otp key file {}: {e}", tmp.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| {
                    OtpError::KeyInit(format!("cannot secure otp key file {}: {e}", tmp.display()))
                })?;
        }
        file.write_all(&key)
            .map_err(|e| OtpError::KeyInit(format!("cannot write otp key: {e}")))?;
        file.sync_all()
            .map_err(|e| OtpError::KeyInit(format!("cannot sync otp key: {e}")))?;
    }

    let publish = std::fs::hard_link(&tmp, path);
    match publish {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            warn_if_permissions_unsafe(path);
            Ok(key.to_vec())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another initializer won the race; adopt its key.
            let _ = std::fs::remove_file(&tmp);
            let bytes = std::fs::read(path).map_err(|e| {
                OtpError::KeyInit(format!("cannot read otp key {}: {e}", path.display()))
            })?;
            if bytes.len() != KEY_LEN {
                return Err(OtpError::KeyInit(
                    "concurrently created otp key has invalid length".into(),
                ));
            }
            Ok(bytes)
        }
        Err(_e) => {
            // Filesystems without hard-link support: fall back to a direct
            // create-new write, accepting the (unsupported-deployment)
            // two-process first-boot race in exchange for working at all.
            let _ = std::fs::remove_file(&tmp);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path).map_err(|err| {
                OtpError::KeyInit(format!(
                    "cannot create otp key file {}: {err}",
                    path.display()
                ))
            })?;
            use std::io::Write;
            file.write_all(&key)
                .map_err(|e| OtpError::KeyInit(format!("cannot write otp key: {e}")))?;
            warn_if_permissions_unsafe(path);
            Ok(key.to_vec())
        }
    }
}

/// Report (never log the contents of) a key file with loose permissions.
#[cfg(unix)]
fn warn_if_permissions_unsafe(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "warning: otp signing key at {} has loose permissions {:o}; \
                 expected 0600 — tighten with chmod",
                path.display(),
                mode
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_permissions_unsafe(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000_000; // fixed epoch for deterministic tests

    fn setup() -> (OtpStore, Arc<TestClock>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new(T0));
        let store =
            OtpStore::open_with_key(dir.path().join("auth.db"), vec![7u8; 32], clock.clone())
                .unwrap();
        (store, clock, dir)
    }

    fn issue(store: &OtpStore) -> String {
        store.issue_challenge("user@soket.ai").unwrap().code
    }

    /// A code guaranteed different from `real`.
    fn wrong_code(real: &str) -> &'static str {
        if real == "000000" {
            "111111"
        } else {
            "000000"
        }
    }

    // ---- issuance & format ----

    #[test]
    fn otp_generation_is_six_uniform_digits() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let code = generate_otp();
            assert_eq!(code.len(), 6, "code must be six characters: {code}");
            assert!(code.bytes().all(|b| b.is_ascii_digit()));
            seen.insert(code.clone());
        }
        // Sanity bound (not a statistical test): 200 draws over 10^6 space
        // should produce well over 190 distinct values.
        assert!(
            seen.len() >= 190,
            "suspiciously low entropy: {}",
            seen.len()
        );
    }

    #[test]
    fn issue_validates_email_domain() {
        let (s, _c, _d) = setup();
        assert!(matches!(
            s.issue_challenge("user@gmail.com"),
            Err(OtpError::Validation(_))
        ));
        assert!(matches!(
            s.issue_challenge("user@soket.ai.evil.com"),
            Err(OtpError::Validation(_))
        ));
        // Normalized uppercase is accepted.
        assert!(s.issue_challenge("USER@SOKET.AI").is_ok());
    }

    #[test]
    fn issue_supersedes_previous_live_challenge() {
        let (s, clock, _d) = setup();
        let c1 = issue(&s);
        clock.advance(OTP_RESEND_COOLDOWN.as_millis() as i64);
        let c2 = issue(&s);
        assert_ne!(c1, c2);
        // The superseded code no longer verifies…
        assert!(matches!(
            s.verify_otp("user@soket.ai", &c1),
            Err(OtpError::WrongCode { .. })
        ));
        // …while the fresh one does.
        assert!(s.verify_otp("user@soket.ai", &c2).is_ok());
    }

    #[test]
    fn resend_cooldown_enforced() {
        let (s, clock, _d) = setup();
        issue(&s);
        match s.issue_challenge("user@soket.ai").unwrap_err() {
            OtpError::Cooldown { retry_in_ms } => {
                assert!(retry_in_ms > 0 && retry_in_ms <= OTP_RESEND_COOLDOWN.as_millis() as i64);
            }
            other => panic!("expected Cooldown, got {other:?}"),
        }
        // After the cooldown, a new challenge is issued.
        clock.advance(OTP_RESEND_COOLDOWN.as_millis() as i64);
        assert!(s.issue_challenge("user@soket.ai").is_ok());
    }

    // ---- verification ----

    #[test]
    fn correct_code_verifies_and_mints_grant() {
        let (s, _c, _d) = setup();
        let code = issue(&s);
        let grant = s.verify_otp("user@soket.ai", &code).unwrap();
        assert_eq!(grant.email, "user@soket.ai");
        assert_eq!(grant.expires_at_ms - T0, GRANT_TTL.as_millis() as i64);
        // Peek works before consumption.
        let info = s.peek_grant(&grant.token).unwrap();
        assert_eq!(info.email, "user@soket.ai");
    }

    #[test]
    fn wrong_code_reports_decreasing_attempts_left() {
        let (s, _c, _d) = setup();
        let real = issue(&s);
        let wrong = wrong_code(&real);
        // Attempts 1..=4 leave 4..=1 chances; the 5th kills the challenge.
        for expected in [4u32, 3, 2, 1, 0] {
            match s.verify_otp("user@soket.ai", wrong).unwrap_err() {
                OtpError::WrongCode { attempts_left } => {
                    assert_eq!(attempts_left, Some(expected));
                }
                other => panic!("expected WrongCode, got {other:?}"),
            }
        }
    }

    #[test]
    fn five_wrong_codes_exhaust_challenge() {
        let (s, _c, _d) = setup();
        let real = issue(&s);
        let wrong = wrong_code(&real);
        for attempt in 1..=OTP_MAX_ATTEMPTS {
            let err = s.verify_otp("user@soket.ai", wrong).unwrap_err();
            assert!(
                matches!(err, OtpError::WrongCode { .. }),
                "attempt {attempt}: {err:?}"
            );
        }
        // The challenge is dead: even the CORRECT code fails now.
        assert!(matches!(
            s.verify_otp("user@soket.ai", &real),
            Err(OtpError::NoActiveChallenge | OtpError::WrongCode { .. })
        ));
        assert_eq!(s.live_challenge_count(), 0);
    }

    #[test]
    fn expired_challenge_rejected_like_wrong_code() {
        let (s, clock, _d) = setup();
        let code = issue(&s);
        clock.advance(OTP_TTL.as_millis() as i64 + 1);
        // Even the correct code fails once expired, with the generic
        // WrongCode message (indistinguishable from a mistype).
        assert!(matches!(
            s.verify_otp("user@soket.ai", &code),
            Err(OtpError::WrongCode { .. })
        ));
    }

    #[test]
    fn challenge_single_use_even_with_correct_code() {
        let (s, _c, _d) = setup();
        let code = issue(&s);
        assert!(s.verify_otp("user@soket.ai", &code).is_ok());
        // Replaying the same (correct) code: the challenge is consumed.
        assert!(matches!(
            s.verify_otp("user@soket.ai", &code),
            Err(OtpError::NoActiveChallenge | OtpError::WrongCode { .. })
        ));
    }

    #[test]
    fn malformed_code_inputs_rejected_without_burning_attempts() {
        let (s, _c, _d) = setup();
        let real = issue(&s);
        for bad in [
            "",
            "12345",
            "1234567",
            "12345a",
            "12 456",
            "١٢٣٤٥٦",
            "12\t456",
        ] {
            assert!(
                matches!(
                    s.verify_otp("user@soket.ai", bad),
                    Err(OtpError::WrongCode {
                        attempts_left: None
                    })
                ),
                "should reject {bad:?}"
            );
        }
        // The real code still works afterwards.
        assert!(s.verify_otp("user@soket.ai", &real).is_ok());
    }

    #[test]
    fn verification_requires_registration_domain() {
        let (s, _c, _d) = setup();
        issue(&s);
        assert!(matches!(
            s.verify_otp("user@gmail.com", "123456"),
            Err(OtpError::Validation(_))
        ));
    }

    // ---- grants ----

    #[test]
    fn grant_is_single_use() {
        let (s, _c, _d) = setup();
        let code = issue(&s);
        let grant = s.verify_otp("user@soket.ai", &code).unwrap();
        let first = s.consume_grant(&grant.token).unwrap();
        assert_eq!(first.email, "user@soket.ai");
        // Second consumption fails.
        assert!(matches!(
            s.consume_grant(&grant.token),
            Err(OtpError::GrantInvalid)
        ));
        // Peek also fails after consumption.
        assert!(matches!(
            s.peek_grant(&grant.token),
            Err(OtpError::GrantInvalid)
        ));
    }

    #[test]
    fn grant_expires() {
        let (s, clock, _d) = setup();
        let code = issue(&s);
        let grant = s.verify_otp("user@soket.ai", &code).unwrap();
        clock.advance(GRANT_TTL.as_millis() as i64 + 1);
        assert!(matches!(
            s.consume_grant(&grant.token),
            Err(OtpError::GrantInvalid)
        ));
        // Peek agrees.
        assert!(matches!(
            s.peek_grant(&grant.token),
            Err(OtpError::GrantInvalid)
        ));
    }

    #[test]
    fn unknown_grant_tokens_rejected() {
        let (s, _c, _d) = setup();
        assert!(matches!(
            s.consume_grant("not-a-real-grant-token"),
            Err(OtpError::GrantInvalid)
        ));
        assert!(matches!(
            s.consume_grant(&"A".repeat(43)),
            Err(OtpError::GrantInvalid)
        ));
    }

    // ---- concurrency ----

    #[test]
    fn concurrent_verification_yields_exactly_one_success() {
        let (s, _c, _d) = setup();
        let code = issue(&s);
        let store = Arc::new(s);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let code = code.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.verify_otp("user@soket.ai", &code).is_ok()
                })
            })
            .collect();
        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(successes, 1, "exactly one verifier must win");
    }

    #[test]
    fn concurrent_grant_consumption_yields_exactly_one_success() {
        let (s, _c, _d) = setup();
        let code = issue(&s);
        let grant = s.verify_otp("user@soket.ai", &code).unwrap();
        let store = Arc::new(s);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let token = grant.token.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.consume_grant(&token).is_ok()
                })
            })
            .collect();
        let successes: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(successes, 1, "exactly one grant consumer must win");
    }

    // ---- cleanup ----

    #[test]
    fn cleanup_removes_only_dead_rows() {
        let (s, clock, _d) = setup();
        issue(&s);
        clock.advance(OTP_RESEND_COOLDOWN.as_millis() as i64);
        issue(&s); // supersedes the first
                   // Nothing is older than the retention window yet.
        let (c, g) = s.cleanup_expired().unwrap();
        assert_eq!((c, g), (0, 0));
        // Advance past expiry + retention.
        clock.advance(OTP_TTL.as_millis() as i64 + RETENTION_AFTER_DEATH + 1);
        let (c, g) = s.cleanup_expired().unwrap();
        assert!(c >= 2, "both dead challenges go, got {c}");
        assert_eq!(g, 0);
        assert_eq!(s.live_challenge_count(), 0);
        assert_eq!(s.unconsumed_grant_count(), 0);
    }

    #[test]
    fn cleanup_keeps_live_rows() {
        let (s, _c, _d) = setup();
        issue(&s);
        let (c, g) = s.cleanup_expired().unwrap();
        assert_eq!((c, g), (0, 0));
        assert_eq!(s.live_challenge_count(), 1);
    }

    // ---- key management ----

    #[test]
    fn key_created_once_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("otp.key");
        let k1 = load_or_create_key(&key_path).unwrap();
        assert_eq!(k1.len(), KEY_LEN);
        let k2 = load_or_create_key(&key_path).unwrap();
        assert_eq!(k1, k2, "existing key must never be replaced");
        // Entropy sanity: not the all-zero key.
        assert!(k1.iter().any(|b| *b != 0));
    }

    #[test]
    fn key_file_created_with_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("nested/otp.key");
        let _ = load_or_create_key(&key_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "key must be 0600");
        }
    }

    #[test]
    fn corrupt_key_is_a_hard_error_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("otp.key");
        std::fs::write(&key_path, b"short").unwrap();
        assert!(matches!(
            load_or_create_key(&key_path),
            Err(OtpError::KeyInit(_))
        ));
        // The corrupted file was NOT overwritten with a fresh key.
        assert_eq!(std::fs::read(&key_path).unwrap(), b"short");
    }

    #[test]
    fn no_plaintext_otp_in_database() {
        let (s, _c, d) = setup();
        let code = issue(&s);
        // Scan the whole DB file for the code's ASCII bytes.
        let db_bytes = std::fs::read(d.path().join("auth.db")).unwrap();
        let needle = code.as_bytes();
        assert!(
            !db_bytes.windows(needle.len()).any(|w| w == needle),
            "plaintext OTP must never touch the database"
        );
    }

    #[test]
    fn store_open_creates_schema_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");
        let key = vec![9u8; 32];
        OtpStore::open_with_key(&db, key.clone(), Arc::new(TestClock::new(T0))).unwrap();
        OtpStore::open_with_key(&db, key, Arc::new(TestClock::new(T0))).unwrap();
    }

    // ---- coexistence with the legacy auth schema ----

    #[test]
    fn otp_tables_do_not_disturb_owner_account_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");
        // Simulate the legacy auth store having created its tables with the
        // existing (grandfathered Gmail) owner row.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS owner_account (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     identifier TEXT NOT NULL UNIQUE,
                     password_hash TEXT NOT NULL,
                     created_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO owner_account VALUES
                     (1, 'owner@gmail.com', '$argon2id$fake', 0);",
            )
            .unwrap();
        }
        let store =
            OtpStore::open_with_key(&db, vec![1u8; 32], Arc::new(TestClock::new(T0))).unwrap();
        let code = store.issue_challenge("new@soket.ai").unwrap().code;
        assert!(store.verify_otp("new@soket.ai", &code).is_ok());
        // The owner row is untouched.
        let conn = Connection::open(&db).unwrap();
        let (identifier, hash): (String, String) = conn
            .query_row(
                "SELECT identifier, password_hash FROM owner_account WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(identifier, "owner@gmail.com");
        assert_eq!(hash, "$argon2id$fake");
    }
}
