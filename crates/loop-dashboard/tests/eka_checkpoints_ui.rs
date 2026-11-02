//! Original EKA Checkpoints by Level webpage — integration checks.
//!
//! Model Evaluation (`#eval/model`) is the entry point to the ORIGINAL
//! "EKA Checkpoints by Level" webpage, copied byte-for-byte into
//! `assets/eka_checkpoints.html` and served verbatim at `/eka-checkpoints`.
//! The console SPA mounts it in an iframe that fills the whole workspace
//! pane. Nothing is recreated or restyled; these tests pin that contract:
//!
//! * the asset keeps the original document intact — its own title, both
//!   style blocks, the verdict panel, all four tables (including the two
//!   collapsible older-checkpoint tables), the tinted accuracy cells and
//!   the reference-model rows;
//! * it serves behind the session guard like every other workspace page,
//!   with an HTML content type so it renders as a webpage, not source;
//! * the eval script's Model Evaluation renderer mounts exactly this page,
//!   full-bleed, with no intermediate landing screen.

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
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|c| c.to_str().ok())
        .map(|c| c.to_string());
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut body = format!("__content_type__={content_type:?}__");
    body.push_str(&String::from_utf8_lossy(&bytes));
    (status, location, body)
}

fn page_src() -> String {
    include_str!("../src/assets/eka_checkpoints.html").to_string()
}

// ---------------------------------------------------------------------------
// The asset is the original file, unchanged.
// ---------------------------------------------------------------------------

#[test]
fn asset_preserves_the_original_html_verbatim() {
    let src = page_src();
    // The original document's own markers, byte-for-byte in place.
    for marker in [
        "<title>EKA Checkpoints by Level</title>",
        "<h1>EKA checkpoints, level by level</h1>",
        /* verdict panel */
        "<section class=\"verdict\"><h2>Best setting per checkpoint (full bank)</h2>",
        /* the four tables' headings, in order */
        "<h2>Level-wise: best setting of each checkpoint</h2>",
        "<h2>Every checkpoint and setting, every level</h2>",
        "<h2>Older checkpoints: full bank, unchanged questions</h2>",
        "<h2>Older checkpoints: sample, unchanged questions</h2>",
        /* the collapsible table keeps its toggle */
        "<details><summary>Show table</summary>",
        /* sources note */
        "<section class=\"notes\">",
    ] {
        assert!(src.contains(marker), "original markup lost: {marker}");
    }
    // Exactly two style blocks and zero scripts — the original had no JS.
    assert_eq!(src.matches("<style>").count(), 2);
    assert_eq!(src.matches("<script").count(), 0);
    // All four tables survive.
    assert_eq!(src.matches("<table>").count(), 4);
    // The tinted-cell styling hook and the reference-row classes survive.
    assert!(src.contains("td.num[style] { background: rgba(var(--tint)"));
    assert!(src.contains("tr.ref th"));
    assert!(src.contains("tr.oth th"));
    // The original's dark-mode media query survives untouched.
    assert!(src.contains("@media (prefers-color-scheme: dark)"));
    // Its only external dependency is the Google Fonts stylesheet.
    assert_eq!(
        src.matches("https://").count(),
        2,
        "exactly the two font links"
    );
}

// ---------------------------------------------------------------------------
// The served page keeps the original webpage intact.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn page_serves_the_original_webpage() {
    let (app, auth, _d) = app_with_auth().await;
    auth.sign_up("owner", "correct horse battery").unwrap();
    let token = auth.sign_in("owner", "correct horse battery").unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");

    for path in ["/eka-checkpoints", "/eka-checkpoints/index.html"] {
        let (status, loc, body) = get(&app, path, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(loc.is_none(), "{path} must not redirect a valid session");
        // Served as a webpage, so an iframe renders it rather than showing
        // source or triggering a download.
        assert!(
            body.starts_with("__content_type__=Some(\"text/html; charset=utf-8\")__"),
            "{path} must answer text/html, got: {}",
            &body[..60]
        );
        // The original content follows, verbatim.
        assert!(
            body.contains("<title>EKA Checkpoints by Level</title>"),
            "{path} lost the original title"
        );
        assert!(
            body.contains("<h1>EKA checkpoints, level by level</h1>"),
            "{path} lost the original heading"
        );
        assert!(
            body.trim_end().ends_with("</html>"),
            "{path} must serve the complete document"
        );
    }
}

#[tokio::test]
async fn page_is_session_guarded_like_other_workspaces() {
    let (app, _auth, _d) = app_with_auth().await;
    for path in ["/eka-checkpoints", "/eka-checkpoints/index.html"] {
        let (status, loc, _) = get(&app, path, None).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{path} must redirect");
        let loc = loc.expect("{path} sends a location");
        assert!(
            loc.starts_with("/signin?next="),
            "{path} redirects to sign-in, got {loc}"
        );
    }
}

// ---------------------------------------------------------------------------
// The console mounts exactly this page at #eval/model.
// ---------------------------------------------------------------------------

const CONSOLE_HTML: &str = include_str!("../src/assets/console.html");
const EVAL_JS: &str = include_str!("../src/assets/eval-workspaces.js");

#[test]
fn model_evaluation_mounts_the_original_page_full_bleed() {
    // The renderer points the iframe at the served original.
    assert!(EVAL_JS.contains(r#"src="/eka-checkpoints""#));
    // Full-bleed: the frame uses the dedicated full-workspace classes.
    assert!(EVAL_JS.contains("evws-full"));
    assert!(EVAL_JS.contains("evws-full-frame"));

    // Scope the "no landing screen" checks to the Model Evaluation
    // renderer itself — other sections keep their own honest empty states.
    let start = EVAL_JS
        .find("async function renderEvalModel(")
        .expect("renderer exists");
    let end = EVAL_JS[start..]
        .find("\n}\n")
        .map(|i| start + i)
        .unwrap_or(EVAL_JS.len());
    let renderer = &EVAL_JS[start..end];
    assert!(
        !renderer.contains("No evaluation runs yet"),
        "the old empty state must be gone from Model Evaluation"
    );
    assert!(
        !renderer.contains("no run folders discovered"),
        "the old empty panel must be gone from Model Evaluation"
    );
    assert!(
        !renderer.contains("sk-line"),
        "no loading skeleton: the page mounts at once"
    );

    // The shell styles the full-bleed mount without padding or a card.
    assert!(CONSOLE_HTML.contains(".evws-main .evws-full"));
    assert!(CONSOLE_HTML.contains(".evws-full-frame"));
}
