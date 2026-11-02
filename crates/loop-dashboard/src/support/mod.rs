//! Header-based support & ticket management (Support phase).
//!
//! Design notes
//! ------------
//! * **One header button, two experiences.** Normal users get "Raise a
//!   Request" (submit + track their own tickets); administrators get
//!   "Support Management" (all tickets, replies, status changes, private
//!   notes). The server decides which — the client only renders what its
//!   role probe says it may see.
//! * **Authorization is server-side and per-request.** Every handler here
//!   resolves the caller's CURRENT role from the session store, exactly
//!   like [`crate::admin_http::require_admin`]. Frontend role information
//!   is never trusted; a normal user who somehow loads the admin UI still
//!   receives 403 from every administrative endpoint.
//! * **Ownership scoping.** A normal user's list/detail requests are
//!   forcibly scoped to their own `user_id`; an attempt to read another
//!   user's ticket yields 404 (not 403) so ids cannot be probed.
//! * **Private notes never leave the admin surface.** They are stored as
//!   `internal_note` events and filtered out of every user-facing
//!   response in this module — by construction, not by convention.
//! * **Email is best-effort after commit.** Tickets are persisted first;
//!   notification failures are logged and reported in the response but
//!   never roll back or lose the submission.

pub mod store;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::{
    SessionInfo, ADMIN_ROLE, STATUS_APPROVED,
};
use crate::auth_http::AuthState;
use store::{
    is_allowed_attachment_type, is_valid_category, is_valid_priority, is_valid_status,
    AttachmentMeta, SupportStore, SupportStoreError, TicketFilter, MAX_ATTACHMENTS_PER_TICKET,
    MAX_ATTACHMENT_BYTES, MAX_DESCRIPTION_LEN, MAX_FILENAME_LEN, MAX_SUBJECT_LEN,
};

/// Shared state for these routes.
pub struct SupportState {
    /// Authentication store: resolves sessions and roles.
    pub auth: Arc<AuthState>,
    /// Ticket storage.
    pub tickets: Arc<SupportStore>,
    /// Email delivery, when configured (`LOOP_EMAIL_*`).
    pub mailer: Option<Arc<dyn crate::mail::MailSender>>,
    /// Destination for new-ticket notifications (administrators).
    pub support_email: Option<String>,
}

/// Body limit for ticket submissions. Attachments ride inside the JSON body
/// as base64 (≈1.37× the decoded size), so this covers the maximum legal
/// payload plus headroom.
const SUBMIT_BODY_LIMIT: usize =
    (MAX_ATTACHMENT_BYTES * MAX_ATTACHMENTS_PER_TICKET * 2) + 64 * 1024;

/// Body limit for admin actions (replies/notes): text only.
const ACTION_BODY_LIMIT: usize = 32 * 1024;

/// Rate limiting: ticket submissions per user per window.
const SUBMIT_WINDOW_MS: i64 = 60 * 60 * 1000; // one hour

/// Maximum submissions per user per window.
const SUBMIT_MAX_PER_WINDOW: usize = 10;

/// Router for `/api/support/*`.
///
/// Mounted under the app's existing guard layer, which already requires a
/// valid session for every `/api/*` path outside `/api/auth/*`. Each
/// handler here adds the role/ownership checks on top.
pub fn support_router(state: Arc<SupportState>) -> Router {
    Router::new()
        // Role probe: which experience does this caller get?
        .route("/api/support/me", get(my_scope))
        // User surface: submit + list own tickets.
        .route("/api/support/tickets", get(list_my_tickets).post(submit_ticket))
        // One ticket: detail (ownership/admin checked inside).
        .route("/api/support/tickets/{id}", get(ticket_detail))
        // Attachment download (ownership/admin checked inside).
        .route(
            "/api/support/attachments/{id}",
            get(download_attachment),
        )
        // Admin surface: everything below requires the administrator role.
        .route("/api/support/admin/summary", get(admin_summary))
        .route("/api/support/admin/tickets", get(admin_list_tickets))
        .route(
            "/api/support/admin/tickets/{id}/status",
            post(admin_set_status),
        )
        .route(
            "/api/support/admin/tickets/{id}/reply",
            post(admin_reply),
        )
        .route(
            "/api/support/admin/tickets/{id}/note",
            post(admin_internal_note),
        )
        .layer(DefaultBodyLimit::max(SUBMIT_BODY_LIMIT.max(ACTION_BODY_LIMIT)))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Authorization helpers
// ---------------------------------------------------------------------------

/// The caller resolved fresh from the session cookie, with their CURRENT
/// role re-read from storage.
enum Caller {
    /// Authenticated administrator (approved, `role = admin`).
    Admin { id: i64, identifier: String },
    /// Authenticated ordinary user.
    User { id: i64, identifier: String },
}

/// Why a request was refused. Deliberately coarse, mirroring
/// `admin_http::AdminAuthError`.
enum SupportAuthError {
    Unauthenticated,
    Forbidden,
    Internal,
}

impl SupportAuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "authentication required",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "administrator access required"),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "support service unavailable",
            ),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

fn token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::auth::session_token_from_cookie_header)
}

/// Resolve the caller's session and CURRENT account role.
///
/// Mirrors `admin_http::require_admin`: three checks (session validity,
/// account existence, current role/status), each answered with a distinct
/// coarse status. The role is re-read from storage on every call — never
/// trusted from the request.
async fn resolve_caller(state: &SupportState, headers: &HeaderMap) -> Result<Caller, SupportAuthError> {
    let Some(token) = token_from_headers(headers) else {
        return Err(SupportAuthError::Unauthenticated);
    };
    let token = token.to_string();
    let store = state.auth.store.clone();
    let Some(session): Option<SessionInfo> =
        tokio::task::spawn_blocking(move || store.validate_session(&token))
            .await
            .ok()
            .flatten()
    else {
        return Err(SupportAuthError::Unauthenticated);
    };
    let auth_store = state.auth.store.clone();
    let key = session.identifier.clone();
    let account = tokio::task::spawn_blocking(move || {
        auth_store.find_account_by_identifier(&key)
    })
    .await
    .map_err(|_| SupportAuthError::Internal)?
    .map_err(|_| SupportAuthError::Internal)?;
    if account.status != STATUS_APPROVED {
        return Err(SupportAuthError::Forbidden);
    }
    if account.role == ADMIN_ROLE {
        Ok(Caller::Admin {
            id: account.id,
            identifier: account.identifier,
        })
    } else {
        Ok(Caller::User {
            id: account.id,
            identifier: account.identifier,
        })
    }
}

/// Require an authenticated administrator.
async fn require_admin(
    state: &SupportState,
    headers: &HeaderMap,
) -> Result<(i64, String), SupportAuthError> {
    match resolve_caller(state, headers).await {
        Ok(Caller::Admin { id, identifier }) => Ok((id, identifier)),
        Ok(Caller::User { .. }) => Err(SupportAuthError::Forbidden),
        Err(e) => Err(e),
    }
}

/// Map a storage error to a safe HTTP response (no SQLite internals).
fn store_error_response(e: SupportStoreError) -> Response {
    (
        e.status(),
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}

/// Process-local sliding-window rate limiter for ticket submissions.
///
/// Simple and honest: a Vec of (user_id, at_ms) pruned to the window. With
/// a local dashboard's volume this stays tiny; the database is not the
/// right place for a limiter that must reject BEFORE any write.
fn submission_rate_limiter() -> &'static parking_lot::Mutex<Vec<(i64, i64)>> {
    static LIMITER: std::sync::OnceLock<parking_lot::Mutex<Vec<(i64, i64)>>> =
        std::sync::OnceLock::new();
    LIMITER.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

/// Record a submission and report whether it was allowed.
fn allow_submission(user_id: i64) -> bool {
    let now = chrono::Utc::now().timestamp_millis();
    let mut window = submission_rate_limiter().lock();
    window.retain(|(_, at)| now - *at < SUBMIT_WINDOW_MS);
    let mine = window.iter().filter(|(uid, _)| *uid == user_id).count();
    if mine >= SUBMIT_MAX_PER_WINDOW {
        return false;
    }
    window.push((user_id, now));
    true
}

// ---------------------------------------------------------------------------
// Handlers: shared probes
// ---------------------------------------------------------------------------

/// `GET /api/support/me`
///
/// Tells the SPA which experience to render. The role comes from the
/// server session; the client uses it ONLY to pick UI, never for
/// authorization (every endpoint below re-checks).
async fn my_scope(State(state): State<Arc<SupportState>>, headers: HeaderMap) -> Response {
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let (is_admin, identifier, user_id) = match caller {
        Caller::Admin { id, identifier } => (true, identifier, id),
        Caller::User { id, identifier } => (false, identifier, id),
    };
    // Admins also get the pending-ticket badge count.
    let open_count = if is_admin {
        tokio::task::spawn_blocking({
            let tickets = state.tickets.clone();
            move || tickets.open_count()
        })
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(0)
    } else {
        0
    };
    (
        StatusCode::OK,
        [(
            header::CACHE_CONTROL,
            "no-store".to_string(),
        )],
        Json(serde_json::json!({
            "isAdmin": is_admin,
            "identifier": identifier,
            "userId": user_id,
            "openTickets": open_count,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Handlers: user surface
// ---------------------------------------------------------------------------

/// `POST /api/support/tickets` — submit a new ticket.
///
/// Full Name and Email are populated from the AUTHENTICATED account, never
/// from the request body: the form shows them pre-filled and read-only.
#[derive(Deserialize)]
struct SubmitTicketRequest {
    subject: String,
    category: String,
    priority: String,
    description: String,
    #[serde(default)]
    attachments: Vec<SubmittedAttachment>,
}

#[derive(Deserialize)]
struct SubmittedAttachment {
    filename: String,
    #[serde(rename = "contentType")]
    content_type: String,
    /// Base64 (standard alphabet, with or without padding).
    data: String,
}

async fn submit_ticket(
    State(state): State<Arc<SupportState>>,
    headers: HeaderMap,
    body: axum::extract::Request<Body>,
) -> Response {
    // 1. Authenticate first; no anonymous submissions ever reach storage.
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let (user_id, user_email) = match &caller {
        Caller::Admin { id, identifier } => (*id, identifier.clone()),
        Caller::User { id, identifier } => (*id, identifier.clone()),
    };

    // 2. Bound the body before parsing (defense in depth; the router layer
    //    already caps it, this keeps the handler honest on its own).
    let bytes = match axum::body::to_bytes(body.into_body(), SUBMIT_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response()
        }
    };
    let req: SubmitTicketRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid request body" })),
            )
                .into_response()
        }
    };

    // 3. Validate every field server-side.
    let subject = req.subject.trim().to_string();
    if subject.is_empty() {
        return bad_request("subject is required");
    }
    if subject.chars().count() > MAX_SUBJECT_LEN {
        return bad_request(format!(
            "subject must be at most {MAX_SUBJECT_LEN} characters"
        ));
    }
    if !is_valid_category(&req.category) {
        return bad_request("unknown category");
    }
    if !is_valid_priority(&req.priority) {
        return bad_request("unknown priority");
    }
    let description = req.description.trim().to_string();
    if description.is_empty() {
        return bad_request("description is required");
    }
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return bad_request(format!(
            "description must be at most {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    if req.attachments.len() > MAX_ATTACHMENTS_PER_TICKET {
        return bad_request(format!(
            "at most {MAX_ATTACHMENTS_PER_TICKET} attachments are allowed"
        ));
    }

    // 4. Validate + decode attachments server-side.
    let mut attachments: Vec<(String, String, Vec<u8>)> = Vec::new();
    for att in &req.attachments {
        let filename = att.filename.trim().to_string();
        if filename.is_empty() {
            return bad_request("attachment filename is required");
        }
        if filename.chars().count() > MAX_FILENAME_LEN {
            return bad_request(format!(
                "attachment filename must be at most {MAX_FILENAME_LEN} characters"
            ));
        }
        // Strip any path components the client may have sent: the stored
        // name is a display label, never a filesystem path.
        let filename = filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_string();
        if !is_allowed_attachment_type(&att.content_type) {
            return bad_request(format!(
                "attachments of type {} are not supported",
                att.content_type
            ));
        }
        let data = match base64_decode(&att.data) {
            Ok(d) => d,
            Err(_) => return bad_request("attachment data is not valid base64"),
        };
        if data.len() > MAX_ATTACHMENT_BYTES {
            return bad_request(format!(
                "each attachment must be at most {} MiB",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
        }
        if data.is_empty() {
            return bad_request("attachment is empty");
        }
        attachments.push((filename, att.content_type.trim().to_lowercase(), data));
    }

    // 5. Rate limit AFTER validation (invalid requests are free) but
    //    BEFORE any write.
    if !allow_submission(user_id) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "3600".to_string())],
            Json(serde_json::json!({
                "error": format!(
                    "too many requests; at most {SUBMIT_MAX_PER_WINDOW} per hour"
                )
            })),
        )
            .into_response();
    }

    // 6. Persist FIRST: a later email failure must never lose the ticket.
    //    Display name: the account identifier is the only name the system
    //    knows; the form shows it pre-filled and read-only.
    let tickets = state.tickets.clone();
    let created = tokio::task::spawn_blocking(move || {
        tickets.create_ticket(
            user_id,
            &user_email,
            &user_email, // display name snapshot = identifier
            &subject,
            &req.category,
            &req.priority,
            &description,
            &attachments,
        )
    })
    .await;
    let row = match created {
        Ok(Ok(row)) => row,
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };

    // 7. Best-effort notifications. Failures are logged and surfaced as a
    //    warning in the response; the ticket itself is already durable.
    let mut email_status = "sent";
    if let Some(mailer) = state.mailer.clone() {
        let user_msg = crate::mail::SupportTicketMessage::confirmation(
            row.user_email.clone(),
            &row.ticket_id,
            &row.subject,
            &row.status,
        );
        let m = mailer.clone();
        let user_send = tokio::task::spawn_blocking(move || m.send_support_notification(&user_msg));
        let support_msg = crate::mail::SupportTicketMessage::notify_support(
            state.support_email.clone().unwrap_or_default(),
            &row.ticket_id,
            &row.subject,
            &row.user_name,
            &row.user_email,
            &row.priority,
            &row.category,
        );
        let m2 = mailer.clone();
        let support_send =
            tokio::task::spawn_blocking(move || m2.send_support_notification(&support_msg));
        let ok_user = matches!(user_send.await, Ok(Ok(())));
        let ok_support = matches!(support_send.await, Ok(Ok(())));
        if !ok_user || !ok_support {
            eprintln!(
                "warning: support ticket {} saved, but notification delivery failed \
                 (user={}, support={ok_support})",
                row.ticket_id, ok_user
            );
            email_status = "ticket saved; email delivery failed";
        }
    } else {
        email_status = "ticket saved; email delivery is not configured";
    }

    (
        StatusCode::CREATED,
        [(
            header::CACHE_CONTROL,
            "no-store".to_string(),
        )],
        Json(serde_json::json!({
            "ok": true,
            "ticket": row.to_json(),
            "emailStatus": email_status,
        })),
    )
        .into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message.into() })),
    )
        .into_response()
}

/// Decode standard base64 (padded or unpadded) into bytes.
fn base64_decode(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    // Standard alphabet first; fall back to URL-safe for robustness.
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cleaned.as_bytes())
        })
}

/// `GET /api/support/tickets` — the caller's OWN tickets, newest first.
#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

async fn list_my_tickets(
    State(state): State<Arc<SupportState>>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Response {
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let user_id = match &caller {
        Caller::Admin { id, .. } | Caller::User { id, .. } => *id,
    };
    // Status filter must be a known value; anything else is a 400 rather
    // than a silently ignored parameter.
    if let Some(s) = q.status.as_deref().filter(|s| !s.is_empty()) {
        if !is_valid_status(s) {
            return bad_request("unknown status filter");
        }
    }
    let filter = TicketFilter {
        search: None,
        status: q.status.filter(|s| !s.is_empty()),
        category: None,
        priority: None,
        page: q.page.unwrap_or(1),
        per_page: q.per_page.unwrap_or(25),
    };
    // Ownership scoping happens HERE: the query can only see this user's
    // rows regardless of what the client asked for.
    let tickets = state.tickets.clone();
    let run_filter = filter.clone();
    let result = tokio::task::spawn_blocking(move || {
        tickets.list_tickets(Some(user_id), &run_filter)
    })
    .await;
    match result {
        Ok(Ok((rows, total))) => {
            let items: Vec<serde_json::Value> = rows.iter().map(|r| r.to_json()).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "page": filter.page.max(1),
                    "perPage": filter.per_page.clamp(1, store::MAX_PAGE),
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => store_error_response(e),
        Err(_) => SupportAuthError::Internal.into_response(),
    }
}

/// `GET /api/support/tickets/{id}` — one ticket's detail.
///
/// Normal users see only their own ticket (others → 404, so ids cannot be
/// probed). Internal notes are stripped for non-admin callers.
async fn ticket_detail(
    State(state): State<Arc<SupportState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let is_admin = matches!(caller, Caller::Admin { .. });
    let user_id = match &caller {
        Caller::Admin { id, .. } | Caller::User { id, .. } => *id,
    };
    let tickets = state.tickets.clone();
    let result =
        tokio::task::spawn_blocking(move || tickets.ticket_detail(&id)).await;
    let detail = match result {
        Ok(Ok(Some(d))) => d,
        Ok(Ok(None)) => return not_found(),
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };
    // Ownership check: a non-admin reading someone else's ticket gets a
    // 404 — indistinguishable from a nonexistent id.
    if !is_admin && detail.row.user_id != user_id {
        return not_found();
    }
    // Strip internal notes for non-admin callers. This is the single
    // projection point; nothing downstream can re-add them.
    let events: Vec<serde_json::Value> = detail
        .events
        .iter()
        .filter(|e| is_admin || e.kind != "internal_note")
        .map(|e| e.to_json())
        .collect();
    let attachments: Vec<serde_json::Value> =
        detail.attachments.iter().map(AttachmentMeta::to_json).collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ticket": detail.row.to_json(),
            "attachments": attachments,
            "events": events,
            // Tell the SPA which actions this caller may take.
            "canManage": is_admin,
        })),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "ticket not found" })),
    )
        .into_response()
}

/// `GET /api/support/attachments/{id}` — download one attachment.
///
/// Ownership rules match [`ticket_detail`]: admins may fetch any; users
/// only their own tickets' files. The stored content type is echoed back
/// only after re-validation, and `Content-Disposition: attachment` keeps
/// browsers from rendering untrusted HTML/SVG inline.
async fn download_attachment(
    State(state): State<Arc<SupportState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let is_admin = matches!(caller, Caller::Admin { .. });
    let user_id = match &caller {
        Caller::Admin { id, .. } | Caller::User { id, .. } => *id,
    };
    let Ok(attachment_id) = id.parse::<i64>() else {
        return bad_request("invalid attachment id");
    };
    let tickets = state.tickets.clone();
    let fetched = tokio::task::spawn_blocking(move || tickets.get_attachment(attachment_id)).await;
    let (meta, data) = match fetched {
        Ok(Ok(Some(pair))) => pair,
        Ok(Ok(None)) => return not_found(),
        Ok(Err(SupportStoreError::NotFound)) => return not_found(),
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };
    // Ownership: resolve the parent ticket's owner.
    let parent = state.tickets.clone();
    let owner_check = tokio::task::spawn_blocking(move || {
        parent.get_ticket(&meta.ticket_id).map(|o| o.map(|t| t.user_id))
    })
    .await;
    let owner_id = match owner_check {
        Ok(Ok(Some(uid))) => uid,
        Ok(Ok(None)) => return not_found(),
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };
    if !is_admin && owner_id != user_id {
        return not_found();
    }
    // Re-validate the stored type before serving it back. A tampered row
    // degrades to a safe octet-stream download rather than executing.
    let ctype = if is_allowed_attachment_type(&meta.content_type) {
        meta.content_type.clone()
    } else {
        "application/octet-stream".to_string()
    };
    let safe_name = meta
        .filename
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect::<String>();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{safe_name}\""),
            ),
        ],
        data,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Handlers: admin surface
// ---------------------------------------------------------------------------

/// `GET /api/support/admin/summary` — dashboard cards.
async fn admin_summary(State(state): State<Arc<SupportState>>, headers: HeaderMap) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }
    let tickets = state.tickets.clone();
    let result = tokio::task::spawn_blocking(move || tickets.summary()).await;
    match result {
        Ok(Ok(s)) => (
            StatusCode::OK,
            [(
                header::CACHE_CONTROL,
                "no-store".to_string(),
            )],
            Json(s.to_json()),
        )
            .into_response(),
        Ok(Err(e)) => store_error_response(e),
        Err(_) => SupportAuthError::Internal.into_response(),
    }
}

/// `GET /api/support/admin/tickets` — all tickets with filters + search.
#[derive(Deserialize)]
struct AdminListQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    priority: Option<String>,
}

async fn admin_list_tickets(
    State(state): State<Arc<SupportState>>,
    Query(q): Query<AdminListQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }
    if let Some(s) = q.status.as_deref().filter(|s| !s.is_empty()) {
        if !is_valid_status(s) {
            return bad_request("unknown status filter");
        }
    }
    if let Some(c) = q.category.as_deref().filter(|s| !s.is_empty()) {
        if !is_valid_category(c) {
            return bad_request("unknown category filter");
        }
    }
    if let Some(p) = q.priority.as_deref().filter(|s| !s.is_empty()) {
        if !is_valid_priority(p) {
            return bad_request("unknown priority filter");
        }
    }
    let filter = TicketFilter {
        search: q.q,
        status: q.status.filter(|s| !s.is_empty()),
        category: q.category.filter(|s| !s.is_empty()),
        priority: q.priority.filter(|s| !s.is_empty()),
        page: q.page.unwrap_or(1),
        per_page: q.per_page.unwrap_or(25),
    };
    let tickets = state.tickets.clone();
    let run_filter = filter.clone();
    let result =
        tokio::task::spawn_blocking(move || tickets.list_tickets(None, &run_filter)).await;
    match result {
        Ok(Ok((rows, total))) => {
            let items: Vec<serde_json::Value> = rows.iter().map(|r| r.to_json()).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "page": filter.page.max(1),
                    "perPage": filter.per_page.clamp(1, store::MAX_PAGE),
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => store_error_response(e),
        Err(_) => SupportAuthError::Internal.into_response(),
    }
}

/// `POST /api/support/admin/tickets/{id}/status` — change status.
#[derive(Deserialize)]
struct SetStatusRequest {
    status: String,
    /// Optional optimistic-concurrency guard: the status the caller
    /// believes the ticket is currently in.
    #[serde(default, rename = "expectedCurrent")]
    expected_current: Option<String>,
}

async fn admin_set_status(
    State(state): State<Arc<SupportState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::extract::Request<Body>,
) -> Response {
    let (admin_id, admin_email) = match require_admin(&state, &headers).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let _ = admin_id;
    let bytes = match axum::body::to_bytes(body.into_body(), ACTION_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => return bad_request("invalid request body"),
    };
    let req: SetStatusRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return bad_request("invalid request body"),
    };
    if !is_valid_status(&req.status) {
        return bad_request("unknown status");
    }
    if let Some(expected) = req.expected_current.as_deref() {
        if !is_valid_status(expected) {
            return bad_request("unknown expectedCurrent status");
        }
    }
    let tickets = state.tickets.clone();
    let ticket_id = id.clone();
    let status = req.status.clone();
    let actor = admin_email.clone();
    let expected = req.expected_current.clone();
    let result = tokio::task::spawn_blocking(move || {
        tickets.set_status(&ticket_id, &status, &actor, expected.as_deref())
    })
    .await;
    let row = match result {
        Ok(Ok(row)) => row,
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };

    // Notify the user about the update (best effort, never blocking the
    // successful status change).
    notify_user_best_effort(&state, &row, &admin_email, "status");

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "ticket": row.to_json() })),
    )
        .into_response()
}

/// `POST /api/support/admin/tickets/{id}/reply` — reply to the user.
#[derive(Deserialize)]
struct ReplyRequest {
    message: String,
}

async fn admin_reply(
    State(state): State<Arc<SupportState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::extract::Request<Body>,
) -> Response {
    let (_admin_id, admin_email) = match require_admin(&state, &headers).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let bytes = match axum::body::to_bytes(body.into_body(), ACTION_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => return bad_request("invalid request body"),
    };
    let req: ReplyRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return bad_request("invalid request body"),
    };
    let message = req.message.trim().to_string();
    if message.is_empty() {
        return bad_request("reply message is required");
    }
    if message.chars().count() > MAX_DESCRIPTION_LEN {
        return bad_request(format!(
            "reply must be at most {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    let tickets = state.tickets.clone();
    let ticket_id = id.clone();
    let actor = admin_email.clone();
    let result =
        tokio::task::spawn_blocking(move || tickets.add_reply(&ticket_id, &message, &actor)).await;
    let row = match result {
        Ok(Ok(row)) => row,
        Ok(Err(e)) => return store_error_response(e),
        Err(_) => return SupportAuthError::Internal.into_response(),
    };

    // Email the reply to the user's registered address (best effort).
    notify_user_best_effort(&state, &row, &admin_email, "reply");

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "ticket": row.to_json() })),
    )
        .into_response()
}

/// `POST /api/support/admin/tickets/{id}/note` — private internal note.
///
/// Stored as an `internal_note` event; the user-facing projections in this
/// module filter those out, so a note can never reach a normal user.
#[derive(Deserialize)]
struct NoteRequest {
    note: String,
}

async fn admin_internal_note(
    State(state): State<Arc<SupportState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::extract::Request<Body>,
) -> Response {
    let (_admin_id, admin_email) = match require_admin(&state, &headers).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let bytes = match axum::body::to_bytes(body.into_body(), ACTION_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => return bad_request("invalid request body"),
    };
    let req: NoteRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return bad_request("invalid request body"),
    };
    let note = req.note.trim().to_string();
    if note.is_empty() {
        return bad_request("note is required");
    }
    if note.chars().count() > MAX_DESCRIPTION_LEN {
        return bad_request(format!(
            "note must be at most {MAX_DESCRIPTION_LEN} characters"
        ));
    }
    let tickets = state.tickets.clone();
    let ticket_id = id.clone();
    let actor = admin_email.clone();
    let result =
        tokio::task::spawn_blocking(move || tickets.add_internal_note(&ticket_id, &note, &actor))
            .await;
    match result {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true })),
        )
            .into_response(),
        Ok(Err(e)) => store_error_response(e),
        Err(_) => SupportAuthError::Internal.into_response(),
    }
}

/// Fire the appropriate user-notification email without ever failing the
/// caller's request. Delivery problems are logged server-side only.
fn notify_user_best_effort(
    state: &SupportState,
    row: &store::TicketRow,
    admin_email: &str,
    kind: &str,
) {
    let Some(mailer) = state.mailer.clone() else { return };
    let message = match kind {
        "reply" => crate::mail::SupportTicketMessage::reply(
            row.user_email.clone(),
            &row.ticket_id,
            &row.subject,
        ),
        _ => crate::mail::SupportTicketMessage::status_update(
            row.user_email.clone(),
            &row.ticket_id,
            &row.subject,
            &row.status,
        ),
    };
    let _ = admin_email; // available for future "answered by" attribution
    // spawn_blocking on the runtime this handler is already on; the
    // response has already been committed by the caller.
    let mailer = mailer.clone();
    let ticket_id = row.ticket_id.clone();
    tokio::task::spawn(async move {
        let res = tokio::task::spawn_blocking(move || mailer.send_support_notification(&message))
            .await;
        if !matches!(res, Ok(Ok(()))) {
            eprintln!(
                "warning: ticket {ticket_id} updated, but the user notification email failed"
            );
        }
    });
}
