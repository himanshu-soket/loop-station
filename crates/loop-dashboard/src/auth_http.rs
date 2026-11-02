//! HTTP surface for authentication: JSON endpoints plus the guard that
//! protects workspace routes.
//!
//! Routes:
//! * `GET  /api/auth/status`   — public session probe (no secrets).
//! * `POST /api/auth/signup`   — **first-run administrator bootstrap only**;
//!   refuses once an administrator exists. Not a public registration route.
//! * `POST /api/auth/register` — public registration; always creates a
//!   `pending` user that cannot sign in until approved.
//! * `POST /api/auth/signin`   — verify credentials, set session cookie.
//! * `POST /api/auth/signout`  — invalidate the session, clear the cookie.
//!
//! Bootstrap policy: `/api/auth/signup` exists so a fresh installation can
//! create its first administrator without a CLI. It is guarded by
//! `is_first_run()` *and* the database's partial unique index on
//! `role = 'admin'`, so it can never be replayed to mint a second
//! administrator. New users must go through `/api/auth/register`, which
//! ignores any client-supplied role/status and always stores
//! `user`/`pending`.
//!
//! Two guard flavors, both driven by the same cookie:
//! * **Page guard** — HTML routes redirect unauthenticated visitors to
//!   `/signin?next=<original>` so a deep link survives sign-in.
//! * **API guard** — `/api/*` answers 401 JSON so the SPAs can react
//!   programmatically instead of following a redirect.
//!
//! CSRF: sessions use `SameSite=Lax` cookies. All state-changing endpoints
//! are JSON-only POSTs under `/api/auth/*`; browsers do not attach Lax
//! cookies to cross-site POSTs, which closes the CSRF vector for this
//! local application without adding token plumbing to every page.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

#[cfg(test)]
use crate::auth::SESSION_COOKIE;
use crate::auth::{
    session_clear_cookie, session_set_cookie, session_token_from_cookie_header, AuthError,
    AuthStore, SessionInfo, STATUS_PENDING,
};

/// Shared auth state handed to routers.
pub struct AuthState {
    /// The authenticator.
    pub store: Arc<AuthStore>,
    /// Email delivery, when configured (`LOOP_EMAIL_*`). `None` means the
    /// instance cannot send mail; password-reset issuance then answers a
    /// clear "email delivery is not configured" instead of pretending.
    pub mailer: Option<Arc<dyn crate::mail::MailSender>>,
}

impl AuthState {
    /// Build state over a store with no mail delivery (the historical
    /// shape; every existing construction site keeps working).
    pub fn new(store: Arc<AuthStore>) -> Self {
        Self {
            store,
            mailer: None,
        }
    }
}

/// Router for `/api/auth/*` plus the public session probe.
pub fn auth_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/signup", post(sign_up))
        .route("/api/auth/register", post(register))
        .route("/api/auth/signin", post(sign_in))
        .route("/api/auth/signout", post(sign_out))
        // Password reset completion (User Management phase). Public by
        // necessity: the whole point is that the user cannot sign in. The
        // token itself is the bearer credential, and it is single-use,
        // expiring and attempt-bounded — all enforced server-side.
        .route("/api/auth/password/reset/preview", post(reset_preview))
        .route("/api/auth/password/reset", post(reset_complete))
        // Registration bodies are tiny (email + password); anything larger
        // is abusive or malformed and can be rejected before it is buffered.
        .layer(DefaultBodyLimit::max(REGISTER_BODY_LIMIT))
        .with_state(state)
}

/// `GET /api/auth/status` — is an owner account provisioned, and is this
/// request signed in? Public: the SPA needs it before showing auth pages.
async fn auth_status(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let first_run = state.store.is_first_run().unwrap_or(false);
    let session = current_session(&state, &headers).await;
    Json(serde_json::json!({
        "firstRun": first_run,
        "signedIn": session.is_some(),
        "identifier": session.map(|s| s.identifier),
    }))
}

#[derive(Deserialize)]
struct SignUpRequest {
    identifier: String,
    password: String,
}

/// `POST /api/auth/signup` — **initial administrator bootstrap** (first run
/// only).
///
/// This is deliberately *not* a public registration route. It exists so a
/// fresh installation can create its first administrator without a CLI, and
/// it refuses to do anything once one exists:
///
/// * On a fresh database (`is_first_run()` true) it creates the sole
///   administrator with `role = admin`, `status = approved`. The identifier
///   is validated by [`AuthStore::sign_up`] (the existing grandfathered
///   Gmail address remains legal here; the `@soket.ai` restriction applies
///   only to new registrations via `/api/auth/register`).
/// * Once an administrator exists it returns `409 CONFLICT` and writes
///   nothing — it cannot be used to mint additional administrators, and the
///   single-admin invariant is additionally enforced by the database's
///   partial unique index on `role = 'admin'`.
///
/// New users must use [`register`], which always creates a **pending**
/// account that cannot sign in until an administrator approves it.
async fn sign_up(State(state): State<Arc<AuthState>>, Json(body): Json<SignUpRequest>) -> Response {
    // Refuse before hashing when an administrator already exists: the
    // expensive Argon2id work should never run for a doomed request.
    match state.store.is_first_run() {
        Ok(true) => {}
        Ok(false) => return auth_error_response(AuthError::OwnerExists),
        Err(e) => return auth_error_response(e),
    }
    // Argon2id + SQLite on the blocking pool, not an async worker.
    let store = state.store.clone();
    let identifier = body.identifier;
    let password = body.password;
    let result = tokio::task::spawn_blocking(move || store.sign_up(&identifier, &password)).await;
    match result {
        Ok(Ok(())) => {
            (StatusCode::CREATED, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Ok(Err(e)) => auth_error_response(e),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "authentication unavailable" })),
        )
            .into_response(),
    }
}

/// Upper bound on a registration request body. A legal request is a small
/// JSON object (`{"email":"…@soket.ai","password":"…"}`); anything beyond
/// this is abusive and is rejected before buffering.
const REGISTER_BODY_LIMIT: usize = 16 * 1024;

/// Registration request shape.
///
/// Deliberately **ignores** any client-supplied `role`, `status`,
/// `isAdmin` or similar authority fields: they are not even deserialized,
/// so no request can promote itself. The server alone decides that new
/// accounts are `user`/`pending`.
#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
}

/// `POST /api/auth/register` — public registration for new users.
///
/// Flow (all validation server-side):
/// 1. Body size is capped by [`REGISTER_BODY_LIMIT`] at the router layer.
/// 2. **Rate limit** — the global registration budget is checked first,
///    before any validation or storage work, so a throttled request costs
///    one lock acquisition and no Argon2id time. See
///    [`AuthStore::allow_registration_attempt`] for the policy.
/// 3. The email is normalized and validated against the exact `@soket.ai`
///    domain by [`crate::auth_email::normalize_registration_email`].
/// 4. Password length is validated (8..=256 chars) inside
///    [`AuthStore::create_pending_user`], which also performs the Argon2id
///    hash with a fresh random salt.
/// 5. The account is created with `role = user`, `status = pending`. No
///    session is minted — a pending account cannot authenticate until an
///    administrator approves it.
///
/// Email delivery is intentionally *not* part of this flow: registration
/// succeeds without Resend credentials or OTP verification (those arrive in
/// a later phase), so a missing mail configuration never blocks it.
///
/// Responses are deliberately generic about the account's existence where
/// that could leak information, and never include a password hash, session
/// token, or the numeric account id.
async fn register(
    State(state): State<Arc<AuthState>>,
    Json(body): Json<RegisterRequest>,
) -> Response {
    // Rate limit BEFORE anything else. Ordering note: the limiter is
    // global, so this check cannot be used to probe whether a particular
    // address is registered — every caller hits the same budget, and the
    // 429 carries no per-identity information.
    //
    // A limited request returns 429 *even when the body would otherwise
    // fail validation*, which is deliberate: answering 400 first would let
    // an attacker distinguish "rate limited" from "rejected input" and
    // infer nothing useful either way, but keeping the limiter first means
    // abusive traffic is shed before the expensive paths run at all.
    if state.store.allow_registration_attempt().is_err() {
        return registration_limited_response();
    }

    // Cheap sanity bound before handing off to the store; the authoritative
    // checks live in create_pending_user.
    if body.email.len() > MAX_REGISTER_EMAIL_LEN || body.password.len() > MAX_REGISTER_PASSWORD_LEN
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "input too long" })),
        )
            .into_response();
    }

    // The blocking work (Argon2id + SQLite insert) runs on the blocking
    // pool: the handler must not stall the async runtime's workers.
    let store = state.store.clone();
    let result =
        tokio::task::spawn_blocking(move || store.create_pending_user(&body.email, &body.password))
            .await;

    match result {
        Ok(Ok(_id)) => {
            // Success: tell the user their request awaits approval. The
            // account id is intentionally not echoed back.
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "ok": true,
                    "status": STATUS_PENDING,
                    "message": "Registration received. An administrator must approve your account before you can sign in."
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        // spawn_blocking panicked or was cancelled — never leak internals.
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "registration unavailable" })),
        )
            .into_response(),
    }
}

/// Defensive pre-store bound on the email field (the real limit, 254, is
/// enforced by `normalize_registration_email`; this stops pathological
/// input before it reaches allocation-heavy code).
const MAX_REGISTER_EMAIL_LEN: usize = 320;

/// Defensive pre-store bound on the password field (the real limit, 256, is
/// enforced by `create_pending_user`).
const MAX_REGISTER_PASSWORD_LEN: usize = 1024;

/// The 429 used when the registration budget is exhausted.
///
/// Deliberately identical for every caller — no per-identity detail, no
/// hint about how much budget remains or which address triggered it — so it
/// cannot be used to probe account existence. Includes a standard
/// `Retry-After` so a legitimate client can back off correctly, and
/// `Cache-Control: no-store` so intermediaries never cache a throttled
/// answer (or a successful one).
fn registration_limited_response() -> Response {
    let seconds = crate::auth::REGISTRATION_WINDOW.as_secs();
    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            (header::RETRY_AFTER, seconds.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Json(serde_json::json!({
            "error": "too many registration attempts; try again later"
        })),
    )
        .into_response()
}

/// The 429 used when the sign-in throttle engages.
///
/// Mirrors [`registration_limited_response`]: the same shape, the same
/// generic body (unchanged from before this header existed), and a
/// `Retry-After` derived from [`crate::auth::SIGNIN_WINDOW`] — the very
/// window the limiter counts over — so the value can never drift from the
/// enforced policy. `Cache-Control: no-store` keeps the throttled answer
/// out of any intermediary cache.
fn signin_limited_response() -> Response {
    let seconds = crate::auth::SIGNIN_WINDOW.as_secs();
    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            (header::RETRY_AFTER, seconds.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Json(serde_json::json!({
            "error": "too many attempts; try again later"
        })),
    )
        .into_response()
}

/// Map an [`crate::auth::AccountError`] to a safe HTTP response.
///
/// Storage failures are reported generically so nothing about the database
/// leaks; everything else uses the error's own status code and message,
/// which contain no secrets.
fn account_error_response(e: crate::auth::AccountError) -> Response {
    let status = e.status();
    let message = if matches!(e, crate::auth::AccountError::Storage(_)) {
        "registration unavailable".to_string()
    } else {
        e.to_string()
    };
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[derive(Deserialize)]
struct SignInRequest {
    identifier: String,
    password: String,
}

/// `POST /api/auth/signin` — verify credentials, set the session cookie.
///
/// The store call performs Argon2id verification and SQLite I/O, so it runs
/// on the blocking pool rather than an async worker thread.
async fn sign_in(State(state): State<Arc<AuthState>>, Json(body): Json<SignInRequest>) -> Response {
    let store = state.store.clone();
    let identifier = body.identifier;
    let password = body.password;
    let result = tokio::task::spawn_blocking(move || store.sign_in(&identifier, &password)).await;
    match result {
        Ok(Ok(token)) => (
            StatusCode::OK,
            [(header::SET_COOKIE, session_set_cookie(&token, false))],
            Json(serde_json::json!({ "ok": true })),
        )
            .into_response(),
        // The store's own error mapping (generic messages, no internals).
        Ok(Err(e)) => auth_error_response(e),
        // spawn_blocking panicked/cancelled: report generically, never leak.
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "authentication unavailable" })),
        )
            .into_response(),
    }
}

/// `POST /api/auth/signout` — invalidate the session, clear the cookie.
async fn sign_out(State(state): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if let Some(token) = token_from_headers(&headers) {
        let _ = state.store.sign_out(token);
    }
    (
        StatusCode::OK,
        [(header::SET_COOKIE, session_clear_cookie(false))],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Password reset completion (User Management phase)
//
// The ADMIN side (issuing a reset) lives in admin_http.rs behind
// require_admin. These two endpoints are the PUBLIC side: the person who
// received the email proves control of the token and chooses a new
// password. They cannot authenticate first — that is the entire problem a
// reset solves.
// ---------------------------------------------------------------------------

/// Request body for both public reset endpoints.
///
/// Only the token and (for completion) the new password. Nothing else from
/// the body is deserialized, so no client field can influence which
/// account is affected.
#[derive(Deserialize)]
struct PasswordResetBody {
    token: String,
    #[serde(default)]
    password: Option<String>,
}

/// `POST /api/auth/password/reset/preview` — does this token still work?
///
/// Lets the reset page decide between "show the form" and "this link has
/// expired" without consuming the token. The answer is boolean and never
/// explains WHY a token is invalid, so it cannot be used to enumerate or
/// brute-force tokens any faster than attempting the reset itself.
async fn reset_preview(
    State(state): State<Arc<AuthState>>,
    Json(body): Json<PasswordResetBody>,
) -> Response {
    let store = state.store.clone();
    let token = body.token;
    let valid = tokio::task::spawn_blocking(move || store.peek_password_reset(&token))
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(false);
    (
        StatusCode::OK,
        [(
            header::CACHE_CONTROL,
            "no-store".to_string(),
        )],
        Json(serde_json::json!({ "valid": valid })),
    )
        .into_response()
}

/// `POST /api/auth/password/reset` — consume the token, set the password.
///
/// Security properties, all enforced in [`AuthStore::complete_password_reset`]:
/// * single-use (two racing callers cannot both consume one token),
/// * expiring, attempt-bounded against token brute force,
/// * the new password is validated and Argon2id-hashed server-side; the
///   plaintext exists only inside this request and is never stored or logged,
/// * completing a reset revokes every existing session for the account.
async fn reset_complete(
    State(state): State<Arc<AuthState>>,
    Json(body): Json<PasswordResetBody>,
) -> Response {
    let Some(password) = body.password else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "a new password is required" })),
        )
            .into_response();
    };
    let store = state.store.clone();
    let token = body.token;
    let result = tokio::task::spawn_blocking(move || {
        store.complete_password_reset(&token, &password)
    })
    .await;

    match result {
        Ok(Ok((account_id, identifier))) => {
            // Audit: the subject acted on their own behalf with the token.
            let store = state.store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                store.record_audit(
                    crate::auth::audit_event::PASSWORD_RESET_COMPLETED,
                    None,
                    Some((account_id, identifier.as_str())),
                    "password changed via administrator-triggered reset",
                );
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
                    "message": "Password updated. Sign in with your new password."
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => account_error_response(e),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "authentication unavailable" })),
        )
            .into_response(),
    }
}

fn auth_error_response(e: AuthError) -> Response {
    // The sign-in throttle advertises how long to wait (Retry-After); every
    // other error keeps its existing body and headers exactly as before.
    if matches!(e, AuthError::RateLimited) {
        return signin_limited_response();
    }
    let status = e.status();
    // Storage errors are logged server-side but reported generically.
    let message = if matches!(e, AuthError::Storage(_)) {
        "authentication unavailable".to_string()
    } else {
        e.to_string()
    };
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Session for the current request, if the cookie validates.
///
/// The store's SQLite work runs on the blocking pool: this is called from
/// middleware on EVERY guarded request, so it must never occupy an async
/// worker thread.
async fn current_session(state: &AuthState, headers: &HeaderMap) -> Option<SessionInfo> {
    // Own the token: the borrowed header slice cannot cross into 'static
    // spawn_blocking closures.
    let token = token_from_headers(headers)?.to_string();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.validate_session(&token))
        .await
        .ok()
        .flatten()
}

fn token_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(session_token_from_cookie_header)
}

/// Page guard: redirect unauthenticated HTML requests to the sign-in page,
/// preserving the originally requested path in `?next=`.
pub async fn require_auth_page(
    State(state): State<Arc<AuthState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if current_session(&state, req.headers()).await.is_some() {
        return next.run(req).await;
    }
    let mut target = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    // The bare observatory root is not a destination anyone asked for: it
    // is just the default URL of the dashboard. Landing back there after
    // sign-in feels like being skipped past the Welcome page, so rewrite
    // it to the console's Welcome route. Real deep links (with a hash or
    // a specific path) are preserved untouched.
    if target == "/" || target == "/index.html" {
        target = "/console#overview".to_string();
    }
    // Only the query VALUE is encoded; the redirect itself must stay a
    // plain relative URL (/signin?next=%2Fconsole).
    let location = format!("/signin?next={}", urlencode(&target));
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

/// API guard: answer 401 JSON for unauthenticated `/api/*` calls.
pub async fn require_auth_api(
    State(state): State<Arc<AuthState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if current_session(&state, req.headers()).await.is_some() {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "authentication required" })),
    )
        .into_response()
}

/// Minimal percent-encoding for query values (path + query preservation).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{STATUS_APPROVED, STATUS_SUSPENDED};
    use tower::ServiceExt;

    fn router() -> (Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
        let state = Arc::new(AuthState::new(store));
        (auth_router(state), dir)
    }

    async fn json_post(router: &Router, uri: &str, body: &str) -> (StatusCode, String, HeaderMap) {
        json_post_with_cookie(router, uri, body, "").await
    }

    async fn json_post_with_cookie(
        router: &Router,
        uri: &str,
        body: &str,
        cookie: &str,
    ) -> (StatusCode, String, HeaderMap) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if !cookie.is_empty() {
            builder = builder.header(header::COOKIE, cookie);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let headers = res.headers().clone();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            String::from_utf8_lossy(&bytes).into_owned(),
            headers,
        )
    }

    #[tokio::test]
    async fn signup_signin_signout_flow_over_http() {
        let (router, _d) = router();

        // Sign-up (first run).
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner@local","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // Duplicate sign-up refused.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"other","password":"another password"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");

        // Sign-in with wrong password.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner@local","password":"nope-nope-nope"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body, r#"{"error":"invalid credentials"}"#);

        // Correct sign-in sets a cookie.
        let (status, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner@local","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set_cookie.starts_with(SESSION_COOKIE), "{set_cookie}");
        assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
        assert!(set_cookie.contains("SameSite=Lax"), "{set_cookie}");
        let token = set_cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches(&format!("{SESSION_COOKIE}="));

        // Status reflects the signed-in session.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], true);
        assert_eq!(json["identifier"], "owner@local");

        // Sign-out invalidates the session (cookie sent so we know which one).
        let (status, _, headers) = json_post_with_cookie(
            &router,
            "/api/auth/signout",
            "{}",
            &format!("{SESSION_COOKIE}={token}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], false);
    }

    #[tokio::test]
    async fn guards_redirect_pages_but_401_apis() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
        store.sign_up("owner", "correct horse battery").unwrap();
        let state = Arc::new(AuthState::new(store));

        // A protected page router: /guarded uses the page guard.
        let app = Router::new()
            .route("/guarded", get(|| async { "<html>ok</html>" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth_page,
            ));

        // Unauthenticated → 303 to /signin?next=/guarded.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let loc = res
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(loc.starts_with("/signin?next="), "{loc}");
        assert!(loc.contains("%2Fguarded"), "path preserved: {loc}");

        // The bare observatory root is rewritten to the console Welcome
        // page, so sign-in never lands on "Overview" instead of Welcome.
        let app_root = Router::new()
            .route("/", get(|| async { "<html>root</html>" }))
            .route("/index.html", get(|| async { "<html>root</html>" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth_page,
            ));
        for page in ["/", "/index.html"] {
            let res = app_root
                .clone()
                .oneshot(Request::builder().uri(page).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::SEE_OTHER, "{page}");
            let loc = res
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap();
            assert!(
                loc.ends_with("%2Fconsole%23overview"),
                "{page} should target the Welcome page, got {loc}"
            );
        }

        // Authenticated → passes through.
        let token = state
            .store
            .sign_in("owner", "correct horse battery")
            .unwrap();
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // API guard flavor.
        let api = Router::new()
            .route("/api/thing", get(|| async { "data" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth_api,
            ));
        let res = api
            .oneshot(
                Request::builder()
                    .uri("/api/thing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes)
            .into_owned()
            .contains("authentication required"));
    }

    #[tokio::test]
    async fn signin_is_rate_limited_over_http() {
        let (router, _d) = router();
        json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        for _ in 0..8 {
            let _ = json_post(
                &router,
                "/api/auth/signin",
                r#"{"identifier":"owner","password":"bad"}"#,
            )
            .await;
        }
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    }

    /// Phase 10 / Phase 9 finding L-1: the sign-in 429 must advertise how
    /// long to wait, exactly as the registration 429 already does. The value
    /// comes from `SIGNIN_WINDOW` — the window the limiter actually counts
    /// over — so it can never drift from the enforced policy.
    #[tokio::test]
    async fn signin_rate_limit_includes_retry_after_header() {
        let (router, _d) = router();
        json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        // Exhaust the throttle with failed attempts.
        for _ in 0..crate::auth::SIGNIN_ATTEMPT_LIMIT {
            let _ = json_post(
                &router,
                "/api/auth/signin",
                r#"{"identifier":"owner","password":"bad"}"#,
            )
            .await;
        }
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        let retry_after = headers
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_else(|| panic!("429 without Retry-After: {headers:?}"));
        assert!(
            retry_after.chars().all(|c| c.is_ascii_digit()) && !retry_after.is_empty(),
            "Retry-After must be numeric seconds, got {retry_after:?}"
        );
        // It must equal the configured sign-in window, not some other value.
        assert_eq!(
            retry_after,
            crate::auth::SIGNIN_WINDOW.as_secs().to_string(),
            "Retry-After should mirror SIGNIN_WINDOW"
        );
        // And the throttled answer must not be cacheable.
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "throttled response must be no-store"
        );
    }

    /// Ordinary failed sign-ins keep their exact pre-existing behaviour:
    /// 401, the same generic message, and no Retry-After or cache headers.
    #[tokio::test]
    async fn ordinary_failed_signins_are_unchanged_by_the_retry_after_fix() {
        let (router, _d) = router();
        json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        for bad in ["wrong", "also wrong", ""] {
            let (status, body, headers) = json_post(
                &router,
                "/api/auth/signin",
                &format!(r#"{{"identifier":"owner","password":"{bad}"}}"#),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
            assert_eq!(body, r#"{"error":"invalid credentials"}"#);
            assert!(
                headers.get(header::RETRY_AFTER).is_none(),
                "401 must not carry Retry-After"
            );
            assert!(
                headers.get(header::CACHE_CONTROL).is_none(),
                "401 keeps its previous header set"
            );
        }
        // Unknown identifier: same shape, still no Retry-After.
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"ghost@soket.ai","password":"whatever"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body, r#"{"error":"invalid credentials"}"#);
        assert!(headers.get(header::RETRY_AFTER).is_none());
    }

    // ==================================================================
    // Phase 4B: public registration endpoint
    //
    // Every test here builds its own throwaway database inside a
    // tempfile::TempDir. The real ~/.loop/dashboard/auth.db is never
    // opened read-write by any test in this module.
    // ==================================================================

    /// Register a user over HTTP, returning (status, body, headers).
    async fn register(
        router: &Router,
        email: &str,
        password: &str,
    ) -> (StatusCode, String, HeaderMap) {
        json_post(
            router,
            "/api/auth/register",
            &format!(r#"{{"email":"{email}","password":"{password}"}}"#),
        )
        .await
    }

    #[tokio::test]
    async fn valid_registration_creates_pending_user() {
        let (router, dir) = router();
        let (status, body, headers) =
            register(&router, "newperson@soket.ai", "a strong passphrase").await;

        // 202 Accepted: the request succeeded but nothing is usable yet.
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["status"], "pending");
        assert!(
            json["message"].as_str().unwrap().contains("administrator"),
            "response must explain approval is required: {body}"
        );

        // No session cookie is ever set for a pending registration.
        assert!(
            headers.get(header::SET_COOKIE).is_none(),
            "registration must not mint a session"
        );

        // The account exists with status pending and role user.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store
            .find_account_by_identifier("newperson@soket.ai")
            .unwrap();
        assert_eq!(rec.status, STATUS_PENDING);
        assert_eq!(rec.role, "user");
        // No administrator was created.
        assert_eq!(store.admin_count().unwrap(), 0);
        // And no session rows exist at all.
        assert_eq!(store.session_count().unwrap(), 0);
    }

    #[tokio::test]
    async fn registered_user_cannot_sign_in_until_approved() {
        let (router, dir) = router();
        register(&router, "waiting@soket.ai", "a strong passphrase").await;

        // Correct credentials, still refused: pending accounts cannot
        // authenticate. Phase 4C: the response now says WHY (awaiting
        // approval) because the password verified — nothing is leaked to a
        // caller with wrong credentials.
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"waiting@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains("approval"),
            "must explain the pending state: {body}"
        );
        assert!(headers.get(header::SET_COOKIE).is_none());

        // Status probe reports signed-out.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], false);

        // After an administrator approves (via the storage API — the admin
        // HTTP endpoints are a later phase), sign-in works.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store
            .find_account_by_identifier("waiting@soket.ai")
            .unwrap();
        store.set_account_status(rec.id, "approved").unwrap();

        let (status, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"waiting@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn invalid_domains_and_malformed_emails_are_rejected() {
        let (router, _d) = router();
        for bad in [
            "someone@gmail.com",
            "someone@example.soket.ai",
            "someone@soket.ai.evil.com",
            "not-an-email",
            "@soket.ai",
            "someone@",
            "some one@soket.ai",
            "double..dot@soket.ai",
            "",
        ] {
            let (status, body, _) = register(&router, bad, "a strong passphrase").await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "email {bad:?} must be rejected: {body}"
            );
            let json: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert!(json["error"].is_string(), "{body}");
        }
        // Nothing was created by any of those attempts.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(store.tracked_failure_identities(), 0); // sanity: helper works
    }

    #[tokio::test]
    async fn invalid_and_oversized_passwords_are_rejected() {
        let (router, _d) = router();
        // Too short.
        let (status, body, _) = register(&router, "short@soket.ai", "tiny").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        // Empty.
        let (status, body, _) = register(&router, "empty@soket.ai", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        // Oversized (beyond the 256-char store limit).
        let long_pw = "x".repeat(300);
        let (status, body, _) = register(&router, "long@soket.ai", &long_pw).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        // A pathological email string far beyond any legal address.
        let huge_email = format!("{}@soket.ai", "a".repeat(500));
        let (status, body, _) = register(&router, &huge_email, "a strong passphrase").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        // Nothing was written.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert!(store.find_account_by_identifier("short@soket.ai").is_err());
        assert!(store.find_account_by_identifier("long@soket.ai").is_err());
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_before_buffering() {
        let (router, _d) = router();
        // A body larger than REGISTER_BODY_LIMIT must be refused outright.
        let big = format!(
            r#"{{"email":"{}@soket.ai","password":"a strong passphrase"}}"#,
            "a".repeat(REGISTER_BODY_LIMIT)
        );
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/register")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(big))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn duplicate_normalized_email_is_rejected_over_http() {
        let (router, _d) = router();
        let (status, _, _) = register(&router, "dave@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Exact duplicate.
        let (status, body, _) = register(&router, "dave@soket.ai", "another passphrase").await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        // Casing/whitespace variants normalize to the same identity.
        for variant in ["DAVE@soket.ai", "  Dave@Soket.AI ", "dave@SOKET.ai"] {
            let (status, body, _) = register(&router, variant, "another passphrase").await;
            assert_eq!(status, StatusCode::CONFLICT, "variant {variant:?}: {body}");
        }
        // Still exactly one row.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        let rec = store.find_account_by_identifier(" DAVE@Soket.AI ").unwrap();
        assert_eq!(rec.identifier, "dave@soket.ai");
    }

    #[tokio::test]
    async fn concurrent_duplicate_registrations_cannot_both_succeed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
        let state = Arc::new(AuthState::new(store.clone()));
        let router = auth_router(state);

        // Fire 8 concurrent registrations of the same normalized address.
        let mut handles = Vec::new();
        for i in 0..8 {
            let router = router.clone();
            handles.push(tokio::spawn(async move {
                let email = if i % 2 == 0 {
                    "racer@soket.ai"
                } else {
                    "RACER@Soket.AI"
                };
                register(&router, email, "a strong passphrase").await.0
            }));
        }
        let statuses: Vec<StatusCode> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        let accepted = statuses
            .iter()
            .filter(|s| **s == StatusCode::ACCEPTED)
            .count();
        let conflicts = statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count();
        assert_eq!(accepted, 1, "exactly one racer may win: {statuses:?}");
        assert_eq!(conflicts, 7, "the rest must conflict: {statuses:?}");

        // Exactly one row exists.
        let rec = store.find_account_by_identifier("racer@soket.ai").unwrap();
        assert_eq!(rec.status, STATUS_PENDING);
    }

    #[tokio::test]
    async fn client_supplied_role_or_status_is_ignored() {
        let (router, dir) = router();
        // Attempt to smuggle authority fields; none are deserialized.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/register",
            r#"{"email":"attacker@soket.ai","password":"a strong passphrase","role":"admin","status":"approved","isAdmin":true,"admin":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");

        // The stored account is a plain pending user regardless.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store
            .find_account_by_identifier("attacker@soket.ai")
            .unwrap();
        assert_eq!(rec.role, "user");
        assert_eq!(rec.status, STATUS_PENDING);
        assert_eq!(store.admin_count().unwrap(), 0);

        // And it still cannot sign in (pending → 403 with the approval
        // message, and never a session cookie).
        let (status, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"attacker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(headers.get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn registration_works_without_email_configuration() {
        // No Resend credentials are consulted anywhere on this path; the
        // test process deliberately has none set. Registration must still
        // succeed — email delivery is a later phase and never blocking.
        let (router, _d) = router();
        let (status, body, _) = register(&router, "nomail@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        // No network call was made (nothing to assert beyond success, but
        // the test would hang or fail if the handler awaited a sender).
    }

    #[tokio::test]
    async fn existing_administrator_untouched_by_registration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        // Bootstrap the administrator exactly as a fresh install would.
        {
            let store = AuthStore::open(&path).unwrap();
            store
                .sign_up(
                    crate::auth::LEGACY_ADMIN_IDENTIFIER,
                    "correct horse battery",
                )
                .unwrap();
            let hash_before = store.stored_password_hash();
            let id_before = store
                .find_account_by_identifier(crate::auth::LEGACY_ADMIN_IDENTIFIER)
                .unwrap()
                .id;
            drop(store);
            // Simulate the admin being signed in.
            let store = AuthStore::open(&path).unwrap();
            let token = store
                .sign_in(
                    crate::auth::LEGACY_ADMIN_IDENTIFIER,
                    "correct horse battery",
                )
                .unwrap();
            drop(store);
            let _ = (hash_before, id_before, token);
        }

        let store = Arc::new(AuthStore::open(&path).unwrap());
        let snapshot_before = admin_snapshot(&store);

        let state = Arc::new(AuthState::new(store.clone()));
        let router = auth_router(state);

        // A new registration arrives.
        let (status, body, _) = register(&router, "newbie@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");

        // The administrator is byte-for-byte unchanged.
        let snapshot_after = admin_snapshot(&store);
        assert_eq!(snapshot_before, snapshot_after);
        assert_eq!(store.admin_count().unwrap(), 1);
        // Its session survived too.
        assert_eq!(store.session_count().unwrap(), 1);
        // And the admin can still sign in.
        assert!(store
            .sign_in(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery"
            )
            .is_ok());
    }

    /// (id, identifier, hash, role, status) of the sole administrator.
    fn admin_snapshot(store: &AuthStore) -> (i64, String, String, String, String) {
        let conn = store.conn_for_tests();
        conn.query_row(
            "SELECT id, identifier, password_hash, role, status FROM auth_account WHERE role = 'admin'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn legacy_signup_route_cannot_bypass_approval() {
        let (router, dir) = router();
        // First run: bootstrap works (fresh installation path preserved).
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"boot@soket.ai","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // After bootstrap, /signup refuses to create anything.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"second@soket.ai","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");

        // It also cannot be used to approve a pending user or mint an
        // administrator: the pending account created via /register is
        // untouched by a later /signup attempt.
        register(&router, "pending@soket.ai", "a strong passphrase").await;
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"pending@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store
            .find_account_by_identifier("pending@soket.ai")
            .unwrap();
        assert_eq!(rec.status, STATUS_PENDING, "still pending");
        assert_eq!(rec.role, "user", "still a plain user");
        assert_eq!(store.admin_count().unwrap(), 1, "exactly one admin");

        // The pending user still cannot sign in (403, awaiting approval).
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"pending@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn signup_smuggled_fields_cannot_create_admin() {
        let (router, dir) = router();
        // Even with authority fields, after bootstrap /signup creates
        // nothing at all.
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"boot@soket.ai","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, body, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"evil@soket.ai","password":"correct horse battery","role":"admin","status":"approved","isAdmin":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        assert_eq!(store.admin_count().unwrap(), 1);
        assert!(store.find_account_by_identifier("evil@soket.ai").is_err());
    }

    #[tokio::test]
    async fn public_responses_leak_no_sensitive_data() {
        let (router, _d) = router();
        // Successful registration response.
        let (_, body, headers) = register(&router, "quiet@soket.ai", "a strong passphrase").await;
        assert!(!body.contains("argon2"), "no hash material: {body}");
        assert!(!body.contains("$"), "no secrets: {body}");
        assert!(headers.get(header::SET_COOKIE).is_none());

        // Error responses stay generic.
        let (_, body, _) = register(&router, "quiet@soket.ai", "another passphrase").await;
        assert!(!body.contains("argon2"), "{body}");
        let (_, body, _) = register(&router, "bad", "a strong passphrase").await;
        assert!(!body.contains("argon2"), "{body}");

        // The status probe never leaks identifiers of other accounts.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let status_body = String::from_utf8_lossy(&bytes).into_owned();
        assert!(!status_body.contains("quiet@soket.ai"), "{status_body}");
        assert!(!status_body.contains("soket_session"), "{status_body}");
    }

    #[tokio::test]
    async fn malformed_registration_json_is_rejected() {
        let (router, _d) = router();
        for bad in [
            "not json at all",
            "{}",
            r#"{"email":123,"password":"a strong passphrase"}"#,
            r#"{"password":"a strong passphrase"}"#,
            r#"{"email":"x@soket.ai"}"#,
        ] {
            let (status, _, _) = json_post(&router, "/api/auth/register", bad).await;
            assert!(
                status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
                "body {bad:?} should be rejected, got {status}"
            );
        }
        // Nothing was created.
        let store = AuthStore::open(_d.path().join("auth.db")).unwrap();
        assert_eq!(store.admin_count().unwrap(), 0);
    }

    // ==================================================================
    // Phase 4C: sign-in status policy + session enforcement
    //
    // Every test uses its own throwaway database in a tempfile::TempDir;
    // the real ~/.loop/dashboard/auth.db is never opened read-write.
    // ==================================================================

    /// Boot an admin + one approved user, returning (router, dir, user_id).
    async fn status_fixture() -> (Router, tempfile::TempDir, i64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = AuthStore::open(&path).unwrap();
        store
            .sign_up(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        let id = store
            .create_pending_user("worker@soket.ai", "a strong passphrase")
            .unwrap();
        store.set_account_status(id, "approved").unwrap();
        drop(store);
        let state = Arc::new(AuthState::new(Arc::new(
            AuthStore::open(&path).unwrap(),
        )));
        (auth_router(state), dir, id)
    }

    #[tokio::test]
    async fn approved_user_can_sign_in_and_reach_protected_routes() {
        let (router, _d, _id) = status_fixture().await;
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set_cookie.starts_with(SESSION_COOKIE), "{set_cookie}");
        assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
        assert!(set_cookie.contains("SameSite=Lax"), "{set_cookie}");
        let token = set_cookie.split(';').next().unwrap().trim();

        // The cookie grants access to the status probe's signed-in view.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], true);
        assert_eq!(json["identifier"], "worker@soket.ai");
    }

    #[tokio::test]
    async fn pending_user_gets_approval_message_and_no_session() {
        let (router, _d, _id) = status_fixture().await;
        // Register a fresh (pending) user.
        register(&router, "hopeful@soket.ai", "a strong passphrase").await;

        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"hopeful@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.to_lowercase().contains("approval"),
            "must explain pending state: {body}"
        );
        assert!(headers.get(header::SET_COOKIE).is_none());

        // WRONG password for the same pending account must NOT reveal the
        // pending state — it stays a generic invalid-credentials 401.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"hopeful@soket.ai","password":"totally wrong pass"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body, r#"{"error":"invalid credentials"}"#);
    }

    #[tokio::test]
    async fn rejected_user_cannot_sign_in() {
        let (router, dir, id) = status_fixture().await;
        // Approve then reject is not a legal transition; go via pending.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        store.set_account_status(id, STATUS_SUSPENDED).unwrap();
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        store.set_account_status(id, STATUS_SUSPENDED).unwrap();
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        store.set_account_status(id, STATUS_SUSPENDED).unwrap();
        drop(store);

        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        // Suspended with CORRECT credentials: 403 with the truthful
        // suspended message (User Management phase). The password verified,
        // so the real owner learns why access is refused; an attacker with
        // wrong credentials still gets the generic 401 below.
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.to_lowercase().contains("suspended"),
            "must explain the suspension: {body}"
        );
        assert!(headers.get(header::SET_COOKIE).is_none());

        // Wrong password on the same suspended account: generic denial,
        // nothing about the account's state leaked.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"totally wrong pass"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body, r#"{"error":"invalid credentials"}"#);
    }

    #[tokio::test]
    async fn suspended_users_sessions_are_revoked_immediately() {
        let (router, dir, id) = status_fixture().await;

        // Sign in while approved.
        let (_, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        let token = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        // Build guarded routers so we can prove the cookie stops working on
        // protected routes, not just the status probe. Each guard flavor
        // gets its own router: stacking both on one router would let the
        // outer layer answer before the inner one is consulted.
        let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
        let auth_state = Arc::new(AuthState::new(store.clone()));
        let api_guarded = Router::new()
            .route("/api/protected", get(|| async { "secret" }))
            .layer(axum::middleware::from_fn_with_state(
                auth_state.clone(),
                require_auth_api,
            ));
        let page_guarded = Router::new()
            .route("/page", get(|| async { "<html>page</html>" }))
            .layer(axum::middleware::from_fn_with_state(
                auth_state.clone(),
                require_auth_page,
            ));

        // Access works before suspension.
        let res = api_guarded
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Suspend — no server restart, same live store handle.
        store.set_account_status(id, STATUS_SUSPENDED).unwrap();

        // API guard now denies the SAME cookie...
        let res = api_guarded
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // ...and the page guard redirects instead of serving HTML.
        let res = page_guarded
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/page")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert!(res
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("/signin?next="));

        // Reinstatement does NOT resurrect the old session: the user signs
        // in again for a fresh one.
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        let res = api_guarded
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/protected")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "revoked session must stay dead after reinstatement"
        );

        // But a fresh sign-in works.
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn existing_administrator_login_is_preserved() {
        let (router, dir, _id) = status_fixture().await;
        // The grandfathered Gmail administrator signs in normally.
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"himanshusudan771@gmail.com","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(headers.get(header::SET_COOKIE).is_some());

        // Its record is untouched by everything above.
        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store
            .find_account_by_identifier(crate::auth::LEGACY_ADMIN_IDENTIFIER)
            .unwrap();
        assert_eq!(rec.role, "admin");
        assert_eq!(rec.status, "approved");
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn status_change_takes_effect_without_restart() {
        // One long-lived store handle; statuses flip through it while the
        // HTTP layer keeps using the very same handle.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let store = Arc::new(AuthStore::open(&path).unwrap());
        store
            .sign_up(
                crate::auth::LEGACY_ADMIN_IDENTIFIER,
                "correct horse battery",
            )
            .unwrap();
        let id = store
            .create_pending_user("live@soket.ai", "a strong passphrase")
            .unwrap();
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        let state = Arc::new(AuthState::new(store.clone()));
        let router = auth_router(state);

        // Approved → sign in OK.
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"live@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Suspend through the same handle: the next HTTP call sees it.
        store.set_account_status(id, STATUS_SUSPENDED).unwrap();
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"live@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        // Correct credentials on a suspended account: 403 with the
        // suspended message (User Management phase).
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.to_lowercase().contains("suspended"), "{body}");

        // Reinstate: works again immediately.
        store.set_account_status(id, STATUS_APPROVED).unwrap();
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"live@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_credentials_and_rate_limiting_remain_intact() {
        let (router, _d, _id) = status_fixture().await;
        // Unknown identifier and wrong password are indistinguishable.
        let a = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"ghost@soket.ai","password":"whatever"}"#,
        )
        .await;
        let b = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"wrong password!"}"#,
        )
        .await;
        assert_eq!(a.0, StatusCode::UNAUTHORIZED);
        assert_eq!(b.0, StatusCode::UNAUTHORIZED);
        assert_eq!(a.1, b.1, "responses must be identical");

        // Rate limiting still engages after repeated failures.
        for _ in 0..8 {
            let _ = json_post(
                &router,
                "/api/auth/signin",
                r#"{"identifier":"worker@soket.ai","password":"bad"}"#,
            )
            .await;
        }
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    }

    #[tokio::test]
    async fn signout_clears_cookie_and_session() {
        let (router, _d, _id) = status_fixture().await;
        let (_, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        let token = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let (status, _, out_headers) =
            json_post_with_cookie(&router, "/api/auth/signout", "{}", &token).await;
        assert_eq!(status, StatusCode::OK);
        let cleared = out_headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cleared.contains("Max-Age=0"), "{cleared}");

        // The token no longer validates.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], false);
    }

    #[tokio::test]
    async fn client_supplied_status_cannot_unlock_an_account() {
        // A client cannot smuggle a status/role past sign-in or registration.
        let (router, dir, _id) = status_fixture().await;
        register(&router, "sneaky@soket.ai", "a strong passphrase").await;

        // Sign-in with authority fields attached: still pending.
        let (status, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"sneaky@soket.ai","password":"a strong passphrase","status":"approved","role":"admin","isAdmin":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(headers.get(header::SET_COOKIE).is_none());

        let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
        let rec = store.find_account_by_identifier("sneaky@soket.ai").unwrap();
        assert_eq!(rec.status, STATUS_PENDING);
        assert_eq!(rec.role, "user");
        assert_eq!(store.admin_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn deleted_account_sessions_stop_validating() {
        // Defence in depth: if an account row disappears entirely, its
        // sessions must not keep working (the JOIN drops them).
        let (router, dir, _id) = status_fixture().await;
        let (_, _, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"worker@soket.ai","password":"a strong passphrase"}"#,
        )
        .await;
        let token = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        // Simulate account deletion directly in SQL (no public API for it).
        {
            let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
            let conn = store.conn_for_tests();
            conn.execute(
                "DELETE FROM auth_account WHERE identifier = 'worker@soket.ai'",
                [],
            )
            .unwrap();
        }

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], false, "orphan session must not validate");
    }

    // ==================================================================
    // Phase 4E: registration rate limiting.
    //
    // The budget is global and process-local, so every test below builds
    // its OWN AuthStore/router pair (never the shared `router()` helper,
    // which would leak budget between tests) and never touches the real
    // ~/.loop/dashboard/auth.db.
    // ==================================================================

    /// A fresh router + store pair for rate-limit tests.
    fn limited_router() -> (Router, Arc<AuthStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
        let state = Arc::new(AuthState::new(store.clone()));
        (auth_router(state), store, dir)
    }

    #[tokio::test]
    async fn registration_is_rate_limited_after_capacity() {
        let (router, store, _d) = limited_router();

        // Fill the budget with distinct valid addresses.
        for i in 0..crate::auth::REGISTRATION_CAPACITY {
            let email = format!("user{i}@soket.ai");
            let (status, body, _) = register(&router, &email, "a strong passphrase").await;
            assert_eq!(
                status,
                StatusCode::ACCEPTED,
                "attempt {i} must be accepted: {body}"
            );
        }

        // The next request — even a perfectly valid one — is refused.
        let (status, body, headers) =
            register(&router, "onemore@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert_eq!(
            headers
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("3600"),
            "Retry-After must advertise the window"
        );

        // And it was NOT stored: the limiter shed it before any work.
        assert!(
            store
                .find_account_by_identifier("onemore@soket.ai")
                .is_err(),
            "a throttled registration must not create an account"
        );

        // The generic message leaks nothing about accounts or budget.
        assert!(!body.contains("onemore"), "{body}");
        assert!(!body.contains("already exists"), "{body}");
    }

    #[tokio::test]
    async fn registration_limit_applies_equally_to_duplicates_and_invalid_input() {
        let (router, _store, _d) = limited_router();
        // Exhaust the budget with one address first.
        let (status, _, _) = register(&router, "first@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Now flood with a mix of duplicates, invalid domains and short
        // passwords until the budget runs out...
        let mut limited_seen = false;
        for i in 0..(crate::auth::REGISTRATION_CAPACITY * 2) {
            let email = if i % 3 == 0 {
                "first@soket.ai" // duplicate
            } else if i % 3 == 1 {
                "nobody@example.com" // wrong domain
            } else {
                "valid@soket.ai"
            };
            let password = if i % 5 == 0 {
                "short"
            } else {
                "a strong passphrase"
            };
            let (status, _body, _) = register(&router, email, password).await;
            if status == StatusCode::TOO_MANY_REQUESTS {
                limited_seen = true;
                break;
            }
        }
        assert!(limited_seen, "the mixed flood must eventually be limited");

        // Once limited, EVERY further request gets 429 regardless of shape:
        // a brand-new valid registration, a duplicate, and garbage input
        // all receive the same answer, so the response cannot be used to
        // distinguish "rate limited" from "already registered".
        for (email, password, label) in [
            ("brandnew@soket.ai", "a strong passphrase", "valid new"),
            ("first@soket.ai", "a strong passphrase", "duplicate"),
            ("garbage", "", "invalid"),
        ] {
            let (status, body, _) = register(&router, email, password).await;
            assert_eq!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "{label} must be uniformly limited: {body}"
            );
        }
    }

    #[tokio::test]
    async fn concurrent_registrations_cannot_exceed_capacity() {
        let (router, store, _d) = limited_router();

        // Fire well over capacity concurrently; the check-and-push under
        // one lock hold means at most REGISTRATION_CAPACITY succeed.
        let total = crate::auth::REGISTRATION_CAPACITY * 3;
        let mut handles = Vec::new();
        for i in 0..total {
            let router = router.clone();
            handles.push(tokio::spawn(async move {
                let email = format!("race{i}@soket.ai");
                register(&router, &email, "a strong passphrase").await.0
            }));
        }
        let statuses: Vec<StatusCode> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        let accepted = statuses
            .iter()
            .filter(|s| **s == StatusCode::ACCEPTED)
            .count();
        let limited = statuses
            .iter()
            .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
            .count();
        assert_eq!(
            accepted,
            crate::auth::REGISTRATION_CAPACITY,
            "exactly the capacity may succeed: {statuses:?}"
        );
        assert_eq!(
            accepted + limited,
            total,
            "every request must be either accepted or limited: {statuses:?}"
        );

        // The database agrees: exactly `capacity` pending rows exist.
        let pending = store.list_pending_accounts(1000).unwrap();
        assert_eq!(pending.len(), crate::auth::REGISTRATION_CAPACITY);
    }

    #[tokio::test]
    async fn registration_limiter_state_stays_bounded() {
        let (router, store, _d) = limited_router();

        // Drive far more requests than the capacity through the endpoint.
        for i in 0..(crate::auth::REGISTRATION_CAPACITY * 10) {
            let _ = register(&router, &format!("flood{i}@soket.ai"), "x").await;
        }

        // The tracked-attempt vector can never exceed the capacity: pushes
        // only happen while below it, and stale entries are pruned each call.
        assert!(
            store.registration_capacity_remaining() <= crate::auth::REGISTRATION_CAPACITY,
            "limiter state must stay bounded by capacity, got {}",
            store.registration_capacity_remaining()
        );
        // Sanity: it is full, not merely small.
        assert_eq!(
            store.registration_capacity_remaining(),
            crate::auth::REGISTRATION_CAPACITY
        );
    }

    #[tokio::test]
    async fn registration_limit_does_not_affect_signin_or_other_routes() {
        let (router, store, _d) = limited_router();
        // Bootstrap an administrator (signup is a different route and is
        // unaffected by the registration budget).
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Exhaust the registration budget entirely.
        for i in 0..crate::auth::REGISTRATION_CAPACITY {
            let _ = register(&router, &format!("fill{i}@soket.ai"), "a strong passphrase").await;
        }
        let (status, _, _) = register(&router, "extra@soket.ai", "a strong passphrase").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

        // Sign-in still works: the budgets are independent, and the admin's
        // ability to administer is never held hostage by a registration flood.
        let (status, body, _) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // So does the public status probe.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(store.session_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn bootstrap_administrator_signin_survives_registration_flood() {
        // Regression guard for the grandfathered administrator: exhausting
        // the registration budget must not impede its sign-in.
        let (router, store, _d) = limited_router();
        let (status, _, _) = json_post(
            &router,
            "/api/auth/signup",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        for i in 0..crate::auth::REGISTRATION_CAPACITY {
            let _ = register(
                &router,
                &format!("noisy{i}@soket.ai"),
                "a strong passphrase",
            )
            .await;
        }

        // The administrator signs in successfully.
        let (status, body, headers) = json_post(
            &router,
            "/api/auth/signin",
            r#"{"identifier":"owner","password":"correct horse battery"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let cookie = headers
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        // Its account is untouched: still the sole approved administrator.
        let rec = store.find_account_by_identifier("owner").unwrap();
        assert_eq!(rec.role, crate::auth::ADMIN_ROLE);
        assert_eq!(rec.status, crate::auth::STATUS_APPROVED);
        assert_eq!(store.admin_count().unwrap(), 1);

        // And the session validates against a protected route.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["signedIn"], true, "admin session must validate");
    }
}
