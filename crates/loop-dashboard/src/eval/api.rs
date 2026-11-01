//! HTTP API for the evaluation workspaces: read-only JSON over
//! `/api/eval/*`.
//!
//! Like [`crate::pipelines::api`], every handler is a read-only summary of
//! what already exists on disk. There are deliberately no execution
//! endpoints: the kit's own `submit.sh` / `sbatch` runners stay
//! authoritative and are documented in the UI instead.
//!
//! Expensive work (bank indexing, run loading) runs on the blocking pool
//! behind a small in-process cache, so an overview auto-refresh never
//! re-scans a 62 MB bank.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::eval::banks::{self, BankIndex, BankQuery};
use crate::eval::catalog::{self, CatalogSummary, Resource, Workspace};
use crate::eval::docs::{self, DocEntry};
use crate::eval::files;
use crate::eval::reports::{self, ReportCard};
use crate::eval::runs::{self, ComparisonRow, EvalRun};
use crate::eval::{EvalPaths, IntegrationState};

/// Cache lifetime: long enough to absorb auto-refreshes, short enough that
/// a newly unpacked file appears promptly.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Shared state for the eval routes.
pub struct EvalState {
    /// Where the kit lives.
    pub paths: EvalPaths,
    /// Cached catalog with its computation time.
    catalog_cache: Mutex<Option<(Instant, Arc<Vec<Resource>>)>>,
    /// Cached catalog summary.
    summary_cache: Mutex<Option<(Instant, Arc<CatalogSummary>)>>,
    /// Cached bank indexes keyed by file name, with the indexed path.
    bank_cache: Mutex<HashMap<String, BankCacheEntry>>,
    /// Cached runs list.
    runs_cache: Mutex<Option<(Instant, Arc<Vec<EvalRun>>)>>,
    /// Cached report cards.
    reports_cache: Mutex<Option<(Instant, Arc<Vec<ReportCard>>)>>,
}

/// One cached bank index: when it was built and from which path.
#[derive(Clone)]
struct BankCacheEntry {
    at: Instant,
    index: Arc<BankIndex>,
    path: PathBuf,
}

impl EvalState {
    /// State rooted at the default location (`~/.loop/eval`).
    pub fn new() -> Self {
        Self {
            paths: EvalPaths::resolve(),
            catalog_cache: Mutex::new(None),
            summary_cache: Mutex::new(None),
            bank_cache: Mutex::new(HashMap::new()),
            runs_cache: Mutex::new(None),
            reports_cache: Mutex::new(None),
        }
    }

    /// State rooted at an explicit path (tests).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            paths: EvalPaths::at(root),
            catalog_cache: Mutex::new(None),
            summary_cache: Mutex::new(None),
            bank_cache: Mutex::new(HashMap::new()),
            runs_cache: Mutex::new(None),
            reports_cache: Mutex::new(None),
        }
    }

    fn cached<T, F>(&self, slot: &mut Option<(Instant, Arc<T>)>, compute: F) -> Arc<T>
    where
        T: Clone,
        F: FnOnce() -> T,
    {
        if let Some((at, value)) = slot {
            if at.elapsed() < CACHE_TTL {
                return Arc::clone(value);
            }
        }
        let value = Arc::new(compute());
        *slot = Some((Instant::now(), Arc::clone(&value)));
        value
    }

    /// The resource catalog (cached).
    fn resources(&self) -> Arc<Vec<Resource>> {
        let mut slot = self.catalog_cache.lock().unwrap();
        let paths = self.paths.clone();
        self.cached(&mut slot, move || catalog::catalog(&paths))
    }

    /// The catalog summary (cached).
    fn summary(&self) -> Arc<CatalogSummary> {
        // Refresh together with the catalog so counts stay consistent.
        let res = self.resources();
        let mut slot = self.summary_cache.lock().unwrap();
        let paths = self.paths.clone();
        let _ = paths;
        self.cached(&mut slot, move || catalog::summarize(&res))
    }

    /// One bank's index, rebuilt when the file changed on disk or the cache
    /// expired. The mtime check means editing a bank invalidates instantly.
    fn bank_index(&self, name: &str) -> Option<Arc<BankIndex>> {
        let path = self
            .resources()
            .iter()
            .find(|r| r.name == name && r.workspace == Workspace::Banks)?
            .abs_path
            .clone();
        let path = PathBuf::from(path);
        let mtime = file_mtime(&path);
        {
            let cache = self.bank_cache.lock().unwrap();
            if let Some(entry) = cache.get(name) {
                if entry.at.elapsed() < CACHE_TTL
                    && entry.path == path
                    && file_mtime(&entry.path) == mtime
                {
                    return Some(Arc::clone(&entry.index));
                }
            }
        }
        // Indexing a 62 MB bank costs ~0.4 s in release (a few seconds in
        // debug), so it runs on the blocking pool and never on the async
        // runtime. The result is cached; only the first request pays.
        let idx = Arc::new(tokio::task::block_in_place(|| banks::index_file(&path)));
        let mut cache = self.bank_cache.lock().unwrap();
        cache.insert(
            name.to_string(),
            BankCacheEntry {
                at: Instant::now(),
                index: Arc::clone(&idx),
                path,
            },
        );
        Some(idx)
    }

    /// Discovered runs (cached).
    fn runs(&self) -> Arc<Vec<EvalRun>> {
        let mut slot = self.runs_cache.lock().unwrap();
        let paths = self.paths.clone();
        self.cached(&mut slot, move || runs::load_runs(&paths))
    }

    /// Discovered report cards (cached). Scanning HTML files is bounded and
    /// cheap relative to the 60 s cache, so an imported bundle appears on
    /// the next refresh.
    fn reports(&self) -> Arc<Vec<ReportCard>> {
        let mut slot = self.reports_cache.lock().unwrap();
        let paths = self.paths.clone();
        self.cached(&mut slot, move || reports::discover(&paths))
    }

    /// Overall integration state for badges.
    fn integration(&self) -> IntegrationState {
        if !self.paths.kit_present() {
            return IntegrationState::NotConfigured;
        }
        let has_any = !self.resources().is_empty();
        if has_any {
            IntegrationState::Connected
        } else {
            IntegrationState::Idle
        }
    }
}

impl Default for EvalState {
    fn default() -> Self {
        Self::new()
    }
}

/// File mtime in ms (0 when unknown).
fn file_mtime(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// JSON error body.
fn err(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// `GET /api/eval/overview` — portal card summaries plus the six workspace
/// counts. One request feeds every Welcome-page badge.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OverviewBody {
    integration: &'static str,
    integration_label: &'static str,
    root: String,
    kit_present: bool,
    files: u64,
    bytes: u64,
    banks: BanksOverview,
    datasets: DatasetsOverview,
    benchmarks: BenchmarksOverview,
    evaluations: EvaluationsOverview,
    reports: ReportsOverview,
    tools: ToolsOverview,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BanksOverview {
    files: u64,
    questions: u64,
    largest: Option<String>,
    suites: Vec<crate::eval::CountBucket>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetsOverview {
    files: u64,
    rows: u64,
    formats: Vec<crate::eval::CountBucket>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarksOverview {
    presets: u64,
    settings: u64,
    comparison_models: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationsOverview {
    runs: u64,
    settings_evaluated: u64,
    best: Option<BestScore>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BestScore {
    run_id: String,
    setting: String,
    label: Option<String>,
    acc: f64,
    n: Option<u64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportsOverview {
    files: u64,
    runs_with_summary: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolsOverview {
    scripts: u64,
    docs: u64,
    tests: u64,
}

async fn overview(State(state): State<Arc<EvalState>>) -> Response {
    let s = state.summary();
    let resources = state.resources();
    let runs = state.runs();

    // Bank totals come from the bank indexes (honest row counts), not from
    // line counting, so large banks are counted once and cached.
    let mut questions: u64 = 0;
    let mut largest: Option<(String, u64)> = None;
    let mut suite_totals: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    for r in resources.iter().filter(|r| r.workspace == Workspace::Banks) {
        if let Some(idx) = state.bank_index(&r.name) {
            questions += idx.rows;
            for b in &idx.facets.suites {
                *suite_totals.entry(b.key.clone()).or_insert(0) += b.count;
            }
            let is_larger = largest
                .as_ref()
                .map(|(_, n)| idx.bytes > *n)
                .unwrap_or(true);
            if is_larger {
                largest = Some((r.name.clone(), idx.bytes));
            }
        }
    }

    // Dataset formats across all row-files.
    let mut formats: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut dataset_rows: u64 = 0;
    for r in resources
        .iter()
        .filter(|r| r.workspace == Workspace::Datasets)
    {
        *formats.entry(r.kind.label().to_string()).or_insert(0) += 1;
        dataset_rows += r.rows.unwrap_or(0);
    }

    // Benchmark presets: the named --input values the kit documents, which
    // we derive from the bank files that actually exist.
    let presets = resources
        .iter()
        .filter(|r| r.workspace == Workspace::Banks && r.dir.starts_with("banks"))
        .count() as u64;
    // Settings swept: distinct setting names across runs.
    let mut settings_seen: Vec<String> = Vec::new();
    for run in runs.iter() {
        for s in &run.settings {
            if !settings_seen.contains(&s.name) {
                settings_seen.push(s.name.clone());
            }
        }
    }
    // Comparison models: distinct keys across runs.
    let mut models_seen: Vec<String> = Vec::new();
    for run in runs.iter() {
        for m in &run.compare {
            if !models_seen.contains(m) {
                models_seen.push(m.clone());
            }
        }
    }

    // Best measured accuracy across runs (model under test only).
    let best = runs
        .iter()
        .flat_map(|r| r.reports.iter().map(move |rep| (r, rep)))
        .find_map(|(run, rep)| {
            rep.overall
                .iter()
                .find(|s| s.model == "eka")
                .and_then(|s| s.acc)
                .map(|acc| BestScore {
                    run_id: run.id.clone(),
                    setting: rep.name.clone(),
                    label: run.label.clone(),
                    acc,
                    n: rep.n,
                })
        });

    let tools_scripts = s.scripts;
    let tools_docs = s.docs;
    let tools_tests = resources
        .iter()
        .filter(|r| r.rel_path.starts_with("tests/"))
        .count() as u64;

    Json(OverviewBody {
        integration: state.integration().wire(),
        integration_label: state.integration().label(),
        root: state.paths.root.display().to_string(),
        kit_present: state.paths.kit_present(),
        files: s.files,
        bytes: s.bytes,
        banks: BanksOverview {
            files: s.banks,
            questions,
            largest: largest.map(|(n, _)| n),
            suites: crate::eval::top_counts(&suite_totals, 6),
        },
        datasets: DatasetsOverview {
            files: s.datasets,
            rows: dataset_rows,
            formats: crate::eval::top_counts(&formats, 8),
        },
        benchmarks: BenchmarksOverview {
            presets,
            settings: settings_seen.len() as u64,
            comparison_models: models_seen.len() as u64,
        },
        evaluations: EvaluationsOverview {
            runs: s.runs,
            settings_evaluated: runs
                .iter()
                .map(|r| r.reports.iter().filter(|x| x.has_report).count() as u64)
                .sum(),
            best,
        },
        reports: ReportsOverview {
            files: s.reports,
            runs_with_summary: runs.iter().filter(|r| r.summary.is_some()).count() as u64,
        },
        tools: ToolsOverview {
            scripts: tools_scripts,
            docs: tools_docs,
            tests: tools_tests,
        },
    })
    .into_response()
}

/// `GET /api/eval/banks` — one entry per question file with real counts.
async fn banks_list(State(state): State<Arc<EvalState>>) -> Response {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BankEntry {
        name: String,
        rel_path: String,
        bytes: u64,
        rows: u64,
        malformed: u64,
        suites: Vec<crate::eval::CountBucket>,
        groups: Vec<crate::eval::CountBucket>,
        difficulties: Vec<crate::eval::CountBucket>,
        languages: Vec<crate::eval::CountBucket>,
        question_types: Vec<crate::eval::CountBucket>,
        subjects: u64,
        topics: u64,
        with_reference: u64,
        with_options: u64,
    }

    let resources = state.resources();
    let mut items = Vec::new();
    for r in resources.iter().filter(|r| r.workspace == Workspace::Banks) {
        let idx = match state.bank_index(&r.name) {
            Some(i) => i,
            None => continue,
        };
        items.push(BankEntry {
            name: r.name.clone(),
            rel_path: r.rel_path.clone(),
            bytes: r.bytes,
            rows: idx.rows,
            malformed: idx.malformed,
            suites: idx.facets.suites.clone(),
            groups: idx.facets.groups.clone(),
            difficulties: idx.facets.difficulties.clone(),
            languages: idx.facets.languages.clone(),
            question_types: idx.facets.question_types.clone(),
            subjects: idx.distinct_subjects,
            topics: idx.distinct_topics,
            with_reference: idx.facets.with_reference,
            with_options: idx.facets.with_options,
        });
    }
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "root": state.paths.root.display().to_string(),
        "items": items,
    }))
    .into_response()
}

/// Query params for a bank page.
#[derive(Debug, Default, Deserialize)]
struct BankPageParams {
    q: Option<String>,
    suite: Option<String>,
    group: Option<String>,
    subject: Option<String>,
    difficulty: Option<String>,
    language: Option<String>,
    question_type: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

/// `GET /api/eval/banks/:name/page` — one page of questions. Only the
/// selected rows' bodies are read from disk.
async fn bank_page(
    State(state): State<Arc<EvalState>>,
    AxumPath(name): AxumPath<String>,
    Query(params): Query<BankPageParams>,
) -> Response {
    // The name must match a catalogued bank file exactly.
    let path = state
        .resources()
        .iter()
        .find(|r| r.name == name && r.workspace == Workspace::Banks)
        .map(|r| PathBuf::from(r.abs_path.clone()));
    let Some(path) = path else {
        return err(
            StatusCode::NOT_FOUND,
            format!("no question bank named {name:?}"),
        );
    };
    let idx = match state.bank_index(&name) {
        Some(i) => i,
        None => return err(StatusCode::NOT_FOUND, format!("could not index {name:?}")),
    };
    let query = BankQuery {
        q: params.q.unwrap_or_default(),
        suite: normalize_filter(params.suite),
        group: normalize_filter(params.group),
        subject: normalize_filter(params.subject),
        difficulty: normalize_filter(params.difficulty),
        language: normalize_filter(params.language),
        question_type: normalize_filter(params.question_type),
        page: params.page.unwrap_or(1),
        per_page: params.per_page.unwrap_or(25),
    };
    let result = tokio::task::spawn_blocking(move || banks::page(&idx, &path, &query)).await;
    match result {
        Ok(page) => Json(page).into_response(),
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// Treat empty strings and `all` as "no filter".
fn normalize_filter(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "all")
}

/// `GET /api/eval/datasets` — row-file datasets with sizes and row counts.
async fn datasets_list(State(state): State<Arc<EvalState>>) -> Response {
    let resources = state.resources();
    let items: Vec<&Resource> = resources
        .iter()
        .filter(|r| r.workspace == Workspace::Datasets)
        .collect();
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "items": items,
    }))
    .into_response()
}

/// `GET /api/eval/files?workspace=banks` — resources filtered by workspace.
#[derive(Debug, Default, Deserialize)]
struct FilesParams {
    workspace: Option<String>,
    kind: Option<String>,
}

async fn files_list(
    State(state): State<Arc<EvalState>>,
    Query(params): Query<FilesParams>,
) -> Response {
    let ws = params.workspace.as_deref().and_then(parse_workspace);
    let kind = params.kind.as_deref().map(str::to_lowercase);
    let resources = state.resources();
    let items: Vec<&Resource> = resources
        .iter()
        .filter(|r| ws.map(|w| r.workspace == w).unwrap_or(true))
        .filter(|r| {
            kind.as_deref()
                .map(|k| r.kind.label().to_lowercase() == k)
                .unwrap_or(true)
        })
        .collect();
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "root": state.paths.root.display().to_string(),
        "kitPresent": state.paths.kit_present(),
        "items": items,
    }))
    .into_response()
}

/// Parse a workspace wire label.
fn parse_workspace(s: &str) -> Option<Workspace> {
    match s.trim().to_lowercase().as_str() {
        "banks" => Some(Workspace::Banks),
        "datasets" => Some(Workspace::Datasets),
        "benchmarks" => Some(Workspace::Benchmarks),
        "evaluations" => Some(Workspace::Evaluations),
        "reports" => Some(Workspace::Reports),
        "tools" => Some(Workspace::Tools),
        _ => None,
    }
}

/// `GET /api/eval/file/:id` — a bounded preview of any catalogued resource.
/// `GET /api/eval/file/{id}?render=html` — serve an HTML resource as a
/// real webpage (`text/html; charset=utf-8`, inline). This is how report
/// pages open rendered instead of as source: the bytes are served verbatim
/// (no escaping) through the same session-guarded, kit-confined route as
/// every other eval artifact.
#[derive(Debug, Default, Deserialize)]
struct FilePreviewParams {
    /// When `html`, an HTML resource is served as a webpage rather than a
    /// JSON preview payload.
    render: Option<String>,
}

async fn file_preview(
    State(state): State<Arc<EvalState>>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<FilePreviewParams>,
) -> Response {
    use axum::http::header;

    let Some(res) = files::resolve(&state.paths, &id) else {
        return err(StatusCode::NOT_FOUND, format!("no resource with id {id:?}"));
    };

    // Rendered-webpage mode: only for HTML resources, and only when the
    // caller explicitly asked for it. The path re-checks against the kit
    // root exactly like the preview does.
    if params.render.as_deref() == Some("html") {
        if res.kind != catalog::ResourceKind::Html {
            return err(
                StatusCode::BAD_REQUEST,
                "only HTML resources can be rendered as a webpage",
            );
        }
        let path = PathBuf::from(&res.abs_path);
        if !path.starts_with(state.paths.kit()) {
            return err(
                StatusCode::FORBIDDEN,
                "resource is outside the evaluation kit",
            );
        }
        let filename = res.name.clone();
        return match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let mut resp = (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    bytes,
                )
                    .into_response();
                if let Ok(v) =
                    header::HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
                {
                    resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
                }
                // Reports are self-contained pages; never let the browser
                // or intermediaries cache a stale copy across edits.
                resp.headers_mut().insert(
                    header::CACHE_CONTROL,
                    header::HeaderValue::from_static("no-cache"),
                );
                resp
            }
            Err(e) => err(StatusCode::NOT_FOUND, format!("{}: {e}", res.rel_path)),
        };
    }

    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || files::preview(&paths, &res)).await;
    match result {
        Ok(Ok(preview)) => Json(preview).into_response(),
        Ok(Err(reason)) => err(StatusCode::UNPROCESSABLE_ENTITY, reason),
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/eval/parquet/:id` — Parquet footer metadata (schema, rows).
async fn parquet_meta(
    State(state): State<Arc<EvalState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(res) = files::resolve(&state.paths, &id) else {
        return err(StatusCode::NOT_FOUND, format!("no resource with id {id:?}"));
    };
    let path = PathBuf::from(&res.abs_path);
    if !path.starts_with(state.paths.kit()) {
        return err(
            StatusCode::FORBIDDEN,
            "resource is outside the evaluation kit",
        );
    }
    let result = tokio::task::spawn_blocking(move || files::parquet_metadata(&path, &res)).await;
    match result {
        Ok(Ok(meta)) => Json(meta).into_response(),
        Ok(Err(reason)) => err(StatusCode::UNPROCESSABLE_ENTITY, reason),
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/eval/runs` — every discovered run with its settings/reports.
async fn runs_list(State(state): State<Arc<EvalState>>) -> Response {
    let runs = state.runs();
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "items": runs.as_slice(),
    }))
    .into_response()
}

/// `GET /api/eval/runs/:id` — one run in full.
async fn run_detail(
    State(state): State<Arc<EvalState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let runs = state.runs();
    let Some(run) = runs.iter().find(|r| r.id == id) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("no evaluation run named {id:?}"),
        );
    };
    Json(run).into_response()
}

/// `GET /api/eval/comparison` — cross-run, cross-model score table.
async fn comparison(State(state): State<Arc<EvalState>>) -> Response {
    let runs = state.runs();
    let rows: Vec<ComparisonRow> = runs::comparisons(&runs);
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "rows": rows,
    }))
    .into_response()
}

/// `GET /api/eval/docs` — the documentation set.
async fn docs_list(State(state): State<Arc<EvalState>>) -> Response {
    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || docs::list(&paths)).await;
    match result {
        Ok(entries) => {
            let entries: Vec<DocEntry> = entries;
            Json(serde_json::json!({
                "integration": state.integration().wire(),
                "items": entries,
            }))
            .into_response()
        }
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/eval/docs/:id` — one document's Markdown source.
async fn doc_read(State(state): State<Arc<EvalState>>, AxumPath(id): AxumPath<String>) -> Response {
    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || docs::read(&paths, &id)).await;
    match result {
        Ok(Ok(doc)) => Json(doc).into_response(),
        Ok(Err(reason)) => err(StatusCode::NOT_FOUND, reason),
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `GET /api/eval/benchmarks` — benchmark definitions derived from what the
/// kit actually contains: bank presets (the `--input` values), decoding
/// settings documented and used, and the comparison model registry.
async fn benchmarks(State(state): State<Arc<EvalState>>) -> Response {
    let resources = state.resources();
    let runs = state.runs();

    // Presets: each bank file under banks/ is a selectable input. Row
    // counts come from the bank index (honest, cached).
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Preset {
        name: String,
        file: String,
        questions: u64,
        bytes: u64,
        suites: Vec<crate::eval::CountBucket>,
        groups: u64,
    }
    let mut presets = Vec::new();
    for r in resources
        .iter()
        .filter(|r| r.workspace == Workspace::Banks && r.dir == "banks")
    {
        let idx = state.bank_index(&r.name);
        let (questions, bytes, suites, groups) = match &idx {
            Some(i) => (
                i.rows,
                r.bytes,
                i.facets.suites.clone(),
                i.facets.groups.len() as u64,
            ),
            None => (0, r.bytes, Vec::new(), 0),
        };
        presets.push(Preset {
            name: preset_name(&r.name),
            file: r.name.clone(),
            questions,
            bytes,
            suites,
            groups,
        });
    }

    // Settings actually configured across runs, with usage counts.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SettingUse {
        name: String,
        temperature: Option<f64>,
        effort: Option<String>,
        runs: u64,
    }
    let mut setting_uses: Vec<SettingUse> = Vec::new();
    for run in runs.iter() {
        for s in &run.settings {
            if let Some(existing) = setting_uses.iter_mut().find(|u| u.name == s.name) {
                existing.runs += 1;
            } else {
                setting_uses.push(SettingUse {
                    name: s.name.clone(),
                    temperature: s.temperature,
                    effort: s.effort.clone(),
                    runs: 1,
                });
            }
        }
    }

    // Comparison models seen in run configs (the registry itself lives in
    // the pipeline source; we report what runs actually used).
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ModelUse {
        key: String,
        runs: u64,
    }
    let mut model_uses: Vec<ModelUse> = Vec::new();
    for run in runs.iter() {
        for m in &run.compare {
            if let Some(existing) = model_uses.iter_mut().find(|u| u.key == *m) {
                existing.runs += 1;
            } else {
                model_uses.push(ModelUse {
                    key: m.clone(),
                    runs: 1,
                });
            }
        }
    }

    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "presets": presets,
        "settings": setting_uses,
        "comparisonModels": model_uses,
    }))
    .into_response()
}

/// Map a bank file name to the `--input` preset name the kit documents.
fn preset_name(file: &str) -> String {
    let stem = file.strip_suffix(".jsonl").unwrap_or(file);
    stem.replace('_', "-")
}

/// `GET /api/eval/reports` — every report card, newest first.
async fn reports_list(State(state): State<Arc<EvalState>>) -> Response {
    let cards = state.reports();
    Json(serde_json::json!({
        "integration": state.integration().wire(),
        "root": state.paths.root.display().to_string(),
        "items": cards.as_slice(),
    }))
    .into_response()
}

/// `GET /api/eval/reports/{id}` — one card with its parsed detail.
async fn report_detail(
    State(state): State<Arc<EvalState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let cards = state.reports();
    let paths = state.paths.clone();
    let id_cl = id.clone();
    let detail = tokio::task::spawn_blocking(move || reports::detail(&paths, &cards, &id_cl)).await;
    match detail {
        Ok(Some(d)) => Json(d).into_response(),
        Ok(None) => err(
            StatusCode::NOT_FOUND,
            format!("no evaluation report with id {id:?}"),
        ),
        Err(join) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("worker task failed: {join}"),
        ),
    }
}

/// `?as=` values for the artifact route.
#[derive(Debug, Default, Deserialize)]
struct ReportFileParams {
    r#as: Option<String>,
}

/// `GET /api/eval/reports/{id}/file?as=html|json|csv|summary` — serve one
/// artifact of a report card. HTML is served verbatim (the original
/// report), JSON is the embedded/real results object, CSV streams the
/// graded rows. Downloads carry a friendly filename.
async fn report_file(
    State(state): State<Arc<EvalState>>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<ReportFileParams>,
) -> Response {
    use axum::http::header;

    let cards = state.reports();
    let Some(card) = cards.iter().find(|c| c.id == id) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("no evaluation report with id {id:?}"),
        );
    };
    let want = params.r#as.unwrap_or_default();
    let kit = state.paths.kit();

    // Resolve the requested artifact to (bytes, filename, content type,
    // inline-vs-download).
    let serve =
        |bytes: Vec<u8>, filename: String, ctype: &'static str, download: bool| -> Response {
            let mut resp = (StatusCode::OK, [(header::CONTENT_TYPE, ctype)], bytes).into_response();
            if download {
                let cd = format!("attachment; filename=\"{filename}\"");
                if let Ok(v) = header::HeaderValue::from_str(&cd) {
                    resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
                }
            } else {
                let cd = format!("inline; filename=\"{filename}\"");
                if let Ok(v) = header::HeaderValue::from_str(&cd) {
                    resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
                }
            }
            resp
        };

    match want.as_str() {
        "html" => {
            let Some(res_id) = &card.artifacts.html else {
                return err(StatusCode::NOT_FOUND, "this report has no HTML artifact");
            };
            let path = kit.join(res_id.replace("::", "/"));
            match tokio::fs::read(&path).await {
                Ok(bytes) => serve(
                    bytes,
                    card.rel_path
                        .rsplit('/')
                        .next()
                        .unwrap_or("report.html")
                        .to_string(),
                    "text/html; charset=utf-8",
                    false,
                ),
                Err(e) => err(StatusCode::NOT_FOUND, format!("{}: {e}", card.rel_path)),
            }
        }
        "json" => {
            // Real report.json when present, else the JSON embedded in the
            // imported HTML page.
            if let Some(res_id) = &card.artifacts.json {
                if !res_id.ends_with("::data") {
                    let path = kit.join(res_id.replace("::", "/"));
                    if let Ok(bytes) = tokio::fs::read(&path).await {
                        return serve(bytes, "report.json".into(), "application/json", true);
                    }
                }
            }
            // Embedded block.
            let html_path = kit.join(card.rel_path.replace("::", "/"));
            let read = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(html_path)
                    .ok()
                    .and_then(|h| reports::extract_report_data_pub(&h).map(str::to_owned))
            })
            .await;
            match read {
                Ok(Some(data)) => serve(
                    data.into_bytes(),
                    "report-data.json".into(),
                    "application/json",
                    true,
                ),
                _ => err(StatusCode::NOT_FOUND, "no JSON results for this report"),
            }
        }
        "csv" => {
            let Some(res_id) = &card.artifacts.csv else {
                return err(StatusCode::NOT_FOUND, "this report has no CSV artifact");
            };
            let path = kit.join(res_id.replace("::", "/"));
            match tokio::fs::read(&path).await {
                Ok(bytes) => serve(
                    bytes,
                    "graded_flat.csv".into(),
                    "text/csv; charset=utf-8",
                    true,
                ),
                Err(e) => err(StatusCode::NOT_FOUND, format!("graded_flat.csv: {e}")),
            }
        }
        "summary" => {
            let Some(res_id) = card
                .artifacts
                .summary_html
                .as_ref()
                .or(card.artifacts.html.as_ref())
            else {
                return err(StatusCode::NOT_FOUND, "this report has no summary page");
            };
            let path = kit.join(res_id.replace("::", "/"));
            match tokio::fs::read(&path).await {
                Ok(bytes) => serve(
                    bytes,
                    card.rel_path
                        .rsplit('/')
                        .next()
                        .unwrap_or("summary.html")
                        .to_string(),
                    "text/html; charset=utf-8",
                    false,
                ),
                Err(e) => err(StatusCode::NOT_FOUND, format!("{}: {e}", card.rel_path)),
            }
        }
        other => err(
            StatusCode::BAD_REQUEST,
            format!("unknown ?as={other:?} (expected html, json, csv or summary)"),
        ),
    }
}

/// Build the eval router.
pub fn eval_router(state: Arc<EvalState>) -> Router {
    Router::new()
        .route("/api/eval/overview", get(overview))
        .route("/api/eval/banks", get(banks_list))
        .route("/api/eval/banks/{name}/page", get(bank_page))
        .route("/api/eval/datasets", get(datasets_list))
        .route("/api/eval/files", get(files_list))
        .route("/api/eval/file/{id}", get(file_preview))
        .route("/api/eval/parquet/{id}", get(parquet_meta))
        .route("/api/eval/runs", get(runs_list))
        .route("/api/eval/runs/{id}", get(run_detail))
        .route("/api/eval/comparison", get(comparison))
        .route("/api/eval/benchmarks", get(benchmarks))
        .route("/api/eval/docs", get(docs_list))
        .route("/api/eval/docs/{id}", get(doc_read))
        .route("/api/eval/reports", get(reports_list))
        .route("/api/eval/reports/{id}", get(report_detail))
        .route("/api/eval/reports/{id}/file", get(report_file))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn router_for(root: &std::path::Path) -> Router {
        eval_router(Arc::new(EvalState::at(root)))
    }

    async fn get_json(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
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

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let kit = dir.path().join("internal_eval");
        for sub in ["banks", "examples", "pipeline", "slurm", "config", "tests"] {
            std::fs::create_dir_all(kit.join(sub)).unwrap();
        }
        let rows: Vec<String> = (0..40)
            .map(|i| {
                format!(
                    r#"{{"id":"G|{i}","suite":"school","group":"Class {i}","subject":"Maths","topic":"T","difficulty":"L1","language":"","question_type":"mcq","has_options":false,"reference":"","question":"question {i}"}}"#
                )
            })
            .collect();
        std::fs::write(kit.join("banks/qbank_sample.jsonl"), rows.join("\n") + "\n").unwrap();
        std::fs::write(kit.join("README.md"), "# internal_eval\n\nhello\n").unwrap();
        std::fs::write(kit.join("pipeline/judge.py"), "# stage\n").unwrap();
        std::fs::write(kit.join("tests/smoke.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(kit.join("config/exclude_nodes.txt"), "node1\n").unwrap();
        // A run with a report.
        let run = kit.join("runs/EKA_x");
        std::fs::create_dir_all(run.join("settings/med06")).unwrap();
        std::fs::write(
            run.join("config.json"),
            r#"{"urls":["http://n:8004"],"model":"eka-1","label":"EKA x","input":"qbank-sample",
               "settings":[{"name":"med06","temperature":0.6,"effort":"medium"}],
               "compare":["gpt-oss-120b"]}"#,
        )
        .unwrap();
        std::fs::write(
            run.join("settings/med06/report.json"),
            r#"{"n":40,"overall":{"eka":{"n":40,"acc":61.2},"gpt-oss-120b":{"n":40,"acc":90.9}}}"#,
        )
        .unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_kit_is_honest_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let router = router_for(dir.path());
        let (status, body) = get_json(&router, "/api/eval/overview").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["integration"], "not_configured");
        assert_eq!(body["files"], 0);
        let (_, banks) = get_json(&router, "/api/eval/banks").await;
        assert_eq!(banks["items"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overview_reports_real_counts() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (status, body) = get_json(&router, "/api/eval/overview").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["integration"], "connected");
        assert_eq!(
            body["banks"]["questions"], 40,
            "row count comes from the index"
        );
        assert_eq!(body["evaluations"]["runs"], 1);
        assert_eq!(body["evaluations"]["best"]["acc"], 61.2);
        assert!(body["tools"]["scripts"].as_u64().unwrap() >= 2);
        assert_eq!(body["tools"]["docs"], 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bank_pages_are_bounded_and_filterable() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (status, page) = get_json(
            &router,
            "/api/eval/banks/qbank_sample.jsonl/page?per_page=10&page=2",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 40);
        assert_eq!(page["pages"], 4);
        assert_eq!(page["items"].as_array().map(Vec::len), Some(10));
        assert_eq!(page["items"][0]["id"], "G|10");

        // Search narrows honestly.
        let (_, hit) = get_json(
            &router,
            "/api/eval/banks/qbank_sample.jsonl/page?q=question%207",
        )
        .await;
        assert_eq!(hit["total"], 1);

        // Unknown bank → 404.
        let (status, _) = get_json(&router, "/api/eval/banks/nope.jsonl/page").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_previews_serve_and_refuse_correctly() {
        let dir = fixture();
        let router = router_for(dir.path());
        // Text preview through the catalog id.
        let (status, pv) = get_json(&router, "/api/eval/file/pipeline::judge.py").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(pv["mode"], "text");
        // Traversal ids never resolve.
        let (status, _) = get_json(&router, "/api/eval/file/..::etc::passwd").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Unknown id → 404.
        let (status, _) = get_json(&router, "/api/eval/file/missing.py").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_and_comparison_flow_through() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (_, runs) = get_json(&router, "/api/eval/runs").await;
        assert_eq!(runs["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(runs["items"][0]["reports"][0]["overall"][0]["acc"], 61.2);

        let (_, one) = get_json(&router, "/api/eval/runs/EKA_x").await;
        assert_eq!(one["id"], "EKA_x");

        let (_, cmp) = get_json(&router, "/api/eval/comparison").await;
        assert_eq!(cmp["rows"].as_array().map(Vec::len), Some(2));

        let (status, _) = get_json(&router, "/api/eval/runs/none").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn benchmarks_derive_from_real_files() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (_, b) = get_json(&router, "/api/eval/benchmarks").await;
        assert_eq!(b["presets"][0]["questions"], 40);
        assert_eq!(b["presets"][0]["name"], "qbank-sample");
        assert_eq!(b["settings"][0]["name"], "med06");
        assert_eq!(b["comparisonModels"][0]["key"], "gpt-oss-120b");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn docs_list_and_read() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (_, docs) = get_json(&router, "/api/eval/docs").await;
        assert_eq!(docs["items"].as_array().map(Vec::len), Some(1));
        let (_, doc) = get_json(&router, "/api/eval/docs/README.md").await;
        assert!(doc["markdown"].as_str().unwrap().contains("hello"));
        let (status, _) = get_json(&router, "/api/eval/docs/nope.md").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn files_filter_by_workspace_and_kind() {
        let dir = fixture();
        let router = router_for(dir.path());
        let (_, banks) = get_json(&router, "/api/eval/files?workspace=banks").await;
        assert!(banks["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["relPath"].as_str().unwrap().starts_with("banks/")
                || r["relPath"].as_str().unwrap().starts_with("examples/")));
        let (_, md) = get_json(&router, "/api/eval/files?kind=Markdown").await;
        assert_eq!(md["items"].as_array().map(Vec::len), Some(1));
    }
}
