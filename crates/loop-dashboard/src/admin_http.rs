//! Administrator-only account management APIs (Phase 4D, extended in
//! Phase 7).
//!
//! Routes (all require an authenticated, **currently approved**
//! administrator session — see [`require_admin`]):
//!
//! * `GET  /api/admin/users/pending`      — list accounts awaiting review.
//! * `GET  /api/admin/users`              — full account list with search,
//!   role/status filters and pagination.
//! * `POST /api/admin/users/{id}/approve`   — pending → approved.
//! * `POST /api/admin/users/{id}/reject`    — pending → rejected.
//! * `POST /api/admin/users/{id}/suspend`   — approved → suspended.
//! * `POST /api/admin/users/{id}/reactivate`— suspended → approved.
//! * `POST /api/admin/users/{id}/role`      — promote/demote (Phase 7).
//!
//! Authorization model
//! -------------------
//! Administrators are no longer capped at one: they may promote other
//! approved accounts via `/role`. The invariant that is enforced instead —
//! and which no request or combination of concurrent requests can break —
//! is **never zero administrators**: demoting the last one is refused
//! inside the same transaction as the change.
//!
//! * no session, or an expired/revoked one → 401,
//! * a valid session belonging to a non-admin account → 403,
//! * a valid admin session whose account has since been suspended or
//!   deleted → 403 (the read happens fresh, never from the request).
//!
//! Client-supplied `role`, `status` or `isAdmin` fields are never
//! deserialized by any handler here — the transition endpoints take no
//! body at all, so there is nothing to forge.
//!
//! Invariants
//! ----------
//! * An administrator account cannot be targeted by any **status**
//!   transition; it stays `approved` forever ([`AuthStore::set_account_status`]
//!   refuses any transition whose subject has `role = admin`). Role changes
//!   are the separate `/role` surface, and they never alter status.
//! * The last administrator can never be demoted: the count check and the
//!   update share one IMMEDIATE transaction, so concurrent demotions
//!   cannot both succeed ([`AuthStore::set_account_role`]).
//! * Ordinary users can never change their own or anyone's role or status:
//!   they are stopped by the 403 guard before the target id is even parsed.
//! * All writes run inside a single transaction, so a concurrent request
//!   cannot interleave between the eligibility check and the update.
//! * Suspending revokes the target's sessions in that same transaction.
//! * Reactivating does **not** resurrect revoked sessions — the user must
//!   sign in again for a fresh token.
//! * A role change mints no session and revokes none; the target's
//!   password hash is untouched.
//!
//! CSRF posture (documented boundary, not a complete defense)
//! ----------------------------------------------------------
//! Sessions ride a `SameSite=Lax` cookie. All five endpoints here are
//! JSON-only POSTs/GETs; mainstream browsers will not attach a Lax cookie
//! to a cross-site POST, which blocks the classic form-submission CSRF
//! vector. **This is not a complete CSRF defense** and is not claimed to be:
//!
//! * Older or non-standard browsers may not honour `SameSite`.
//! * Sub-domains of the host origin, or any XSS foothold on the page, can
//!   still issue same-origin requests with the cookie attached.
//! * A top-level navigation (GET) can carry the cookie cross-site; the only
//!   GET here is read-only, so it cannot mutate state, but it does mean the
//!   pending list could be observed by a page the admin visits.
//!
//! The mitigation for those residual vectors is the deployment boundary:
//! the dashboard binds to loopback only unless `--allow-lan` is passed.
//! If it is ever exposed beyond localhost, request-scoped CSRF tokens
//! should be added before relying on this surface in anger.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::auth::{
    AccountError, SessionInfo, ADMIN_ROLE, STATUS_APPROVED, STATUS_REJECTED, STATUS_SUSPENDED,
};
use crate::auth_http::AuthState;

/// Upper bound on the pending-registration listing. Bounded so a flood of
/// registrations cannot produce an unbounded response payload.
const PENDING_LIST_LIMIT: i64 = 500;

/// Upper bound on the per-account session listing in the details panel.
const DETAIL_SESSION_LIMIT: i64 = 20;

/// Router for `/api/admin/*`.
///
/// Mounted under the app's existing guard layer, which already requires a
/// valid session for every `/api/*` path outside `/api/auth/*`. The admin
/// guard below adds the role/status check on top.
pub fn admin_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/api/admin/users/pending", get(list_pending))
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users/{id}", get(user_details).delete(remove_user))
        .route("/api/admin/users/{id}/activity", get(user_activity))
        .route("/api/admin/users/{id}/approve", post(approve_user))
        .route("/api/admin/users/{id}/reject", post(reject_user))
        .route("/api/admin/users/{id}/suspend", post(suspend_user))
        .route("/api/admin/users/{id}/reactivate", post(reactivate_user))
        .route("/api/admin/users/{id}/role", post(set_role))
        .route("/api/admin/users/{id}/reset-password", post(reset_password))
        .route(
            "/api/admin/users/{id}/revoke-sessions",
            post(revoke_sessions),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// Resolve a request to its administrator account, or a safe error response.
///
/// This is the single authorization point for every admin endpoint. It
/// performs three checks, in order, each answered with a distinct but
/// deliberately coarse status code:
///
/// 1. **Session validity** — the cookie must resolve to an unexpired,
///    unrevoked session whose account is currently `approved`
///    ([`AuthStore::validate_session`] re-reads status from storage on
///    every call, so a suspension takes effect immediately).
/// 2. **Account existence** — the JOIN in `validate_session` drops
///    sessions whose account row disappeared.
/// 3. **Role** — the account's current role must be `admin`, read fresh
///    from storage rather than trusted from any client input.
///
/// Why an admin request was refused. Kept deliberately small so it can be
/// returned by value from `Result` without tripping `result_large_err`;
/// the HTTP mapping lives in [`AdminAuthError::into_response`].
enum AdminAuthError {
    /// No session, or an expired/revoked/non-approved one.
    Unauthenticated,
    /// A valid session belonging to a non-administrator account.
    Forbidden,
    /// Storage or task failure. Details are logged nowhere and leaked
    /// nowhere; the client sees a generic 500.
    Internal,
}

impl AdminAuthError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "authentication required" })),
            )
                .into_response(),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "administrator access required" })),
            )
                .into_response(),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "account administration unavailable" })),
            )
                .into_response(),
        }
    }
}

/// Resolve a request to its administrator account, or a safe refusal.
///
/// Returns `Err(AdminAuthError::Unauthenticated)` (401) when there is no
/// valid session and `Err(AdminAuthError::Forbidden)` (403) when the
/// caller is authenticated but not an active administrator.
async fn require_admin(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<AdminCaller, AdminAuthError> {
    // 1 + 2: a validated session implies the account exists and is approved.
    let Some(session) = current_session(state, headers).await else {
        return Err(AdminAuthError::Unauthenticated);
    };
    // 3: re-read the caller's CURRENT role from storage. Never trust the
    // request body, a header, or a cached snapshot.
    let account = match lookup_account(state, &session.identifier).await {
        Ok(Some(record)) => record,
        Ok(None) => return Err(AdminAuthError::Forbidden),
        Err(_) => return Err(AdminAuthError::Internal),
    };
    if account.role != ADMIN_ROLE || account.status != STATUS_APPROVED {
        return Err(AdminAuthError::Forbidden);
    }
    Ok(account)
}

/// The caller's account as resolved fresh from storage during
/// authorization. Only the fields the guard and handlers need.
struct AdminCaller {
    id: i64,
    /// Used by handlers that audit who acted, and by the self-delete guard.
    identifier: String,
    role: String,
    status: String,
}

/// Session for the current request, if the cookie validates.
///
/// Mirrors `auth_http::current_session` but is kept private to this module
/// so the admin surface cannot drift from the auth surface's semantics.
async fn current_session(state: &AuthState, headers: &HeaderMap) -> Option<SessionInfo> {
    let token = token_from_headers(headers)?.to_string();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.validate_session(&token))
        .await
        .ok()
        .flatten()
}

/// Look up an account by identifier, returning `None` when absent.
async fn lookup_account(
    state: &AuthState,
    identifier: &str,
) -> Result<Option<AdminCaller>, AdminAuthError> {
    let store = state.store.clone();
    let key = identifier.to_string();
    match tokio::task::spawn_blocking(move || store.find_account_by_identifier(&key)).await {
        Ok(Ok(rec)) => Ok(Some(AdminCaller {
            id: rec.id,
            identifier: rec.identifier,
            role: rec.role,
            status: rec.status,
        })),
        Ok(Err(AccountError::NotFound)) => Ok(None),
        // Storage failure: generic 500, no internals leaked.
        Ok(Err(_)) => Err(AdminAuthError::Internal),
        Err(_) => Err(AdminAuthError::Internal),
    }
}

fn token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::auth::session_token_from_cookie_header)
}

/// Map a storage-layer error to a safe HTTP response.
///
/// Storage failures are reported as a generic 500 without the underlying
/// SQLite message; user-facing errors keep their own status codes.
fn account_error_response(e: AccountError) -> Response {
    let status = e.status();
    let message = if matches!(e, AccountError::Storage(_)) {
        "account administration unavailable".to_string()
    } else {
        e.to_string()
    };
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Middleware flavor of the admin check, for mounting on routers that want
/// blanket protection rather than calling [`require_admin`] per handler.
#[allow(clippy::unused_async)]
pub async fn require_admin_middleware(
    State(state): State<Arc<AuthState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers().clone();
    match require_admin(&state, &headers).await {
        Ok(_) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/admin/users/pending`
///
/// Returns the minimum fields an administrator needs to review a
/// registration: id, email, registration timestamp and current status.
/// Password hashes, session tokens and secrets are never included.
///
/// The listing is bounded at [`PENDING_LIST_LIMIT`] rows. When more
/// pending accounts exist than fit, the response says so **explicitly**
/// (`truncated: true` plus `totalCount`) rather than silently dropping
/// rows — an administrator must never mistake a truncated page for the
/// complete queue. Pagination proper (offset/cursor parameters) is
/// deliberately deferred: with one administrator on a local service and a
/// registration budget of [`crate::auth::REGISTRATION_CAPACITY`] per hour,
/// a backlog exceeding 500 rows is not a realistic operating state, and
/// adding cursor plumbing now would be speculative surface area.
async fn list_pending(State(state): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let accounts = store.list_pending_accounts(PENDING_LIST_LIMIT)?;
        let total = store.count_pending_accounts()?;
        Ok::<_, AccountError>((accounts, total))
    })
    .await;
    match result {
        Ok(Ok((accounts, total))) => {
            // Explicitly project only the safe fields; AccountRecord itself
            // never carries a hash, but projecting keeps it that way even if
            // the struct grows.
            let rows: Vec<serde_json::Value> = accounts
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "identifier": a.identifier,
                        "status": a.status,
                        "createdAtMs": a.created_at_ms,
                    })
                })
                .collect();
            let returned = rows.len() as i64;
            Json(serde_json::json!({
                "users": rows,
                // How many pending accounts exist in total (independent of
                // the page bound), so truncation is observable.
                "totalCount": total,
                // True when the page omitted rows. A client can also derive
                // this from totalCount > users.len(), but the explicit flag
                // removes any ambiguity about intent.
                "truncated": total > returned,
            }))
            .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// Shared implementation for the four transition endpoints.
///
/// `target_status` is always a server-side constant — it never comes from
/// the request — so a client cannot approve itself by sending a crafted
/// body. The endpoints take no request body at all.
/// `GET /api/admin/users` — the complete account list (Phase 7).
///
/// Query parameters (all optional):
/// * `search`  — case-insensitive substring match on identifier.
/// * `role`    — `admin` | `user`.
/// * `status`  — one of the four statuses.
/// * `page`    — 1-based page number (default 1).
/// * `perPage` — rows per page (default 50, capped at
///   [`crate::auth::ACCOUNT_LIST_MAX_PER_PAGE`]).
///
/// Returns `{ users:[…], totalCount, page, perPage, totalPages }`. Each row
/// carries only safe fields: id, identifier, role, status, createdAtMs and
/// lastSignInAtMs (null when never recorded). Password hashes and session
/// material are never selected, so they cannot leak here.
///
/// Unknown `role`/`status` values are a 400 rather than silently ignored:
/// a caller filtering on `"superuser"` has a bug worth surfacing.
async fn list_users(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }

    // ---- parse + validate query parameters ----
    let q = uri.query().unwrap_or("");
    let get_param = |name: &str| -> Option<String> { form_urlencoded_get(q, name) };

    let mut filter = crate::auth::AccountFilter {
        page: get_param("page")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(1),
        per_page: get_param("perPage")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(50),
        ..crate::auth::AccountFilter::default()
    };
    if let Some(s) = get_param("search") {
        if !s.trim().is_empty() {
            filter.search = Some(s);
        }
    }
    if let Some(r) = get_param("role") {
        if !r.trim().is_empty() {
            filter.role = Some(r);
        }
    }
    if let Some(s) = get_param("status") {
        if !s.trim().is_empty() {
            filter.status = Some(s);
        }
    }

    let store = state.store.clone();
    // Snapshot the paging values for the response before the filter is
    // moved into the blocking task.
    let (req_page, req_per_page) = (filter.page, filter.per_page);
    let result = tokio::task::spawn_blocking(move || store.list_accounts(&filter)).await;
    match result {
        Ok(Ok((accounts, total))) => {
            let per_page = if req_per_page < 1 {
                50
            } else {
                req_per_page.min(crate::auth::ACCOUNT_LIST_MAX_PER_PAGE)
            };
            let page = req_page.max(1);
            let rows: Vec<serde_json::Value> = accounts
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "identifier": a.identifier,
                        "role": a.role,
                        "status": a.status,
                        "createdAtMs": a.created_at_ms,
                        // Null, never fabricated, when no sign-in is recorded.
                        "lastSignInAtMs": a.last_sign_in_at_ms,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "users": rows,
                "totalCount": total,
                "page": page,
                "perPage": per_page,
                "totalPages": if per_page > 0 {
                    (total + per_page - 1) / per_page
                } else { 0 },
            }))
            .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// Extract one value for `name` from a URL-encoded query string.
///
/// Only the first occurrence is honoured; duplicates are ignored. Returns
/// `None` when absent. Values are percent-decoded.
fn form_urlencoded_get(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if percent_decode(k) == name {
            return Some(percent_decode(v));
        }
    }
    None
}

/// Minimal percent-decoding for a single query-string token.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                // Two hex digits must follow '%'.
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The request body for `POST /api/admin/users/{id}/role`.
///
/// Deliberately minimal: just the target role. Nothing else from the body
/// is deserialized, so a client cannot smuggle a status change, an email
/// change or a password reset through this endpoint.
#[derive(Debug, serde::Deserialize)]
struct RoleBody {
    role: String,
}

/// `POST /api/admin/users/{id}/role` — promote or demote an account
/// (Phase 7).
///
/// Authorization and validation, all server-side:
///
/// * [`require_admin`] re-reads the caller's CURRENT role and status from
///   storage — a client-supplied `isAdmin` is never consulted.
/// * The caller cannot change its own role (409), which prevents both
///   accidental self-lockout and pointless self-promotion.
/// * The target must exist (404) and, for promotion, be `approved` (409):
///   authority is never handed to an account that cannot even sign in.
/// * Demoting the last administrator is refused (409) inside the same
///   transaction as the change, so concurrent requests cannot strand the
///   installation without an administrator.
/// * The endpoint never touches `password_hash`, `status` or sessions: no
///   session is minted, none is revoked, the target's credentials are
///   exactly what they were.
///
/// The body is parsed as raw bytes and deserialized **after** the guard, so
/// an unauthenticated caller learns nothing about the expected shape — not
/// even which fields exist.
async fn set_role(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    let admin = match require_admin(&state, &headers).await {
        Ok(admin) => admin,
        Err(e) => return e.into_response(),
    };
    // Self-targeting guard. Distinct from the transition endpoints' guard:
    // there it is a UX nicety, here it is a real safety property (an
    // administrator demoting themselves could be the last one).
    if admin.id == id {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "administrators cannot change their own role"
            })),
        )
            .into_response();
    }
    // Parse the body only now that the caller is authorized. A malformed or
    // missing body is a plain 400 with no schema details.
    let parsed: RoleBody = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "a role is required" })),
            )
                .into_response()
        }
    };
    let role = parsed.role;

    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || store.set_account_role(id, &role)).await;
    match result {
        Ok(Ok(new_role)) => {
            // Audit the role change: who promoted/demoted whom, and to what.
            let store = state.store.clone();
            let actor_id = admin.id;
            let actor_email = admin.identifier.clone();
            let detail = format!("role set to {new_role}");
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(target) = store.find_account_by_id(id) {
                    store.record_audit(
                        crate::auth::audit_event::ROLE_CHANGED,
                        Some((actor_id, actor_email.as_str())),
                        Some((target.id, target.identifier.as_str())),
                        &detail,
                    );
                }
            })
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "ok": true, "role": new_role })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// Shared implementation for the four transition endpoints.
///
/// `target_status` is always a server-side constant — it never comes from
/// the request — so a client cannot approve itself by sending a crafted
/// body. The endpoints take no request body at all.
async fn transition(
    state: &Arc<AuthState>,
    headers: &HeaderMap,
    id: i64,
    target_status: &'static str,
) -> Response {
    let admin = match require_admin(state, headers).await {
        Ok(admin) => admin,
        Err(e) => return e.into_response(),
    };

    // Self-targeting guard: an administrator approving/rejecting/etc. its
    // own account would be a no-op anyway (admin accounts are immune to
    // transitions), but refusing it explicitly yields a clearer, safer
    // error than relying on the storage invariant alone.
    if admin.id == id {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "administrators cannot modify their own account"
            })),
        )
            .into_response();
    }

    let store = state.store.clone();
    let result =
        tokio::task::spawn_blocking(move || store.set_account_status(id, target_status)).await;
    match result {
        Ok(Ok(())) => {
            // Audit the transition: who approved/suspended/reactivated whom.
            // The subject's email is read AFTER the change so the entry
            // reflects the account as it now stands.
            let store = state.store.clone();
            let actor_id = admin.id;
            let actor_email = admin.identifier.clone();
            let kind = match target_status {
                STATUS_APPROVED => crate::auth::audit_event::ACCOUNT_ACTIVATED,
                STATUS_REJECTED => crate::auth::audit_event::ACCOUNT_REJECTED,
                STATUS_SUSPENDED => crate::auth::audit_event::ACCOUNT_SUSPENDED,
                _ => crate::auth::audit_event::ACCOUNT_APPROVED,
            };
            let detail = format!("status set to {target_status}");
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(target) = store.find_account_by_id(id) {
                    store.record_audit(
                        kind,
                        Some((actor_id, actor_email.as_str())),
                        Some((target.id, target.identifier.as_str())),
                        &detail,
                    );
                }
            })
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "ok": true, "status": target_status })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// Parse `{id}` from the path. `None` means the segment was not a number,
/// which callers answer with a 400 rather than a 404 so a typo'd URL is
/// distinguishable from an unknown account.
fn parse_id(id: &str) -> Option<i64> {
    id.parse::<i64>().ok()
}

// ---------------------------------------------------------------------------
// User Management handlers (details, reset, sessions, delete, activity)
// ---------------------------------------------------------------------------

/// `GET /api/admin/users/{id}` — the full detail record for one account.
///
/// Returns identity, role, status, registration date, last sign-in, last
/// password change (when tracked), the live-session count and the live
/// sessions themselves (timestamps only — never token digests). Password
/// hashes and session/reset material are never selected, so they cannot
/// leak here.
async fn user_details(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let detail = store.account_detail(id)?;
        let sessions = store.account_sessions(id, DETAIL_SESSION_LIMIT)?;
        Ok::<_, AccountError>((detail, sessions))
    })
    .await;
    match result {
        Ok(Ok((d, sessions))) => {
            let sessions_json: Vec<serde_json::Value> = sessions
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "createdAtMs": s.created_at_ms,
                        "expiresAtMs": s.expires_at_ms,
                        "lastSeenAtMs": s.last_seen_at_ms,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "id": d.id,
                "identifier": d.identifier,
                "role": d.role,
                "status": d.status,
                "createdAtMs": d.created_at_ms,
                // Null (never fabricated) when no sign-in is recorded.
                "lastSignInAtMs": d.last_sign_in_at_ms,
                // Null for accounts that predate password-change tracking.
                "lastPasswordChangeMs": d.last_password_change_ms,
                "liveSessionCount": d.live_session_count,
                "sessions": sessions_json,
            }))
            .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// `POST /api/admin/users/{id}/reset-password` — trigger a password reset.
///
/// This NEVER sets or reveals a password. It mints a single-use, expiring
/// token (stored as a SHA-256 digest) and emails the reset link to the
/// account's own address through the configured provider. The user then
/// completes the reset on the public `/reset-password` page; their new
/// password is transmitted once, hashed server-side, and every existing
/// session is revoked on completion.
///
/// Failure modes, each answered distinctly:
/// * no mail configuration → 503 "email delivery is not configured" (the
///   administrator learns the instance cannot send mail; nothing is minted);
/// * provider rejection/transport failure → 502, and the just-minted token
///   is invalidated so it can never be used undelivered;
/// * cooldown → 429 with Retry-After.
async fn reset_password(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    let admin = match require_admin(&state, &headers).await {
        Ok(admin) => admin,
        Err(e) => return e.into_response(),
    };

    // Fail fast when the instance cannot deliver email: minting a token
    // nobody can receive would be a silent failure.
    let Some(mailer) = state.mailer.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "email delivery is not configured on this instance; set LOOP_EMAIL_PROVIDER, LOOP_EMAIL_API_KEY and LOOP_EMAIL_FROM"
            })),
        )
            .into_response();
    };

    // Mint the token (blocking pool; releases the store lock on return).
    let store = state.store.clone();
    let actor_id = admin.id;
    let issued = tokio::task::spawn_blocking(move || store.issue_password_reset(id, actor_id)).await;
    let issued = match issued {
        Ok(Ok(i)) => i,
        Ok(Err(e)) => {
            // Cooldown advertises how long to wait.
            if let AccountError::ResetCooldown { retry_in_ms } = &e {
                let secs = (*retry_in_ms / 1000).max(1);
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, secs.to_string())],
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            return account_error_response(e);
        }
        Err(_) => return AdminAuthError::Internal.into_response(),
    };

    // Deliver outside any store lock. On failure, invalidate exactly the
    // row we created so an undelivered link can never be used.
    let message = crate::mail::PasswordResetMessage {
        to: issued.identifier.clone(),
        admin_email: admin.identifier.clone(),
        // The link target: same origin the admin browser is on. The public
        // reset page reads the token from the query string.
        reset_url: format!("/reset-password?token={}", issued.token),
        expires_in_minutes: (crate::auth::PASSWORD_RESET_TTL.as_secs() / 60).max(1),
    };
    let mailer_for_send = mailer.clone();
    let delivery = tokio::task::spawn_blocking(move || {
        mailer_for_send.send_password_reset(&message)
    })
    .await;

    match delivery {
        Ok(Ok(())) => {
            // Audit who triggered the reset and for whom.
            let store = state.store.clone();
            let actor_id = admin.id;
            let actor_email = admin.identifier.clone();
            let subject_id = issued.account_id;
            let subject_email = issued.identifier.clone();
            let _ = tokio::task::spawn_blocking(move || {
                store.record_audit_checked(
                    crate::auth::audit_event::PASSWORD_RESET_ISSUED,
                    Some((actor_id, actor_email.as_str())),
                    Some((subject_id, subject_email.as_str())),
                    "password reset email sent",
                )
            })
            .await;
            (
                StatusCode::OK,
                [(
                    header::CACHE_CONTROL,
                    "no-store".to_string(),
                )],
                Json(serde_json::json!({
                    "ok": true,
                    // Never echo the token; only where it went.
                    "sentTo": issued.identifier,
                    "expiresInMinutes": (crate::auth::PASSWORD_RESET_TTL.as_secs() / 60).max(1),
                })),
            )
                .into_response()
        }
        // Delivery failed: kill the token we just made, then report.
        Ok(Err(delivery_err)) => {
            let store = state.store.clone();
            let account_id = issued.account_id;
            let _ = tokio::task::spawn_blocking(move || {
                // Supersede (consume) every live token for this account:
                // the one we minted is the only possible live one.
                let _ = store.revoke_live_reset_tokens(account_id);
            })
            .await;
            (
                delivery_err.status(),
                Json(serde_json::json!({ "error": delivery_err.to_string() })),
            )
                .into_response()
        }
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// `POST /api/admin/users/{id}/revoke-sessions` — invalidate every live
/// session for the account, immediately.
///
/// Confirmation is the client's job (a dialog before calling); the server
/// simply performs the revocation atomically and reports how many sessions
/// were actually removed, so the UI can say something truthful.
async fn revoke_sessions(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    let admin = match require_admin(&state, &headers).await {
        Ok(admin) => admin,
        Err(e) => return e.into_response(),
    };
    // Verify the target exists first so a clean 404 beats a silent no-op.
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let target = store.find_account_by_id(id)?;
        let removed = store.revoke_all_sessions_for(id)?;
        Ok::<_, AccountError>((target, removed))
    })
    .await;
    match result {
        Ok(Ok((target, removed))) => {
            let store = state.store.clone();
            let actor_id = admin.id;
            let actor_email = admin.identifier.clone();
            let subject_id = target.id;
            let subject_email = target.identifier.clone();
            let detail = format!("{removed} session(s) revoked");
            let _ = tokio::task::spawn_blocking(move || {
                store.record_audit(
                    crate::auth::audit_event::SESSIONS_REVOKED,
                    Some((actor_id, actor_email.as_str())),
                    Some((subject_id, subject_email.as_str())),
                    &detail,
                )
            })
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "revoked": removed,
                    "identifier": target.identifier,
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// `DELETE /api/admin/users/{id}` — permanently remove an account.
///
/// Destructive and irreversible. All guards run server-side inside one
/// transaction (see [`crate::auth::AuthStore::delete_account`]):
/// * the caller must be an active administrator,
/// * an administrator cannot delete their own account,
/// * the last administrator cannot be deleted,
/// * sessions, reset tokens and failure counters are removed with the
///   account; the audit trail deliberately survives (it references ids
///   without foreign keys and keeps an email snapshot).
///
/// The endpoint takes no body: there is nothing a client could smuggle
/// past the guards. Confirmation is the UI's dialog; the server enforces
/// the invariants regardless of what the client did or did not ask.
async fn remove_user(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    let admin = match require_admin(&state, &headers).await {
        Ok(admin) => admin,
        Err(e) => return e.into_response(),
    };
    let store = state.store.clone();
    let acting = admin.id;
    let result =
        tokio::task::spawn_blocking(move || store.delete_account(id, acting)).await;
    match result {
        Ok(Ok(identifier)) => {
            // Audit AFTER the delete committed: the entry records what was
            // removed, including the email snapshot, because the account
            // row itself is gone.
            let store = state.store.clone();
            let actor_id = admin.id;
            let actor_email = admin.identifier.clone();
            let subject_email = identifier.clone();
            let _ = tokio::task::spawn_blocking(move || {
                store.record_audit(
                    crate::auth::audit_event::ACCOUNT_DELETED,
                    Some((actor_id, actor_email.as_str())),
                    // subject_id kept, subject_email snapshotted: history
                    // without retaining the account record.
                    Some((id, subject_email.as_str())),
                    "account permanently deleted",
                )
            })
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "ok": true, "deleted": identifier })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// `GET /api/admin/users/{id}/activity` — the audit trail for one account.
///
/// Works for deleted accounts too (the log outlives the rows it describes):
/// the response carries the subject email snapshot from the newest entry
/// so the panel can title itself honestly even after deletion.
///
/// Query parameters: `page` (1-based, default 1) and `perPage` (default
/// 25, capped at 200 server-side).
async fn user_activity(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    if let Err(e) = require_admin(&state, &headers).await {
        return e.into_response();
    }

    let q = uri.query().unwrap_or("");
    let page = form_urlencoded_get(q, "page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let per_page = form_urlencoded_get(q, "perPage")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(25);
    let per_page = per_page.clamp(1, 200);
    let offset = (page - 1).saturating_mul(per_page);

    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        store.list_account_activity(id, per_page, offset)
    })
    .await;
    match result {
        Ok(Ok((entries, total))) => {
            let rows: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "atMs": e.at_ms,
                        "kind": e.kind,
                        // Readable, human-facing description derived
                        // server-side; the client never interprets kinds.
                        "description": describe_audit_entry(e),
                        "actorEmail": e.actor_email,
                        "subjectEmail": e.subject_email,
                        "detail": e.detail,
                    })
                })
                .collect();
            let subject_email = entries.first().and_then(|e| e.subject_email.clone());
            Json(serde_json::json!({
                "events": rows,
                "totalCount": total,
                "page": page,
                "perPage": per_page,
                "totalPages": if per_page > 0 {
                    (total + per_page - 1) / per_page
                } else { 0 },
                // Present when history exists even for a deleted account.
                "subjectEmail": subject_email,
            }))
            .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => AdminAuthError::Internal.into_response(),
    }
}

/// Human-readable description of one audit entry, for the activity view.
/// Static text keyed on the kind; dynamic parts come from `detail`.
fn describe_audit_entry(e: &crate::auth::AuditEntry) -> String {
    match e.kind.as_str() {
        crate::auth::audit_event::SIGNIN_SUCCESS => "Signed in successfully".to_string(),
        crate::auth::audit_event::SIGNIN_FAILED => {
            format!("Sign-in attempt failed{}", detail_suffix(e))
        }
        crate::auth::audit_event::PASSWORD_RESET_ISSUED => {
            "Password reset emailed by an administrator".to_string()
        }
        crate::auth::audit_event::PASSWORD_RESET_COMPLETED => {
            "Password changed via reset link".to_string()
        }
        crate::auth::audit_event::ROLE_CHANGED => {
            format!("Role changed{}", detail_suffix(e))
        }
        crate::auth::audit_event::ACCOUNT_SUSPENDED => {
            "Account suspended by an administrator".to_string()
        }
        crate::auth::audit_event::ACCOUNT_ACTIVATED => {
            "Account reactivated by an administrator".to_string()
        }
        crate::auth::audit_event::SESSIONS_REVOKED => {
            format!("All sessions revoked{}", detail_suffix(e))
        }
        crate::auth::audit_event::ACCOUNT_DELETED => {
            "Account permanently deleted".to_string()
        }
        crate::auth::audit_event::ACCOUNT_APPROVED => {
            "Registration approved".to_string()
        }
        crate::auth::audit_event::ACCOUNT_REJECTED => {
            "Registration rejected".to_string()
        }
        other => format!("Event: {other}"),
    }
}

/// ` — <detail>` suffix, empty when the entry has no detail.
fn detail_suffix(e: &crate::auth::AuditEntry) -> String {
    match e.detail.as_deref() {
        Some(d) if !d.trim().is_empty() => format!(" — {d}"),
        _ => String::new(),
    }
}

/// The 400 used for a malformed `{id}` path segment.
fn invalid_id_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "invalid account id" })),
    )
        .into_response()
}

/// `POST /api/admin/users/{id}/approve` — pending → approved.
///
/// Does NOT create a session for the approved user; they must sign in
/// themselves, which also means their credentials are re-verified.
async fn approve_user(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    transition(&state, &headers, id, STATUS_APPROVED).await
}

/// `POST /api/admin/users/{id}/reject` — pending → rejected.
async fn reject_user(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    transition(&state, &headers, id, STATUS_REJECTED).await
}

/// `POST /api/admin/users/{id}/suspend` — approved → suspended.
///
/// The target's sessions are revoked in the same transaction as the status
/// change, so its cookies stop working on the very next request.
async fn suspend_user(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    transition(&state, &headers, id, STATUS_SUSPENDED).await
}

/// `POST /api/admin/users/{id}/reactivate` — suspended → approved.
///
/// Previously revoked sessions stay dead; the user signs in again.
async fn reactivate_user(
    State(state): State<Arc<AuthState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return invalid_id_response();
    };
    transition(&state, &headers, id, STATUS_APPROVED).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthStore, STATUS_PENDING, STATUS_REJECTED, STATUS_SUSPENDED, USER_ROLE};
    use tower::ServiceExt;

    /// Build an admin router over a throwaway database with one
    /// administrator and (optionally) named extra accounts.
    ///
    /// Returns (router, dir, admin_token, admin_id, ids_by_email).
    async fn fixture(
        extras: &[(&str, &str)],
    ) -> (
        Router,
        tempfile::TempDir,
        String,
        i64,
        std::collections::HashMap<String, i64>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = AuthStore::open(&path).unwrap();
        store
            .sign_up(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        let mut ids = std::collections::HashMap::new();
        for (email, status) in extras {
            let id = store
                .create_pending_user(email, "a strong passphrase")
                .unwrap();
            // Reach the requested status through legal transitions only:
            //   pending -> rejected
            //   pending -> approved
            //   pending -> approved -> suspended
            match *status {
                STATUS_PENDING => {}
                STATUS_REJECTED => {
                    store.set_account_status(id, STATUS_REJECTED).unwrap();
                }
                STATUS_APPROVED => {
                    store.set_account_status(id, STATUS_APPROVED).unwrap();
                }
                STATUS_SUSPENDED => {
                    store.set_account_status(id, STATUS_APPROVED).unwrap();
                    store.set_account_status(id, STATUS_SUSPENDED).unwrap();
                }
                other => panic!("fixture does not support status {other:?}"),
            }
            ids.insert((*email).to_string(), id);
        }
        let admin_rec = store
            .find_account_by_identifier(crate::auth::LEGACY_ADMIN_IDENTIFIER)
            .unwrap();
        let admin_id = admin_rec.id;
        // Admin signs in through the real storage path for a real session.
        let admin_token = store
            .sign_in(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        drop(store);

        let state = Arc::new(AuthState::new(Arc::new(
            AuthStore::open(&path).unwrap(),
        )));
        (admin_router(state), dir, admin_token, admin_id, ids)
    }

    /// Wrap a raw token in the real cookie header form. An empty input
    /// means "no cookie at all" (the unauthenticated case).
    fn cookie_header(token: &str) -> Option<String> {
        if token.is_empty() {
            None
        } else {
            Some(format!("{}={}", crate::auth::SESSION_COOKIE, token))
        }
    }

    async fn get_with_cookie(router: &Router, uri: &str, cookie: &str) -> (StatusCode, String) {
        let mut builder = Request::builder().uri(uri).method("GET");
        if let Some(value) = cookie_header(cookie) {
            builder = builder.header(header::COOKIE, value);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn post_with_cookie(
        router: &Router,
        uri: &str,
        cookie: &str,
        body: &str,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder()
            .uri(uri)
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = cookie_header(cookie) {
            builder = builder.header(header::COOKIE, value);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn unauthenticated_requests_are_rejected() {
        let (router, _d, _t, _a, _i) = fixture(&[]).await;
        for uri in [
            "/api/admin/users/pending",
            "/api/admin/users/2/approve",
            "/api/admin/users/2/reject",
            "/api/admin/users/2/suspend",
            "/api/admin/users/2/reactivate",
        ] {
            let method = if uri.ends_with("/pending") {
                "GET"
            } else {
                "POST"
            };
            let (status, body) = if method == "GET" {
                get_with_cookie(&router, uri, "").await
            } else {
                post_with_cookie(&router, uri, "", "{}").await
            };
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
            assert!(body.contains("authentication required"), "{uri}: {body}");
        }
    }

    #[tokio::test]
    async fn ordinary_users_cannot_access_admin_endpoints() {
        let (router, dir, _t, _a, ids) = fixture(&[("worker@soket.ai", STATUS_APPROVED)]).await;
        // Sign in as the ordinary user.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let token = store
            .sign_in("worker@soket.ai", "a strong passphrase")
            .unwrap();

        for uri in [
            "/api/admin/users/pending",
            "/api/admin/users/999/approve",
            "/api/admin/users/999/reject",
            "/api/admin/users/999/suspend",
            "/api/admin/users/999/reactivate",
        ] {
            let (status, body) = if uri.ends_with("/pending") {
                get_with_cookie(&router, uri, &token).await
            } else {
                post_with_cookie(&router, uri, &token, "{}").await
            };
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
            assert!(body.contains("administrator access required"), "{body}");
        }
        // No state changed.
        assert_eq!(
            store
                .account_role_and_status(*ids.get("worker@soket.ai").unwrap())
                .unwrap()
                .1,
            STATUS_APPROVED
        );
    }

    #[tokio::test]
    async fn pending_or_suspended_accounts_cannot_administer() {
        for status in [STATUS_PENDING, STATUS_SUSPENDED, STATUS_REJECTED] {
            let (router, dir, _t, _a, _i) = fixture(&[("worker@soket.ai", status)]).await;
            let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
            // These accounts cannot sign in at all, so simulate a session
            // that predates their status change by minting one directly.
            // `validate_session` refuses non-approved accounts, so the
            // request dies at the SESSION check (401) — one layer earlier
            // than the role check. Either way: denied, safely.
            let rec = store.find_account_by_identifier("worker@soket.ai").unwrap();
            let token = store.debug_mint_session_for(rec.id).unwrap();
            let (code, body) = get_with_cookie(&router, "/api/admin/users/pending", &token).await;
            assert!(
                code == StatusCode::UNAUTHORIZED || code == StatusCode::FORBIDDEN,
                "{status}: unexpected {code}: {body}"
            );
            assert!(
                !body.contains("users"),
                "{status}: must not leak the pending list: {body}"
            );
            // And the write endpoints are equally closed.
            let (code, _) =
                post_with_cookie(&router, "/api/admin/users/999/approve", &token, "{}").await;
            assert!(
                code == StatusCode::UNAUTHORIZED || code == StatusCode::FORBIDDEN,
                "{status}: write endpoint unexpectedly allowed: {code}"
            );
        }
    }

    #[tokio::test]
    async fn admin_can_list_pending_users() {
        let (router, _d, admin_token, _a, ids) = fixture(&[
            ("first@soket.ai", STATUS_PENDING),
            ("second@soket.ai", STATUS_PENDING),
            ("approved@soket.ai", STATUS_APPROVED),
        ])
        .await;
        let (status, body) =
            get_with_cookie(&router, "/api/admin/users/pending", &admin_token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let users = json["users"].as_array().unwrap();
        assert_eq!(users.len(), 2, "only pending accounts listed: {body}");
        for u in users {
            assert!(u["id"].is_number());
            assert!(u["identifier"].as_str().unwrap().contains("@soket.ai"));
            assert!(u["status"] == STATUS_PENDING);
            assert!(u["createdAtMs"].is_number());
            // Sensitive fields must be absent.
            for absent in ["password_hash", "passwordHash", "hash", "token", "secret"] {
                assert!(u.get(absent).is_none(), "leaked {absent}: {u}");
            }
        }
        // Ordering: oldest registration first.
        assert!(users[0]["id"] == serde_json::json!(*ids.get("first@soket.ai").unwrap()));
    }

    #[tokio::test]
    async fn approving_a_pending_user_changes_status() {
        let (router, dir, _t, _a, ids) = fixture(&[("new@soket.ai", STATUS_PENDING)]).await;
        let id = *ids.get("new@soket.ai").unwrap();
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/approve"),
            &_t,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"approved\""));

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        assert_eq!(
            store.account_role_and_status(id).unwrap().1,
            STATUS_APPROVED
        );
        // No session was created on the approved user's behalf.
        assert_eq!(store.session_count().unwrap(), 1, "only the admin session");
    }

    #[tokio::test]
    async fn rejecting_a_pending_user_changes_status() {
        let (router, dir, _t, _a, ids) = fixture(&[("bad@soket.ai", STATUS_PENDING)]).await;
        let id = *ids.get("bad@soket.ai").unwrap();
        let (status, body) =
            post_with_cookie(&router, &format!("/api/admin/users/{id}/reject"), &_t, "{}").await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        assert_eq!(
            store.account_role_and_status(id).unwrap().1,
            STATUS_REJECTED
        );
    }

    #[tokio::test]
    async fn suspending_revokes_sessions() {
        let (router, dir, admin_token, _a, ids) =
            fixture(&[("victim@soket.ai", STATUS_APPROVED)]).await;
        let id = *ids.get("victim@soket.ai").unwrap();

        // Victim holds a live session before suspension.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let victim_token = store
            .sign_in("victim@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(store.validate_session(&victim_token).is_some());

        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/suspend"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // The victim's session no longer validates...
        assert!(store.validate_session(&victim_token).is_none());
        // ...and the admin's still does.
        assert!(store.validate_session(&admin_token).is_some());
        assert_eq!(
            store.account_role_and_status(id).unwrap().1,
            STATUS_SUSPENDED
        );
    }

    #[tokio::test]
    async fn reactivation_does_not_restore_old_sessions() {
        let (router, dir, admin_token, _a, ids) =
            fixture(&[("back@soket.ai", STATUS_APPROVED)]).await;
        let id = *ids.get("back@soket.ai").unwrap();

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let old_token = store
            .sign_in("back@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(store.validate_session(&old_token).is_some());

        // Suspend, then reactivate.
        post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/suspend"),
            &admin_token,
            "{}",
        )
        .await;
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/reactivate"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        assert_eq!(
            store.account_role_and_status(id).unwrap().1,
            STATUS_APPROVED
        );
        // The pre-suspension session is NOT resurrected.
        assert!(
            store.validate_session(&old_token).is_none(),
            "revoked session must stay dead"
        );
        // But a fresh sign-in works.
        let fresh = store
            .sign_in("back@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(store.validate_session(&fresh).is_some());
    }

    #[tokio::test]
    async fn invalid_and_repeated_transitions_are_rejected() {
        let (router, _d, admin_token, _a, ids) = fixture(&[
            ("p@soket.ai", STATUS_PENDING),
            ("a@soket.ai", STATUS_APPROVED),
        ])
        .await;
        let pending = *ids.get("p@soket.ai").unwrap();
        let approved = *ids.get("a@soket.ai").unwrap();

        // Suspend a pending account: not a legal transition.
        let (status, _) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{pending}/suspend"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Approve an already-approved account: repeated transition.
        let (status, _) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{approved}/approve"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Unknown account id.
        let (status, _) = post_with_cookie(
            &router,
            "/api/admin/users/99999/approve",
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Non-numeric id.
        let (status, _) = post_with_cookie(
            &router,
            "/api/admin/users/not-a-number/approve",
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Nothing changed.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(
            store.account_role_and_status(pending).unwrap().1,
            STATUS_PENDING
        );
        assert_eq!(
            store.account_role_and_status(approved).unwrap().1,
            STATUS_APPROVED
        );
    }

    #[tokio::test]
    async fn users_cannot_approve_themselves() {
        let (router, dir, admin_token, admin_id, ids) =
            fixture(&[("self@soket.ai", STATUS_PENDING)]).await;
        let self_id = *ids.get("self@soket.ai").unwrap();

        // The administrator targeting its own id is refused explicitly.
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{admin_id}/approve"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");

        // An ordinary user trying to approve ITSELF with its own cookie:
        // stopped by the role check before the target id matters.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        store.set_account_status(self_id, STATUS_APPROVED).unwrap();
        let user_token = store
            .sign_in("self@soket.ai", "a strong passphrase")
            .unwrap();
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{self_id}/approve"),
            &user_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        // And nothing changed for that account.
        assert_eq!(
            store.account_role_and_status(self_id).unwrap(),
            (USER_ROLE.to_string(), STATUS_APPROVED.to_string())
        );
    }

    #[tokio::test]
    async fn client_supplied_admin_fields_do_not_grant_privileges() {
        let (router, dir, _t, _a, ids) = fixture(&[("sneaky@soket.ai", STATUS_PENDING)]).await;
        let id = *ids.get("sneaky@soket.ai").unwrap();

        // Sign in as nobody-but-a-pending user cannot happen; instead prove
        // that a forged body does nothing even for a signed-in ordinary user.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        let token = store
            .sign_in("sneaky@soket.ai", "a strong passphrase")
            .unwrap();

        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/approve"),
            &token,
            r#"{"role":"admin","isAdmin":true,"status":"approved"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let rec = store.find_account_by_identifier("sneaky@soket.ai").unwrap();
        assert_eq!(rec.role, USER_ROLE, "role must remain user");
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn last_administrator_cannot_be_removed_or_demoted() {
        let (router, dir, admin_token, admin_id, _i) = fixture(&[]).await;
        // Try every transition against the sole administrator.
        for verb in ["approve", "reject", "suspend", "reactivate"] {
            let (status, body) = post_with_cookie(
                &router,
                &format!("/api/admin/users/{admin_id}/{verb}"),
                &admin_token,
                "{}",
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{verb}: {body}");
        }
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let (role, status) = store.account_role_and_status(admin_id).unwrap();
        assert_eq!(role, crate::auth::ADMIN_ROLE);
        assert_eq!(status, STATUS_APPROVED);
        assert_eq!(store.admin_count().unwrap(), 1);
        assert!(store.validate_session(&admin_token).is_some());
    }

    #[tokio::test]
    async fn responses_do_not_expose_hashes_or_tokens() {
        let (router, _d, admin_token, _a, _i) = fixture(&[("x@soket.ai", STATUS_PENDING)]).await;
        let (_, body) = get_with_cookie(&router, "/api/admin/users/pending", &admin_token).await;
        // The listing must not contain Argon2 hashes or session digests.
        assert!(!body.contains("$argon2"), "{body}");
        assert!(!body.contains("password_hash"), "{body}");
        assert!(!body.contains("token_digest"), "{body}");
        assert!(!body.contains("soket_session="), "{body}");
    }

    // ==================================================================
    // Phase 4E: pending-list truncation reporting, reactivation
    // regression, and authorization re-verification.
    // ==================================================================

    /// A fixture that pre-creates `n` pending accounts directly through
    /// storage. Uses the unhashed bulk helper: Argon2id per row would make
    /// a 500-row test take minutes for no additional coverage.
    async fn fixture_with_pending(n: usize) -> (Router, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = AuthStore::open(&path).unwrap();
        store
            .sign_up(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        store.create_pending_users_unhashed("bulk", n).unwrap();
        let admin_token = store
            .sign_in(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        drop(store);
        let state = Arc::new(AuthState::new(Arc::new(
            AuthStore::open(&path).unwrap(),
        )));
        (admin_router(state), dir, admin_token)
    }

    #[tokio::test]
    async fn pending_list_reports_truncation_explicitly() {
        // One more pending account than the page bound.
        let n = PENDING_LIST_LIMIT as usize + 1;
        let (router, _d, admin_token) = fixture_with_pending(n).await;

        let (status, body) =
            get_with_cookie(&router, "/api/admin/users/pending", &admin_token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();

        // The page is bounded...
        assert_eq!(
            json["users"].as_array().unwrap().len(),
            PENDING_LIST_LIMIT as usize,
            "the page must be exactly the bound"
        );
        // ...and the response says so, rather than silently dropping rows.
        assert_eq!(json["truncated"], serde_json::json!(true), "{body}");
        assert_eq!(
            json["totalCount"],
            serde_json::json!(PENDING_LIST_LIMIT + 1),
            "totalCount must report the real backlog"
        );

        // Ordering is oldest-first, so the omitted row is the newest.
        let newest_shown = json["users"].as_array().unwrap().last().unwrap()["identifier"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            newest_shown,
            format!("bulk{}@soket.ai", PENDING_LIST_LIMIT - 1),
            "the omitted row must be the newest"
        );
    }

    #[tokio::test]
    async fn pending_list_reports_no_truncation_when_it_fits() {
        let (router, _d, admin_token) = fixture_with_pending(3).await;
        let (status, body) =
            get_with_cookie(&router, "/api/admin/users/pending", &admin_token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["users"].as_array().unwrap().len(), 3);
        assert_eq!(json["truncated"], serde_json::json!(false), "{body}");
        assert_eq!(json["totalCount"], serde_json::json!(3));
        // Response stays compatible: `users` is still an array of the same
        // four safe fields.
        for u in json["users"].as_array().unwrap() {
            for field in ["id", "identifier", "status", "createdAtMs"] {
                assert!(u.get(field).is_some(), "missing {field}: {u}");
            }
        }
    }

    #[tokio::test]
    async fn suspended_users_sessions_stay_invalid_after_reactivation() {
        // Full lifecycle regression: approved → suspended (sessions die) →
        // reactivated (old sessions STAY dead; only a fresh sign-in works).
        let (router, dir, admin_token, _a, ids) =
            fixture(&[("lifecycle@soket.ai", STATUS_APPROVED)]).await;
        let id = *ids.get("lifecycle@soket.ai").unwrap();

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let token_before = store
            .sign_in("lifecycle@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(store.validate_session(&token_before).is_some());

        // Suspend, then reactivate through the real endpoints.
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/suspend"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{id}/reactivate"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // Status is approved again...
        assert_eq!(
            store.account_role_and_status(id).unwrap().1,
            STATUS_APPROVED
        );
        // ...but the pre-suspension session was never resurrected.
        assert!(
            store.validate_session(&token_before).is_none(),
            "reactivation must not restore revoked sessions"
        );
        // A fresh sign-in works immediately.
        let fresh = store
            .sign_in("lifecycle@soket.ai", "a strong passphrase")
            .unwrap();
        assert!(store.validate_session(&fresh).is_some());
    }

    #[tokio::test]
    async fn unauthorized_status_changes_never_occur() {
        // An ordinary user attempts every transition against another
        // ordinary user's account; nothing may change.
        let (router, dir, _t, _a, ids) = fixture(&[
            ("actor@soket.ai", STATUS_APPROVED),
            ("target@soket.ai", STATUS_PENDING),
        ])
        .await;
        let target = *ids.get("target@soket.ai").unwrap();

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let actor_token = store
            .sign_in("actor@soket.ai", "a strong passphrase")
            .unwrap();

        for verb in ["approve", "reject", "suspend", "reactivate"] {
            let (status, body) = post_with_cookie(
                &router,
                &format!("/api/admin/users/{target}/{verb}"),
                &actor_token,
                "{}",
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{verb} by an ordinary user must be forbidden: {body}"
            );
        }

        // The target's status is untouched by all four attempts.
        assert_eq!(
            store.account_role_and_status(target).unwrap(),
            (USER_ROLE.to_string(), STATUS_PENDING.to_string())
        );
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn rejected_account_cannot_use_admin_endpoints() {
        // Rejected accounts cannot sign in; prove the guard also refuses a
        // session minted before rejection took effect.
        let (router, dir, _t, _a, ids) = fixture(&[("gone@soket.ai", STATUS_REJECTED)]).await;
        let gone_id = *ids.get("gone@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store.find_account_by_identifier("gone@soket.ai").unwrap();
        let stale_token = store.debug_mint_session_for(rec.id).unwrap();

        for uri in [
            "/api/admin/users/pending",
            &format!("/api/admin/users/{gone_id}/approve"),
        ] {
            let (status, body) = if uri.ends_with("/pending") {
                get_with_cookie(&router, uri, &stale_token).await
            } else {
                post_with_cookie(&router, uri, &stale_token, "{}").await
            };
            assert!(
                status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
                "{uri}: unexpected {status}: {body}"
            );
            assert!(!body.contains("\"users\""), "{uri} leaked the list: {body}");
        }
    }

    // ==================================================================
    // Phase 7: full user list + role management over HTTP
    // ==================================================================

    /// POST helper that sends a JSON body.
    async fn post_json(
        router: &Router,
        uri: &str,
        cookie: &str,
        body: &str,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = cookie_header(cookie) {
            builder = builder.header(header::COOKIE, value);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn admin_can_list_all_users_with_safe_fields() {
        let (router, _d, token, _a, _i) = fixture(&[
            ("alice@soket.ai", STATUS_APPROVED),
            ("bob@soket.ai", STATUS_PENDING),
            ("carol@soket.ai", STATUS_REJECTED),
        ])
        .await;
        let (status, body) = get_with_cookie(&router, "/api/admin/users", &token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["totalCount"], 4, "admin + three users: {body}");
        assert_eq!(v["users"].as_array().unwrap().len(), 4);
        // Safe fields only.
        for u in v["users"].as_array().unwrap() {
            assert!(u.get("id").is_some());
            assert!(u.get("identifier").is_some());
            assert!(u.get("role").is_some());
            assert!(u.get("status").is_some());
            assert!(u.get("createdAtMs").is_some());
            assert!(u.get("lastSignInAtMs").is_some());
        }
        // No secrets.
        assert!(!body.contains("$argon2"), "{body}");
        assert!(!body.contains("password_hash"), "{body}");
        assert!(!body.contains("soket_session="), "{body}");
        assert!(!body.contains("token_digest"), "{body}");
    }

    #[tokio::test]
    async fn user_list_search_filters_and_pagination_work() {
        let mut extras = vec![
            ("alice@soket.ai", STATUS_APPROVED),
            ("alina@soket.ai", STATUS_PENDING),
            ("bob@soket.ai", STATUS_APPROVED),
        ];
        for i in 0..7 {
            extras.push((
                Box::leak(format!("u{i}@soket.ai").into_boxed_str()),
                STATUS_PENDING,
            ));
        }
        let (router, _d, token, _a, _i) = fixture(&extras).await;

        // Search.
        let (s, b) = get_with_cookie(&router, "/api/admin/users?search=ali", &token).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["totalCount"], 2, "{b}");

        // Role filter.
        let (s, b) = get_with_cookie(&router, "/api/admin/users?role=admin", &token).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["totalCount"], 1);

        // Status filter.
        let (s, b) = get_with_cookie(&router, "/api/admin/users?status=pending", &token).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["totalCount"], 8, "alina + 7 extra pending");

        // Combined.
        let (s, b) = get_with_cookie(
            &router,
            "/api/admin/users?search=ali&status=approved",
            &token,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["totalCount"], 1, "only alice");

        // Pagination.
        let (s, b) = get_with_cookie(&router, "/api/admin/users?perPage=5&page=1", &token).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["users"].as_array().unwrap().len(), 5);
        assert_eq!(v["totalPages"], 3); // 11 accounts / 5

        let (s, b) = get_with_cookie(&router, "/api/admin/users?perPage=5&page=3", &token).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["users"].as_array().unwrap().len(), 1, "last page has 1");

        // perPage is clamped to the maximum.
        let (s, b) = get_with_cookie(
            &router,
            &format!(
                "/api/admin/users?perPage={}",
                crate::auth::ACCOUNT_LIST_MAX_PER_PAGE * 10
            ),
            &token,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["perPage"], crate::auth::ACCOUNT_LIST_MAX_PER_PAGE);

        // Unknown role/status are a 400, not silently ignored.
        let (s, _b) = get_with_cookie(&router, "/api/admin/users?role=superuser", &token).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _b) = get_with_cookie(&router, "/api/admin/users?status=deleted", &token).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // URL-encoded search terms decode correctly.
        let (s, b) = get_with_cookie(&router, "/api/admin/users?search=ali%40soket", &token).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["totalCount"], 0, "no identifier contains 'ali@soket'");
    }

    #[tokio::test]
    async fn ordinary_users_cannot_list_or_change_roles() {
        let (router, dir, _t, _a, ids) = fixture(&[("norm@soket.ai", STATUS_APPROVED)]).await;
        let norm_id = *ids.get("norm@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let norm_token = store
            .sign_in("norm@soket.ai", "a strong passphrase")
            .unwrap();

        // Listing is refused.
        let (s, b) = get_with_cookie(&router, "/api/admin/users", &norm_token).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
        assert!(!b.contains("\"users\""), "list leaked: {b}");

        // So is every role change, in either direction.
        for role in ["admin", "user"] {
            let (s, b) = post_json(
                &router,
                &format!("/api/admin/users/{norm_id}/role"),
                &norm_token,
                &format!("{{\"role\":\"{role}\"}}"),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{role}: {b}");
        }
        // And the target's role is unchanged.
        let (r, _) = store.account_role_and_status(norm_id).unwrap();
        assert_eq!(r, USER_ROLE);
    }

    #[tokio::test]
    async fn phase7_unauthenticated_requests_are_rejected() {
        let (router, _d, _t, _a, _i) = fixture(&[]).await;
        // GET without a cookie.
        let (s, b) = get_with_cookie(&router, "/api/admin/users", "").await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
        // POST without a cookie (with a well-formed body so the request
        // reaches the authorization guard rather than failing extraction).
        let (s, b) = post_json(&router, "/api/admin/users/1/role", "", "{}").await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
    }

    #[tokio::test]
    async fn promotion_changes_role_without_touching_anything_else() {
        let (router, dir, token, _a, ids) = fixture(&[("rise@soket.ai", STATUS_APPROVED)]).await;
        let rise_id = *ids.get("rise@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let sessions_before = store.session_count().unwrap();

        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{rise_id}/role"),
            &token,
            "{\"role\":\"admin\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(
            store.account_role_and_status(rise_id).unwrap(),
            (ADMIN_ROLE.to_string(), STATUS_APPROVED.to_string()),
            "promotion must leave status alone"
        );
        // No session minted or revoked.
        assert_eq!(store.session_count().unwrap(), sessions_before);
        // Two admins now.
        assert_eq!(store.admin_count().unwrap(), 2);
    }

    #[tokio::test]
    async fn promotion_requires_approved_status_over_http() {
        let (router, _d, token, _a, ids) = fixture(&[
            ("p@soket.ai", STATUS_PENDING),
            ("r@soket.ai", STATUS_REJECTED),
            ("s@soket.ai", STATUS_SUSPENDED),
        ])
        .await;
        for email in ["p@soket.ai", "r@soket.ai", "s@soket.ai"] {
            let id = *ids.get(email).unwrap();
            let (s, b) = post_json(
                &router,
                &format!("/api/admin/users/{id}/role"),
                &token,
                "{\"role\":\"admin\"}",
            )
            .await;
            assert_eq!(s, StatusCode::CONFLICT, "{email}: {b}");
            assert!(
                b.contains("approved"),
                "{email}: message should explain the requirement: {b}"
            );
        }
    }

    #[tokio::test]
    async fn admin_cannot_change_own_role_over_http() {
        let (router, _d, token, admin_id, _i) = fixture(&[]).await;
        for role in ["admin", "user"] {
            let (s, b) = post_json(
                &router,
                &format!("/api/admin/users/{admin_id}/role"),
                &token,
                &format!("{{\"role\":\"{role}\"}}"),
            )
            .await;
            assert_eq!(s, StatusCode::CONFLICT, "{role}: {b}");
        }
    }

    #[tokio::test]
    async fn last_admin_cannot_be_demoted_over_http() {
        let (router, _d, token, admin_id, _i) = fixture(&[]).await;
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{admin_id}/role"),
            &token,
            "{\"role\":\"user\"}",
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{b}");
        // With exactly one administrator the self-target guard fires first,
        // which is equally protective: either refusal is correct here. The
        // dedicated storage-level test covers the pure last-admin path.
        assert!(
            b.contains("own role") || b.contains("last"),
            "message should say why: {b}"
        );
    }

    #[tokio::test]
    async fn demotion_is_separate_and_requires_a_second_admin() {
        // With two admins, either may be demoted — including the original.
        let (router, dir, token, admin_id, ids) =
            fixture(&[("peer@soket.ai", STATUS_APPROVED)]).await;
        let peer_id = *ids.get("peer@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();

        // Promote the peer first.
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{peer_id}/role"),
            &token,
            "{\"role\":\"admin\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(store.admin_count().unwrap(), 2);

        // Now the ORIGINAL admin can be demoted by... themselves? No: the
        // self-target guard forbids it. Use the peer's session instead.
        let peer_token = store
            .sign_in("peer@soket.ai", "a strong passphrase")
            .unwrap();
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{admin_id}/role"),
            &peer_token,
            "{\"role\":\"user\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(store.admin_count().unwrap(), 1);
        assert_eq!(
            store.account_role_and_status(admin_id).unwrap().0,
            USER_ROLE
        );

        // The demoted admin immediately loses authority.
        let (s, b) = get_with_cookie(&router, "/api/admin/users", &token).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    }

    #[tokio::test]
    async fn approval_and_promotion_are_separate_actions() {
        // Approving an account does NOT make it an admin, and promoting an
        // approved account does not change its status.
        let (router, dir, token, _a, ids) = fixture(&[("split@soket.ai", STATUS_PENDING)]).await;
        let split_id = *ids.get("split@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();

        // Approve.
        let (s, b) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{split_id}/approve"),
            &token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(
            store.account_role_and_status(split_id).unwrap(),
            (USER_ROLE.to_string(), STATUS_APPROVED.to_string()),
            "approval must not grant admin"
        );

        // Promote: status stays approved.
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{split_id}/role"),
            &token,
            "{\"role\":\"admin\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(
            store.account_role_and_status(split_id).unwrap(),
            (ADMIN_ROLE.to_string(), STATUS_APPROVED.to_string()),
            "promotion must not alter status"
        );
    }

    #[tokio::test]
    async fn role_body_is_validated_and_rejects_extras() {
        let (router, _d, token, _a, ids) = fixture(&[("x@soket.ai", STATUS_APPROVED)]).await;
        let x_id = *ids.get("x@soket.ai").unwrap();

        // Missing body.
        let (s, _b) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{x_id}/role"),
            &token,
            "",
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // Unknown role.
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{x_id}/role"),
            &token,
            "{\"role\":\"root\"}",
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");

        // A smuggled status/email/password must not be honoured (and is
        // simply ignored — unknown fields are dropped by serde by default).
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{x_id}/role"),
            &token,
            "{\"role\":\"admin\",\"status\":\"approved\",\"password\":\"nope\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(
            store.account_role_and_status(x_id).unwrap(),
            (ADMIN_ROLE.to_string(), STATUS_APPROVED.to_string())
        );
    }

    #[tokio::test]
    async fn pending_suspended_and_rejected_accounts_cannot_become_admins_via_api() {
        let (router, _d, token, _a, ids) = fixture(&[
            ("p@soket.ai", STATUS_PENDING),
            ("s@soket.ai", STATUS_SUSPENDED),
            ("r@soket.ai", STATUS_REJECTED),
        ])
        .await;
        for email in ["p@soket.ai", "s@soket.ai", "r@soket.ai"] {
            let id = *ids.get(email).unwrap();
            let (s, b) = post_json(
                &router,
                &format!("/api/admin/users/{id}/role"),
                &token,
                "{\"role\":\"admin\"}",
            )
            .await;
            assert_eq!(s, StatusCode::CONFLICT, "{email}: {b}");
        }
        // No admin was created.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn user_list_reports_last_sign_in_only_when_recorded() {
        let (router, _d, token, _a, ids) = fixture(&[
            ("seen@soket.ai", STATUS_APPROVED),
            ("never@soket.ai", STATUS_PENDING),
        ])
        .await;
        // `seen` signs in; `never` does not.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        store
            .sign_in("seen@soket.ai", "a strong passphrase")
            .unwrap();

        let (s, b) = get_with_cookie(&router, "/api/admin/users", &token).await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        let mut seen_at: Option<i64> = None;
        let mut never_at: Option<i64> = None;
        for u in v["users"].as_array().unwrap() {
            match u["identifier"].as_str().unwrap() {
                "seen@soket.ai" => seen_at = u["lastSignInAtMs"].as_i64(),
                "never@soket.ai" => never_at = u["lastSignInAtMs"].as_i64(),
                _ => {}
            }
        }
        assert!(seen_at.is_some(), "sign-in must be reported: {b}");
        assert!(
            never_at.is_none(),
            "never-signed-in account must report null, got {never_at:?}"
        );
        let _ = ids;
    }

    #[tokio::test]
    async fn concurrent_role_changes_via_http_preserve_the_admin_invariant() {
        // Two administrators concurrently demote EACH OTHER. Exactly one
        // demotion must succeed, leaving exactly one administrator.
        let (router, dir, token, admin_id, ids) =
            fixture(&[("peer@soket.ai", STATUS_APPROVED)]).await;
        let peer_id = *ids.get("peer@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        // Promote the peer through the API.
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{peer_id}/role"),
            &token,
            "{\"role\":\"admin\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let peer_token = store
            .sign_in("peer@soket.ai", "a strong passphrase")
            .unwrap();

        // Each admin tries to demote the other at the same time.
        let r1 = {
            let router = router.clone();
            let uri = format!("/api/admin/users/{peer_id}/role");
            let cookie = token.clone();
            tokio::spawn(
                async move { post_json(&router, &uri, &cookie, "{\"role\":\"user\"}").await },
            )
        };
        let r2 = {
            let router = router.clone();
            let uri = format!("/api/admin/users/{admin_id}/role");
            let cookie = peer_token.clone();
            tokio::spawn(
                async move { post_json(&router, &uri, &cookie, "{\"role\":\"user\"}").await },
            )
        };
        let (s1, b1) = r1.await.unwrap();
        let (s2, b2) = r2.await.unwrap();
        // At most one succeeded; a CONFLICT for the loser is expected.
        let ok1 = s1 == StatusCode::OK;
        let ok2 = s2 == StatusCode::OK;
        assert!(
            !(ok1 && ok2),
            "both demotions succeeded ({b1} / {b2}): installation would have no admin"
        );
        let final_store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        assert_eq!(
            final_store.admin_count().unwrap(),
            1,
            "exactly one administrator must survive"
        );
    }

    // ==================================================================
    // User Management over HTTP: details, reset, sessions, delete,
    // activity — plus the authorization matrix for each new endpoint.
    // ==================================================================

    /// DELETE helper with a cookie.
    async fn delete_with_cookie(router: &Router, uri: &str, cookie: &str) -> (StatusCode, String) {
        let mut builder = Request::builder().uri(uri).method("DELETE");
        if let Some(value) = cookie_header(cookie) {
            builder = builder.header(header::COOKIE, value);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A fixture with an approved ordinary user signed in, returning
    /// (router, dir, admin_token, admin_id, user_id, user_token).
    async fn um_fixture()
    -> (
        Router,
        tempfile::TempDir,
        String,
        i64,
        i64,
        String,
    ) {
        let (_admin_only_router, dir, admin_token, admin_id, ids) =
            fixture(&[("victim@soket.ai", STATUS_APPROVED)]).await;
        let user_id = *ids.get("victim@soket.ai").unwrap();
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let user_token = store
            .sign_in("victim@soket.ai", "a strong passphrase")
            .unwrap();
        drop(store);

        // Both routers are mounted, exactly as the real app composes them:
        // admin endpoints for the management flows, /api/auth/* for the
        // public session + password-reset behaviour some tests exercise.
        let state = Arc::new(AuthState::new(Arc::new(
            AuthStore::open(dir.path().join("auth.db")).unwrap(),
        )));
        let router =
            admin_router(state.clone()).merge(crate::auth_http::auth_router(state));
        (router, dir, admin_token, admin_id, user_id, user_token)
    }

    #[tokio::test]
    async fn new_endpoints_are_closed_to_anonymous_and_ordinary_users() {
        let (router, _d, _admin_token, _admin_id, user_id, user_token) =
            um_fixture().await;
        // Each endpoint is exercised with its REAL method: a GET against a
        // POST-only route never reaches the auth layer (405), which would
        // make this check pass vacuously — or fail spuriously as it did.
        let get_endpoints = [
            format!("/api/admin/users/{user_id}"),
            format!("/api/admin/users/{user_id}/activity"),
        ];
        for ep in &get_endpoints {
            // Anonymous: 401.
            let (s, b) = get_with_cookie(&router, ep, "").await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{ep}: {b}");
            // Ordinary signed-in user: 403, nothing leaked.
            let (s, b) = get_with_cookie(&router, ep, &user_token).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{ep}: {b}");
            assert!(!b.contains("victim@soket.ai"), "{ep} leaked data: {b}");
        }
        for ep in [
            format!("/api/admin/users/{user_id}/reset-password"),
            format!("/api/admin/users/{user_id}/revoke-sessions"),
        ] {
            let (s, b) = post_with_cookie(&router, &ep, "", "{}").await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{ep}: {b}");
            let (s, b) = post_with_cookie(&router, &ep, &user_token, "{}").await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{ep}: {b}");
            assert!(!b.contains("victim@soket.ai"), "{ep} leaked data: {b}");
        }
        // DELETE likewise.
        let uri = format!("/api/admin/users/{user_id}");
        let (s, _) = delete_with_cookie(&router, &uri, "").await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _) = delete_with_cookie(&router, &uri, &user_token).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        // And nothing was deleted.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert!(store.find_account_by_id(user_id).is_ok());
    }

    #[tokio::test]
    async fn user_details_returns_safe_fields_only() {
        let (router, _d, admin_token, _admin_id, user_id, _u) = um_fixture().await;
        // um_fixture already signed the user in exactly once — that is the
        // single live session the details endpoint must report. (Signing in
        // again here would create a second session and the count below
        // would legitimately be 2.)
        let (s, body) =
            get_with_cookie(&router, &format!("/api/admin/users/{user_id}"), &admin_token).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["identifier"], "victim@soket.ai");
        assert_eq!(v["role"], "user");
        assert_eq!(v["status"], "approved");
        assert!(v["createdAtMs"].is_number());
        assert!(v["lastSignInAtMs"].is_number());
        assert_eq!(v["liveSessionCount"], 1);
        assert!(v["sessions"].as_array().unwrap().len() == 1);
        // Sensitive fields must be absent.
        for absent in [
            "password_hash",
            "passwordHash",
            "hash",
            "token_digest",
            "tokenDigest",
            "token",
            "secret",
        ] {
            assert!(
                !body.contains(absent),
                "details response mentions {absent}: {body}"
            );
        }
        assert!(!body.contains("$argon2"), "no hash material: {body}");

        // Unknown id: 404.
        let (s, _) =
            get_with_cookie(&router, "/api/admin/users/99999", &admin_token).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // Malformed id: 400.
        let (s, _) =
            get_with_cookie(&router, "/api/admin/users/not-a-number", &admin_token).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn reset_password_without_mail_configuration_is_honest() {
        // The test fixture has no mailer configured, so the endpoint must
        // answer 503 and mint NOTHING — never a silent fake success.
        let (router, dir, admin_token, _admin_id, user_id, _u) = um_fixture().await;
        let (s, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}/reset-password"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(
            body.to_lowercase().contains("email delivery is not configured"),
            "must explain why: {body}"
        );
        // No reset token row was created.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let conn = store.conn_for_tests();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM password_reset_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "nothing may be minted when mail cannot send");
    }

    #[tokio::test]
    async fn reset_password_with_mailer_delivers_and_audits() {
        // Build a router whose state carries a capturing sender.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = AuthStore::open(&path).unwrap();
        store
            .sign_up(crate::auth::LEGACY_ADMIN_IDENTIFIER, "correct horse battery")
            .unwrap();
        let user = store
            .create_pending_user("mailme@soket.ai", "a strong passphrase")
            .unwrap();
        store.set_account_status(user, STATUS_APPROVED).unwrap();
        let admin_token = store
            .sign_in(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        drop(store);

        let sender = std::sync::Arc::new(crate::mail::CapturingSender::new());
        let state = Arc::new(AuthState {
            store: Arc::new(AuthStore::open(&path).unwrap()),
            mailer: Some(sender.clone() as Arc<dyn crate::mail::MailSender>),
        });
        // The end-to-end completion flow below exercises the PUBLIC reset
        // endpoints too, so both routers are mounted — exactly as the real
        // app composes them.
        let router = admin_router(state.clone()).merge(crate::auth_http::auth_router(state));

        let (s, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{user}/reset-password"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["sentTo"], "mailme@soket.ai");
        assert!(v["expiresInMinutes"].is_number());

        // Exactly one email went out, to the right mailbox, carrying a link.
        let sent = sender.reset_messages();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to, "mailme@soket.ai");
        assert!(sent[0].reset_url.starts_with("/reset-password?token="));
        // The RESPONSE never echoes the token.
        assert!(
            !body.contains(sent[0].reset_url.split('=').nth(1).unwrap()),
            "the raw token must never appear in the API response: {body}"
        );

        // The audit trail records the issuance.
        let check = AuthStore::open(&path).unwrap();
        let (entries, total) = check.list_account_activity(user, 10, 0).unwrap();
        assert!(total >= 1, "issuance must be audited");
        assert!(entries
            .iter()
            .any(|e| e.kind == crate::auth::audit_event::PASSWORD_RESET_ISSUED));

        // --- and the full public completion flow works end-to-end ---
        let token = sent[0].reset_url.split('=').nth(1).unwrap().to_string();

        // Preview says valid.
        let (ps, pb) = post_with_cookie(
            &router,
            "/api/auth/password/reset/preview",
            "",
            &format!("{{\"token\":\"{token}\"}}"),
        )
        .await;
        assert_eq!(ps, StatusCode::OK, "{pb}");
        assert!(pb.contains("\"valid\":true"), "{pb}");

        // Complete with a too-short password: 400, token still live.
        let (cs, cb) = post_with_cookie(
            &router,
            "/api/auth/password/reset",
            "",
            &format!("{{\"token\":\"{token}\",\"password\":\"short\"}}"),
        )
        .await;
        assert_eq!(cs, StatusCode::BAD_REQUEST, "{cb}");

        // Complete properly.
        let (cs, cb) = post_with_cookie(
            &router,
            "/api/auth/password/reset",
            "",
            &format!("{{\"token\":\"{token}\",\"password\":\"brand new passphrase\"}}"),
        )
        .await;
        assert_eq!(cs, StatusCode::OK, "{cb}");

        // Replay: refused.
        let (rs, rb) = post_with_cookie(
            &router,
            "/api/auth/password/reset",
            "",
            &format!("{{\"token\":\"{token}\",\"password\":\"another one ok\"}}"),
        )
        .await;
        assert_eq!(rs, StatusCode::BAD_REQUEST, "{rb}");

        // The new password signs in; the old one does not.
        let final_store = AuthStore::open(&path).unwrap();
        assert!(final_store
            .sign_in("mailme@soket.ai", "brand new passphrase")
            .is_ok());
        assert!(matches!(
            final_store.sign_in("mailme@soket.ai", "a strong passphrase"),
            Err(crate::auth::AuthError::InvalidCredentials)
        ));
        // Completion was audited too.
        let (_, total) = final_store.list_account_activity(user, 10, 0).unwrap();
        assert!(total >= 2, "completion must be audited");
    }

    #[tokio::test]
    async fn failed_delivery_invalidates_the_minted_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = AuthStore::open(&path).unwrap();
        store
            .sign_up(crate::auth::LEGACY_ADMIN_IDENTIFIER, "correct horse battery")
            .unwrap();
        let user = store
            .create_pending_user("undelivered@soket.ai", "a strong passphrase")
            .unwrap();
        store.set_account_status(user, STATUS_APPROVED).unwrap();
        let admin_token = store
            .sign_in(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        drop(store);

        let sender = std::sync::Arc::new(crate::mail::CapturingSender::new());
        sender.fail(crate::mail::DeliveryError::Transport("down".into()));
        let state = Arc::new(AuthState {
            store: Arc::new(AuthStore::open(&path).unwrap()),
            mailer: Some(sender.clone() as Arc<dyn crate::mail::MailSender>),
        });
        let router = admin_router(state);

        let (s, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{user}/reset-password"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{body}");
        // Nothing was delivered…
        assert!(sender.reset_messages().is_empty());
        // …and no live token remains: an undelivered link can never be used.
        let check = AuthStore::open(&path).unwrap();
        let conn = check.conn_for_tests();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM password_reset_tokens
                  WHERE consumed_at_ms IS NULL AND expires_at_ms > ?1",
                [chrono::Utc::now().timestamp_millis()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 0, "failed delivery must invalidate the token");
    }

    #[tokio::test]
    async fn revoke_sessions_endpoint_revokes_and_reports() {
        let (router, dir, admin_token, _admin_id, user_id, user_token) =
            um_fixture().await;
        // Two live sessions for the victim.
        {
            let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
            let _ = store
                .sign_in("victim@soket.ai", "a strong passphrase")
                .unwrap();
        }

        let (s, body) = post_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}/revoke-sessions"),
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["revoked"], 2, "both victim sessions: {body}");

        // The victim's cookie is dead on the very next request.
        let (s, _) = get_with_cookie(&router, "/api/admin/users/pending", &user_token).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);

        // The admin's still works, and the action was audited.
        let (s, _) = get_with_cookie(&router, "/api/admin/users/pending", &admin_token).await;
        assert_eq!(s, StatusCode::OK);
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let (entries, _) = store.list_account_activity(user_id, 10, 0).unwrap();
        assert!(entries
            .iter()
            .any(|e| e.kind == crate::auth::audit_event::SESSIONS_REVOKED));

        // Unknown account: 404.
        let (s, _) = post_with_cookie(
            &router,
            "/api/admin/users/99999/revoke-sessions",
            &admin_token,
            "{}",
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn remove_user_deletes_and_cleans_up_everything() {
        let (router, dir, admin_token, _admin_id, user_id, user_token) =
            um_fixture().await;

        let (s, body) = delete_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert!(body.contains("victim@soket.ai"));

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        // Account gone.
        assert!(matches!(
            store.find_account_by_id(user_id),
            Err(AccountError::NotFound)
        ));
        // Sessions and tokens cleaned up. Scoped: the raw-connection guard
        // must be released before the store calls below, because they lock
        // the same (non-reentrant) connection mutex.
        {
            let conn = store.conn_for_tests();
            let sessions: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE account_id = ?1",
                    [user_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sessions, 0);
            let resets: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM password_reset_tokens WHERE account_id = ?1",
                    [user_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(resets, 0);
        }
        // The victim's cookie stopped validating (account join fails).
        assert!(store.validate_session(&user_token).is_none());
        // The audit trail survived with the email snapshot.
        let (entries, total) = store.list_account_activity(user_id, 10, 0).unwrap();
        assert!(total >= 1, "deletion must be audited");
        assert!(entries
            .iter()
            .any(|e| e.kind == crate::auth::audit_event::ACCOUNT_DELETED
                && e.subject_email.as_deref() == Some("victim@soket.ai")));
        // The admin survives untouched.
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn admin_cannot_delete_themselves_or_the_last_admin() {
        let (router, _d, admin_token, admin_id, _u, _ut) = um_fixture().await;
        // Self-delete: refused with a clear conflict.
        let (s, body) = delete_with_cookie(
            &router,
            &format!("/api/admin/users/{admin_id}"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{body}");
        assert!(
            body.to_lowercase().contains("own account"),
            "must say why: {body}"
        );
        // Still exactly one admin, still signed in.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(store.admin_count().unwrap(), 1);
        assert!(store.validate_session(&admin_token).is_some());
    }

    #[tokio::test]
    async fn last_admin_cannot_be_deleted_even_by_another_admin() {
        // Two admins; each tries to delete the other. The second deletion
        // (which would strand the install) must be refused.
        let (router, dir, admin_token, admin_id, ids) =
            fixture(&[("peer@soket.ai", STATUS_APPROVED)]).await;
        let peer_id = *ids.get("peer@soket.ai").unwrap();
        // Promote the peer through the API.
        let (s, b) = post_json(
            &router,
            &format!("/api/admin/users/{peer_id}/role"),
            &admin_token,
            "{\"role\":\"admin\"}",
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");

        // Peer deletes the original admin: legal.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let peer_token = store
            .sign_in("peer@soket.ai", "a strong passphrase")
            .unwrap();
        drop(store);
        let (s, b) = delete_with_cookie(
            &router,
            &format!("/api/admin/users/{admin_id}"),
            &peer_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");

        // Now the peer is the LAST admin: deleting them is refused.
        let (s, b) = delete_with_cookie(
            &router,
            &format!("/api/admin/users/{peer_id}"),
            &peer_token,
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{b}");
        let final_store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        assert_eq!(final_store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn activity_endpoint_lists_events_with_descriptions() {
        let (router, dir, admin_token, _admin_id, user_id, _u) = um_fixture().await;
        // Generate history THROUGH THE API so the audit trail is written by
        // the same path production uses: um_fixture's sign-in produced one
        // signin_success; suspend + reactivate over HTTP each record their
        // own audit event.
        {
            let (s, b) = post_with_cookie(
                &router,
                &format!("/api/admin/users/{user_id}/suspend"),
                &admin_token,
                "{}",
            )
            .await;
            assert_eq!(s, StatusCode::OK, "suspend: {b}");
            let (s, b) = post_with_cookie(
                &router,
                &format!("/api/admin/users/{user_id}/reactivate"),
                &admin_token,
                "{}",
            )
            .await;
            assert_eq!(s, StatusCode::OK, "reactivate: {b}");
        }
        let (s, body) = get_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}/activity"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let events = v["events"].as_array().unwrap();
        assert!(
            events.len() >= 3,
            "sign-in + suspend + activate: {body}"
        );
        // Every event carries a timestamp and a readable description.
        for e in events {
            assert!(e["atMs"].is_number(), "{e}");
            assert!(
                e["description"].as_str().unwrap_or("").len() > 3,
                "readable description required: {e}"
            );
        }
        // Kinds present.
        let kinds: Vec<&str> = events
            .iter()
            .filter_map(|e| e["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"signin_success"), "{kinds:?}");
        assert!(kinds.contains(&"account_suspended"), "{kinds:?}");
        assert!(kinds.contains(&"account_activated"), "{kinds:?}");
        // No secrets in the payload.
        assert!(!body.contains("$argon2"));
        assert!(!body.contains("token_digest"));
    }

    #[tokio::test]
    async fn activity_survives_deletion_and_paginates() {
        let (router, _d, admin_token, _admin_id, user_id, _u) = um_fixture().await;
        // Delete the account, then read its (preserved) history.
        let (s, _) = delete_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        let (s, body) = get_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}/activity"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["totalCount"].as_i64().unwrap() >= 1);
        // The email snapshot titles the panel even after deletion.
        assert_eq!(v["subjectEmail"], "victim@soket.ai");

        // Pagination parameters are honoured and clamped.
        let (s, body) = get_with_cookie(
            &router,
            &format!("/api/admin/users/{user_id}/activity?page=1&perPage=1"),
            &admin_token,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["events"].as_array().unwrap().len() <= 1);
        assert_eq!(v["perPage"], 1);
    }

    #[tokio::test]
    async fn suspended_account_gets_distinct_message_over_http() {
        let (router, dir, _admin_token, _admin_id, user_id, _u) = um_fixture().await;
        {
            let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
            store.set_account_status(user_id, STATUS_SUSPENDED).unwrap();
        }
        // Correct credentials → 403 with the suspended message.
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/signin")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"identifier":"victim@soket.ai","password":"a strong passphrase"}"#,
            ))
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            body.to_lowercase().contains("suspended"),
            "must explain the suspension: {body}"
        );

        // Wrong credentials → still a generic 401, no status leak.
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/signin")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"identifier":"victim@soket.ai","password":"wrong password!"}"#,
            ))
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes), r#"{"error":"invalid credentials"}"#);
    }
}
