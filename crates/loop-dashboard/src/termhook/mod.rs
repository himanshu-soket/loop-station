//! Terminal (Zsh) observability: secure event ingestion and hook installation.
//!
//! Design:
//! * `loop install-terminal` writes a self-contained Zsh hook library to
//!   `~/.loop/agent/hooks/loop-terminal.zsh` and sources it from `~/.zshrc`
//!   between managed markers. `loop uninstall-terminal` removes exactly that
//!   block and the library file — nothing else in the user's shell config.
//! * The hooks (`preexec` / `precmd`) capture, for every interactive command:
//!   the command text (secret-scrubbed), start/end timestamps with
//!   millisecond precision, duration, exit code, working directory, shell
//!   name/PID, hostname, and a client-generated UUID trace id that is shared
//!   by the start and completion events.
//! * Reporting is asynchronous: `precmd` fires a detached background subshell
//!   that POSTs one JSON document to the dashboard. It never blocks the
//!   prompt, never writes to the terminal, and fails silently when the
//!   dashboard is not running.
//! * Authentication: a 256-bit bearer token generated per machine, stored in
//!   `~/.loop/dashboard/loop.json` with mode 0600. Requests must present it
//!   in the `Authorization: Bearer` header; comparison is constant-time.
//!   When no token exists the endpoints answer 404, so their existence is
//!   not advertised to unauthenticated callers.
//! * Every accepted event becomes real durable storage: one trace row
//!   (operation `terminal.command`, provider `terminal`), one root span
//!   (`shell.exec`) and one request row, all keyed by the same trace id —
//!   so terminal commands appear in both Traces and Requests. Ingestion is
//!   idempotent: re-delivered events update rather than duplicate.
//! * Output capture is opt-in only: wrapping a command with `loopterm`
//!   captures stdout+stderr (size-capped, secret-scrubbed) and stores it as
//!   a `terminal.output` event on the same trace. Ordinary hooks never claim
//!   to capture output.

pub mod hook;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::persist::{DashboardStore, TraceStatus};

/// Operation recorded for every terminal command trace.
pub const TERMINAL_OPERATION: &str = "terminal.command";

/// Maximum accepted command length (characters, after scrubbing).
pub const MAX_COMMAND_LEN: usize = 2000;

/// Maximum stored captured output (bytes).
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Maximum accepted HTTP body for terminal endpoints.
const BODY_LIMIT: usize = 256 * 1024;

/// How far a client timestamp may drift from server time before rejection.
const MAX_CLOCK_SKEW_MS: i64 = 24 * 60 * 60 * 1000;

// =====================================================================
// Endpoint configuration (token + port)
// =====================================================================

/// Local endpoint/token configuration, persisted at
/// `~/.loop/dashboard/loop.json` with mode 0600.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// Config schema version.
    pub version: u32,
    /// 256-bit bearer token (64 hex characters).
    pub token: String,
    /// Port the dashboard is currently serving on (written at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// When this file was last written (unix ms).
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

impl EndpointConfig {
    fn path() -> PathBuf {
        if let Some(dir) = std::env::var_os("LOOP_DASHBOARD_DIR") {
            return PathBuf::from(dir).join("loop.json");
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".loop").join("dashboard").join("loop.json")
    }

    /// Load the existing config, if present and well-formed.
    pub fn load() -> Option<EndpointConfig> {
        let path = Self::path();
        let raw = std::fs::read_to_string(path).ok()?;
        let cfg: EndpointConfig = serde_json::from_str(&raw).ok()?;
        if cfg.is_valid() {
            Some(cfg)
        } else {
            None
        }
    }

    fn is_valid(&self) -> bool {
        !self.token.is_empty()
            && self.token.len() == 64
            && self.token.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Load the config or create one with a fresh token (mode 0600).
    pub fn ensure() -> std::io::Result<EndpointConfig> {
        if let Some(existing) = Self::load() {
            return Ok(existing);
        }
        let cfg = EndpointConfig {
            version: 1,
            token: generate_token(),
            port: None,
            updated_at_ms: now_ms(),
        };
        cfg.write()?;
        Ok(cfg)
    }

    /// Persist atomically with mode 0600.
    fn write(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        // Write via a temp sibling + rename so a crash never leaves a
        // truncated token file behind.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw.as_bytes())?;
        restrict_permissions(&tmp)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Record the port the dashboard is bound to (preserves the token).
    pub fn set_port(port: u16) -> std::io::Result<()> {
        let mut cfg = Self::ensure()?;
        cfg.port = Some(port);
        cfg.updated_at_ms = now_ms();
        cfg.write()
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Generate a 256-bit token as 64 lowercase hex characters.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS entropy source unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Constant-time bearer-token comparison. Length differences are handled
/// without early exit so timing does not leak the token length.
pub fn token_matches(stored: &str, provided: &str) -> bool {
    let a = stored.as_bytes();
    let b = provided.as_bytes();
    let max = a.len().max(b.len());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..max {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        diff |= av ^ bv;
    }
    diff == 0
}

// =====================================================================
// Secret scrubbing
// =====================================================================

/// Marker replacing redacted values.
pub const REDACTED: &str = "<redacted>";

/// Redact obvious secrets from text before persisting it. Multi-line input
/// (captured output) is scrubbed line by line so line structure survives.
///
/// Handled patterns (best-effort, applied in addition to the client-side
/// scrub in the Zsh hook):
/// * `--token <v>` / `--token=<v>` (and key/password/secret/api-key/auth…)
/// * `NAME=value` where NAME looks like a credential variable
/// * `Bearer <v>` and `Authorization:Bearer <v>` (any position)
/// * URL credentials `https://user:pass@host`
pub fn scrub_command(raw: &str) -> String {
    raw.lines().map(scrub_line).collect::<Vec<_>>().join("\n")
}

fn scrub_line(line: &str) -> String {
    let tokens: Vec<&str> = line.split(' ').filter(|t| !t.is_empty()).collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut skip_value = false;
    for tok in tokens {
        if skip_value {
            // Value of a secret flag/header seen in the previous token.
            out.push(REDACTED.to_string());
            skip_value = false;
            continue;
        }
        if let Some((name, _value)) = split_key_value(tok) {
            if is_secret_name(name) {
                out.push(format!("{name}={REDACTED}"));
                continue;
            }
        }
        if tok.starts_with('-') {
            let flag = tok.trim_start_matches('-');
            if is_secret_flag(flag) {
                if tok.contains('=') {
                    out.push(format!("{tok}={REDACTED}"));
                } else {
                    out.push(tok.to_string());
                    skip_value = true;
                }
                continue;
            }
        }
        if introduces_secret_value(tok) {
            out.push(tok.to_string());
            skip_value = true;
            continue;
        }
        out.push(scrub_url_credentials(tok));
    }
    let joined = out.join(" ");
    truncate_chars(&joined, MAX_COMMAND_LEN)
}

/// Does this token introduce a credential value in the NEXT token?
/// Covers `Bearer`, `Authorization:Bearer`, `Authorization:` headers.
fn introduces_secret_value(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    let trimmed = lower.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    trimmed == "bearer"
        || trimmed.ends_with(":bearer")
        || trimmed == "authorization"
        || trimmed.ends_with(":authorization")
}

fn split_key_value(tok: &str) -> Option<(&str, &str)> {
    let idx = tok.find('=')?;
    let name = &tok[..idx];
    let value = &tok[idx + 1..];
    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((name, value))
}

/// Does a variable/flag name look like a credential?
fn is_secret_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    for seg in lowered.split(|c: char| c == '_' || c == '-' || c == '.' || c == ':') {
        match seg {
            "key" | "token" | "secret" | "password" | "passwd" | "pass" | "auth"
            | "authorization" | "credential" | "credentials" | "apikey" | "bearer" => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Flag form: `--token`, `-token` (with or without a trailing `=`).
fn is_secret_flag(flag: &str) -> bool {
    let bare = flag.trim_end_matches('=');
    is_secret_name(bare)
}

/// Replace `scheme://user:password@host` credentials.
fn scrub_url_credentials(tok: &str) -> String {
    if let Some(scheme_end) = tok.find("://") {
        let after = &tok[scheme_end + 3..];
        if let Some(at) = after.find('@') {
            let userinfo = &after[..at];
            if !userinfo.is_empty() && userinfo.contains(':') {
                return format!(
                    "{}://{}{REDACTED}@{}",
                    &tok[..scheme_end],
                    "",
                    &after[at + 1..]
                );
            }
        }
    }
    tok.to_string()
}

/// Truncate on a character boundary, never splitting UTF-8.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(max).collect();
    cut.shrink_to_fit();
    cut
}

// =====================================================================
// Event ingestion
// =====================================================================

/// Shared state for the terminal ingestion router.
pub struct TerminalState {
    /// Durable dashboard store (traces/requests).
    pub store: Arc<DashboardStore>,
    /// Bearer token; `None` disables the endpoints (they answer 404).
    pub token: Option<Arc<str>>,
}

/// Body of `POST /api/terminal/events` (start, completion or both at once).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEvent {
    /// Client-generated trace id shared by start and completion.
    trace_id: String,
    /// Raw command text; scrubbed again server-side before storage.
    command: String,
    /// Command start (unix ms).
    started_at_ms: i64,
    /// Command end (unix ms); absent for a pure start event.
    ended_at_ms: Option<i64>,
    /// Exit status of the command.
    exit_code: Option<i32>,
    /// Working directory at execution time.
    cwd: Option<String>,
    /// Hostname.
    host: Option<String>,
    /// Shell name (`zsh`).
    shell: Option<String>,
    /// Shell process id.
    shell_pid: Option<u32>,
    /// Stable per-shell-session identifier.
    session_id: Option<String>,
}

/// Body of `POST /api/terminal/output`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput {
    trace_id: String,
    /// Captured stdout+stderr (already size-limited by the client).
    output: String,
    /// Whether the client truncated the capture.
    truncated: bool,
    /// Enough metadata to create the trace when the completion event has
    /// not arrived yet (output is reported before the command finishes).
    command: String,
    started_at_ms: i64,
    cwd: Option<String>,
    host: Option<String>,
    shell: Option<String>,
    shell_pid: Option<u32>,
    session_id: Option<String>,
}

/// Result of ingesting one terminal event.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    /// The trace id under which data was recorded.
    pub trace_id: String,
    /// Whether this call created the trace rows.
    pub created: bool,
    /// Final status, when the event completed the command.
    pub status: Option<&'static str>,
    /// Whether captured output was stored.
    pub output_stored: bool,
}

/// Validation / ingestion failure.
#[derive(Debug)]
pub enum IngestError {
    /// Payload failed validation; the message is safe to return.
    Invalid(String),
    /// Storage failure.
    Store(crate::persist::PersistError),
}

impl From<crate::persist::PersistError> for IngestError {
    fn from(e: crate::persist::PersistError) -> Self {
        IngestError::Store(e)
    }
}

impl IngestError {
    fn message(&self) -> String {
        match self {
            IngestError::Invalid(m) => m.clone(),
            IngestError::Store(e) => format!("storage error: {e}"),
        }
    }
    fn status(&self) -> StatusCode {
        match self {
            IngestError::Invalid(_) => StatusCode::BAD_REQUEST,
            IngestError::Store(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// Validate a client trace id: 8–64 chars of `[A-Za-z0-9_-]`.
fn valid_trace_id(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Map an exit code to a trace status.
///
/// Zsh reports signal deaths as 128+N: SIGINT (2) → 130 and SIGTERM (15) →
/// 143 are user-initiated cancellations; other signals are errors.
pub fn status_for_exit_code(exit_code: i32) -> (TraceStatus, Option<String>) {
    match exit_code {
        0 => (TraceStatus::Success, None),
        130 => (
            TraceStatus::CancelledUser,
            Some("interrupted (SIGINT)".into()),
        ),
        143 => (
            TraceStatus::CancelledUser,
            Some("terminated (SIGTERM)".into()),
        ),
        code @ 129..=192 => (
            TraceStatus::Error,
            Some(format!("terminated by signal {}", code - 128)),
        ),
        code => (TraceStatus::Error, Some(format!("exit code {code}"))),
    }
}

/// Attributes attached to the root span of a terminal trace.
fn span_attributes(command: &str, exit_code: Option<i32>) -> String {
    serde_json::json!({
        "command": command,
        "exitCode": exit_code,
        "origin": "terminal",
    })
    .to_string()
}

/// Core ingestion: validate, upsert the trace, optionally append output and
/// finalize. All storage work happens on the caller's (blocking) thread.
pub fn ingest_event(
    store: &DashboardStore,
    event: &TerminalEvent,
    output: Option<(&str, bool)>,
) -> Result<IngestOutcome, IngestError> {
    if !valid_trace_id(&event.trace_id) {
        return Err(IngestError::Invalid(
            "traceId must be 8-64 characters of [A-Za-z0-9_-]".into(),
        ));
    }
    let command = scrub_command(event.command.trim());
    if command.is_empty() {
        return Err(IngestError::Invalid("command must not be empty".into()));
    }
    let now = now_ms();
    if (event.started_at_ms - now).abs() > MAX_CLOCK_SKEW_MS {
        return Err(IngestError::Invalid(
            "startedAtMs is too far from server time".into(),
        ));
    }
    if let Some(ended) = event.ended_at_ms {
        if ended < event.started_at_ms {
            return Err(IngestError::Invalid(
                "endedAtMs precedes startedAtMs".into(),
            ));
        }
    }
    let session = event
        .session_id
        .clone()
        .unwrap_or_else(|| format!("terminal:{}", event.shell_pid.unwrap_or(0)));

    // Upsert the trace (idempotent by trace id).
    let created = store.insert_terminal_trace(
        &event.trace_id,
        &command,
        event.cwd.as_deref(),
        event.host.as_deref(),
        event.shell.as_deref(),
        event.shell_pid,
        &session,
        event.started_at_ms,
    )?;

    // Optional captured output (only via loopterm).
    let mut output_stored = false;
    if let Some((output, truncated)) = output {
        let scrubbed = scrub_command(output);
        let capped = cap_output_bytes(&scrubbed);
        let data = serde_json::json!({
            "output": capped.0,
            "truncated": truncated || capped.1,
            "origin": "loopterm",
        });
        store.append_terminal_event(
            &event.trace_id,
            now_ms(),
            "terminal.output",
            &data.to_string(),
        )?;
        output_stored = true;
    }

    // Completion: finalize trace + span + request.
    let mut status = None;
    if let Some(ended) = event.ended_at_ms {
        let duration = (ended - event.started_at_ms).max(0);
        let exit = event.exit_code.unwrap_or(0);
        let (st, err) = status_for_exit_code(exit);
        store.finalize_terminal_trace(
            &event.trace_id,
            st,
            ended,
            duration,
            err.as_deref(),
            output_stored,
            &span_attributes(&command, Some(exit)),
        )?;
        status = Some(st.as_str());
    }

    Ok(IngestOutcome {
        trace_id: event.trace_id.clone(),
        created,
        status,
        output_stored,
    })
}

/// Ingest captured output: ensures the trace exists, then appends the
/// `terminal.output` event.
pub fn ingest_output(
    store: &DashboardStore,
    out: &TerminalOutput,
) -> Result<IngestOutcome, IngestError> {
    let event = TerminalEvent {
        trace_id: out.trace_id.clone(),
        command: out.command.clone(),
        started_at_ms: out.started_at_ms,
        ended_at_ms: None,
        exit_code: None,
        cwd: out.cwd.clone(),
        host: out.host.clone(),
        shell: out.shell.clone(),
        shell_pid: out.shell_pid,
        session_id: out.session_id.clone(),
    };
    ingest_event(store, &event, Some((&out.output, out.truncated)))
}

/// Cap captured output to [`MAX_OUTPUT_BYTES`] on a UTF-8 boundary.
fn cap_output_bytes(s: &str) -> (String, bool) {
    if s.len() <= MAX_OUTPUT_BYTES {
        return (s.to_string(), false);
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

// =====================================================================
// HTTP layer
// =====================================================================

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    Some(rest.trim().to_string())
}

fn auth_error() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "invalid terminal token" })),
    )
        .into_response()
}

/// `POST /api/terminal/events`
///
/// Body parsing happens *after* authentication so unauthenticated callers
/// never get payload-shaped feedback (404/401 outrank 400/422).
async fn events_handler(
    State(state): State<Arc<TerminalState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(token) = state.token.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not found"})),
        )
            .into_response();
    };
    let Some(provided) = bearer_token(&headers) else {
        return auth_error();
    };
    if !token_matches(token, &provided) {
        return auth_error();
    }
    let event: TerminalEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid JSON body: {e}") })),
            )
                .into_response()
        }
    };
    let store = Arc::clone(&state.store);
    let result = tokio::task::spawn_blocking(move || ingest_event(&store, &event, None))
        .await
        .unwrap_or_else(|e| Err(IngestError::Invalid(format!("ingestion task failed: {e}"))));
    match result {
        Ok(outcome) => {
            (StatusCode::OK, Json(serde_json::to_value(outcome).unwrap())).into_response()
        }
        Err(e) => (
            e.status(),
            Json(serde_json::json!({ "error": e.message() })),
        )
            .into_response(),
    }
}

/// `POST /api/terminal/output`
async fn output_handler(
    State(state): State<Arc<TerminalState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(token) = state.token.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not found"})),
        )
            .into_response();
    };
    let Some(provided) = bearer_token(&headers) else {
        return auth_error();
    };
    if !token_matches(token, &provided) {
        return auth_error();
    }
    let payload: TerminalOutput = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid JSON body: {e}") })),
            )
                .into_response()
        }
    };
    let store = Arc::clone(&state.store);
    let result = tokio::task::spawn_blocking(move || ingest_output(&store, &payload))
        .await
        .unwrap_or_else(|e| Err(IngestError::Invalid(format!("ingestion task failed: {e}"))));
    match result {
        Ok(outcome) => {
            (StatusCode::OK, Json(serde_json::to_value(outcome).unwrap())).into_response()
        }
        Err(e) => (
            e.status(),
            Json(serde_json::json!({ "error": e.message() })),
        )
            .into_response(),
    }
}

/// Build the terminal ingestion router. Mounts `/api/terminal/events` and
/// `/api/terminal/output` with a strict body limit. These paths are exempt
/// from the cookie guard because curl cannot hold a browser session — they
/// carry their own bearer authentication.
pub fn terminal_router(state: Arc<TerminalState>) -> Router {
    Router::new()
        .route("/api/terminal/events", post(events_handler))
        .route("/api/terminal/output", post(output_handler))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> DashboardStore {
        DashboardStore::open_in_memory().unwrap()
    }

    fn event(trace_id: &str, exit: Option<i32>, start: i64, end: Option<i64>) -> TerminalEvent {
        TerminalEvent {
            trace_id: trace_id.to_string(),
            command: "cargo test --workspace".into(),
            started_at_ms: start,
            ended_at_ms: end,
            exit_code: exit,
            cwd: Some("/Users/dev/loop".into()),
            host: Some("devbox.local".into()),
            shell: Some("zsh".into()),
            shell_pid: Some(4242),
            session_id: Some("terminal:4242".into()),
        }
    }

    #[test]
    fn token_is_64_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn token_comparison_is_exact() {
        let t = "a".repeat(64);
        assert!(token_matches(&t, &t));
        assert!(!token_matches(&t, &format!("{}x", &t[..63])));
        assert!(!token_matches(&t, ""));
        assert!(!token_matches("", &t));
        assert!(!token_matches(&t, &t.to_uppercase()));
    }

    #[test]
    fn endpoint_config_round_trip_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        // Scoped env change is fine in tests: serial execution within this
        // test binary's process is guaranteed by the single-threaded runner.
        std::env::set_var("LOOP_DASHBOARD_DIR", dir.path());
        let first = EndpointConfig::ensure().unwrap();
        assert_eq!(first.token.len(), 64);
        if cfg!(unix) {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("loop.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "token file must be 0600");
        }
        // ensure() preserves an existing token
        let second = EndpointConfig::ensure().unwrap();
        assert_eq!(first.token, second.token);
        // set_port preserves the token too
        EndpointConfig::set_port(4173).unwrap();
        let third = EndpointConfig::load().unwrap();
        assert_eq!(third.token, first.token);
        assert_eq!(third.port, Some(4173));
        std::env::remove_var("LOOP_DASHBOARD_DIR");
    }

    #[test]
    fn scrub_redacts_common_secrets() {
        let out = scrub_command("curl -H Authorization:Bearer eyJhbGciOi --token abc123 https://u:secretpw@db.internal/x API_KEY=sk-live-999 deploy");
        assert!(!out.contains("eyJhbGciOi"), "bearer token leaked: {out}");
        assert!(!out.contains("abc123"), "flag value leaked: {out}");
        assert!(!out.contains("secretpw"), "url password leaked: {out}");
        assert!(!out.contains("sk-live-999"), "env var leaked: {out}");
        assert!(out.contains("<redacted>"), "no redaction marker: {out}");
        // benign parts survive
        assert!(out.contains("curl"));
        assert!(out.contains("deploy"));
        assert!(out.contains("db.internal/x"));
    }

    #[test]
    fn scrub_keeps_benign_commands_intact() {
        for cmd in [
            "cargo test --workspace",
            "git status --short",
            "python train.py --epochs 10 --lr 0.001",
            "ls -la /tmp",
        ] {
            assert_eq!(scrub_command(cmd), cmd, "unexpected mutation of {cmd}");
        }
    }

    #[test]
    fn scrub_truncates_long_commands() {
        let long = "x".repeat(5000);
        let out = scrub_command(&long);
        assert!(out.chars().count() <= MAX_COMMAND_LEN);
    }

    #[test]
    fn exit_status_mapping() {
        assert!(matches!(status_for_exit_code(0).0, TraceStatus::Success));
        let (s, _) = status_for_exit_code(130);
        assert!(matches!(s, TraceStatus::CancelledUser));
        let (s, _) = status_for_exit_code(143);
        assert!(matches!(s, TraceStatus::CancelledUser));
        let (s, m) = status_for_exit_code(139);
        assert!(matches!(s, TraceStatus::Error));
        assert!(m.unwrap().contains("signal 11"));
        let (s, m) = status_for_exit_code(2);
        assert!(matches!(s, TraceStatus::Error));
        assert_eq!(m.unwrap(), "exit code 2");
    }

    #[test]
    fn ingestion_creates_trace_span_request() {
        let store = store();
        let start = now_ms() - 1000;
        let outcome =
            ingest_event(&store, &event("trace-aaaa-0001", None, start, None), None).unwrap();
        assert!(outcome.created);
        assert!(outcome.status.is_none());

        let detail = store.get_trace_detail("trace-aaaa-0001").unwrap().unwrap();
        assert_eq!(detail.trace.operation, TERMINAL_OPERATION);
        assert_eq!(detail.trace.provider.as_deref(), Some("terminal"));
        assert_eq!(detail.trace.request_id.as_deref(), Some("trace-aaaa-0001"));
        assert_eq!(detail.spans.len(), 1);
        assert_eq!(detail.spans[0].name, "shell.exec");
        assert_eq!(detail.spans[0].kind, "phase");

        let (reqs, total) = store
            .list_requests(&crate::persist::RequestFilter {
                q: "trace-aaaa-0001".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(reqs[0].provider.as_deref(), Some("terminal"));
    }

    #[test]
    fn completion_finalizes_all_rows() {
        let store = store();
        let start = now_ms() - 5000;
        ingest_event(&store, &event("trace-bbbb-0002", None, start, None), None).unwrap();
        let outcome = ingest_event(
            &store,
            &event("trace-bbbb-0002", Some(0), start, Some(start + 1500)),
            None,
        )
        .unwrap();
        assert!(!outcome.created, "duplicate must not create rows");
        assert_eq!(outcome.status, Some("success"));

        let detail = store.get_trace_detail("trace-bbbb-0002").unwrap().unwrap();
        assert_eq!(detail.trace.status.as_str(), "success");
        assert_eq!(detail.trace.duration_ms, Some(1500));
        assert_eq!(detail.trace.ended_at_ms, Some(start + 1500));
        assert_eq!(detail.spans[0].duration_ms, Some(1500));
        assert!(detail.spans[0]
            .attributes_json
            .as_deref()
            .unwrap()
            .contains("\"exitCode\":0"));

        let (reqs, _) = store
            .list_requests(&crate::persist::RequestFilter {
                q: "trace-bbbb-0002".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reqs[0].status.as_str(), "success");
        assert_eq!(reqs[0].duration_ms, Some(1500));
    }

    #[test]
    fn failed_and_cancelled_commands_map_correctly() {
        let store = store();
        let start = now_ms() - 1000;
        ingest_event(
            &store,
            &event("trace-cccc-0003", Some(1), start, Some(start + 80)),
            None,
        )
        .unwrap();
        let t = store.get_trace("trace-cccc-0003").unwrap().unwrap();
        assert_eq!(t.status.as_str(), "error");
        assert_eq!(t.error_message.as_deref(), Some("exit code 1"));

        ingest_event(
            &store,
            &event("trace-dddd-0004", Some(130), start, Some(start + 40)),
            None,
        )
        .unwrap();
        let t = store.get_trace("trace-dddd-0004").unwrap().unwrap();
        assert_eq!(t.status.as_str(), "cancelled_user");
    }

    #[test]
    fn late_completion_without_start_is_recovered() {
        // precmd fired but preexec's report was lost (dashboard down): the
        // completion event alone must still produce complete rows.
        let store = store();
        let start = now_ms() - 2000;
        let outcome = ingest_event(
            &store,
            &event("trace-eeee-0005", Some(0), start, Some(start + 300)),
            None,
        )
        .unwrap();
        assert!(outcome.created);
        let t = store.get_trace("trace-eeee-0005").unwrap().unwrap();
        assert_eq!(t.status.as_str(), "success");
        assert_eq!(t.duration_ms, Some(300));
    }

    #[test]
    fn duplicate_events_do_not_duplicate_rows() {
        let store = store();
        let start = now_ms() - 1000;
        for _ in 0..3 {
            ingest_event(
                &store,
                &event("trace-ffff-0006", Some(0), start, Some(start + 10)),
                None,
            )
            .unwrap();
        }
        let (traces, total_t) = store
            .list_traces(&crate::persist::TraceFilter {
                q: "trace-ffff-0006".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total_t, 1, "duplicate traces created: {traces:?}");
        let (_, total_r) = store
            .list_requests(&crate::persist::RequestFilter {
                q: "trace-ffff-0006".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(total_r, 1);
        let detail = store.get_trace_detail("trace-ffff-0006").unwrap().unwrap();
        assert_eq!(detail.spans.len(), 1, "duplicate spans created");
        assert!(detail.events.is_empty(), "no output events expected");
    }

    #[test]
    fn invalid_payloads_are_rejected() {
        let store = store();
        let start = now_ms();

        // bad trace ids
        for bad in ["short", "has spaces in it!", "x".repeat(65).as_str()] {
            let mut e = event(bad, Some(0), start, Some(start + 1));
            e.trace_id = bad.to_string();
            let err = ingest_event(&store, &e, None).unwrap_err();
            assert!(matches!(err, IngestError::Invalid(_)), "{bad}");
        }
        // empty command
        let mut e = event("trace-gggg-0007", Some(0), start, Some(start + 1));
        e.command = "   ".into();
        assert!(ingest_event(&store, &e, None).is_err());
        // end before start
        let e = event("trace-hhhh-0008", Some(0), start, Some(start - 100));
        assert!(ingest_event(&store, &e, None).is_err());
        // implausible clock skew
        let e = event(
            "trace-iiii-0009",
            Some(0),
            start - MAX_CLOCK_SKEW_MS * 2,
            None,
        );
        assert!(ingest_event(&store, &e, None).is_err());
        // after all rejections, nothing was stored
        let (_, total) = store
            .list_traces(&crate::persist::TraceFilter::default())
            .unwrap();
        assert_eq!(total, 0);
    }

    #[test]
    fn output_capture_stores_scrubbed_event() {
        let store = store();
        let start = now_ms() - 1000;
        let payload = TerminalOutput {
            trace_id: "trace-jjjj-0010".into(),
            output: "BUILD OK\nAPI_TOKEN=super-secret-value\ndone".into(),
            truncated: false,
            command: "loopterm make build".into(),
            started_at_ms: start,
            cwd: Some("/Users/dev/loop".into()),
            host: Some("devbox.local".into()),
            shell: Some("zsh".into()),
            shell_pid: Some(4242),
            session_id: Some("terminal:4242".into()),
        };
        let outcome = ingest_output(&store, &payload).unwrap();
        assert!(outcome.created);
        assert!(outcome.output_stored);

        let detail = store.get_trace_detail("trace-jjjj-0010").unwrap().unwrap();
        assert_eq!(detail.events.len(), 1);
        assert_eq!(detail.events[0].name, "terminal.output");
        let data: serde_json::Value =
            serde_json::from_str(detail.events[0].data_json.as_deref().unwrap()).unwrap();
        assert_eq!(data["output"], "BUILD OK\nAPI_TOKEN=<redacted>\ndone");

        // completing the same trace later must keep the output event
        ingest_event(
            &store,
            &event("trace-jjjj-0010", Some(0), start, Some(start + 900)),
            None,
        )
        .unwrap();
        let detail = store.get_trace_detail("trace-jjjj-0010").unwrap().unwrap();
        assert_eq!(detail.events.len(), 1);
        assert_eq!(detail.trace.status.as_str(), "success");
        assert!(detail.trace.content_captured);
    }

    #[test]
    fn oversized_output_is_capped() {
        let big = "y".repeat(MAX_OUTPUT_BYTES + 10_000);
        let (capped, truncated) = cap_output_bytes(&big);
        assert!(capped.len() <= MAX_OUTPUT_BYTES);
        assert!(truncated);
        assert!(capped.ends_with('y'));
    }

    // -----------------------------------------------------------------
    // HTTP contract
    // -----------------------------------------------------------------

    async fn post_json(
        app: &Router,
        uri: &str,
        token: Option<&str>,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let res = app
            .clone()
            .oneshot(
                builder
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn http_auth_contract() {
        let store = Arc::new(store());
        let token: Arc<str> = Arc::from("t".repeat(64));
        let app = terminal_router(Arc::new(TerminalState {
            store,
            token: Some(token.clone()),
        }));

        let now = now_ms();
        let body = serde_json::json!({
            "traceId": "trace-kkkk-0011",
            "command": "git status",
            "startedAtMs": now - 50,
            "endedAtMs": now,
            "exitCode": 0,
            "cwd": "/tmp",
            "shell": "zsh",
            "shellPid": 99,
        });

        // no token -> 401
        let (status, body_out) =
            post_json(&app, "/api/terminal/events", None, &body.to_string()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body_out:?}");
        // wrong token -> 401
        let (status, _) = post_json(
            &app,
            "/api/terminal/events",
            Some(&"w".repeat(64)),
            &body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // correct token -> 200 + real rows
        let (status, out) = post_json(
            &app,
            "/api/terminal/events",
            Some(&token.to_string()),
            &body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{out:?}");
        assert_eq!(out["status"], "success");
        assert_eq!(out["traceId"], "trace-kkkk-0011");

        // malformed JSON -> 400
        let (status, _) = post_json(
            &app,
            "/api/terminal/events",
            Some(&token.to_string()),
            "{not json",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // invalid payload (bad trace id) -> 400 with a message
        let bad = serde_json::json!({
            "traceId": "x",
            "command": "ls",
            "startedAtMs": now,
        });
        let (status, err) = post_json(
            &app,
            "/api/terminal/events",
            Some(&token.to_string()),
            &bad.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{err:?}");
        assert!(err["error"].as_str().unwrap().contains("traceId"));

        // GET is not allowed
        use tower::ServiceExt;
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/terminal/events")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn http_disabled_when_no_token() {
        let store = Arc::new(store());
        let app = terminal_router(Arc::new(TerminalState { store, token: None }));
        let (status, _) = post_json(&app, "/api/terminal/events", Some("anything"), "{}").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn http_oversized_body_rejected() {
        let store = Arc::new(store());
        let token: Arc<str> = Arc::from("t".repeat(64));
        let app = terminal_router(Arc::new(TerminalState {
            store,
            token: Some(token.clone()),
        }));
        let huge = format!(
            "{{\"traceId\":\"trace-llll-0012\",\"command\":\"{}\",\"startedAtMs\":{}}}",
            "z".repeat(BODY_LIMIT + 100),
            now_ms()
        );
        let (status, _) = post_json(
            &app,
            "/api/terminal/events",
            Some(&token.to_string()),
            &huge,
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
