//! Original EKA SFT data-pipeline board — integration checks.
//!
//! The workspace's "Open pipeline map" entry opens the ORIGINAL
//! `eka-sft-board-2.html` (copied byte-for-byte into
//! `assets/sft_pipeline_map.html`), served verbatim at `/sft-pipeline-map`
//! and opened in a new browser tab. Nothing is recreated or restyled here;
//! these tests pin that contract:
//!
//! * the asset keeps the original document intact — its own title, one
//!   style block, one script block, its interactive controls, drawer,
//!   reject lane and the October 2026 counts;
//! * it serves behind the session guard like every other workspace page;
//! * the console SPA's "Open pipeline map" entries point at the served
//!   original in a new tab (`target="_blank"`), staying inside the SFT
//!   DataTrove workflow.

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
        console_state: None,
        auth_store: Some(auth.clone()),
    })
    .unwrap();
    (app, auth, dir)
}

/// GET through the real router, returning (status, location, body).
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
        .map(|l| l.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn map_src() -> String {
    include_str!("../src/assets/sft_pipeline_map.html").to_string()
}

// ---------------------------------------------------------------------------
// The asset is the original file, wrapped only in a minimal document shell.
// ---------------------------------------------------------------------------

#[test]
fn map_asset_preserves_the_original_html_verbatim() {
    let src = map_src();
    // Document wrapper added for standalone serving — nothing more.
    assert!(src.starts_with("<!DOCTYPE html>"), "needs the doc wrapper");
    assert!(src.ends_with("</html>\n"), "needs the closing tags");
    // The original fragment's own markers, byte-for-byte in place.
    for marker in [
        "<title>EKA SFT Data Pipeline</title>",
        "var STAGES = [",
        "document.getElementById('closeBtn').addEventListener('click', closeDrawer);",
        "if (document.fonts && document.fonts.ready) document.fonts.ready.then(drawWires);",
    ] {
        assert!(src.contains(marker), "original code lost: {marker}");
    }
    // Exactly one style block and one script block — nothing injected
    // alongside the original implementation.
    assert_eq!(src.matches("<style>").count(), 1);
    assert_eq!(src.matches("<script>").count(), 1);
}

// ---------------------------------------------------------------------------
// The served page keeps the original board intact.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn map_page_serves_the_original_board() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    for path in ["/sft-pipeline-map", "/sft-pipeline-map/index.html"] {
        let (status, loc, body) = get(&app, path, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(loc.is_none(), "{path} must not redirect a valid session");

        // The six documented stages, in order.
        let mut prev = 0;
        for stage in [
            "Collect",
            "Standardize",
            "Clean corpus",
            "Curate gold",
            "Audit + handoff",
            "Train + feedback",
        ] {
            let at = body
                .find(stage)
                .unwrap_or_else(|| panic!("{path} must contain stage '{stage}'"));
            assert!(
                at > prev,
                "{path}: '{stage}' must follow the previous stage"
            );
            prev = at;
        }

        // The original interactive controls and views survive serving:
        // run/reset/next buttons, speed selector, progress, the "now
        // running" panel + log, the wired board, the drawer with its
        // repair/trace buttons, and the engine functions behind them.
        for marker in [
            "id=\"runBtn\"",
            "id=\"resetBtn\"",
            "id=\"nextBtn\"",
            "id=\"speedSel\"",
            "id=\"progBar\"",
            "id=\"stepCount\"",
            "id=\"nState\"",
            "id=\"log\"",
            "id=\"board\"",
            "id=\"wires\"",
            "id=\"repairs\"",
            "id=\"drawer\"",
            "id=\"dArrow\"",
            "id=\"dBack\"",
            "id=\"dFwd\"",
            "drawWires",
            "traceFrom",
            "reduceMotion",
        ] {
            assert!(body.contains(marker), "{path} lost '{marker}'");
        }

        // And the counts from the October 2026 build are untouched.
        for count in ["553,922,376", "552,465,424", "59,033,810", "57,868,265"] {
            assert!(body.contains(count), "{path} lost count {count}");
        }
    }
}

// ---------------------------------------------------------------------------
// Guard behaviour matches every other workspace page.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn map_page_is_session_guarded() {
    let (app, _a, _d) = app_with_auth().await;
    let (status, loc, _) = get(&app, "/sft-pipeline-map", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        loc.unwrap().starts_with("/signin?next="),
        "unauthenticated /sft-pipeline-map must redirect to sign-in"
    );
}

// ---------------------------------------------------------------------------
// The console SPA opens the original map from the SFT DataTrove workspace.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn console_spa_opens_the_original_map_from_the_sft_workspace() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    let (status, _, spa) = get(&app, "/console/console-app.js", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);

    // Both "Open pipeline map" entry cards (configured + not-configured
    // views) point at the served original…
    assert_eq!(
        spa.matches("href=\"/sft-pipeline-map\" target=\"_blank\"").count(),
        4,
        "the Open pipeline map entries and cross-links must all target /sft-pipeline-map in a new tab"
    );
    // …opened in a new browser tab, safely.
    assert!(
        spa.contains("rel=\"noopener\">Open pipeline map"),
        "the entries must open in a new tab with rel=noopener"
    );
    // The workspace's own cross-links reach the same original file.
    assert_eq!(
        spa.matches("Pipeline map \u{2197}</a>").count(),
        2,
        "the board and code-explorer cross-links must reach the original map"
    );
    // And the SFT workspace route family is untouched.
    assert!(
        spa.contains("h.startsWith('sft/')"),
        "#sft/* must remain the sft parent route"
    );
}
