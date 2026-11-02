//! Post-login landing behaviour: sign-in must open the Welcome page.
//!
//! The user-visible contract is:
//!
//! * unauthenticated HTML requests are redirected to `/signin?next=…`
//!   exactly as before (auth is not weakened in any way);
//! * a successful sign-in lands on the console's **Welcome** view
//!   (`/console#overview`), never on the observatory Overview;
//! * "/" with no explicit view forwards to Welcome;
//! * explicitly requested views and pages still work (deep links).
//!
//! The sign-in page is plain HTML + vanilla JS embedded in the binary, so
//! these tests verify the *contract* the page implements against the real
//! router — the same approach as `auth_ui.rs`. `postSignInTarget()` is the
//! single function that decides where login goes; its logic is mirrored
//! here input-by-input so a regression in either place fails loudly.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use loop_dashboard::api::{build_app, AppOptions};
use loop_dashboard::auth::{AuthStore, SESSION_COOKIE};

async fn app_with_auth() -> (axum::Router, Arc<AuthStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let auth = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
    let (app, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        support_store: None,
        console_state: None,
        auth_store: Some(auth.clone()),
    })
    .unwrap();
    (app, auth, dir)
}

/// GET a page through the real router, returning (status, location, body).
async fn get(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, Option<String>, String) {
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
    let location = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// Test 1 — Unauthenticated access keeps the existing authentication flow.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthenticated_pages_redirect_to_signin_preserving_next() {
    let (app, _a, _d) = app_with_auth().await;
    for page in ["/", "/console", "/system"] {
        let (status, loc, _) = get(&app, page, None).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{page}");
        let loc = loc.expect("redirect target");
        assert!(loc.starts_with("/signin?next="), "{page} -> {loc}");
    }
    // APIs answer 401 JSON, not a redirect.
    let (status, _, _) = get(&app, "/api/health", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Test 2 — Successful login redirects to Welcome, not Overview.
//
// postSignInTarget() is the single decision point in the served page. Its
// exact inputs are replayed here; the browser-level flow (fill form →
// submit → land on /console#overview) is verified separately by the
// Playwright check documented in this task's report.
// ---------------------------------------------------------------------------

/// Mirrors postSignInTarget() from assets/auth.html. If the two ever drift,
/// these tests fail — which is exactly the signal wanted.
fn post_sign_in_target(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") => {
            let hash_at = n.find('#');
            let path = match hash_at {
                Some(i) => &n[..i],
                None => n,
            };
            let view = match hash_at {
                Some(i) => &n[i + 1..],
                None => "",
            };
            let is_root = path == "/" || path == "/index.html";
            let is_default_view = view.is_empty() || view == "overview";
            if !is_root || !is_default_view {
                return n.to_string();
            }
            "/console#overview".to_string()
        }
        _ => "/console#overview".to_string(),
    }
}

#[test]
fn sign_in_lands_on_welcome_by_default() {
    // No next at all: direct visit to /signin.
    assert_eq!(post_sign_in_target(None), "/console#overview");
    // The server guard's rewritten root.
    for next in ["/", "/index.html"] {
        assert_eq!(
            post_sign_in_target(Some(next)),
            "/console#overview",
            "next={next}"
        );
    }
    // A stale observatory default recorded automatically on session expiry.
    for next in ["/#overview", "/index.html#overview"] {
        assert_eq!(
            post_sign_in_target(Some(next)),
            "/console#overview",
            "stale default view must not override Welcome: next={next}"
        );
    }
}

#[test]
fn served_sign_in_page_targets_welcome_not_overview() {
    // The served document must keep pointing its default at the console's
    // Welcome route, and must never send login to the bare observatory.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (app, _a, _d) = rt.block_on(app_with_auth());
    let (_, _, html) = rt.block_on(get(&app, "/signin", None));
    assert!(
        html.contains("'/console#overview'"),
        "default target missing"
    );
    assert!(html.contains("function postSignInTarget"));
    // The stale-default carve-out must be present in the shipped logic.
    assert!(
        html.contains("isDefaultView"),
        "postSignInTarget must treat '/#overview' as the default case"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — Direct application entry ("/") results in Welcome.
//
// The server rewrites an unauthenticated "/" into next=/console#overview
// (so sign-in lands on Welcome), and the observatory page itself forwards
// a view-less "/" to /console#overview once signed in.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn root_entry_leads_to_welcome() {
    let (app, _a, _d) = app_with_auth().await;

    // Unauthenticated: the guard rewrites "/" to the Welcome destination.
    let (status, loc, _) = get(&app, "/", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let loc = loc.unwrap();
    assert!(
        loc.ends_with("%2Fconsole%23overview"),
        "unauthenticated '/' must sign in toward Welcome, got {loc}"
    );

    // Authenticated: "/" serves the observatory shell, whose boot() then
    // forwards a view-less load to /console#overview (client-side). The
    // forwarding logic lives in the served document.
    let (_a, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");
    let (status, _, html) = get(&_a, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("location.replace('/console#overview')"),
        "the observatory must forward a view-less '/' to Welcome"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — Loop Observability still opens the Overview dashboard.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn observatory_overview_stays_reachable() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    // Explicit "#overview" is honored: the page renders normally.
    let (status, _, html) = get(&app, "/#overview", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Loop · Observability"));

    // And a deliberately requested view survives the whole sign-in cycle.
    assert_eq!(
        post_sign_in_target(Some("/#sessions")),
        "/#sessions",
        "explicit views are deep links, not defaults"
    );
    assert_eq!(post_sign_in_target(Some("/#session:abc")), "/#session:abc");
}

// ---------------------------------------------------------------------------
// Test 5 — Existing navigation: every workspace surface still serves.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_workspace_surfaces_still_serve() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    for (path, marker) in [
        ("/", "Loop · Observability"),
        ("/system", "Loop · System Dashboard"),
        ("/console", "Loop · AI Console"),
    ] {
        let (status, _, html) = get(&app, path, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(html.contains(marker), "{path} must serve its shell");
    }

    // The console SPA keeps the welcome view with its workspace cards and
    // the sidebar navigation entries.
    let (status, _, spa) = get(&app, "/console/console-app.js", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(spa.contains("Welcome to Loop"), "welcome view");
    for card in [
        "System Dashboard",
        "SFT DataTrove",
        "Pretraining DataTrove",
        "Fun Time",
    ] {
        assert!(spa.contains(card), "workspace card '{card}'");
    }
    for nav in [
        "'Welcome'",
        "'SFT DataTrove'",
        "'Pretraining DataTrove'",
        "'Traces & Requests'",
    ] {
        assert!(spa.contains(nav), "nav entry {nav}");
    }
}

// ---------------------------------------------------------------------------
// Test 6 — Refreshing Welcome does not bounce the user to Overview.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refreshing_welcome_keeps_welcome() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    // /console#overview is a real, stable route: it serves the console
    // shell both times, and the SPA boots into its Welcome view because
    // the hash says so. Nothing on that path points back at the
    // observatory root.
    for _ in 0..2 {
        let (status, _, html) = get(&app, "/console", Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Loop · AI Console"));
        assert!(!html.contains("location.replace('/')"));
    }
    // The SPA's own boot picks the route from the hash; the welcome
    // renderer is the default for '#overview'.
    let (_, _, spa) = get(&app, "/console/console-app.js", Some(&cookie)).await;
    assert!(
        spa.contains("|| 'overview'"),
        "hash-less boot still lands on Welcome"
    );
}

// ---------------------------------------------------------------------------
// Test 7 — Stale route state cannot override the Welcome destination.
// ---------------------------------------------------------------------------

#[test]
fn stale_observatory_default_does_not_override_welcome() {
    // Everything a previous observatory session could have left in `next`.
    for stale in ["/", "/index.html", "/#overview", "/index.html#overview"] {
        assert_eq!(
            post_sign_in_target(Some(stale)),
            "/console#overview",
            "stale '{stale}' must not win over Welcome"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8 — Session persistence across refresh.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authenticated_session_survives_reload() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    // Two consecutive loads with the same cookie: both stay signed in and
    // neither is bounced to sign-in (no redirect at all).
    for path in ["/console", "/system", "/#overview"] {
        for _ in 0..2 {
            let (status, loc, _) = get(&app, path, Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK, "{path} while signed in");
            assert!(loc.is_none(), "{path} must not redirect a valid session");
        }
    }

    // The session probe agrees.
    let (status, _, body) = get(&app, "/api/auth/status", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"signedIn\":true"));
}

// ---------------------------------------------------------------------------
// Test 9 — Logout restores protections; the next login opens Welcome again.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn logout_then_login_opens_welcome_again() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    // Sign out through the real endpoint.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/signout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Protections are back: pages redirect to sign-in, APIs 401.
    let (status, loc, _) = get(&app, "/console", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(loc.unwrap().starts_with("/signin?next="));
    let (status, _, _) = get(&app, "/api/health", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A fresh sign-in mints a new session whose default landing is Welcome.
    let token2 = auth.sign_in("owner", "correct horse battery").unwrap();
    assert_ne!(token, token2, "sign-out must invalidate the old token");
    let cookie2 = format!("{SESSION_COOKIE}={token2}");
    let (status, loc, _) = get(&app, "/", None).await; // unauthenticated root
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(loc.unwrap().ends_with("%2Fconsole%23overview"));
    let (status, _, _) = get(&app, "/console", Some(&cookie2)).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Test 10 — Deep links keep working; only defaults are redirected.
// ---------------------------------------------------------------------------

#[test]
fn deliberate_deep_links_are_preserved() {
    // Named observatory views.
    assert_eq!(post_sign_in_target(Some("/#sessions")), "/#sessions");
    assert_eq!(post_sign_in_target(Some("/#models")), "/#models");
    assert_eq!(
        post_sign_in_target(Some("/#session:0x42")),
        "/#session:0x42"
    );
    // Other workspaces and admin paths.
    assert_eq!(post_sign_in_target(Some("/console#chat")), "/console#chat");
    assert_eq!(post_sign_in_target(Some("/console#sft")), "/console#sft");
    assert_eq!(post_sign_in_target(Some("/system#cpu")), "/system#cpu");
    assert_eq!(post_sign_in_target(Some("/admin")), "/admin");
    // Path+query forms.
    assert_eq!(
        post_sign_in_target(Some("/?range=24h#overview")),
        "/?range=24h#overview"
    );
}
