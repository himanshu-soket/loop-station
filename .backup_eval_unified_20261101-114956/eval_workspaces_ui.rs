//! Internal evaluation workspaces — integration checks.
//!
//! The six workspaces (question bank, datasets, benchmarks, model
//! evaluation, reports, tools) are served by `/api/eval/*` and rendered by
//! `assets/eval-workspaces.js`, mounted inside the console SPA. These tests
//! pin that contract end to end:
//!
//! * **API** — through the *full* app (guard layer included), so the
//!   session-guard behaviour is exercised exactly as a browser hits it;
//! * **UI wiring** — the console SPA must load the eval script, register
//!   all six routes, render the six cards and reach them from the nav;
//! * **data honesty** — a fixture kit yields real counts, and a missing kit
//!   answers "not configured" instead of an error or invented numbers.

use std::sync::{Arc, Mutex, OnceLock};

/// `LOOP_EVAL_ROOT` is process-global, so tests that set it hold this lock
/// for their entire body (including awaits).
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // Ignore poisoning: an assertion failure in one test must not cascade
    // into every other env-sensitive test in this binary.
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use loop_dashboard::api::{build_app, AppOptions};
use loop_dashboard::auth::{AuthStore, SESSION_COOKIE};
use loop_dashboard::web::web_router;

/// Full app (guards + auth) over a temp DB, with one approved owner signed
/// in. Returns the session cookie.
async fn app_with_owner() -> (axum::Router, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AuthStore::open(dir.path().join("auth.db")).unwrap());
    // Bootstrap the sole owner exactly as a fresh install would.
    store
        .sign_up(
            loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
            "correct horse battery",
        )
        .unwrap();
    let session = store
        .sign_in(
            loop_dashboard::auth::LEGACY_ADMIN_IDENTIFIER,
            "correct horse battery",
        )
        .unwrap();
    let cookie = format!("{SESSION_COOKIE}={session}");
    let (app, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        console_state: None,
        auth_store: Some(store),
    })
    .unwrap();
    (app, cookie, dir)
}

/// GET through the full router with an optional cookie.
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
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// A small but representative fixture kit: banks, examples, pipeline,
/// slurm, docs, config, tests and one run with a graded setting.
fn fixture_kit() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let kit = dir.path().join("internal_eval");
    for sub in [
        "banks",
        "examples",
        "pipeline",
        "slurm",
        "config",
        "tests",
        "compare_cache",
    ] {
        std::fs::create_dir_all(kit.join(sub)).unwrap();
    }
    // Two banks with real schemas.
    let mk_row = |i: usize, group: &str, subject: &str, diff: &str| {
        format!(
            r#"{{"id":"{group}|Q{i:03}","suite":"school","group":"{group}","subject":"{subject}","topic":"T{i}","difficulty":"{diff}","language":"","question_type":"mcq","has_options":false,"reference":"correct_answer: {i}","question":"question number {i} about triangles","qhash":"h{i}"}}"#
        )
    };
    let rows: Vec<String> = (0..30)
        .map(|i| {
            mk_row(
                i,
                "Class 1",
                "Mathematics",
                if i % 2 == 0 { "L1" } else { "L2" },
            )
        })
        .collect();
    std::fs::write(kit.join("banks/qbank_sample.jsonl"), rows.join("\n") + "\n").unwrap();
    let rows2: Vec<String> = (0..12).map(|i| mk_row(i, "Coding", "CS", "L3")).collect();
    std::fs::write(kit.join("banks/qbank_v2.jsonl"), rows2.join("\n") + "\n").unwrap();

    std::fs::write(
        kit.join("examples/custom_questions.jsonl"),
        concat!(
            r#"{"id":"q1","group":"Physics","suite":"general","difficulty":"L2","question":"How long?","correct_answer":"2 s"}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(kit.join("examples/refs.tsv"), "label\tpath\nA\t/runs/a\n").unwrap();
    std::fs::write(kit.join("pipeline/judge.py"), "# stage 3: blind grading\n").unwrap();
    std::fs::write(kit.join("pipeline/report.py"), "# stage 4b\n").unwrap();
    std::fs::write(kit.join("slurm/gen.sbatch"), "#SBATCH --job-name=gen\n").unwrap();
    std::fs::write(
        kit.join("README.md"),
        "# internal_eval\n\n## Setup\n\ntext\n",
    )
    .unwrap();
    std::fs::write(kit.join("FAQ.md"), "# FAQ\n\nbody\n").unwrap();
    std::fs::write(kit.join("config/exclude_nodes.txt"), "node1,node2\n").unwrap();
    std::fs::write(kit.join("tests/smoke.sh"), "#!/bin/bash\n").unwrap();

    // One run with a graded setting + summary.
    let run = kit.join("runs/EKA_smoke_20261101");
    std::fs::create_dir_all(run.join("settings/med06")).unwrap();
    std::fs::create_dir_all(run.join("compare")).unwrap();
    std::fs::write(
        run.join("config.json"),
        r#"{"urls":["http://n:8004"],"model":"eka-v1","label":"EKA smoke","input":"qbank-sample",
           "max_tokens":8192,"workers":32,
           "settings":[{"name":"med06","temperature":0.6,"effort":"medium"}],
           "compare":["gpt-oss-120b","qwen3-32b"]}"#,
    )
    .unwrap();
    std::fs::write(run.join("questions.jsonl"), "{}\n{}\n{}\n{}\n").unwrap();
    std::fs::write(
        run.join("settings/med06/report.json"),
        r#"{"n":42,
           "overall":{"eka":{"n":42,"acc":61.2,"strict":52.0,"lenient":62.0,"score":0.63},
                      "gpt-oss-120b":{"n":42,"acc":90.9,"strict":88.0,"lenient":91.0,"score":0.91}},
           "by_group":{"Class 1":{"eka":{"n":30,"acc":64.0},"gpt-oss-120b":{"n":30,"acc":95.0}}},
           "errors":{"eka":{"conceptual":7}},
           "eka_gap_questions":11,"eka_only_right":3}"#,
    )
    .unwrap();
    std::fs::write(run.join("summary.json"), r#"{"n_questions":42}"#).unwrap();
    std::fs::write(run.join("summary.html"), "<html>summary</html>").unwrap();
    std::fs::write(run.join("compare/gen_gpt-oss-120b.jsonl"), "{}\n{}\n").unwrap();
    dir
}

// ---------------------------------------------------------------- API ----

#[tokio::test(flavor = "multi_thread")]
async fn eval_api_requires_a_session() {
    let (app, _cookie, _dir) = app_with_owner().await;
    for uri in [
        "/api/eval/overview",
        "/api/eval/banks",
        "/api/eval/runs",
        "/api/eval/files",
        "/api/eval/docs",
        "/api/eval/benchmarks",
        "/api/eval/comparison",
    ] {
        let (status, _) = get(&app, uri, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} must be guarded");
    }
    // With a session every endpoint answers 200.
    for uri in [
        "/api/eval/overview",
        "/api/eval/banks",
        "/api/eval/runs",
        "/api/eval/files",
        "/api/eval/docs",
        "/api/eval/benchmarks",
        "/api/eval/comparison",
    ] {
        let (status, body) = get(&app, uri, Some(&_cookie)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn eval_script_is_served_and_guarded_like_the_page() {
    // Assets (.js) are public by design: no user data, and the SPA must
    // survive a mid-session expiry. Its API calls are separately guarded.
    let app = web_router();
    let (status, body) = get(&app, "/console/eval-workspaces.js", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("renderEvalBank"), "the workspace script body");
    assert!(body.contains("/api/eval/"), "it talks to the eval API");
}

// The guard is deliberately held across awaits: LOOP_EVAL_ROOT must stay
// set for the whole test body, including every HTTP round-trip.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread")]
async fn fixture_kit_flows_through_every_endpoint() {
    let _env = env_lock();
    let kit = fixture_kit();
    std::env::set_var("LOOP_EVAL_ROOT", kit.path());
    let (app, cookie, _auth) = app_with_owner().await;

    // Overview carries only real counts.
    let (_, body) = get(&app, "/api/eval/overview", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["integration"], "connected");
    assert_eq!(v["banks"]["questions"], 43, "30 + 12 + 1 example rows");
    assert_eq!(v["evaluations"]["runs"], 1);
    assert_eq!(v["evaluations"]["best"]["acc"], 61.2);
    assert_eq!(v["tools"]["docs"], 2);

    // Banks list both files with facet counts.
    let (_, body) = get(&app, "/api/eval/banks", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "two banks + the example question file");
    let qb = items
        .iter()
        .find(|b| b["name"] == "qbank_sample.jsonl")
        .unwrap();
    assert_eq!(qb["rows"], 30);
    assert_eq!(qb["withReference"], 30);
    // Facet fields are flattened onto the bank entry (camelCase).
    assert_eq!(qb["suites"][0]["key"], "school");
    assert_eq!(qb["suites"][0]["count"], 30);

    // Bank pages filter and search.
    let (_, page) = get(
        &app,
        "/api/eval/banks/qbank_sample.jsonl/page?q=triangles&per_page=10",
        Some(&cookie),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&page).unwrap();
    assert_eq!(v["total"], 30, "every row mentions triangles");
    assert_eq!(v["items"].as_array().unwrap().len(), 10);
    let (_, page) = get(
        &app,
        "/api/eval/banks/qbank_sample.jsonl/page?difficulty=L2&per_page=50",
        Some(&cookie),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&page).unwrap();
    assert_eq!(v["total"], 15, "odd indices are L2");

    // Unknown bank → honest 404.
    let (status, _) = get(&app, "/api/eval/banks/nope.jsonl/page", Some(&cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Runs + comparison expose the graded numbers.
    let (_, runs) = get(&app, "/api/eval/runs", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&runs).unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["items"][0]["reports"][0]["overall"][0]["acc"], 61.2);
    let (_, cmp) = get(&app, "/api/eval/comparison", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&cmp).unwrap();
    assert_eq!(v["rows"].as_array().unwrap().len(), 2);

    // Benchmarks derive presets from the real bank files.
    let (_, b) = get(&app, "/api/eval/benchmarks", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    let names: Vec<&str> = v["presets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"qbank-sample"), "{names:?}");
    assert_eq!(v["settings"][0]["name"], "med06");
    assert_eq!(v["comparisonModels"].as_array().unwrap().len(), 2);

    // Reports workspace finds the summary artifacts.
    let (_, files) = get(&app, "/api/eval/files?workspace=reports", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&files).unwrap();
    let rels: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["relPath"].as_str().unwrap())
        .collect();
    assert!(
        rels.contains(&"runs/EKA_smoke_20261101/summary.json"),
        "{rels:?}"
    );
    assert!(
        rels.contains(&"runs/EKA_smoke_20261101/summary.html"),
        "{rels:?}"
    );

    // Tools lists the scripts and the docs.
    let (_, tools) = get(&app, "/api/eval/files?workspace=tools", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&tools).unwrap();
    let rels: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["relPath"].as_str().unwrap())
        .collect();
    assert!(rels.contains(&"pipeline/judge.py"), "{rels:?}");
    assert!(rels.contains(&"slurm/gen.sbatch"), "{rels:?}");
    assert!(rels.contains(&"README.md"), "{rels:?}");

    // Docs serve their markdown.
    let (_, doc) = get(&app, "/api/eval/docs/README.md", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert!(v["markdown"].as_str().unwrap().contains("Setup"));

    // File previews work for text and tables.
    let (_, pv) = get(&app, "/api/eval/file/pipeline::judge.py", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&pv).unwrap();
    assert_eq!(v["mode"], "text");
    assert!(v["text"].as_str().unwrap().contains("blind grading"));
    let (_, pv) = get(&app, "/api/eval/file/examples::refs.tsv", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&pv).unwrap();
    assert_eq!(v["mode"], "rows");
    assert_eq!(v["columns"].as_array().unwrap().len(), 2);

    // Traversal ids never resolve.
    for bad in ["..::etc::passwd", "banks::..::qbank.jsonl", "missing.py"] {
        let (status, _) = get(&app, &format!("/api/eval/file/{bad}"), Some(&cookie)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
    }

    std::env::remove_var("LOOP_EVAL_ROOT");
}

// Same as above: the env var must outlive the awaits in this test.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread")]
async fn missing_kit_reports_not_configured_without_errors() {
    let _env = env_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("LOOP_EVAL_ROOT", dir.path());
    let (app, cookie, _auth) = app_with_owner().await;

    for uri in [
        "/api/eval/overview",
        "/api/eval/banks",
        "/api/eval/runs",
        "/api/eval/datasets",
        "/api/eval/benchmarks",
        "/api/eval/docs",
        "/api/eval/comparison",
    ] {
        let (status, body) = get(&app, uri, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["integration"], "not_configured", "{uri}");
    }
    // The overview never invents numbers.
    let (_, body) = get(&app, "/api/eval/overview", Some(&cookie)).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["files"], 0);
    assert_eq!(v["banks"]["questions"], 0);
    assert_eq!(v["evaluations"]["best"], serde_json::Value::Null);

    std::env::remove_var("LOOP_EVAL_ROOT");
}

// ---------------------------------------------------------- UI wiring ----

const CONSOLE_HTML: &str = include_str!("../src/assets/console.html");
const CONSOLE_APP_JS: &str = include_str!("../src/assets/console-app.js");
const EVAL_JS: &str = include_str!("../src/assets/eval-workspaces.js");

#[test]
fn console_loads_the_eval_script_before_the_app() {
    let idx = CONSOLE_HTML
        .find("/console/eval-workspaces.js")
        .expect("script tag");
    let app = CONSOLE_HTML
        .find("/console/console-app.js")
        .expect("app tag");
    assert!(
        idx < app,
        "eval-workspaces.js must load before console-app.js"
    );
}

#[test]
fn console_app_registers_all_six_routes_and_cards() {
    for route in [
        "evalbank",
        "evaldata",
        "evalbench",
        "evalmodel",
        "evalreport",
        "evaltools",
    ] {
        assert!(
            CONSOLE_APP_JS.contains(&format!("case '{route}':")),
            "route {route} dispatched"
        );
        assert!(
            CONSOLE_APP_JS.contains(&format!("'{route}',")),
            "route {route} in WS_ROUTES or nav"
        );
    }
    // The six cards with their icons and destinations.
    for id in [
        "evalbank",
        "evaldata",
        "evalbench",
        "evalmodel",
        "evalreport",
        "evaltools",
    ] {
        assert!(
            CONSOLE_APP_JS.contains(&format!("id: '{id}',")),
            "card {id}"
        );
        assert!(
            CONSOLE_APP_JS.contains(&format!("{id}:'<svg")),
            "icon {id} in ICONS"
        );
    }
    // The badge filler runs from the overview renderer.
    assert!(CONSOLE_APP_JS.contains("fillEvalCards()"));
    assert!(CONSOLE_APP_JS.contains("/api/eval/overview"));
}

#[test]
fn eval_script_defines_all_six_renderers() {
    for name in [
        "renderEvalBank",
        "renderEvalData",
        "renderEvalBench",
        "renderEvalModel",
        "renderEvalReport",
        "renderEvalTools",
    ] {
        assert!(
            EVAL_JS.contains(&format!("async function {name}(")),
            "{name} must exist"
        );
    }
    // It reuses the SPA's helpers rather than duplicating them.
    for helper in ["api(", "esc(", "$('#pageTitle')", "renderPager", "mdToHtml"] {
        assert!(EVAL_JS.contains(helper), "uses {helper}");
    }
    // No iframes: the workspaces render natively inside the SPA.
    assert!(!EVAL_JS.contains("<iframe"));
    assert!(!EVAL_JS.contains("iframe"));
}

#[test]
fn eval_styles_are_part_of_the_console_stylesheet() {
    for cls in [
        ".ev-grid",
        ".ev-facets",
        ".ev-chip",
        ".ev-md",
        ".ev-portal-head",
    ] {
        assert!(CONSOLE_HTML.contains(cls), "{cls} styled");
    }
}
