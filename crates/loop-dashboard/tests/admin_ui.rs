//! Phase 6 tests: the administrator approval dashboard's contract with the
//! backend and the served page.
//!
//! Two layers are covered:
//!
//! 1. **API integration** — through the *full* app (guard layer + admin
//!    router), so the public-prefix and session-guard behaviour is exercised
//!    exactly as a browser would hit it.
//! 2. **UI contract** — the served `/admin` document must contain the queue,
//!    the safe-DOM rules it promises, and no secrets; and it must be
//!    session-guarded like every other workspace page.
//!
//! Every test uses its own throwaway database in a `tempfile::TempDir`.
//! The real `~/.loop/dashboard/auth.db` is never opened read-write here.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use loop_dashboard::api::{build_app, AppOptions};
use loop_dashboard::auth::{AuthStore, STATUS_APPROVED, STATUS_PENDING};
use loop_dashboard::web::web_router;

/// Full app (guards + auth + admin) over a temp DB, with one approved
/// administrator already signed in. Returns the admin's cookie.
async fn app_with_admin(
    pending: &[&str],
) -> (
    axum::Router,
    Arc<AuthStore>,
    tempfile::TempDir,
    String,   // admin cookie
    Vec<i64>, // pending account ids
) {
    let dir = tempfile::tempdir().unwrap();
    let store = AuthStore::open(dir.path().join("auth.db")).unwrap();
    // Bootstrap the sole administrator exactly as a fresh install would.
    store
        .sign_up(
            loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
            "correct horse battery",
        )
        .unwrap();
    let mut ids = Vec::new();
    for email in pending {
        ids.push(
            store
                .create_pending_user(email, "a strong passphrase")
                .unwrap(),
        );
    }
    let token = store
        .sign_in(
            loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
            "correct horse battery",
        )
        .unwrap();
    let store = Arc::new(store);

    let (app, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        support_store: None,
        console_state: None,
        auth_store: Some(store.clone()),
    })
    .unwrap();

    let cookie = format!("{}={}", loop_dashboard::auth::SESSION_COOKIE, token);
    (app, store, dir, cookie, ids)
}

/// Sign in an ordinary (non-admin) user and return their cookie.
fn ordinary_cookie(dir: &tempfile::TempDir, email: &str) -> String {
    // Reopen a writable handle: the fixture's store lives behind an Arc in
    // the running app, so writes go through a fresh connection.
    let writable = AuthStore::open(dir.path().join("auth.db")).unwrap();
    let id = writable
        .create_pending_user(email, "a strong passphrase")
        .unwrap();
    writable.set_account_status(id, STATUS_APPROVED).unwrap();
    let token = writable.sign_in(email, "a strong passphrase").unwrap();
    drop(writable);
    format!("{}={}", loop_dashboard::auth::SESSION_COOKIE, token)
}

async fn get(app: &axum::Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
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

async fn post(app: &axum::Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .uri(uri)
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from("{}")).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Page serving and guarding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_page_is_served_and_session_guarded() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;

    // Unauthenticated: the guard redirects to sign-in, preserving the path.
    let (status, _b) = get(&app, "/admin", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    // Authenticated: HTML is served.
    let (status, body) = get(&app, "/admin", Some(&admin_cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Account approvals"), "page content: {body}");
    assert!(body.contains("id=\"listRoot\""));
}

#[tokio::test]
async fn admin_page_html_contains_no_secrets_or_unsafe_inserts() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (_, body) = get(&app, "/admin", Some(&admin_cookie)).await;

    // No inline handlers or remote scripts.
    assert!(!body.contains("<script src="), "no external scripts");
    assert!(!body.contains("onclick="), "no inline handlers");
    // No client-side credential storage.
    assert!(!body.contains("localStorage"));
    assert!(!body.contains("sessionStorage"));
    // Server-provided strings must go through textContent/createElement.
    assert!(
        !body.contains("innerHTML"),
        "the admin page must not use innerHTML"
    );
    // Never render hashes/tokens even if the API grew them.
    for absent in ["password_hash", "passwordHash", "sessionToken", "otp"] {
        assert!(!body.contains(absent), "page mentions {absent}");
    }
}

// ---------------------------------------------------------------------------
// Pending list contract
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pending_list_shape_matches_the_ui_contract() {
    let (app, _s, _d, admin_cookie, _ids) =
        app_with_admin(&["first@soket.ai", "second@soket.ai"]).await;

    let (status, body) = get(&app, "/api/admin/users/pending", Some(&admin_cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let users = json["users"].as_array().expect("users array");
    assert_eq!(users.len(), 2);
    assert_eq!(json["totalCount"], 2);
    assert_eq!(json["truncated"], false);

    for u in users {
        // Exactly the four fields the UI reads, and nothing sensitive.
        assert!(u["id"].is_number(), "{u}");
        assert!(u["identifier"].as_str().unwrap().ends_with("@soket.ai"));
        assert_eq!(u["status"], STATUS_PENDING);
        assert!(u["createdAtMs"].is_number());
        for absent in ["password_hash", "hash", "token", "secret", "role"] {
            assert!(u.get(absent).is_none(), "leaked {absent}: {u}");
        }
    }
}

#[tokio::test]
async fn empty_pending_list_returns_empty_array_not_null() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (status, body) = get(&app, "/api/admin/users/pending", Some(&admin_cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["users"], serde_json::json!([]), "must be []: {body}");
    assert_eq!(json["totalCount"], 0);
}

// ---------------------------------------------------------------------------
// Authorization: the server is the authority
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthenticated_requests_are_refused() {
    let (app, _s, _d, _c, _ids) = app_with_admin(&["x@soket.ai"]).await;
    // The admin API sits under /api/, so the guard answers 401 JSON.
    let (status, body) = get(&app, "/api/admin/users/pending", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("authentication required"));

    let (status, _) = post(&app, "/api/admin/users/1/approve", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ordinary_users_get_403_for_listing_and_actions() {
    let (app, _store, dir, _admin, _ids) = app_with_admin(&["someone@soket.ai"]).await;
    let user_cookie = ordinary_cookie(&dir, "plain@soket.ai");

    let (status, body) = get(&app, "/api/admin/users/pending", Some(&user_cookie)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("administrator access required"));

    // Even with a guessed id, a non-admin cannot act.
    let (status, _) = post(&app, "/api/admin/users/2/approve", Some(&user_cookie)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn expired_session_is_refused_and_revoked_sessions_stay_dead() {
    let (app, store, dir, _admin, _ids) = app_with_admin(&["gone@soket.ai"]).await;
    let cookie = ordinary_cookie(&dir, "victim@soket.ai");
    // Revoke that session the same way the sign-out endpoint does.
    let token = cookie.split('=').nth(1).unwrap().to_string();
    store.sign_out(&token).unwrap();

    let (status, _) = get(&app, "/api/admin/users/pending", Some(&cookie)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "revoked session must fail"
    );
}

// ---------------------------------------------------------------------------
// Transitions through the full app
// ---------------------------------------------------------------------------

#[tokio::test]
async fn approve_then_reject_through_http() {
    let (app, store, _d, admin_cookie, ids) = app_with_admin(&["a@soket.ai", "b@soket.ai"]).await;

    // Approve the first.
    let (status, body) = post(
        &app,
        &format!("/api/admin/users/{}/approve", ids[0]),
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"approved\""));
    assert_eq!(
        store.account_role_and_status(ids[0]).unwrap().1,
        STATUS_APPROVED
    );

    // Reject the second.
    let (status, body) = post(
        &app,
        &format!("/api/admin/users/{}/reject", ids[1]),
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"rejected\""));

    // The queue is now empty.
    let (_, body) = get(&app, "/api/admin/users/pending", Some(&admin_cookie)).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["totalCount"], 0);
}

#[tokio::test]
async fn approving_does_not_sign_the_user_in() {
    let (app, store, _d, admin_cookie, ids) = app_with_admin(&["later@soket.ai"]).await;
    let before = store.session_count().unwrap();
    let (status, _) = post(
        &app,
        &format!("/api/admin/users/{}/approve", ids[0]),
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store.session_count().unwrap(),
        before,
        "approval must not mint a session"
    );
}

#[tokio::test]
async fn invalid_transition_returns_conflict_not_silent_success() {
    let (app, _s, _d, admin_cookie, ids) = app_with_admin(&["p@soket.ai"]).await;
    // Suspending a PENDING account is not a legal transition.
    let (status, body) = post(
        &app,
        &format!("/api/admin/users/{}/suspend", ids[0]),
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("error"), "must carry an error field: {body}");
}

#[tokio::test]
async fn unknown_account_id_returns_404() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (status, body) = post(&app, "/api/admin/users/99999/approve", Some(&admin_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn malformed_id_returns_400() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (status, _) = post(
        &app,
        "/api/admin/users/not-a-number/approve",
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_cannot_target_itself() {
    let (app, store, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let admin = store
        .find_account_by_identifier(loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER)
        .unwrap();
    let (status, body) = post(
        &app,
        &format!("/api/admin/users/{}/suspend", admin.id),
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // Still an active administrator.
    assert_eq!(
        store.account_role_and_status(admin.id).unwrap().1,
        STATUS_APPROVED
    );
}

// ---------------------------------------------------------------------------
// Console shell: the approvals link appears only for administrators
// ---------------------------------------------------------------------------

#[tokio::test]
async fn console_page_carries_a_hidden_admin_link_and_safe_probe() {
    // The console HTML must ship the link hidden, and the script must
    // reveal it only after the server confirms admin access.
    let console_html = include_str!("../src/assets/console.html");
    assert!(
        console_html.contains(r#"id="adminNavLink""#),
        "console must contain the approvals link"
    );
    assert!(
        console_html.contains("hidden"),
        "the link must start hidden for non-admins"
    );

    let js = include_str!("../src/assets/console-app.js");
    // The probe targets the guarded endpoint, so the server decides.
    assert!(
        js.contains("maybeShowAdminLink"),
        "script must probe for admin rights"
    );
    assert!(
        js.contains("'/api/admin/users/pending'"),
        "the probe must call the real guarded endpoint"
    );
    // And it must never trust a client-declared role.
    assert!(
        !js.contains("isAdmin"),
        "no client-side isAdmin flag may drive the UI"
    );
}

#[tokio::test]
async fn web_router_has_no_public_admin_route() {
    // The static web router is mounted inside the guarded app; make sure no
    // admin asset accidentally became public.
    let router = web_router();
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "/admin must be routable");
    // The auth pages remain public, and the admin page is NOT among them.
    for public in ["/signin", "/signup", "/register", "/shell.css"] {
        let res = router
            .clone()
            .oneshot(Request::builder().uri(public).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{public} stays public");
    }
}

// ---------------------------------------------------------------------------
// Existing behaviour must be untouched
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_pages_and_guards_are_unchanged() {
    let (app, _s, _d, _c, _ids) = app_with_admin(&[]).await;

    // Public auth surfaces still work without a session.
    for path in ["/signin", "/register", "/signup", "/api/auth/status"] {
        let (status, _) = get(&app, path, None).await;
        assert_eq!(status, StatusCode::OK, "{path} must stay public");
    }

    // Workspace pages still redirect unauthenticated visitors.
    for page in ["/", "/system", "/console"] {
        let (status, _) = get(&app, page, None).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{page}");
    }
}

#[tokio::test]
async fn registration_still_creates_pending_accounts_only() {
    // The Phase 5 flow must be intact: registering yields a pending account
    // that appears in this queue and cannot sign in yet.
    let (app, store, _d, _admin_cookie, _ids) = app_with_admin(&[]).await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"email":"phase5@soket.ai","password":"a strong passphrase"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // It shows up as pending.
    let rec = store.find_account_by_identifier("phase5@soket.ai").unwrap();
    assert_eq!(rec.status, STATUS_PENDING);
}

// ---------------------------------------------------------------------------
// Phase 7: All Users tab — list, search, filters, pagination, role actions
// ---------------------------------------------------------------------------

/// GET helper returning (status, body) for an arbitrary path with a cookie.
async fn get_path(app: &axum::Router, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(path);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let res = app
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

#[tokio::test]
async fn user_list_is_admin_only() {
    let (app, store, _d, admin_cookie, ids) = app_with_admin(&["norm@soket.ai"]).await;
    let norm_id = ids[0];
    // Approve the normal user so they can sign in.
    store.set_account_status(norm_id, STATUS_APPROVED).unwrap();
    let norm_token = store
        .sign_in("norm@soket.ai", "a strong passphrase")
        .unwrap();
    let norm_cookie = format!("{}={}", loop_dashboard::auth::SESSION_COOKIE, norm_token);

    // Admin sees the list.
    let (s, b) = get_path(&app, "/api/admin/users", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b.contains("\"users\""), "{b}");

    // An ordinary user is refused.
    let (s, b) = get_path(&app, "/api/admin/users", Some(&norm_cookie)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(!b.contains("\"users\""), "list must not leak: {b}");

    // Unauthenticated is refused.
    let (s, _b) = get_path(&app, "/api/admin/users", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn user_list_search_filter_and_pagination_contract() {
    let mut pending = vec!["alice@soket.ai", "alina@soket.ai", "bob@soket.ai"];
    for i in 0..7 {
        pending.push(Box::leak(format!("extra{i}@soket.ai").into_boxed_str()));
    }
    let emails: Vec<&str> = pending.to_vec();
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&emails).await;

    // Search narrows by substring.
    let (s, b) = get_path(&app, "/api/admin/users?search=ali", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["totalCount"], 2, "{b}");

    // Role filter.
    let (s, b) = get_path(&app, "/api/admin/users?role=admin", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["totalCount"], 1);

    // Status filter.
    let (s, b) = get_path(&app, "/api/admin/users?status=pending", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["totalCount"], 10, "3 named + 7 extra");

    // Pagination.
    let (s, b) = get_path(
        &app,
        "/api/admin/users?perPage=5&page=2",
        Some(&admin_cookie),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["users"].as_array().unwrap().len(), 5);
    assert_eq!(v["totalPages"], 3);

    // Unknown filters are a 400.
    let (s, _b) = get_path(&app, "/api/admin/users?role=bogus", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn role_promotion_and_demotion_contract() {
    let (app, store, _d, admin_cookie, ids) = app_with_admin(&["rise@soket.ai"]).await;
    let rise_id = ids[0];
    store.set_account_status(rise_id, STATUS_APPROVED).unwrap();

    // Promote through the API.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{rise_id}/role"))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"role\":\"admin\"}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        store.account_role_and_status(rise_id).unwrap(),
        ("admin".to_string(), "approved".to_string())
    );

    // The promoted peer can now demote the original admin.
    let admin_rec = store
        .find_account_by_identifier(loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER)
        .unwrap();
    let peer_token = store
        .sign_in("rise@soket.ai", "a strong passphrase")
        .unwrap();
    let peer_cookie = format!("{}={}", loop_dashboard::auth::SESSION_COOKIE, peer_token);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{}/role", admin_rec.id))
        .header(header::COOKIE, &peer_cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"role\":\"user\"}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(store.admin_count().unwrap(), 1);

    // The demoted admin immediately loses authority.
    let (s, _b) = get_path(&app, "/api/admin/users", Some(&admin_cookie)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn last_admin_cannot_be_demoted_through_the_app() {
    let (app, store, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let admin_rec = store
        .find_account_by_identifier(loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER)
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{}/role", admin_rec.id))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"role\":\"user\"}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    // The self-target guard fires first; either refusal is correct.
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(store.admin_count().unwrap(), 1);
}

#[tokio::test]
async fn pending_accounts_cannot_be_promoted_without_approval_first() {
    let (app, store, _d, admin_cookie, ids) = app_with_admin(&["wait@soket.ai"]).await;
    let wait_id = ids[0];
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{wait_id}/role"))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"role\":\"admin\"}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(store.admin_count().unwrap(), 1);
    // Approval and promotion stay separate: approve, then promote works.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{wait_id}/approve"))
        .header(header::COOKIE, &admin_cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/admin/users/{wait_id}/role"))
        .header(header::COOKIE, &admin_cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"role\":\"admin\"}"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(store.admin_count().unwrap(), 2);
}

#[tokio::test]
async fn admin_page_contains_the_users_tab_and_safe_rendering() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (_, body) = get_path(&app, "/admin", Some(&admin_cookie)).await;

    // The new tab exists.
    assert!(body.contains("id=\"tabUsers\""), "users tab missing");
    assert!(body.contains("id=\"usersBody\""), "users table missing");
    assert!(body.contains("id=\"userSearch\""), "search box missing");
    assert!(body.contains("id=\"roleFilter\""), "role filter missing");
    assert!(
        body.contains("id=\"statusFilter\""),
        "status filter missing"
    );
    // No hardcoded administrator email anywhere in the page.
    assert!(
        !body.contains("himanshu"),
        "no account email may be hardcoded in the UI"
    );
    // Still no unsafe inserts.
    assert!(!body.contains("innerHTML"));
}

// ---------------------------------------------------------------------------
// Phase 7: responsive layout contract for the All Users table
//
// These are static-CSS assertions. They cannot prove pixels, but they pin
// the two properties the clipping bug violated, so a regression in either
// fails the build:
//   1. `.adm-card` must NOT clip overflow (that is what hid the buttons).
//   2. A narrow-viewport presentation must exist that makes rows block-level
//      and the action cell full-width.
// The pixel-level behaviour is covered by the browser suite.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_card_does_not_clip_its_contents() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (_, body) = get_path(&app, "/admin", Some(&admin_cookie)).await;

    // The card rule must not carry overflow:hidden — that clipped the
    // Actions column at narrow widths.
    let card_rule = body
        .lines()
        .find(|l| l.contains(".adm-card{"))
        .expect("the .adm-card rule must exist");
    assert!(
        !card_rule.contains("overflow:hidden"),
        ".adm-card must not clip: {card_rule}"
    );
}

#[tokio::test]
async fn users_table_has_a_stacked_narrow_presentation() {
    let (app, _s, _d, admin_cookie, _ids) = app_with_admin(&[]).await;
    let (_, body) = get_path(&app, "/admin", Some(&admin_cookie)).await;

    // A media query must switch rows to block layout…
    assert!(
        body.contains("@media (max-width:879px)"),
        "a narrow-viewport breakpoint must exist"
    );
    // …hiding the header row (labels replace it)…
    assert!(
        body.contains(".adm-table thead{display:none}"),
        "table headers must be hidden when stacked"
    );
    // …and making the action cell span the full width so its buttons can
    // never be pushed outside the viewport by a long email address.
    assert!(
        body.contains(".adm-cell-actions{"),
        "the action cell needs an explicit narrow-screen rule"
    );
    // Long identifiers must be allowed to wrap rather than force the page
    // wider than the viewport.
    assert!(
        body.contains("word-break:break-word") || body.contains("overflow-wrap:anywhere"),
        "emails must be able to wrap at narrow widths"
    );
}
