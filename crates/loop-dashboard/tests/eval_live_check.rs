// Live checks against the real kit at ~/.loop/eval (skipped when absent).
use axum::body::Body;
use axum::http::Request;
use std::sync::Arc;
use tower::ServiceExt;

// One shared state so caching behaves like the real server (a fresh state
// per request would rebuild the bank index every time).
static STATE: std::sync::OnceLock<Arc<loop_dashboard::eval::api::EvalState>> =
    std::sync::OnceLock::new();

fn state() -> Arc<loop_dashboard::eval::api::EvalState> {
    STATE
        .get_or_init(|| Arc::new(loop_dashboard::eval::api::EvalState::new()))
        .clone()
}

fn router() -> axum::Router {
    loop_dashboard::eval::api::eval_router(state())
}

async fn get(uri: &str) -> (axum::http::StatusCode, String) {
    let res = router()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn kit_present() -> bool {
    std::path::Path::new(
        &std::env::var("HOME")
            .map(|h| format!("{h}/.loop/eval"))
            .unwrap_or_default(),
    )
    .join("internal_eval")
    .is_dir()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_kit_overview() {
    if !kit_present() {
        eprintln!("kit absent; skipping");
        return;
    }
    let (s, body) = get("/api/eval/overview").await;
    assert_eq!(s, 200);
    assert!(body.contains("\"questions\":266633"), "real bank total");
    assert!(body.contains("\"files\":50"));
}

#[tokio::test(flavor = "multi_thread")]
async fn live_large_bank_pages_are_fast_and_correct() {
    if !kit_present() {
        eprintln!("kit absent; skipping");
        return;
    }
    // Deep page of the largest bank must not scan the whole file.
    // First request builds (and caches) the index; measure the warm path.
    let (_s, _warm) = get("/api/eval/banks/qbank_v2_all.jsonl/page?per_page=25&page=1").await;
    let t0 = std::time::Instant::now();
    let (s, body) = get("/api/eval/banks/qbank_v2_all.jsonl/page?per_page=25&page=3000").await;
    let elapsed = t0.elapsed();
    assert_eq!(s, 200, "{body}");
    assert!(body.contains("\"total\":96500"), "row count from the index");
    // 96,500 rows / 25 per page = 3,860 pages, so page 3000 is valid.
    assert!(body.contains("\"page\":3000"), "requested page served");
    assert!(body.contains("\"pages\":3860"), "page count from the index");
    assert!(body.contains("\"id\":\""), "rows carry ids");
    println!("deep page served in {elapsed:?}");
    assert!(
        elapsed.as_millis() < 1500,
        "deep page must be fast, took {elapsed:?}"
    );

    // A search across the whole bank stays bounded too.
    let t0 = std::time::Instant::now();
    let (s, body) = get("/api/eval/banks/qbank_v2_all.jsonl/page?q=triangle&per_page=10").await;
    assert_eq!(s, 200);
    assert!(
        body.contains("\"total\":") && !body.contains("\"total\":0"),
        "search finds hits"
    );
    println!("search served in {:?}", t0.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn live_docs_and_tools_serve() {
    if !kit_present() {
        eprintln!("kit absent; skipping");
        return;
    }
    let (s, body) = get("/api/eval/docs").await;
    assert_eq!(s, 200);
    assert!(body.contains("README.md"));
    let (s, doc) = get("/api/eval/docs/README.md").await;
    assert_eq!(s, 200);
    assert!(doc.contains("internal_eval"));
    let (s, pv) = get("/api/eval/file/pipeline::judge.py").await;
    assert_eq!(s, 200);
    assert!(pv.contains("blind"));
    // The refs TSV previews as a table with its two columns.
    let (s, tsv) = get("/api/eval/file/examples::refs_iter3700.tsv").await;
    assert_eq!(s, 200);
    assert!(tsv.contains("label"), "tsv columns missing");
}
