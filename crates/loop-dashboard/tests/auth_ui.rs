//! Phase 5 tests: the authentication UI's contract with the backend.
//!
//! The page is plain HTML + vanilla JS embedded in the binary, so these
//! tests verify the *contract* rather than drive a browser:
//!
//! * the served document contains the sign-in, registration and pending
//!   approval views with accessible labels;
//! * it targets the correct endpoints (`/api/auth/register` for new users,
//!   never `/api/auth/signup`);
//! * the backend responses the UI branches on actually carry the shapes
//!   the JS expects (202 pending, 403 awaiting-approval, 429 rate limit);
//! * registration never mints a session, and first-run bootstrap still
//!   works exactly as before.
//!
//! Every test builds its own throwaway database inside a
//! `tempfile::TempDir`; the real `~/.loop/dashboard/auth.db` is never
//! opened read-write here.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use loop_dashboard::api::{build_app, AppOptions};
use loop_dashboard::auth::AuthStore;

/// The embedded auth page, fetched through the real router.
async fn auth_page_html(app: &axum::Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/signin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Full app with an auth store over a temp DB (no console/pipeline state).
async fn app_with_auth() -> (axum::Router, Arc<AuthStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let auth = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
    let (app, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        console_state: None,
        auth_store: Some(auth.clone()),
    })
    .unwrap();
    (app, auth, dir)
}

/// POST JSON to the app, optionally with a cookie.
async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let set_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, set_cookie)
}

// ---------------------------------------------------------------------------
// Document structure: the three views exist and are labelled accessibly.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_page_renders_signin_register_and_pending_views() {
    let (app, _a, _d) = app_with_auth().await;
    let html = auth_page_html(&app).await;

    // Sign-in view.
    assert!(html.contains("id=\"signinCard\""), "sign-in card missing");
    assert!(html.contains(r#"for="identifier""#), "email label missing");
    assert!(
        html.contains(r#"type="password"#),
        "password input must default to masked"
    );
    // Registration view.
    assert!(
        html.contains("id=\"registerCard\""),
        "registration card missing"
    );
    assert!(
        html.contains(r#"for="regEmail""#),
        "registration email label missing"
    );
    assert!(
        html.contains(r#"for="regConfirm""#),
        "confirm-password label missing"
    );
    // Pending-approval view.
    assert!(html.contains("id=\"pendingCard\""), "pending card missing");

    // Accessibility: every input has an associated <label for=...>.
    for id in [
        "identifier",
        "password",
        "regEmail",
        "regPassword",
        "regConfirm",
    ] {
        assert!(
            html.contains(&format!(r#"for="{id}""#)),
            "input #{id} lacks a label"
        );
    }
    // Show/hide controls announce their state.
    assert!(
        html.contains("aria-pressed"),
        "pw toggles must expose aria-pressed"
    );
    // Live region for screen readers.
    assert!(html.contains("aria-live"), "alerts need a live region");
}

#[tokio::test]
async fn auth_page_documents_soket_domain_requirement() {
    let (app, _a, _d) = app_with_auth().await;
    let html = auth_page_html(&app).await;
    // The registration form must explain the @soket.ai restriction up front.
    assert!(html.contains("@soket.ai"), "domain guidance missing");
    // Password requirements mirror the backend (8..=256).
    assert!(html.contains("At least 8 characters"));
}

// ---------------------------------------------------------------------------
// Endpoint wiring: register for new users, signup only for bootstrap.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_page_uses_register_endpoint_for_new_users() {
    let (app, _a, _d) = app_with_auth().await;
    let html = auth_page_html(&app).await;

    // The registration handler posts to /api/auth/register.
    assert!(
        html.contains("'/api/auth/register'"),
        "registration must POST /api/auth/register"
    );

    // Extract ONLY the registration submit handler (from its listener to
    // the next section comment) and prove it never references the legacy
    // bootstrap endpoint. The bootstrap view is allowed to use /signup —
    // that is its job on a fresh install — but the user-registration flow
    // must not be able to reach it.
    let marker = "$('#registerForm').addEventListener('submit'";
    let start = html.find(marker).expect("register form submit handler");
    let end = html[start..]
        .find("/* ---------- ")
        .map(|i| start + i)
        .unwrap_or(html.len());
    let registration_js = &html[start..end];
    assert!(
        !registration_js.contains("/api/auth/signup"),
        "registration must never fall back to the bootstrap endpoint:\n{registration_js}"
    );
    assert!(
        !registration_js.contains("signup"),
        "no signup reference may appear in the registration flow"
    );
    // And it must not auto-sign-in after a successful registration.
    assert!(
        !registration_js.contains("/api/auth/signin"),
        "registration success must not trigger a sign-in request:\n{registration_js}"
    );
}

#[tokio::test]
async fn register_route_is_public_and_serves_the_auth_page() {
    let (app, _a, _d) = app_with_auth().await;
    // /register must be reachable without a session (it is a public route).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "/register must be public");
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        body.contains("registerCard"),
        "/register must serve the auth page"
    );
}

// ---------------------------------------------------------------------------
// Backend semantics the UI branches on.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registration_returns_202_pending_and_no_session() {
    let (app, auth, _d) = app_with_auth().await;
    // Provision the administrator so this is not a first-run install.
    auth.sign_up("owner", "correct horse battery").unwrap();

    let (status, body, set_cookie) = post_json(
        &app,
        "/api/auth/register",
        r#"{"email":"newbie@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;

    // The exact contract the UI's success branch keys on.
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "pending");
    assert_eq!(body["ok"], true);
    // No session was minted: the UI must NOT auto-sign-in.
    assert!(
        set_cookie.is_none(),
        "registration must not set a session cookie"
    );
    assert_eq!(auth.session_count().unwrap(), 0, "no session rows either");
}

#[tokio::test]
async fn pending_account_signin_returns_403_with_approval_message() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let _ = post_json(
        &app,
        "/api/auth/register",
        r#"{"email":"waiting@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;

    // Correct credentials, but still pending: the UI shows the
    // awaiting-approval message for this shape.
    let (status, body, _) = post_json(
        &app,
        "/api/auth/signin",
        r#"{"identifier":"waiting@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("approval"),
        "the 403 message must mention approval so the UI can branch on it: {msg}"
    );
    assert_eq!(auth.session_count().unwrap(), 0);
}

#[tokio::test]
async fn invalid_credentials_and_rate_limit_shapes_are_stable() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();

    // Unknown account → 401 with an error field.
    let (status, body, _) = post_json(
        &app,
        "/api/auth/signin",
        r#"{"identifier":"ghost@example.com","password":"whatever123"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["error"].is_string(),
        "must carry an error field: {body}"
    );

    // Exhaust the sign-in budget → 429 (the UI's rate-limit branch).
    for _ in 0..8 {
        let _ = post_json(
            &app,
            "/api/auth/signin",
            r#"{"identifier":"owner","password":"wrong password"}"#,
            None,
        )
        .await;
    }
    let (status, body, _) = post_json(
        &app,
        "/api/auth/signin",
        r#"{"identifier":"owner","password":"correct horse battery"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn duplicate_registration_conflict_shape_is_stable() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let _ = post_json(
        &app,
        "/api/auth/register",
        r#"{"email":"dupe@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;

    let (status, body, _) = post_json(
        &app,
        "/api/auth/register",
        r#"{"email":"dupe@soket.ai","password":"another passphrase"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"].is_string(),
        "conflict must carry an error field: {body}"
    );
}

// ---------------------------------------------------------------------------
// First-run bootstrap compatibility (unchanged behaviour).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn first_run_bootstrap_still_works_over_http() {
    let (app, auth, _d) = app_with_auth().await;

    // Fresh install: no administrator yet.
    assert!(auth.is_first_run().unwrap());

    // Bootstrap via the legacy endpoint — the UI's bootstrap view uses it.
    let (status, _body, _) = post_json(
        &app,
        "/api/auth/signup",
        r#"{"identifier":"owner","password":"correct horse battery"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Then sign in, exactly as the embedded test does.
    let (status, _body, cookie) = post_json(
        &app,
        "/api/auth/signin",
        r#"{"identifier":"owner","password":"correct horse battery"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some(), "sign-in must set the session cookie");
    assert!(!auth.is_first_run().unwrap());
}

#[tokio::test]
async fn bootstrap_cannot_create_a_second_administrator() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    assert_eq!(auth.admin_count().unwrap(), 1);

    // A second attempt through the bootstrap endpoint is refused.
    let (status, body, _) = post_json(
        &app,
        "/api/auth/signup",
        r#"{"identifier":"intruder@example.com","password":"some password"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        auth.admin_count().unwrap(),
        1,
        "still exactly one administrator"
    );
}

// ---------------------------------------------------------------------------
// Guard behaviour must be unchanged by the new public route.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn protected_routes_still_redirect_to_signin_with_next() {
    let (app, _a, _d) = app_with_auth().await;
    for page in ["/", "/system", "/console"] {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(page).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "{page}");
        let loc = res.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.starts_with("/signin?next="), "{page} -> {loc}");
    }
    // APIs still answer 401 JSON.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_status_reports_first_run_and_signed_in() {
    let (app, auth, _d) = app_with_auth().await;

    // Before bootstrap: firstRun true.
    let res = app
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
    let st: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(st["firstRun"], true);
    assert_eq!(st["signedIn"], false);

    // After bootstrap + sign-in: firstRun false, signedIn true.
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/status")
                .header(
                    header::COOKIE,
                    format!("{}={}", loop_dashboard::auth::SESSION_COOKIE, token),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let st: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(st["firstRun"], false);
    assert_eq!(st["signedIn"], true);
}

// ---------------------------------------------------------------------------
// The UI must not leak secrets into the document or rely on client trust.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_page_contains_no_secrets_or_remote_scripts() {
    let (app, _a, _d) = app_with_auth().await;
    let html = auth_page_html(&app).await;

    // No inline event handlers (CSP-friendly, no string-built DOM).
    assert!(!html.contains("onclick="), "no inline handlers");
    // No remote scripts at all.
    assert!(!html.contains("<script src="), "no external scripts");
    // The grandfathered administrator address must never be pre-filled or
    // advertised as a default credential.
    assert!(
        !html.to_lowercase().contains("himanshusudan771"),
        "the administrator address must not appear in the page"
    );
    // Nothing that looks like a hardcoded token.
    assert!(!html.contains("localStorage"), "no localStorage usage");
    assert!(!html.contains("sessionStorage"), "no sessionStorage usage");
}

#[tokio::test]
async fn registration_does_not_autosignin_even_after_approval() {
    // End-to-end: register → still no session → admin approves → the user
    // must still sign in themselves (the UI never holds a session for them).
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();

    let (status, _, cookie) = post_json(
        &app,
        "/api/auth/register",
        r#"{"email":"late@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(cookie.is_none(), "no cookie at registration time");

    // An administrator approves the account directly through storage.
    let rec = auth.find_account_by_identifier("late@soket.ai").unwrap();
    auth.set_account_status(rec.id, loop_dashboard::auth::STATUS_APPROVED)
        .unwrap();

    // Still no session until the user signs in.
    assert_eq!(auth.session_count().unwrap(), 0);
    let (status, _, cookie) = post_json(
        &app,
        "/api/auth/signin",
        r#"{"identifier":"late@soket.ai","password":"a strong passphrase"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some(), "only now is a session minted");
}

// ---------------------------------------------------------------------------
// Phase 8: Dino Runner static contract
//
// The mini-game is pure client-side presentation. These assertions pin the
// invariants that keep it from ever becoming an auth surface:
//   * it starts hidden and is revealed only by real 401 responses,
//   * its failure counter is page-scoped (no storage, no cookies),
//   * it exposes no endpoint and stores no credentials.
// Behavioural coverage lives in the browser suite.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dino_game_is_present_but_hidden_and_storage_free() {
    let (app, _auth, _dir) = app_with_auth().await;
    let body = auth_page_html(&app).await;

    // The fullscreen screen exists and is hidden by default…
    assert!(body.contains("id=\"dinoScreen\""), "game screen missing");
    assert!(
        body.contains(
            "<section class=\"dino-screen\" id=\"dinoScreen\" aria-labelledby=\"dinoTitle\" hidden>"
        ),
        "the game must start hidden"
    );
    // …and it is a sibling of the auth column, so it can claim the viewport
    // rather than being clipped by the 420px card shell.
    assert!(
        body.contains("<div id=\"authViews\" class=\"auth-views\">"),
        "auth views must be wrapped so the game can replace them wholesale"
    );

    // …and the trigger threshold is exactly three.
    assert!(
        body.contains("const THRESHOLD = 3;"),
        "failure threshold must be 3"
    );

    // The counter must live in a page-scoped variable only.
    assert!(
        !body.contains("localStorage.setItem") && !body.contains("sessionStorage.setItem"),
        "the game must not persist state to browser storage"
    );
    // No credential material may be captured by the game.
    for secret in ["password", "token"] {
        assert!(
            !body.contains(&format!("__dino.{secret}")),
            "the game API must never touch {secret}"
        );
    }

    // Progressive difficulty: the constants that govern the ramp are pinned
    // so an accidental change to the fairness model is caught here.
    for pin in [
        "const BASE_SPEED   = 260;",
        "const MAX_SPEED    = 620;",
        "const SPEED_STEP    = 22;",
        "const SPEED_EVERY_MS = 4200;",
    ] {
        assert!(body.contains(pin), "difficulty constant missing: {pin}");
    }
    // The cap is genuinely a cap: it must exceed the base and be reachable
    // only through many steps, never in one jump.
    assert!(
        body.contains("speed = Math.min(speed, MAX_SPEED);"),
        "the speed cap must be enforced every frame"
    );
    // Restarting must begin from the initial speed.
    assert!(
        body.contains("function resetDifficulty() {"),
        "a dedicated difficulty reset must exist for restarts"
    );
}

#[tokio::test]
async fn dino_counter_only_counts_unauthorized_responses() {
    let (app, _auth, _dir) = app_with_auth().await;
    let body = auth_page_html(&app).await;

    // The increment must be inside the 401 branch of the sign-in handler…
    let unauthorized_branch = body
        .find("} else if (res.status === 401) {")
        .expect("401 branch present");
    let tail = &body[unauthorized_branch..];
    let next_branch = tail.find("} else {").expect("fallback branch present");
    let branch = &tail[..next_branch];
    assert!(
        branch.contains("noteCredentialFailure"),
        "the failure counter must be incremented ONLY on HTTP 401:\n{branch}"
    );

    // …and must NOT appear in the pending (403) or rate-limit (429)
    // branches. Scope each check to that branch's own extent rather than a
    // fixed character window, which could bleed into the next branch.
    for marker in ["res.status === 429", "res.status === 403"] {
        let Some(start) = body.find(marker) else {
            continue;
        };
        // The branch ends at the next "} else" at the same nesting level.
        let rest = &body[start..];
        let end = rest.find("} else").unwrap_or(rest.len());
        let branch = &rest[..end];
        assert!(
            !branch.contains("noteCredentialFailure"),
            "{marker} branch must not increment the failure counter:\n{branch}"
        );
    }

    // Success resets the counter before navigating away.
    let ok_idx = body.find("if (res.ok) {").expect("success branch");
    let ok_tail = &body[ok_idx..ok_idx + 700];
    assert!(
        ok_tail.contains("resetFailureState"),
        "successful sign-in must reset the failure display state:\n{ok_tail}"
    );
}
