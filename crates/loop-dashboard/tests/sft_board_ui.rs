//! EKA SFT data-pipeline board — integration checks.
//!
//! The board (`assets/sft_board.html`) is a prebuilt self-contained page
//! served verbatim at `/sft-board` and embedded by the console SPA
//! (`#sft/board`) in an iframe — the same pattern as the pretraining
//! architecture canvas and the sftqc pipeline map. These tests pin that
//! contract:
//!
//! * the page serves with its six stages, run/reset controls, repair
//!   arrows and detail panel intact (nothing stripped or rewritten);
//! * it stays behind the session guard like every other workspace page;
//! * the console SPA knows the route: sub-route dispatch, the renderer,
//!   the iframe mount and the workspace entry card all point at it.

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

fn board_src() -> String {
    include_str!("../src/assets/sft_board.html").to_string()
}

// ---------------------------------------------------------------------------
// The served page keeps the original implementation intact.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn board_page_serves_the_original_stages_and_controls() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    for path in ["/sft-board", "/sft-board/index.html"] {
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

        // The interactive controls and views survive serving.
        for marker in [
            "id=\"runBtn\"",
            "id=\"resetBtn\"",
            "id=\"flow\"",
            "id=\"backSvg\"",
            "id=\"panel\"",
            "drawBack",
            "finishStage",
            "runStage",
        ] {
            assert!(body.contains(marker), "{path} lost '{marker}'");
        }

        // And the counts from the October 2026 build are untouched.
        for count in ["553,922,376", "552,465,424", "59,033,810", "57,868,265"] {
            assert!(body.contains(count), "{path} lost count {count}");
        }
    }
}

/// The asset on disk is what the binary serves: the embedded copy must be
/// the original file's bytes, wrapped only in a minimal HTML document.
#[test]
fn board_asset_preserves_the_original_html_verbatim() {
    let src = board_src();
    // Document wrapper added for standalone serving.
    assert!(src.starts_with("<!DOCTYPE html>"), "needs the doc wrapper");
    assert!(src.ends_with("</html>\n"), "needs the closing tags");
    // The original fragment's own markers, byte-for-byte in place.
    for marker in [
        "<title>EKA SFT Data Pipeline</title>",
        "var STAGES = [",
        "document.getElementById('closeBtn').addEventListener('click', closePanel);",
        "if (document.fonts && document.fonts.ready) document.fonts.ready.then",
    ] {
        assert!(src.contains(marker), "original code lost: {marker}");
    }
    // Exactly one script block — nothing injected alongside the original.
    assert_eq!(src.matches("<script>").count(), 1);
}

// ---------------------------------------------------------------------------
// Guard behaviour matches every other workspace page.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn board_page_is_session_guarded() {
    let (app, _a, _d) = app_with_auth().await;
    let (status, loc, _) = get(&app, "/sft-board", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        loc.unwrap().starts_with("/signin?next="),
        "unauthenticated /sft-board must redirect to sign-in"
    );
}

// ---------------------------------------------------------------------------
// The console SPA mounts the board inside the SFT DataTrove workspace.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn console_spa_wires_the_board_into_the_sft_workspace() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    let (status, _, spa) = get(&app, "/console/console-app.js", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);

    // Sub-route dispatch: #sft/board resolves to the board renderer.
    assert!(
        spa.contains("h.startsWith('sft/board')"),
        "paSubRoute must map #sft/board"
    );
    // The dispatcher hands it to the renderer before anything else renders.
    assert!(
        spa.contains("=== 'sft-board') return renderSftBoard()"),
        "renderSftPipeline must dispatch to renderSftBoard"
    );
    // The renderer exists and embeds the served page in an iframe.
    assert!(
        spa.contains("async function renderSftBoard()"),
        "renderSftBoard must exist"
    );
    assert!(
        spa.contains("src=\"/sft-board\""),
        "the board must be mounted from /sft-board"
    );
    // The SFT workspace home links to it.
    assert!(
        spa.contains("href=\"#sft/board\""),
        "the workspace needs an entry card for the board"
    );
    // And it stays inside the SFT workspace's route family.
    assert!(
        spa.contains("h.startsWith('sft/')"),
        "#sft/* must remain the sft parent route"
    );
}
