//! HTTP API for the pipeline workspaces: read-only JSON over
//! `/api/pipelines/*`.
//!
//! All handlers are read-only summaries of artifacts the pipelines already
//! wrote. There are deliberately no execution endpoints: starting a Slurm
//! array or a GPU scoring pass from a page load would be unsafe, and the
//! authoritative runners (`sbatch`, `python -m codepipe…`) are documented in
//! the UI instead.
//!
//! Discovery runs on the blocking pool and is cheap (bounded directory
//! listings + small JSON parses). A 30-second in-process cache keeps an
//! overview auto-refresh from re-scanning on every tick.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::pipelines::pretrain::{self as pretrain_mod, PretrainSummary};
use crate::pipelines::sft::{self as sft_mod, SftSummary};
use crate::pipelines::sft_code;
use crate::pipelines::{state_wire, PipelinePaths};

/// Shared state for pipeline routes.
pub struct PipelineState {
    /// Where the pipeline workspaces live.
    pub paths: PipelinePaths,
    /// Cached summaries with their computation time.
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    sft: Option<(Instant, SftSummary)>,
    pretrain: Option<(Instant, PretrainSummary)>,
}

/// Cache lifetime: long enough to absorb overview auto-refreshes, short
/// enough that a newly finished run appears promptly.
const CACHE_TTL: Duration = Duration::from_secs(30);

impl PipelineState {
    /// State rooted at the default workspace (`~/soket`).
    pub fn new() -> Self {
        Self {
            paths: PipelinePaths::resolve(),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// State rooted at an explicit path (tests).
    pub fn at(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            paths: PipelinePaths::at(root),
            cache: Mutex::new(Cache::default()),
        }
    }

    fn cached<T, F>(&self, slot: &mut Option<(Instant, T)>, compute: F) -> Arc<T>
    where
        T: Clone,
        F: FnOnce() -> T,
    {
        if let Some((at, value)) = slot {
            if at.elapsed() < CACHE_TTL {
                return Arc::new(value.clone());
            }
        }
        let value = compute();
        *slot = Some((Instant::now(), value.clone()));
        Arc::new(value)
    }

    fn sft_summary(&self) -> Arc<SftSummary> {
        let mut cache = self.cache.lock().unwrap();
        let paths = self.paths.clone();
        self.cached(&mut cache.sft, move || {
            tokio::task::block_in_place(|| sft_mod::summarize(&paths))
        })
    }

    fn pretrain_summary(&self) -> Arc<PretrainSummary> {
        let mut cache = self.cache.lock().unwrap();
        let paths = self.paths.clone();
        self.cached(&mut cache.pretrain, move || {
            tokio::task::block_in_place(|| pretrain_mod::summarize(&paths))
        })
    }
}

impl Default for PipelineState {
    fn default() -> Self {
        Self::new()
    }
}

/// Combined status used by the portal overview: one compact entry per
/// workspace so the overview never fetches full run lists.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortalStatus {
    sft: SftCard,
    pretrain: PretrainCard,
}

/// Compact SFT card payload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SftCard {
    integration: String,
    integration_label: String,
    runs: u64,
    rows: u64,
    high: u64,
    low: u64,
    latest_run_id: Option<String>,
    latest_completed_at_ms: Option<u64>,
    latest_model: Option<String>,
    unavailable_reason: Option<String>,
}

/// Compact pretraining card payload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PretrainCard {
    integration: String,
    integration_label: String,
    stages: u64,
    tasks_done: u64,
    latest_stage: Option<String>,
    kept_docs: u64,
    est_tokens_kept: Option<f64>,
    output_root: Option<String>,
    unavailable_reason: Option<String>,
}

/// `GET /api/pipelines/portal` — overview card summaries only.
async fn portal(State(state): State<Arc<PipelineState>>) -> Response {
    let sft = state.sft_summary();
    let pretrain = state.pretrain_summary();

    let sft_card = SftCard {
        integration: state_wire(sft.integration).to_string(),
        integration_label: sft.integration_label.clone(),
        runs: sft.totals.runs,
        rows: sft.totals.rows,
        high: sft.totals.high,
        low: sft.totals.low,
        latest_run_id: sft.latest.as_ref().map(|r| r.id.clone()),
        latest_completed_at_ms: sft.latest.as_ref().map(|r| r.completed_at_ms),
        latest_model: sft.latest.as_ref().and_then(|r| r.model.clone()),
        unavailable_reason: sft.unavailable_reason.clone(),
    };
    let pretrain_card = PretrainCard {
        integration: state_wire(pretrain.integration).to_string(),
        integration_label: pretrain.integration_label.clone(),
        stages: pretrain.totals.stages,
        tasks_done: pretrain.totals.tasks_done,
        latest_stage: pretrain.latest_stage.clone(),
        kept_docs: pretrain.totals.final_kept_docs,
        est_tokens_kept: (pretrain.totals.final_kept_docs > 0)
            .then_some(pretrain.totals.final_est_tokens_kept),
        output_root: pretrain.output_root.clone(),
        unavailable_reason: pretrain.unavailable_reason.clone(),
    };

    Json(PortalStatus {
        sft: sft_card,
        pretrain: pretrain_card,
    })
    .into_response()
}

/// `GET /api/pipelines/sft`
async fn sft_handler(State(state): State<Arc<PipelineState>>) -> Response {
    let s = state.sft_summary();
    Json((*s).clone()).into_response()
}

/// `GET /api/pipelines/sft/code` — the source-file index for the code
/// explorer (file names, line counts, top-level symbols, docstrings).
async fn sft_code_handler(State(state): State<Arc<PipelineState>>) -> Response {
    let paths = state.paths.clone();
    let idx = tokio::task::block_in_place(|| sft_code::code_index(&paths));
    Json(idx).into_response()
}

/// `GET /api/pipelines/sft/code/file?name=rules.py` — one file's text.
/// Query-bound and served as plain text; the explorer renders it with its
/// own highlighter. Names are validated by the adapter (no traversal).
async fn sft_source_handler(
    State(state): State<Arc<PipelineState>>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
) -> Response {
    // Minimal form-decoding of ?name=<value> without pulling in a crate.
    let name = raw
        .as_deref()
        .and_then(|qs| qs.split('&').find(|p| p.starts_with("name=")))
        .map(|p| p["name=".len()..].to_string())
        .map(|v| v.replace('+', " "))
        .and_then(|v| percent_decode(&v));

    let Some(name) = name else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing ?name="})),
        )
            .into_response();
    };
    match tokio::task::block_in_place(|| sft_code::read_source(&state.paths, &name)) {
        Ok(src) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            src,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

/// Percent-decode just enough for a file-name query parameter.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let hex = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `GET /api/pipelines/pretrain`
async fn pretrain_handler(State(state): State<Arc<PipelineState>>) -> Response {
    let s = state.pretrain_summary();
    Json((*s).clone()).into_response()
}

/// Build the pipelines router.
pub fn pipelines_router(state: Arc<PipelineState>) -> Router {
    Router::new()
        .route("/api/pipelines/portal", get(portal))
        .route("/api/pipelines/sft", get(sft_handler))
        .route("/api/pipelines/sft/code", get(sft_code_handler))
        .route("/api/pipelines/sft/code/file", get(sft_source_handler))
        .route("/api/pipelines/pretrain", get(pretrain_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn router_for(root: &std::path::Path) -> Router {
        pipelines_router(Arc::new(PipelineState::at(root)))
    }

    async fn get_json(router: &Router, uri: &str) -> (axum::http::StatusCode, serde_json::Value) {
        let res = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_workspace_is_honest_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let router = router_for(dir.path());
        for uri in [
            "/api/pipelines/portal",
            "/api/pipelines/sft",
            "/api/pipelines/pretrain",
        ] {
            let (status, body) = get_json(&router, uri).await;
            assert_eq!(status, axum::http::StatusCode::OK, "{uri}");
            let has_reason = body.get("unavailableReason").is_some()
                || body["sft"]["unavailableReason"].is_string();
            assert!(has_reason, "{uri} must explain itself");
        }
        let (_, portal) = get_json(&router, "/api/pipelines/portal").await;
        assert_eq!(portal["sft"]["integration"], "not_configured");
        assert_eq!(portal["pretrain"]["integration"], "not_configured");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn real_fixture_shape_flows_through() {
        let dir = tempfile::tempdir().unwrap();
        // SFT run
        let proj = dir.path().join("sftqc/sftqc");
        std::fs::create_dir_all(&proj).unwrap();
        let run = dir.path().join("qc_out");
        std::fs::create_dir_all(run.join("high")).unwrap();
        std::fs::write(
            run.join("report.json"),
            r#"{"input":"d.jsonl","rows":20,"settings":{"model":"m"},"high":18,"low":2,
               "languages":{"en":18},"seconds":10.0}"#,
        )
        .unwrap();
        // Pretrain stage
        let pproj = dir.path().join("codepipe/codepipe");
        std::fs::create_dir_all(&pproj).unwrap();
        let stage = dir.path().join("codepipe/01_filter/_done");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(
            stage.join("t00000.json"),
            r#"{"Python":{"in_docs":10,"in_bytes":1000,"kept_docs":8,"kept_bytes":800,
                "removed":{"vendored":[2,200]}}}"#,
        )
        .unwrap();

        let router = router_for(dir.path());
        let (_, sft) = get_json(&router, "/api/pipelines/sft").await;
        assert_eq!(sft["integration"], "connected");
        assert_eq!(sft["totals"]["rows"], 20);
        assert_eq!(sft["runs"][0]["high"], 18);

        let (_, pre) = get_json(&router, "/api/pipelines/pretrain").await;
        assert_eq!(pre["integration"], "connected");
        assert_eq!(pre["stages"][0]["docsIn"], 10);
        assert_eq!(pre["stages"][0]["removalReasons"][0]["reason"], "vendored");

        let (_, portal) = get_json(&router, "/api/pipelines/portal").await;
        assert_eq!(portal["sft"]["runs"], 1);
        assert_eq!(portal["pretrain"]["stages"], 1);
        assert_eq!(portal["pretrain"]["keptDocs"], 8);
        assert!(portal["pretrain"]["estTokensKept"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_explorer_serves_real_source_files() {
        let dir = tempfile::tempdir().unwrap();
        // The real project layout: <root>/sftqc/*.py
        let proj = dir.path().join("sftqc");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("rules.py"),
            "\"\"
Layer 1 docstring.
\"\"\n\ndef check_row(obj):\n    return []\n\nclass Flag:\n    pass\n",
        )
        .unwrap();

        let router = router_for(dir.path());
        let (_, idx) = get_json(&router, "/api/pipelines/sft/code").await;
        assert_eq!(idx["integration"], "connected");
        assert_eq!(idx["files"][0]["name"], "rules.py");
        assert_eq!(idx["files"][0]["symbols"][0]["name"], "check_row");
        assert_eq!(idx["files"][0]["symbols"][0]["line"], 5);
        assert_eq!(idx["files"][0]["symbols"][1]["name"], "Flag");

        // File body served as text through the same router.
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/pipelines/sft/code/file?name=rules.py")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("def check_row"));

        // Traversal and missing files are refused honestly.
        for (uri, want) in [
            ("/api/pipelines/sft/code/file?name=../secrets.py", 404),
            ("/api/pipelines/sft/code/file?name=nope.py", 404),
            ("/api/pipelines/sft/code/file", 400),
        ] {
            let res = router
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                res.status().as_u16(),
                want,
                "{uri} must answer {want}, got {}",
                res.status()
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn selftest_results_surface_in_sft_summary() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("sftqc");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("pipeline.py"), "# p").unwrap();
        let st = dir.path().join("selftest_0.5b");
        std::fs::create_dir_all(&st).unwrap();
        std::fs::write(
            st.join("selftest_results.json"),
            r#"{"clean": {"rows_in_low": 0, "total": 20},
               "stray_qmark": {"caught": 17, "low": 17, "total": 20},
               "junk_sweep": [{"nll": 3}, {"nll": 4}]}"#,
        )
        .unwrap();

        let router = router_for(dir.path());
        let (_, sft) = get_json(&router, "/api/pipelines/sft").await;
        let sts = &sft["selftests"];
        assert_eq!(sts.as_array().map(|a| a.len()), Some(1));
        assert_eq!(sts[0]["id"], "selftest_0.5b");
        assert_eq!(sts[0]["cleanRowsInLow"], 0);
        let kinds = sts[0]["kinds"].as_array().unwrap();
        assert_eq!(kinds.len(), 1, "junk_sweep is not a kind");
        assert_eq!(kinds[0]["kind"], "stray_qmark");
        assert_eq!(kinds[0]["caught"], 17);
        assert_eq!(sts[0]["sweepPoints"], 2);
    }
}
