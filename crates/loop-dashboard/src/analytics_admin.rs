//! Admin-only advanced agent observability analytics (Phase 3).
//!
//! Everything here is computed from **persisted** trace data only — the
//! `traces`, `spans` and `trace_events` tables written by Phase 1/2. No
//! metric is invented; when the underlying column is NULL the metric is
//! reported as absent (`null` on the wire, "Not reported" in the UI), never
//! as zero.
//!
//! Authorization: every handler resolves [`crate::console::CallerAuthority`]
//! first and refuses non-administrators with 403/401 before touching the
//! store. The role is re-read from the auth database on every request.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::Connection;
use serde::Deserialize;

use crate::console::{caller_authority, CallerAuthority, ConsoleState};

/// JSON error body.
fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// Refuse non-admin callers. Returns `Err(response)` on refusal.
async fn require_admin(state: &ConsoleState, headers: &HeaderMap) -> Result<(), Response> {
    match caller_authority(state, headers).await {
        // `--no-auth` standalone mode has no administrators: the local
        // owner keeps access, exactly like every other console surface.
        CallerAuthority::Owner | CallerAuthority::Admin => Ok(()),
        CallerAuthority::NotAdmin => Err(err(StatusCode::FORBIDDEN, "administrator access required")),
    }
}

// ---------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------

/// Shared filter parameters for every analytics endpoint.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnalyticsQuery {
    /// Range start (unix ms, inclusive).
    pub from_ms: Option<i64>,
    /// Range end (unix ms, inclusive).
    pub to_ms: Option<i64>,
    /// Exact model filter.
    pub model: Option<String>,
    /// Exact provider filter.
    pub provider: Option<String>,
    /// Comma-separated statuses.
    pub status: Option<String>,
    /// Operation filter (`cli.session`, `dashboard.chat`, …).
    pub operation: Option<String>,
}

impl AnalyticsQuery {
    /// SQL fragment + params filtering `traces` by the shared dimensions.
    /// `alias` prefixes the columns when the query joins (`t.` etc.).
    fn traces_where(&self, alias: &str) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let a = |col: &str| format!("{alias}{col}");
        if let Some(from) = self.from_ms {
            clauses.push(format!("{} >= ?", a("started_at_ms")));
            args.push(Box::new(from));
        }
        if let Some(to) = self.to_ms {
            clauses.push(format!("{} <= ?", a("started_at_ms")));
            args.push(Box::new(to));
        }
        if let Some(model) = self.model.as_deref().filter(|m| !m.trim().is_empty()) {
            clauses.push(format!("{} = ?", a("model")));
            args.push(Box::new(model.trim().to_string()));
        }
        if let Some(provider) = self.provider.as_deref().filter(|p| !p.trim().is_empty()) {
            clauses.push(format!("{} = ?", a("provider")));
            args.push(Box::new(provider.trim().to_string()));
        }
        if let Some(op) = self.operation.as_deref().filter(|o| !o.trim().is_empty()) {
            clauses.push(format!("{} = ?", a("operation")));
            args.push(Box::new(op.trim().to_string()));
        }
        if let Some(statuses) = parse_statuses(self.status.as_deref()) {
            let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            clauses.push(format!("{} IN ({placeholders})", a("status")));
            for s in statuses {
                args.push(Box::new(s));
            }
        }
        (
            if clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", clauses.join(" AND "))
            },
            args,
        )
    }
}

/// Parse a comma-separated status list into wire strings.
fn parse_statuses(raw: Option<&str>) -> Option<Vec<String>> {
    let raw = raw?.trim();
    if raw.is_empty() || raw == "all" {
        return None;
    }
    let out: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------
// Metric helpers
// ---------------------------------------------------------------------

/// Nearest-rank percentile over an unsorted list. Never interpolates.
fn percentile_of(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((p / 100.0) * values.len() as f64).ceil() as usize;
    let idx = idx.clamp(1, values.len()) - 1;
    Some(values[idx])
}

/// Mean of a slice; `None` when empty (never 0).
fn mean_of(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// A latency distribution reported honestly: counts of *measured* samples
/// only, percentiles suppressed when the sample is too small to support
/// them (P99 needs ≥ 20 samples to be meaningful).
struct Dist {
    count: usize,
    mean: Option<f64>,
    p50: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
}

impl Dist {
    fn build(mut samples: Vec<f64>) -> Self {
        let count = samples.len();
        let min = samples.iter().cloned().reduce(f64::min);
        let max = samples.iter().cloned().reduce(f64::max);
        let p50 = percentile_of(&mut samples, 50.0);
        let p95 = percentile_of(&mut samples, 95.0);
        // P99 only when there are enough samples to support the rank.
        let p99 = (count >= 20)
            .then(|| percentile_of(&mut samples, 99.0))
            .flatten();
        Self { count, mean: mean_of(&samples), p50, p95, p99, min, max }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "count": self.count,
            "meanMs": self.mean,
            "p50Ms": self.p50,
            "p95Ms": self.p95,
            "p99Ms": self.p99,
            "minMs": self.min,
            "maxMs": self.max,
        })
    }
}

/// Run `f` with the store connection on the blocking pool.
async fn with_conn<T, F>(state: &ConsoleState, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
{
    let store = Arc::clone(&state.store);
    let joined = tokio::task::spawn_blocking(move || {
        // `with_conn` requires the closure to return Result<_, PersistError>;
        // rusqlite::Error converts into it via From.
        store.with_conn(|c| f(c).map_err(crate::persist::PersistError::from))
    })
    .await;
    match joined {
        Ok(v) => v.map_err(|e| format!("storage error: {e}")),
        Err(e) => Err(format!("worker task failed: {e}")),
    }
}

/// UTC day bucket for a unix-ms timestamp (`YYYY-MM-DD`).
fn day_bucket(at_ms: i64) -> String {
    let days = at_ms.div_euclid(1000).div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days_from_civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of days_from_civil.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Monday of the ISO week containing `YYYY-MM-DD`, as `YYYY-MM-DD`.
fn iso_week_start(date: &str) -> Option<String> {
    let parts: Vec<i64> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return None;
    }
    let days = days_from_civil(parts[0], parts[1], parts[2]);
    // Monday=0 … Sunday=6. Day 0 (1970-01-01) was a Thursday, which is
    // index 3 in that scheme — not 4. The previous `(days + 4)` shifted
    // every week one day early (Sunday buckets landed on the prior Monday).
    let weekday = (days + 3).rem_euclid(7);
    let monday = days - weekday;
    let (yy, mm, dd) = civil_from_days(monday);
    Some(format!("{yy:04}-{mm:02}-{dd:02}"))
}

// ---------------------------------------------------------------------
// 1. Model performance
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/models`
///
/// Per-model request counts by status, success/failure rates, latency
/// distribution (duration + TTFT), token totals and tokens per second.
/// Only measured samples contribute; missing values stay absent.
async fn models_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("");
        let sql = append_condition(
            &where_sql,
            "SELECT model, status, duration_ms, ttft_ms, input_tokens, output_tokens FROM traces",
            "model IS NOT NULL",
        );
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();
        let mut stmt = c.prepare(&sql)?;

        #[derive(Default)]
        struct Acc {
            total: u64,
            success: u64,
            error: u64,
            cancelled: u64,
            interrupted: u64,
            durations: Vec<f64>,
            ttfts: Vec<f64>,
            input_tokens: Option<u64>,
            output_tokens: Option<u64>,
        }
        let mut rows: std::collections::BTreeMap<String, Acc> = Default::default();
        let mut queried = stmt.query(params_refs.as_slice())?;
        while let Some(r) = queried.next()? {
            let model: String = r.get(0)?;
            let status: String = r.get(1)?;
            let acc = rows.entry(model).or_default();
            acc.total += 1;
            match status.as_str() {
                "success" => acc.success += 1,
                "error" => acc.error += 1,
                "cancelled_user" | "cancelled_client" => acc.cancelled += 1,
                "interrupted" => acc.interrupted += 1,
                _ => {}
            }
            if let Some(d) = r.get::<_, Option<i64>>(2)? {
                if d > 0 {
                    acc.durations.push(d as f64);
                }
            }
            if let Some(t) = r.get::<_, Option<i64>>(3)? {
                if t > 0 {
                    acc.ttfts.push(t as f64);
                }
            }
            let tin: Option<i64> = r.get(4)?;
            let tout: Option<i64> = r.get(5)?;
            if tin.is_some() || tout.is_some() {
                acc.input_tokens =
                    Some(acc.input_tokens.unwrap_or(0) + tin.unwrap_or(0).max(0) as u64);
                acc.output_tokens =
                    Some(acc.output_tokens.unwrap_or(0) + tout.unwrap_or(0).max(0) as u64);
            }
        }

        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(model, a)| {
                // Tokens per second: output tokens over summed generation
                // duration — only when BOTH sides were actually measured.
                let gen_ms_total: f64 = a.durations.iter().sum();
                let tps = match (a.output_tokens, gen_ms_total > 0.0) {
                    (Some(out), true) if out > 0 => Some(out as f64 / (gen_ms_total / 1000.0)),
                    _ => None,
                };
                serde_json::json!({
                    "model": model,
                    "totalRequests": a.total,
                    "successful": a.success,
                    "failed": a.error,
                    "cancelled": a.cancelled,
                    "interrupted": a.interrupted,
                    "successRate": (a.total > 0).then(|| a.success as f64 / a.total as f64),
                    "failureRate": (a.total > 0).then(|| a.error as f64 / a.total as f64),
                    "latency": Dist::build(a.durations.clone()).to_json(),
                    "ttft": Dist::build(a.ttfts.clone()).to_json(),
                    "inputTokens": a.input_tokens,
                    "outputTokens": a.output_tokens,
                    "totalTokens": token_sum(a.input_tokens, a.output_tokens),
                    "tokensPerSecond": tps,
                })
            })
            .collect();
        Ok(serde_json::json!({ "models": items }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

/// `input + output` when either side is known; `None` when neither is.
fn token_sum(input: Option<u64>, output: Option<u64>) -> Option<u64> {
    match (input, output) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        (Some(i), None) => Some(i),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

/// Fold an extra condition into a possibly-empty WHERE clause.
fn append_condition(where_sql: &str, base: &str, extra: &str) -> String {
    if where_sql.is_empty() {
        format!("{base} WHERE {extra}")
    } else {
        format!("{base} {where_sql} AND {extra}")
    }
}

// ---------------------------------------------------------------------
// 2. Provider performance
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/providers`
async fn providers_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("");
        let sql = append_condition(
            &where_sql,
            "SELECT provider, status, duration_ms, ttft_ms, input_tokens, output_tokens FROM traces",
            "provider IS NOT NULL",
        );
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();
        let mut stmt = c.prepare(&sql)?;

        #[derive(Default)]
        struct Acc {
            total: u64,
            success: u64,
            error: u64,
            cancelled: u64,
            durations: Vec<f64>,
            ttfts: Vec<f64>,
            input_tokens: Option<u64>,
            output_tokens: Option<u64>,
        }
        let mut rows: std::collections::BTreeMap<String, Acc> = Default::default();
        let mut queried = stmt.query(params_refs.as_slice())?;
        while let Some(r) = queried.next()? {
            let provider: String = r.get(0)?;
            let status: String = r.get(1)?;
            let acc = rows.entry(provider).or_default();
            acc.total += 1;
            match status.as_str() {
                "success" => acc.success += 1,
                "error" => acc.error += 1,
                "cancelled_user" | "cancelled_client" => acc.cancelled += 1,
                _ => {}
            }
            if let Some(d) = r.get::<_, Option<i64>>(2)? {
                if d > 0 {
                    acc.durations.push(d as f64);
                }
            }
            if let Some(t) = r.get::<_, Option<i64>>(3)? {
                if t > 0 {
                    acc.ttfts.push(t as f64);
                }
            }
            let tin: Option<i64> = r.get(4)?;
            let tout: Option<i64> = r.get(5)?;
            if tin.is_some() || tout.is_some() {
                acc.input_tokens =
                    Some(acc.input_tokens.unwrap_or(0) + tin.unwrap_or(0).max(0) as u64);
                acc.output_tokens =
                    Some(acc.output_tokens.unwrap_or(0) + tout.unwrap_or(0).max(0) as u64);
            }
        }

        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(provider, a)| {
                serde_json::json!({
                    "provider": provider,
                    "totalRequests": a.total,
                    "successful": a.success,
                    "failed": a.error,
                    "cancelled": a.cancelled,
                    "successRate": (a.total > 0).then(|| a.success as f64 / a.total as f64),
                    "failureRate": (a.total > 0).then(|| a.error as f64 / a.total as f64),
                    "responseTime": Dist::build(a.durations.clone()).to_json(),
                    "ttft": Dist::build(a.ttfts.clone()).to_json(),
                    "inputTokens": a.input_tokens,
                    "outputTokens": a.output_tokens,
                    "totalTokens": token_sum(a.input_tokens, a.output_tokens),
                })
            })
            .collect();
        Ok(serde_json::json!({ "providers": items }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

// ---------------------------------------------------------------------
// 3. Token usage
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/tokens`
///
/// Totals, per-model / per-provider splits, per-session average and
/// daily/weekly/monthly series. Rows without token data never contribute
/// (their absence is not zero).
async fn tokens_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();

        #[derive(Default)]
        struct Tok {
            input: Option<u64>,
            output: Option<u64>,
            rows_reporting: u64,
            rows_total: u64,
        }
        impl Tok {
            fn add(&mut self, tin: Option<i64>, tout: Option<i64>) {
                self.rows_total += 1;
                if tin.is_some() || tout.is_some() {
                    self.rows_reporting += 1;
                    self.input = Some(self.input.unwrap_or(0) + tin.unwrap_or(0).max(0) as u64);
                    self.output = Some(self.output.unwrap_or(0) + tout.unwrap_or(0).max(0) as u64);
                }
            }
        }
        let mut overall = Tok::default();
        let mut by_model: std::collections::BTreeMap<String, Tok> = Default::default();
        let mut by_provider: std::collections::BTreeMap<String, Tok> = Default::default();
        let mut by_session: std::collections::BTreeMap<String, Tok> = Default::default();
        {
            let sql = format!(
                "SELECT model, provider, input_tokens, output_tokens, session_id \
                 FROM traces {where_sql}"
            );
            let mut stmt = c.prepare(&sql)?;
            let mut queried = stmt.query(params_refs.as_slice())?;
            while let Some(r) = queried.next()? {
                let model: Option<String> = r.get(0)?;
                let provider: Option<String> = r.get(1)?;
                let tin: Option<i64> = r.get(2)?;
                let tout: Option<i64> = r.get(3)?;
                let session: Option<String> = r.get(4)?;
                overall.add(tin, tout);
                if let Some(m) = &model {
                    by_model.entry(m.clone()).or_default().add(tin, tout);
                }
                if let Some(p) = &provider {
                    by_provider.entry(p.clone()).or_default().add(tin, tout);
                }
                if let Some(s) = &session {
                    by_session.entry(s.clone()).or_default().add(tin, tout);
                }
            }
        }

        // Average tokens per session over sessions that reported any.
        let session_totals: Vec<f64> = by_session
            .values()
            .filter_map(|t| token_sum(t.input, t.output).map(|v| v as f64))
            .collect();

        let daily = bucket_tokens(c, &where_sql, &args)?;
        let (weekly, monthly) = roll_up_daily(&daily);

        let models_json: Vec<serde_json::Value> = by_model
            .iter()
            .map(|(k, t)| serde_json::json!({
                "model": k, "inputTokens": t.input, "outputTokens": t.output,
                "totalTokens": token_sum(t.input, t.output),
            }))
            .collect();
        let providers_json: Vec<serde_json::Value> = by_provider
            .iter()
            .map(|(k, t)| serde_json::json!({
                "provider": k, "inputTokens": t.input, "outputTokens": t.output,
                "totalTokens": token_sum(t.input, t.output),
            }))
            .collect();

        Ok(serde_json::json!({
            "totals": {
                "inputTokens": overall.input,
                "outputTokens": overall.output,
                "totalTokens": token_sum(overall.input, overall.output),
                "rowsReportingTokens": overall.rows_reporting,
                "rowsTotal": overall.rows_total,
            },
            "sessions": {
                "sessionsReporting": session_totals.len(),
                "averageTokensPerSession": mean_of(&session_totals).map(|v| v as u64),
            },
            "byModel": models_json,
            "byProvider": providers_json,
            "daily": daily,
            "weekly": weekly,
            "monthly": monthly,
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

/// Bucket token consumption by UTC day. Only reporting rows are counted.
fn bucket_tokens(
    c: &Connection,
    where_sql: &str,
    args: &[Box<dyn rusqlite::types::ToSql>],
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let base = "SELECT strftime('%Y-%m-%d', started_at_ms / 1000, 'unixepoch') AS bucket, \
                SUM(input_tokens), SUM(output_tokens), COUNT(*) FROM traces";
    let sql = append_condition(
        where_sql,
        base,
        "(input_tokens IS NOT NULL OR output_tokens IS NOT NULL)",
    ) + " GROUP BY bucket ORDER BY bucket ASC";
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = c.prepare(&sql)?;
    let mut out = Vec::new();
    let mut rows = stmt.query(params_refs.as_slice())?;
    while let Some(r) = rows.next()? {
        let bucket: String = r.get(0)?;
        let input: Option<i64> = r.get(1)?;
        let output: Option<i64> = r.get(2)?;
        let count: i64 = r.get(3)?;
        out.push(serde_json::json!({
            "bucket": bucket,
            "inputTokens": input.map(|v| v.max(0) as u64),
            "outputTokens": output.map(|v| v.max(0) as u64),
            "requests": count,
        }));
    }
    Ok(out)
}

/// Roll daily buckets up into ISO weeks and calendar months.
fn roll_up_daily(
    daily: &[serde_json::Value],
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use std::collections::BTreeMap;
    let mut weeks: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut months: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for d in daily {
        let Some(bucket) = d.get("bucket").and_then(|b| b.as_str()) else {
            continue;
        };
        let input = d.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output = d.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        if bucket.len() >= 7 {
            let e = months.entry(bucket[..7].to_string()).or_insert((0, 0));
            e.0 += input;
            e.1 += output;
        }
        if let Some(monday) = iso_week_start(bucket) {
            let e = weeks.entry(monday).or_insert((0, 0));
            e.0 += input;
            e.1 += output;
        }
    }
    let wk: Vec<serde_json::Value> = weeks
        .iter()
        .map(|(k, (i, o))| serde_json::json!({ "bucket": k, "inputTokens": i, "outputTokens": o }))
        .collect();
    let mo: Vec<serde_json::Value> = months
        .iter()
        .map(|(k, (i, o))| serde_json::json!({ "bucket": k, "inputTokens": i, "outputTokens": o }))
        .collect();
    (wk, mo)
}

// ---------------------------------------------------------------------
// 4. Latency
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/latency`
///
/// Distinguishes model latency (`llm.generate` spans) from complete agent
/// execution latency (whole traces), plus TTFT and tool execution duration.
async fn latency_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("t.");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();

        let collect = |c: &Connection, extra: &str| -> rusqlite::Result<Vec<f64>> {
            let sql = format!("SELECT {extra} ");
            let mut stmt = c.prepare(&sql)?;
            let mut rows = stmt.query(params_refs.as_slice())?;
            let mut v = Vec::new();
            while let Some(r) = rows.next()? {
                if let Some(d) = r.get::<_, Option<i64>>(0)? {
                    if d > 0 {
                        v.push(d as f64);
                    }
                }
            }
            Ok(v)
        };

        let trace_durations = collect(
            c,
            &format!("t.duration_ms FROM traces t {where_sql}"),
        )?;
        // The WHERE may be empty; rebuild the simple form when it is.
        let trace_durations = if where_sql.is_empty() {
            simple_col(c, "SELECT duration_ms FROM traces WHERE duration_ms > 0")?
        } else {
            trace_durations
        };
        let ttfts = if where_sql.is_empty() {
            simple_col(c, "SELECT ttft_ms FROM traces WHERE ttft_ms > 0")?
        } else {
            collect(c, &format!("t.ttft_ms FROM traces t {where_sql}"))?
        };
        let llm_durations = if where_sql.is_empty() {
            simple_col(
                c,
                "SELECT s.duration_ms FROM spans s WHERE s.name = 'llm.generate' AND s.duration_ms > 0",
            )?
        } else {
            collect(
                c,
                &format!(
                    "s.duration_ms FROM spans s JOIN traces t ON t.trace_id = s.trace_id \
                     {where_sql} AND s.name = 'llm.generate'"
                ),
            )?
        };
        let tool_durations = if where_sql.is_empty() {
            simple_col(
                c,
                "SELECT json_extract(data_json, '$.durationMs') FROM trace_events \
                 WHERE name IN ('tool.completed','tool.failed') \
                   AND json_extract(data_json, '$.durationMs') > 0",
            )?
        } else {
            collect(
                c,
                &format!(
                    "json_extract(e.data_json, '$.durationMs') FROM trace_events e \
                     JOIN traces t ON t.trace_id = e.trace_id {where_sql} \
                     AND e.name IN ('tool.completed','tool.failed')"
                ),
            )?
        };

        Ok(serde_json::json!({
            "agentExecution": Dist::build(trace_durations).to_json(),
            "modelExecution": Dist::build(llm_durations).to_json(),
            "timeToFirstToken": Dist::build(ttfts).to_json(),
            "toolExecution": Dist::build(tool_durations).to_json(),
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

/// Collect positive i64 values from a parameterless query.
fn simple_col(c: &Connection, sql: &str) -> rusqlite::Result<Vec<f64>> {
    let mut stmt = c.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut v = Vec::new();
    while let Some(r) = rows.next()? {
        if let Some(d) = r.get::<_, Option<i64>>(0)? {
            if d > 0 {
                v.push(d as f64);
            }
        }
    }
    Ok(v)
}

// ---------------------------------------------------------------------
// 5. Tool analytics
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/tools`
async fn tools_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("t.");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();

        #[derive(Default)]
        struct ToolAcc {
            started: u64,
            completed: u64,
            failed: u64,
            durations: Vec<f64>,
            recent_traces: Vec<String>,
        }
        let mut tools: std::collections::BTreeMap<String, ToolAcc> = Default::default();
        {
            let base = "SELECT e.name, json_extract(e.data_json, '$.tool'), \
                        json_extract(e.data_json, '$.durationMs'), t.trace_id \
                 FROM trace_events e JOIN traces t ON t.trace_id = e.trace_id";
            let sql = append_condition(
                &where_sql,
                base,
                "e.name IN ('tool.started','tool.completed','tool.failed')",
            ) + " ORDER BY e.at_ms DESC";
            let mut stmt = c.prepare(&sql)?;
            let mut rows = stmt.query(params_refs.as_slice())?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(0)?;
                let tool: Option<String> = r.get(1)?;
                let dur: Option<i64> = r.get(2)?;
                let trace_id: String = r.get(3)?;
                let key = tool.unwrap_or_else(|| "unknown".into());
                let acc = tools.entry(key).or_default();
                match name.as_str() {
                    "tool.started" => acc.started += 1,
                    "tool.completed" => acc.completed += 1,
                    "tool.failed" => acc.failed += 1,
                    _ => {}
                }
                if let Some(d) = dur {
                    if d > 0 {
                        acc.durations.push(d as f64);
                    }
                }
                // Keep the most recent trace links for drill-down.
                if acc.recent_traces.len() < 10 && !acc.recent_traces.contains(&trace_id) {
                    acc.recent_traces.push(trace_id);
                }
            }
        }

        let mut items: Vec<serde_json::Value> = tools
            .iter()
            .map(|(name, a)| {
                let finished = a.completed + a.failed;
                serde_json::json!({
                    "tool": name,
                    // Executions observed: max of starts and finishes so a
                    // missing start event on legacy rows still counts.
                    "executions": a.started.max(finished),
                    "completed": a.completed,
                    "failed": a.failed,
                    "failureRate": (finished > 0).then(|| a.failed as f64 / finished as f64),
                    "duration": Dist::build(a.durations.clone()).to_json(),
                    "recentTraces": a.recent_traces,
                })
            })
            .collect();
        items.sort_by(|a, b| b["executions"].as_u64().cmp(&a["executions"].as_u64()));

        Ok(serde_json::json!({ "tools": items }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

// ---------------------------------------------------------------------
// 6. Error analytics
// ---------------------------------------------------------------------

/// Classify an error message into a coarse category from its actual text.
fn classify_error(msg: &str) -> &'static str {
    let m = msg.to_lowercase();
    if m.contains("timeout") || m.contains("timed out") {
        "timeout"
    } else if m.contains("401") || m.contains("unauthorized") || m.contains("authentication") {
        "authentication"
    } else if m.contains("429") || m.contains("rate limit") || m.contains("quota") {
        "rate_limit"
    } else if m.contains("context length") || m.contains("token count") || m.contains("too long") {
        "context_length"
    } else if m.contains("connection") || m.contains("network") || m.contains("dns")
        || m.contains("broken pipe") || m.contains("reset")
    {
        "network"
    } else if m.contains("cursor position") || m.contains("terminal") {
        "environment"
    } else if m.contains("notfound") || m.contains("no such file") {
        "not_found"
    } else {
        "other"
    }
}

/// `GET /api/admin/analytics/errors`
async fn errors_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("t.");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();

        #[derive(Default)]
        struct ErrAcc {
            count: u64,
            models: std::collections::BTreeSet<String>,
            providers: std::collections::BTreeSet<String>,
            tools: std::collections::BTreeSet<String>,
            traces: Vec<String>,
            last_at: i64,
        }

        let mut by_category: std::collections::BTreeMap<String, ErrAcc> = Default::default();
        let mut timeline: std::collections::BTreeMap<String, u64> = Default::default();
        let mut total_errors = 0u64;
        let mut failed_traces = 0u64;
        let mut cancelled_traces = 0u64;
        let mut failed_turns = 0u64;
        let mut failed_sessions = 0u64;

        // ---- trace-level failures ----
        {
            let base = "SELECT t.status, t.model, t.provider, t.error_message, t.trace_id, \
                        t.started_at_ms, t.operation FROM traces t";
            let sql = append_condition(&where_sql, base, "1=1");
            let mut stmt = c.prepare(&sql)?;
            let mut rows = stmt.query(params_refs.as_slice())?;
            while let Some(r) = rows.next()? {
                let status: String = r.get(0)?;
                let model: Option<String> = r.get(1)?;
                let provider: Option<String> = r.get(2)?;
                let message: Option<String> = r.get(3)?;
                let trace_id: String = r.get(4)?;
                let at: i64 = r.get(5)?;
                let operation: String = r.get(6)?;
                match status.as_str() {
                    "error" => {
                        failed_traces += 1;
                        if operation == "cli.session" {
                            failed_sessions += 1;
                        }
                        let msg = message.as_deref().unwrap_or("unspecified failure");
                        let cat = classify_error(msg);
                        let acc = by_category.entry(cat.to_string()).or_default();
                        acc.count += 1;
                        acc.last_at = acc.last_at.max(at);
                        if let Some(m) = &model {
                            acc.models.insert(m.clone());
                        }
                        if let Some(p) = &provider {
                            acc.providers.insert(p.clone());
                        }
                        if !acc.traces.contains(&trace_id) {
                            acc.traces.push(trace_id);
                        }
                        total_errors += 1;
                        *timeline.entry(day_bucket(at)).or_insert(0) += 1;
                    }
                    "cancelled_user" | "cancelled_client" => cancelled_traces += 1,
                    _ => {}
                }
            }
        }

        // ---- event-level errors ----
        {
            let base = "SELECT e.name, e.data_json, t.model, t.provider, t.trace_id, e.at_ms \
                 FROM trace_events e JOIN traces t ON t.trace_id = e.trace_id";
            let sql = append_condition(
                &where_sql,
                base,
                "e.name IN ('turn.ended','tool.failed','stream.error','error','session.failed')",
            );
            let mut stmt = c.prepare(&sql)?;
            let mut rows = stmt.query(params_refs.as_slice())?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(0)?;
                let data_raw: Option<String> = r.get(1)?;
                let model: Option<String> = r.get(2)?;
                let provider: Option<String> = r.get(3)?;
                let trace_id: String = r.get(4)?;
                let at: i64 = r.get(5)?;
                let data: serde_json::Value = data_raw
                    .as_deref()
                    .and_then(|d| serde_json::from_str(d).ok())
                    .unwrap_or(serde_json::Value::Null);

                let msg = match name.as_str() {
                    // turn.ended carries stop=Error with a message.
                    "turn.ended" => {
                        let stop = data.get("stop").and_then(|s| s.as_str()).unwrap_or("");
                        if stop == "Error" {
                            failed_turns += 1;
                            data.get("error").and_then(|e| e.as_str()).map(String::from)
                        } else {
                            None
                        }
                    }
                    "tool.failed" => data.get("output").and_then(|o| o.as_str()).map(String::from),
                    "stream.error" => data.get("reason").and_then(|o| o.as_str()).map(String::from),
                    "error" => data.get("message").and_then(|o| o.as_str()).map(String::from),
                    "session.failed" => {
                        data.get("error").and_then(|o| o.as_str()).map(String::from)
                    }
                    _ => None,
                };
                let Some(msg) = msg.filter(|m| !m.is_empty()) else {
                    continue;
                };
                let cat = classify_error(&msg);
                let acc = by_category.entry(cat.to_string()).or_default();
                acc.count += 1;
                acc.last_at = acc.last_at.max(at);
                if let Some(m) = &model {
                    acc.models.insert(m.clone());
                }
                if let Some(p) = &provider {
                    acc.providers.insert(p.clone());
                }
                if name == "tool.failed" {
                    if let Some(tool) = data.get("tool").and_then(|t| t.as_str()) {
                        acc.tools.insert(tool.to_string());
                    }
                }
                if !acc.traces.contains(&trace_id) {
                    acc.traces.push(trace_id);
                }
                total_errors += 1;
                *timeline.entry(day_bucket(at)).or_insert(0) += 1;
            }
        }

        let categories: Vec<serde_json::Value> = by_category
            .iter()
            .map(|(cat, a)| {
                serde_json::json!({
                    "category": cat,
                    "count": a.count,
                    "models": a.models.iter().cloned().collect::<Vec<_>>(),
                    "providers": a.providers.iter().cloned().collect::<Vec<_>>(),
                    "tools": a.tools.iter().cloned().collect::<Vec<_>>(),
                    "traces": a.traces.iter().take(10).cloned().collect::<Vec<_>>(),
                    "lastAtMs": (a.last_at > 0).then_some(a.last_at),
                })
            })
            .collect();
        let timeline_json: Vec<serde_json::Value> = timeline
            .iter()
            .map(|(day, n)| serde_json::json!({ "day": day, "errors": n }))
            .collect();

        Ok(serde_json::json!({
            "totalErrors": total_errors,
            "failedTraces": failed_traces,
            "cancelledTraces": cancelled_traces,
            "failedTurns": failed_turns,
            "failedSessions": failed_sessions,
            "byCategory": categories,
            "timeline": timeline_json,
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

// ---------------------------------------------------------------------
// 7. Session analytics
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/sessions`
async fn sessions_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let (where_sql, args) = q.traces_where("t.");
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();

        // One row per cli.session trace; turns counted from its spans.
        let base = "SELECT t.trace_id, t.session_id, t.status, t.duration_ms, t.started_at_ms, \
                    t.input_tokens, t.output_tokens, t.model, \
                    (SELECT COUNT(*) FROM spans s WHERE s.trace_id = t.trace_id \
                     AND s.name LIKE 'turn %') AS turns \
             FROM traces t";
        let sql = append_condition(&where_sql, base, "t.operation = 'cli.session'");
        let mut stmt = c.prepare(&sql)?;

        struct Sess {
            trace_id: String,
            session_id: Option<String>,
            status: String,
            duration_ms: Option<i64>,
            started_at_ms: i64,
            tokens: Option<u64>,
            turns: i64,
            model: Option<String>,
        }
        let mut sessions: Vec<Sess> = Vec::new();
        let mut rows = stmt.query(params_refs.as_slice())?;
        while let Some(r) = rows.next()? {
            let tin: Option<i64> = r.get(5)?;
            let tout: Option<i64> = r.get(6)?;
            sessions.push(Sess {
                trace_id: r.get(0)?,
                session_id: r.get(1)?,
                status: r.get(2)?,
                duration_ms: r.get(3)?,
                started_at_ms: r.get(4)?,
                tokens: token_sum(tin.map(|v| v.max(0) as u64), tout.map(|v| v.max(0) as u64)),
                turns: r.get(8)?,
                model: r.get(7)?,
            });
        }

        let total = sessions.len() as u64;
        let count_status = |s: &str| sessions.iter().filter(|x| x.status == s).count() as u64;
        let active = count_status("running");
        let completed = count_status("success");
        let failed = count_status("error");
        let cancelled = count_status("cancelled_user") + count_status("cancelled_client");
        let interrupted = count_status("interrupted");

        let durations: Vec<f64> = sessions
            .iter()
            .filter_map(|s| s.duration_ms.filter(|d| *d > 0).map(|d| d as f64))
            .collect();
        let turn_counts: Vec<f64> =
            sessions.iter().map(|s| s.turns as f64).filter(|t| *t > 0.0).collect();
        let token_counts: Vec<f64> =
            sessions.iter().filter_map(|s| s.tokens.map(|t| t as f64)).collect();

        // Longest sessions by measured duration, newest first on ties.
        let mut longest: Vec<&Sess> =
            sessions.iter().filter(|s| s.duration_ms.is_some()).collect();
        longest.sort_by(|a, b| {
            b.duration_ms.cmp(&a.duration_ms).then(b.started_at_ms.cmp(&a.started_at_ms))
        });

        // Most active models by session count.
        let mut model_counts: std::collections::BTreeMap<String, u64> = Default::default();
        for s in &sessions {
            if let Some(m) = &s.model {
                *model_counts.entry(m.clone()).or_insert(0) += 1;
            }
        }
        let mut top_models: Vec<(String, u64)> = model_counts.into_iter().collect();
        top_models.sort_by(|a, b| b.1.cmp(&a.1));

        // Trend: sessions per UTC day.
        let mut trend: std::collections::BTreeMap<String, u64> = Default::default();
        for s in &sessions {
            *trend.entry(day_bucket(s.started_at_ms)).or_insert(0) += 1;
        }
        let trend_json: Vec<serde_json::Value> = trend
            .iter()
            .map(|(day, n)| serde_json::json!({ "day": day, "sessions": n }))
            .collect();

        Ok(serde_json::json!({
            "totalSessions": total,
            "activeSessions": active,
            "completedSessions": completed,
            "failedSessions": failed,
            "cancelledSessions": cancelled,
            "interruptedSessions": interrupted,
            "averageDurationMs": mean_of(&durations),
            "medianDurationMs": percentile_of(&mut durations.clone(), 50.0),
            "averageTurns": mean_of(&turn_counts),
            "averageTokens": mean_of(&token_counts).map(|v| v as u64),
            "longest": longest.iter().take(10).map(|s| serde_json::json!({
                "traceId": s.trace_id,
                "sessionId": s.session_id,
                "status": s.status,
                "durationMs": s.duration_ms,
                "turns": s.turns,
                "startedAtMs": s.started_at_ms,
            })).collect::<Vec<_>>(),
            "topModels": top_models.iter().take(10).map(|(m, n)| serde_json::json!({
                "model": m, "sessions": n,
            })).collect::<Vec<_>>(),
            "trend": trend_json,
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

// ---------------------------------------------------------------------
// 8. Cost analytics
// ---------------------------------------------------------------------

/// Pricing configured by the administrator, stored in `dashboard_settings`
/// under the `costPricing` key:
///
/// ```json
/// { "soket/GLM-5.3": { "inputPerMillion": 1.0, "outputPerMillion": 2.0 } }
/// ```
///
/// Prices are USD per one million tokens. Nothing is hardcoded; a model
/// without an entry reports `null` cost ("Unknown" in the UI), never zero.
const PRICING_SETTING_KEY: &str = "costPricing";

/// Load the admin-configured pricing table.
fn load_pricing(c: &Connection) -> std::collections::BTreeMap<String, (Option<f64>, Option<f64>)> {
    let Ok(raw) = c.query_row(
        "SELECT value FROM dashboard_settings WHERE key = ?1",
        rusqlite::params![PRICING_SETTING_KEY],
        |r| r.get::<_, String>(0),
    ) else {
        return Default::default();
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&raw)
    else {
        return Default::default();
    };
    let mut out: std::collections::BTreeMap<String, (Option<f64>, Option<f64>)> =
        Default::default();
    for (key, val) in map {
        let (input, output) = match &val {
            serde_json::Value::Object(fields) => (
                fields.get("inputPerMillion").and_then(|v| v.as_f64()),
                fields.get("outputPerMillion").and_then(|v| v.as_f64()),
            ),
            _ => (None, None),
        };
        out.insert(key, (input, output));
    }
    out
}

/// `GET /api/admin/analytics/cost` — estimated cost from the admin's own
/// pricing configuration. Every figure is labelled estimated; unpriced
/// models report `null`.
async fn cost_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<AnalyticsQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, move |c| {
        let pricing = load_pricing(c);
        let (where_sql, args) = q.traces_where("");
        let sql = append_condition(
            &where_sql,
            "SELECT model, provider, input_tokens, output_tokens FROM traces",
            "(input_tokens IS NOT NULL OR output_tokens IS NOT NULL)",
        );
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();
        let mut stmt = c.prepare(&sql)?;

        #[derive(Default)]
        struct CostAcc {
            input_tokens: u64,
            output_tokens: u64,
            cost: Option<f64>,
            priced_rows: u64,
            unpriced_rows: u64,
        }
        let mut by_key: std::collections::BTreeMap<String, CostAcc> = Default::default();
        let mut rows = stmt.query(params_refs.as_slice())?;
        while let Some(r) = rows.next()? {
            let model: Option<String> = r.get(0)?;
            let provider: Option<String> = r.get(1)?;
            let tin: Option<i64> = r.get(2)?;
            let tout: Option<i64> = r.get(3)?;
            let label = match (&model, &provider) {
                (Some(m), Some(p)) => format!("{p}/{m}"),
                (Some(m), None) => m.clone(),
                (None, Some(p)) => format!("{p}/(unknown model)"),
                (None, None) => "(unknown)".into(),
            };
            // Pricing keys accept either "provider/model" or bare "model".
            let price = pricing
                .get(&label)
                .or_else(|| model.as_deref().and_then(|m| pricing.get(m)));
            let acc = by_key.entry(label.clone()).or_default();
            acc.input_tokens += tin.unwrap_or(0).max(0) as u64;
            acc.output_tokens += tout.unwrap_or(0).max(0) as u64;
            match price {
                Some((in_p, out_p)) => {
                    let cost_in = in_p.map(|p| acc.input_tokens as f64 * p / 1_000_000.0);
                    let cost_out = out_p.map(|p| acc.output_tokens as f64 * p / 1_000_000.0);
                    let row_cost = match (cost_in, cost_out) {
                        (Some(i), Some(o)) => Some(i + o),
                        (Some(i), None) => Some(i),
                        (None, Some(o)) => Some(o),
                        (None, None) => None,
                    };
                    if let Some(row_cost) = row_cost {
                        acc.priced_rows += 1;
                        acc.cost = Some(acc.cost.unwrap_or(0.0) + row_cost);
                    } else {
                        acc.unpriced_rows += 1;
                    }
                }
                None => acc.unpriced_rows += 1,
            }
        }

        let items: Vec<serde_json::Value> = by_key
            .iter()
            .map(|(key, a)| {
                serde_json::json!({
                    "key": key,
                    "inputTokens": a.input_tokens,
                    "outputTokens": a.output_tokens,
                    // null = pricing not configured ("Unknown"), never 0.
                    "estimatedCostUsd": a.cost,
                    "pricingConfigured": a.priced_rows > 0,
                    "unpricedRequests": a.unpriced_rows,
                })
            })
            .collect();
        let total_estimated: Option<f64> = {
            let parts: Vec<f64> = by_key.values().filter_map(|a| a.cost).collect();
            (!parts.is_empty()).then(|| parts.iter().sum())
        };
        Ok(serde_json::json!({
            "estimated": true,
            "note": "estimated from admin-configured pricing — not billed amounts",
            "totalEstimatedCostUsd": total_estimated,
            "pricingConfigured": !pricing.is_empty(),
            "byModel": items,
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

/// `GET /api/admin/analytics/cost/pricing` — the current pricing table.
async fn pricing_get_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, |c| {
        let pricing = load_pricing(c);
        let items: Vec<serde_json::Value> = pricing
            .iter()
            .map(|(k, (i, o))| {
                serde_json::json!({
                    "key": k,
                    "inputPerMillion": i,
                    "outputPerMillion": o,
                })
            })
            .collect();
        Ok(serde_json::json!({ "pricing": items }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingEntry {
    input_per_million: Option<f64>,
    output_per_million: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingBody {
    /// Full replacement table: key → per-million prices.
    pricing: std::collections::BTreeMap<String, PricingEntry>,
}

/// `POST /api/admin/analytics/cost/pricing` — replace the pricing table.
async fn pricing_post_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    body: Option<Json<PricingBody>>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let Some(Json(body)) = body else {
        return err(StatusCode::BAD_REQUEST, "pricing table required");
    };
    // Validate before storing: prices must be finite and non-negative.
    for (key, entry) in &body.pricing {
        if key.trim().is_empty() {
            return err(StatusCode::BAD_REQUEST, "pricing keys must not be empty");
        }
        for v in [entry.input_per_million, entry.output_per_million] {
            if let Some(v) = v {
                if !v.is_finite() || v < 0.0 {
                    return err(
                        StatusCode::BAD_REQUEST,
                        &format!("invalid price for {key}: must be a finite non-negative number"),
                    );
                }
            }
        }
    }
    let mut map = serde_json::Map::new();
    for (key, entry) in &body.pricing {
        map.insert(
            key.clone(),
            serde_json::json!({
                "inputPerMillion": entry.input_per_million,
                "outputPerMillion": entry.output_per_million,
            }),
        );
    }
    let raw = serde_json::Value::Object(map).to_string();

    let store = Arc::clone(&state.store);
    let result = tokio::task::spawn_blocking(move || {
        store.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO dashboard_settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![PRICING_SETTING_KEY, raw],
            )?;
            Ok(())
        })
    })
    .await;
    match result {
        Ok(Ok(())) => Json(
            serde_json::json!({ "saved": true, "entries": body.pricing.len() }),
        )
        .into_response(),
        Ok(Err(e)) => err(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------------
// 9. Trace comparison
// ---------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CompareQuery {
    ids: String,
}

/// `GET /api/admin/analytics/compare?ids=a,b,c`
///
/// Side-by-side metrics for 2–5 traces, straight from their rows + spans.
async fn compare_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
    Query(q): Query<CompareQuery>,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let ids: Vec<String> = q
        .ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if !(2..=5).contains(&ids.len()) {
        return err(
            StatusCode::BAD_REQUEST,
            "provide between 2 and 5 trace ids separated by commas",
        );
    }
    let store = Arc::clone(&state.store);
    let result = tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        for id in &ids {
            let Some(detail) = store.get_trace_detail(id).ok().flatten() else {
                continue;
            };
            let t = &detail.trace;
            let turns = detail
                .spans
                .iter()
                .filter(|s| s.name.starts_with("turn "))
                .count();
            let tool_count = detail
                .events
                .iter()
                .filter(|e| e.name == "tool.started")
                .count();
            let tool_duration: Option<i64> = {
                let ds: Vec<i64> = detail
                    .events
                    .iter()
                    .filter(|e| e.name == "tool.completed" || e.name == "tool.failed")
                    .filter_map(|e| {
                        e.data_json
                            .as_deref()
                            .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
                            .and_then(|v| v.get("durationMs").and_then(|d| d.as_i64()))
                    })
                    .collect();
                (!ds.is_empty()).then(|| ds.iter().sum())
            };
            let errors = detail
                .events
                .iter()
                .filter(|e| {
                    matches!(
                        e.name.as_str(),
                        "error" | "tool.failed" | "stream.error" | "session.failed"
                    )
                })
                .count();
            items.push(serde_json::json!({
                "traceId": t.trace_id,
                "sessionId": t.session_id,
                "operation": t.operation,
                "model": t.model,
                "provider": t.provider,
                "status": t.status.as_str(),
                "startedAtMs": t.started_at_ms,
                "durationMs": t.duration_ms,
                "ttftMs": t.ttft_ms,
                "inputTokens": t.input_tokens,
                "outputTokens": t.output_tokens,
                "totalTokens": token_sum(t.input_tokens, t.output_tokens),
                "turns": turns,
                "toolExecutions": tool_count,
                "toolDurationMs": tool_duration,
                "errors": errors,
                "errorMessage": t.error_message,
            }));
        }
        if items.len() < 2 {
            return Err("fewer than two of the requested traces exist".to_string());
        }
        Ok(serde_json::json!({ "traces": items }))
    })
    .await;
    match result {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------------
// 10. Live observability
// ---------------------------------------------------------------------

/// `GET /api/admin/analytics/live`
///
/// A single cheap query set describing what is running right now. The
/// frontend polls this at a slow cadence; each poll is a handful of
/// indexed queries, never a full scan.
async fn live_handler(
    State(state): State<Arc<ConsoleState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = require_admin(&state, &headers).await {
        return e;
    }
    let result = with_conn(&state, |c| {
        let running_sessions: Vec<serde_json::Value> = {
            let mut stmt = c.prepare(
                "SELECT trace_id, session_id, model, provider, started_at_ms, operation \
                 FROM traces WHERE status = 'running' ORDER BY started_at_ms DESC LIMIT 50",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(serde_json::json!({
                    "traceId": r.get::<_, String>(0)?,
                    "sessionId": r.get::<_, Option<String>>(1)?,
                    "model": r.get::<_, Option<String>>(2)?,
                    "provider": r.get::<_, Option<String>>(3)?,
                    "startedAtMs": r.get::<_, i64>(4)?,
                    "operation": r.get::<_, String>(5)?,
                }));
            }
            out
        };

        let active_models: Vec<String> = {
            let mut stmt = c.prepare(
                "SELECT DISTINCT model FROM traces WHERE status = 'running' AND model IS NOT NULL",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(r.get::<_, String>(0)?);
            }
            out
        };

        // Recent tool activity on still-running traces (last hour).
        let running_tools: Vec<serde_json::Value> = {
            let cutoff = chrono::Utc::now().timestamp_millis() - 3_600_000;
            let mut stmt = c.prepare(
                "SELECT json_extract(e.data_json, '$.tool'), t.trace_id, e.at_ms, t.session_id \
                 FROM trace_events e JOIN traces t ON t.trace_id = e.trace_id \
                 WHERE t.status = 'running' AND e.name = 'tool.started' AND e.at_ms >= ?1 \
                 ORDER BY e.at_ms DESC LIMIT 25",
            )?;
            let mut rows = stmt.query(rusqlite::params![cutoff])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(serde_json::json!({
                    "tool": r.get::<_, Option<String>>(0)?,
                    "traceId": r.get::<_, String>(1)?,
                    "atMs": r.get::<_, i64>(2)?,
                    "sessionId": r.get::<_, Option<String>>(3)?,
                }));
            }
            out
        };

        let recent_completed: Vec<serde_json::Value> = {
            let mut stmt = c.prepare(
                "SELECT trace_id, model, duration_ms, ended_at_ms, operation \
                 FROM traces WHERE status = 'success' ORDER BY ended_at_ms DESC LIMIT 20",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(serde_json::json!({
                    "traceId": r.get::<_, String>(0)?,
                    "model": r.get::<_, Option<String>>(1)?,
                    "durationMs": r.get::<_, Option<i64>>(2)?,
                    "endedAtMs": r.get::<_, Option<i64>>(3)?,
                    "operation": r.get::<_, String>(4)?,
                }));
            }
            out
        };

        let recent_failures: Vec<serde_json::Value> = {
            let mut stmt = c.prepare(
                "SELECT trace_id, model, error_message, ended_at_ms, status, operation \
                 FROM traces WHERE status IN ('error','interrupted') \
                 ORDER BY ended_at_ms DESC LIMIT 20",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(serde_json::json!({
                    "traceId": r.get::<_, String>(0)?,
                    "model": r.get::<_, Option<String>>(1)?,
                    "errorMessage": r.get::<_, Option<String>>(2)?,
                    "endedAtMs": r.get::<_, Option<i64>>(3)?,
                    "status": r.get::<_, String>(4)?,
                    "operation": r.get::<_, String>(5)?,
                }));
            }
            out
        };

        Ok(serde_json::json!({
            "runningSessions": running_sessions,
            "activeModels": active_models,
            "recentToolActivity": running_tools,
            "recentCompleted": recent_completed,
            "recentFailures": recent_failures,
        }))
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e),
    }
}

// ---------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------

/// Build the admin analytics router. Every route re-verifies the caller's
/// administrator role server-side before any data leaves the store.
pub fn analytics_admin_router(state: Arc<ConsoleState>) -> Router {
    Router::new()
        .route("/api/admin/analytics/models", get(models_handler))
        .route("/api/admin/analytics/providers", get(providers_handler))
        .route("/api/admin/analytics/tokens", get(tokens_handler))
        .route("/api/admin/analytics/latency", get(latency_handler))
        .route("/api/admin/analytics/tools", get(tools_handler))
        .route("/api/admin/analytics/errors", get(errors_handler))
        .route("/api/admin/analytics/sessions", get(sessions_handler))
        .route("/api/admin/analytics/cost", get(cost_handler))
        .route(
            "/api/admin/analytics/cost/pricing",
            get(pricing_get_handler).post(pricing_post_handler),
        )
        .route("/api/admin/analytics/compare", get(compare_handler))
        .route("/api/admin/analytics/live", get(live_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a console router over a temp store with auth configured:
    /// one admin, one ordinary user, both signed in.
    async fn test_router() -> (
        Router,
        std::path::PathBuf,
        String, // admin cookie
        String, // user cookie
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("store.db");
        let store = Arc::new(crate::persist::DashboardStore::open(&db).unwrap());
        std::fs::write(
            dir.path().join("models.json"),
            r#"{"providers":[{"id":"local","baseUrl":"http://127.0.0.1:1/v1","models":["tiny"]}]}"#,
        )
        .unwrap();
        let engine = Arc::new(crate::engine::Engine::load(crate::engine::EnginePaths {
            agent_dir: dir.path().to_path_buf(),
        }));
        let playground = Arc::new(crate::playground::Playground::new(
            Arc::clone(&engine),
            Arc::clone(&store),
        ));
        let auth_store =
            Arc::new(crate::auth::AuthStore::open(dir.path().join("auth.db")).unwrap());
        auth_store.sign_up("admin@soket.ai", "admin-pass-123").unwrap();
        let uid = auth_store
            .create_pending_user("user@soket.ai", "user-pass-123")
            .unwrap();
        auth_store.set_account_status(uid, crate::auth::STATUS_APPROVED).unwrap();
        let state = Arc::new(ConsoleState {
            engine,
            store,
            playground,
            auth: Some(Arc::new(crate::auth_http::AuthState::new(Arc::clone(
                &auth_store,
            )))),
        });
        let admin = auth_store.sign_in("admin@soket.ai", "admin-pass-123").unwrap();
        let user = auth_store.sign_in("user@soket.ai", "user-pass-123").unwrap();
        let admin_ck = format!("{}={}", crate::auth::SESSION_COOKIE, admin);
        let user_ck = format!("{}={}", crate::auth::SESSION_COOKIE, user);
        // Hold the tempdir alive for the whole test.
        std::mem::forget(dir);
        (analytics_admin_router(state), db, admin_ck, user_ck)
    }

    /// Insert a finished trace row with the given measurements.
    fn seed_trace(
        db: &std::path::Path,
        id: &str,
        model: Option<&str>,
        provider: Option<&str>,
        status: &str,
        duration_ms: Option<i64>,
        ttft_ms: Option<i64>,
        input: Option<u64>,
        output: Option<u64>,
        at_ms: i64,
    ) {
        let store = crate::persist::DashboardStore::open(db).unwrap();
        store
            .insert_trace(&crate::persist::TraceRow {
                trace_id: id.into(),
                root_span_id: format!("{id}-root"),
                operation: if id.starts_with("cli") {
                    "cli.session".into()
                } else {
                    "dashboard.chat".into()
                },
                status: crate::persist::TraceStatus::parse(status).unwrap(),
                model: model.map(String::from),
                provider: provider.map(String::from),
                request_id: None,
                session_id: Some(format!("sess-{id}")),
                started_at_ms: at_ms,
                ended_at_ms: Some(at_ms + duration_ms.unwrap_or(0)),
                duration_ms,
                input_tokens: input,
                output_tokens: output,
                ttft_ms: ttft_ms.map(|v| v as u64),
                error_message: None,
                params_json: None,
                content_captured: false,
                owner_pid: None,
            })
            .unwrap();
    }

    fn seed_event(db: &std::path::Path, trace: &str, seq: i64, at: i64, name: &str, data: &str) {
        let store = crate::persist::DashboardStore::open(db).unwrap();
        store.insert_event(trace, seq, at, name, Some(data)).unwrap();
    }

    async fn get(router: &Router, uri: &str, cookie: &str) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let res = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header("cookie", cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn post_json(
        router: &Router,
        uri: &str,
        cookie: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let res = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn admin_only_authorization_on_every_endpoint() {
        let (router, _db, admin_ck, user_ck) = test_router().await;
        let endpoints = [
            "/api/admin/analytics/models",
            "/api/admin/analytics/providers",
            "/api/admin/analytics/tokens",
            "/api/admin/analytics/latency",
            "/api/admin/analytics/tools",
            "/api/admin/analytics/errors",
            "/api/admin/analytics/sessions",
            "/api/admin/analytics/cost",
            "/api/admin/analytics/cost/pricing",
            "/api/admin/analytics/live",
        ];
        for ep in endpoints {
            // Admin passes the guard.
            let (s, _) = get(&router, ep, &admin_ck).await;
            assert_eq!(s, StatusCode::OK, "admin should access {ep}");
            // Ordinary user is refused.
            let (s, body) = get(&router, ep, &user_ck).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "user must be refused on {ep}");
            assert!(body["error"].as_str().unwrap().contains("administrator"));
            // Anonymous is refused.
            let (s, _) = get(&router, ep, "").await;
            assert_eq!(s, StatusCode::FORBIDDEN, "anonymous must be refused on {ep}");
        }
        // POST pricing: same rules.
        let (s, _) = post_json(
            &router,
            "/api/admin/analytics/cost/pricing",
            &user_ck,
            r#"{"pricing":{}}"#,
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = get(&router, "/api/admin/analytics/compare?ids=a,b", "").await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn model_performance_aggregates_real_rows() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "m1", Some("GLM-5.3"), Some("soket"), "success", Some(1000), Some(200), Some(10), Some(90), t0);
        seed_trace(&db, "m2", Some("GLM-5.3"), Some("soket"), "success", Some(3000), Some(400), Some(20), Some(180), t0 + 1000);
        seed_trace(&db, "m3", Some("GLM-5.3"), Some("soket"), "error", None, None, None, None, t0 + 2000);
        seed_trace(&db, "m4", Some("llama-8b"), Some("other"), "success", Some(500), Some(100), Some(5), Some(50), t0 + 3000);

        let (s, body) = get(&router, "/api/admin/analytics/models", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let models = body["models"].as_array().unwrap();
        let glm = models.iter().find(|m| m["model"] == "GLM-5.3").unwrap();
        assert_eq!(glm["totalRequests"], 3);
        assert_eq!(glm["successful"], 2);
        assert_eq!(glm["failed"], 1);
        // success rate = 2/3
        assert!((glm["successRate"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
        // latency mean over the two measured durations (1000, 3000) = 2000
        assert!((glm["latency"]["meanMs"].as_f64().unwrap() - 2000.0).abs() < 1e-9);
        assert_eq!(glm["latency"]["count"], 2);
        // tokens: only rows that reported them
        assert_eq!(glm["inputTokens"], 30);
        assert_eq!(glm["outputTokens"], 270);
        assert_eq!(glm["totalTokens"], 300);
        // tokens/sec = 270 out tokens over 4s of measured generation
        assert!((glm["tokensPerSecond"].as_f64().unwrap() - 67.5).abs() < 1e-6);
        // P99 suppressed below 20 samples
        assert!(glm["latency"]["p99Ms"].is_null());
    }

    #[tokio::test]
    async fn missing_metrics_stay_null_never_zero() {
        let (router, db, admin_ck, _user) = test_router().await;
        // A trace with NO token and NO ttft data.
        seed_trace(&db, "bare", Some("m"), Some("p"), "success", Some(1200), None, None, None, 1_700_000_000_000);
        let (s, body) = get(&router, "/api/admin/analytics/models", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let m = &body["models"][0];
        assert!(m["inputTokens"].is_null(), "missing tokens must be null");
        assert!(m["outputTokens"].is_null());
        assert!(m["totalTokens"].is_null());
        assert!(m["tokensPerSecond"].is_null());
        assert!(m["ttft"]["count"] == 0);

        // Token dashboard: totals absent, not zero.
        let (s, tok) = get(&router, "/api/admin/analytics/tokens", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert!(tok["totals"]["inputTokens"].is_null());
        assert!(tok["totals"]["rowsReportingTokens"] == 0);
        assert_eq!(tok["totals"]["rowsTotal"], 1);

        // Cost with no pricing configured: Unknown, never 0.
        let (s, cost) = get(&router, "/api/admin/analytics/cost", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert!(cost["totalEstimatedCostUsd"].is_null());
        assert_eq!(cost["pricingConfigured"], false);
        assert_eq!(cost["byModel"][0]["estimatedCostUsd"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn provider_aggregation_and_comparison() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "p1", Some("a"), Some("fast-prov"), "success", Some(100), Some(50), Some(10), Some(10), t0);
        seed_trace(&db, "p2", Some("b"), Some("slow-prov"), "success", Some(900), Some(80), Some(10), Some(10), t0);
        seed_trace(&db, "p3", Some("c"), Some("slow-prov"), "error", Some(1100), None, None, None, t0);

        let (s, body) = get(&router, "/api/admin/analytics/providers", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let providers = body["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 2, "two distinct providers");
        let slow = providers.iter().find(|p| p["provider"] == "slow-prov").unwrap();
        assert_eq!(slow["totalRequests"], 2);
        assert_eq!(slow["failed"], 1);
        assert_eq!(slow["successful"], 1);
        // mean response time over the two measured rows (900, 1100) = 1000
        assert!((slow["responseTime"]["meanMs"].as_f64().unwrap() - 1000.0).abs() < 1e-9);
        let fast = providers.iter().find(|p| p["provider"] == "fast-prov").unwrap();
        assert!((fast["responseTime"]["meanMs"].as_f64().unwrap() - 100.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn token_series_and_date_range_filtering() {
        let (router, db, admin_ck, _user) = test_router().await;
        // Two different UTC days.
        let day1 = 1_700_000_000_000i64; // 2023-11-14
        let day2 = day1 + 86_400_000; // next day
        seed_trace(&db, "d1", Some("m"), Some("p"), "success", Some(10), None, Some(100), Some(50), day1);
        seed_trace(&db, "d2", Some("m"), Some("p"), "success", Some(10), None, Some(200), Some(60), day2);

        let (s, body) = get(&router, "/api/admin/analytics/tokens", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["totals"]["inputTokens"], 300);
        assert_eq!(body["totals"]["outputTokens"], 110);
        assert_eq!(body["daily"].as_array().unwrap().len(), 2, "two daily buckets");

        // Range filter excludes the first day entirely.
        let from = day2;
        let (s, body) = get(
            &router,
            &format!("/api/admin/analytics/tokens?fromMs={from}"),
            &admin_ck,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["totals"]["inputTokens"], 200, "range filter applies");
        assert_eq!(body["daily"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn latency_distinguishes_model_from_agent_execution() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        // Agent execution: whole-trace duration 5000ms.
        seed_trace(&db, "l1", Some("m"), Some("p"), "success", Some(5000), Some(300), Some(1), Some(1), t0);
        // Model execution: an llm.generate span of 2000ms under that trace.
        let store = crate::persist::DashboardStore::open(&db).unwrap();
        store
            .insert_span(&crate::persist::SpanRow {
                trace_id: "l1".into(),
                span_id: "l1-llm".into(),
                parent_span_id: Some("l1-root".into()),
                name: "llm.generate".into(),
                kind: "llm".into(),
                status: crate::persist::TraceStatus::Success,
                start_ms: t0,
                end_ms: Some(t0 + 2000),
                duration_ms: Some(2000),
                attributes_json: None,
            })
            .unwrap();
        // Tool execution: a completed tool event with durationMs.
        seed_event(&db, "l1", 1, t0, "tool.completed", r#"{"tool":"bash","durationMs":700}"#);

        let (s, body) = get(&router, "/api/admin/analytics/latency", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["agentExecution"]["count"], 1);
        assert_eq!(body["agentExecution"]["meanMs"], 5000.0);
        assert_eq!(body["modelExecution"]["count"], 1);
        assert_eq!(body["modelExecution"]["meanMs"], 2000.0);
        assert_eq!(body["toolExecution"]["count"], 1);
        assert_eq!(body["toolExecution"]["meanMs"], 700.0);
        assert_eq!(body["timeToFirstToken"]["meanMs"], 300.0);
    }

    #[tokio::test]
    async fn percentile_math_is_nearest_rank() {
        // 1..=100 → p50 = 50, p95 = 95, p99 = 99.
        let mut vals: Vec<f64> = (1..=100).map(|v| v as f64).collect();
        assert_eq!(percentile_of(&mut vals, 50.0), Some(50.0));
        assert_eq!(percentile_of(&mut vals, 95.0), Some(95.0));
        assert_eq!(percentile_of(&mut vals, 99.0), Some(99.0));
        assert_eq!(percentile_of(&mut vals, 100.0), Some(100.0));
        let empty: Vec<f64> = vec![];
        let mut e = empty.clone();
        assert_eq!(percentile_of(&mut e, 50.0), None);
        assert_eq!(mean_of(&empty), None, "mean of nothing is absent, not 0");
        // Dist suppresses p99 below 20 samples but reports p50/p95.
        let small = Dist::build(vec![10.0, 20.0, 30.0]);
        assert_eq!(small.p50, Some(20.0));
        assert_eq!(small.p95, Some(30.0));
        assert!(small.p99.is_none(), "p99 needs >= 20 samples");
    }

    #[tokio::test]
    async fn tool_statistics_and_failure_rates() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "t1", Some("m"), Some("p"), "success", Some(1000), None, None, None, t0);
        seed_event(&db, "t1", 1, t0, "tool.started", r#"{"tool":"bash","toolCallId":"a"}"#);
        seed_event(&db, "t1", 2, t0 + 100, "tool.completed", r#"{"tool":"bash","toolCallId":"a","durationMs":100}"#);
        seed_event(&db, "t1", 3, t0 + 200, "tool.started", r#"{"tool":"read","toolCallId":"b"}"#);
        seed_event(&db, "t1", 4, t0 + 300, "tool.failed", r#"{"tool":"read","toolCallId":"b","durationMs":50,"output":"not found"}"#);
        seed_event(&db, "t1", 5, t0 + 400, "tool.started", r#"{"tool":"bash","toolCallId":"c"}"#);
        seed_event(&db, "t1", 6, t0 + 600, "tool.completed", r#"{"tool":"bash","toolCallId":"c","durationMs":200}"#);

        let (s, body) = get(&router, "/api/admin/analytics/tools", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let tools = body["tools"].as_array().unwrap();
        let bash = tools.iter().find(|t| t["tool"] == "bash").unwrap();
        assert_eq!(bash["executions"], 2);
        assert_eq!(bash["completed"], 2);
        assert_eq!(bash["failed"], 0);
        assert_eq!(bash["failureRate"], 0.0);
        assert_eq!(bash["duration"]["meanMs"], 150.0); // (100+200)/2
        let read = tools.iter().find(|t| t["tool"] == "read").unwrap();
        assert_eq!(read["failed"], 1);
        assert_eq!(read["failureRate"], 1.0);
        // Sorted by executions: bash first.
        assert_eq!(tools[0]["tool"], "bash");
        // Drill-down links present.
        assert!(bash["recentTraces"].as_array().unwrap().contains(&"t1".into()));
    }

    #[tokio::test]
    async fn error_classification_and_drilldown() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "e1", Some("m"), Some("p"), "error", Some(500), None, None, None, t0);
        // Patch its error message through a direct update.
        {
            let store = crate::persist::DashboardStore::open(&db).unwrap();
            store
                .with_conn(|c| {
                    c.execute(
                        "UPDATE traces SET error_message = 'HTTP 429 rate limit exceeded' WHERE trace_id = 'e1'",
                        [],
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        seed_trace(&db, "e2", Some("m"), Some("p"), "cancelled_user", Some(500), None, None, None, t0 + 1);
        seed_trace(&db, "e3", Some("m"), Some("p"), "success", Some(500), None, None, None, t0 + 2);
        seed_event(&db, "e3", 1, t0 + 3, "tool.failed", r#"{"tool":"read","output":"NotFound: No such file or directory"}"#);
        seed_event(&db, "e3", 2, t0 + 4, "turn.ended", r#"{"stop":"Error","error":"connection reset by peer"}"#);

        let (s, body) = get(&router, "/api/admin/analytics/errors", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["failedTraces"], 1);
        assert_eq!(body["cancelledTraces"], 1);
        assert_eq!(body["failedTurns"], 1);
        let cats = body["byCategory"].as_array().unwrap();
        let rate = cats.iter().find(|c| c["category"] == "rate_limit").unwrap();
        assert_eq!(rate["count"], 1);
        assert!(rate["traces"].as_array().unwrap().contains(&"e1".into()), "drill-down link preserved");
        let notfound = cats.iter().find(|c| c["category"] == "not_found").unwrap();
        assert_eq!(notfound["count"], 1);
        assert!(notfound["tools"].as_array().unwrap().contains(&"read".into()));
        let net = cats.iter().find(|c| c["category"] == "network").unwrap();
        assert_eq!(net["count"], 1);
        // Timeline has entries on the seeded day.
        assert!(!body["timeline"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_analytics_counts_turns_and_duration() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        // Three sessions: one running, one success with 3 turns, one error.
        seed_trace(&db, "cli-a", Some("m"), Some("p"), "running", None, None, None, None, t0);
        seed_trace(&db, "cli-b", Some("m"), Some("p"), "success", Some(6000), None, Some(100), Some(50), t0);
        seed_trace(&db, "cli-c", Some("m2"), Some("p"), "error", Some(2000), None, None, None, t0);
        // Turn spans on cli-b.
        let store = crate::persist::DashboardStore::open(&db).unwrap();
        for n in 1..=3 {
            store
                .insert_span(&crate::persist::SpanRow {
                    trace_id: "cli-b".into(),
                    span_id: format!("cli-b-turn{n}"),
                    parent_span_id: Some("cli-b-root".into()),
                    name: format!("turn {n}"),
                    kind: "phase".into(),
                    status: crate::persist::TraceStatus::Success,
                    start_ms: t0 + n * 100,
                    end_ms: Some(t0 + n * 100 + 500),
                    duration_ms: Some(500),
                    attributes_json: None,
                })
                .unwrap();
        }

        let (s, body) = get(&router, "/api/admin/analytics/sessions", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["totalSessions"], 3);
        assert_eq!(body["activeSessions"], 1);
        assert_eq!(body["completedSessions"], 1);
        assert_eq!(body["failedSessions"], 1);
        // average turns over sessions that have any: cli-b has 3.
        assert_eq!(body["averageTurns"], 3.0);
        // average duration over the two measured: (6000+2000)/2 = 4000
        assert_eq!(body["averageDurationMs"], 4000.0);
        // longest first
        assert_eq!(body["longest"][0]["traceId"], "cli-b");
        // top model by session count: m with 2
        assert_eq!(body["topModels"][0]["model"], "m");
        assert_eq!(body["topModels"][0]["sessions"], 2);
    }

    #[tokio::test]
    async fn cost_requires_admin_pricing_and_labels_estimates() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "c1", Some("GLM-5.3"), Some("soket"), "success", Some(100), None, Some(1_000_000), Some(500_000), t0);
        seed_trace(&db, "c2", Some("unpriced-model"), Some("soket"), "success", Some(100), None, Some(100), Some(100), t0);

        // Before configuration: everything Unknown.
        let (_, before) = get(&router, "/api/admin/analytics/cost", &admin_ck).await;
        assert!(before["totalEstimatedCostUsd"].is_null());

        // Configure pricing: $1/M input, $2/M output for soket/GLM-5.3.
        let (s, _) = post_json(
            &router,
            "/api/admin/analytics/cost/pricing",
            &admin_ck,
            r#"{"pricing":{"soket/GLM-5.3":{"inputPerMillion":1.0,"outputPerMillion":2.0}}}"#,
        )
        .await;
        assert_eq!(s, StatusCode::OK);

        let (s, after) = get(&router, "/api/admin/analytics/cost", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(after["pricingConfigured"], true);
        assert_eq!(after["estimated"], true);
        // 1M input × $1/M + 0.5M output × $2/M = $2.00
        assert!((after["totalEstimatedCostUsd"].as_f64().unwrap() - 2.0).abs() < 1e-9);
        let glm = after["byModel"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["key"] == "soket/GLM-5.3")
            .unwrap();
        assert!((glm["estimatedCostUsd"].as_f64().unwrap() - 2.0).abs() < 1e-9);
        // The unpriced model stays Unknown with its row counted.
        let unpriced = after["byModel"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["key"] == "soket/unpriced-model")
            .unwrap();
        assert!(unpriced["estimatedCostUsd"].is_null());
        assert_eq!(unpriced["unpricedRequests"], 1);

        // Invalid prices are rejected.
        let (s, _) = post_json(
            &router,
            "/api/admin/analytics/cost/pricing",
            &admin_ck,
            r#"{"pricing":{"x":{"inputPerMillion":-5}}}"#,
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // Pricing table reads back.
        let (s, table) = get(&router, "/api/admin/analytics/cost/pricing", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(table["pricing"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn trace_comparison_side_by_side() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "cmp-a", Some("GLM-5.3"), Some("soket"), "success", Some(1000), Some(100), Some(10), Some(20), t0);
        seed_trace(&db, "cmp-b", Some("llama-8b"), Some("other"), "error", Some(4000), Some(900), Some(50), Some(5), t0);
        seed_event(&db, "cmp-b", 1, t0, "tool.started", r#"{"tool":"bash"}"#);
        seed_event(&db, "cmp-b", 2, t0 + 50, "tool.failed", r#"{"tool":"bash","durationMs":40,"output":"boom"}"#);

        // Too few ids → 400.
        let (s, _) = get(&router, "/api/admin/analytics/compare?ids=only-one", &admin_ck).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        let (s, body) = get(
            &router,
            "/api/admin/analytics/compare?ids=cmp-a,cmp-b",
            &admin_ck,
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let traces = body["traces"].as_array().unwrap();
        assert_eq!(traces.len(), 2);
        let a = traces.iter().find(|t| t["traceId"] == "cmp-a").unwrap();
        let b = traces.iter().find(|t| t["traceId"] == "cmp-b").unwrap();
        assert_eq!(a["model"], "GLM-5.3");
        assert_eq!(a["status"], "success");
        assert_eq!(a["totalTokens"], 30);
        assert_eq!(b["errors"], 1);
        assert_eq!(b["toolExecutions"], 1);
        assert_eq!(b["toolDurationMs"], 40);
        assert_eq!(b["ttftMs"], 900);
        // Unknown ids are tolerated but two known ones are required.
        let (s, _) = get(
            &router,
            "/api/admin/analytics/compare?ids=cmp-a,nope,alsonope",
            &admin_ck,
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn live_view_reports_running_state() {
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = chrono::Utc::now().timestamp_millis();
        seed_trace(&db, "live-run", Some("GLM-5.3"), Some("soket"), "running", None, None, None, None, t0);
        seed_trace(&db, "live-done", Some("GLM-5.3"), Some("soket"), "success", Some(1200), None, None, None, t0 - 5000);
        seed_event(&db, "live-run", 1, t0, "tool.started", r#"{"tool":"bash"}"#);

        let (s, body) = get(&router, "/api/admin/analytics/live", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let running = body["runningSessions"].as_array().unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0]["traceId"], "live-run");
        assert_eq!(body["activeModels"].as_array().unwrap()[0], "GLM-5.3");
        assert_eq!(body["recentCompleted"].as_array().unwrap().len(), 1);
        assert!(!body["recentToolActivity"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_events_do_not_double_count() {
        // insert_event uses INSERT OR REPLACE on (trace_id, seq): replaying
        // the same event must not create a second row.
        let (router, db, admin_ck, _user) = test_router().await;
        let t0 = 1_700_000_000_000i64;
        seed_trace(&db, "dup", Some("m"), Some("p"), "success", Some(100), None, Some(10), Some(10), t0);
        seed_event(&db, "dup", 1, t0, "tool.started", r#"{"tool":"bash"}"#);
        seed_event(&db, "dup", 1, t0, "tool.started", r#"{"tool":"bash"}"#); // same seq

        let (s, body) = get(&router, "/api/admin/analytics/tools", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let bash = &body["tools"][0];
        assert_eq!(bash["executions"], 1, "replayed event counted once");
    }

    #[tokio::test]
    async fn historical_phase1_rows_are_compatible() {
        // Phase 1 rows carry no events at all — analytics must still answer
        // with honest zeros-for-counts and nulls-for-measurements.
        let (router, db, admin_ck, _user) = test_router().await;
        seed_trace(&db, "old", Some("GLM-4.6"), Some("soket"), "interrupted", Some(999), None, None, None, 1_600_000_000_000);
        let (s, body) = get(&router, "/api/admin/analytics/models", &admin_ck).await;
        assert_eq!(s, StatusCode::OK);
        let m = &body["models"][0];
        assert_eq!(m["interrupted"], 1);
        assert!(m["inputTokens"].is_null());
        assert_eq!(m["latency"]["count"], 1, "duration was measured");
        // Tools/errors stay empty without inventing anything.
        let (_, tools) = get(&router, "/api/admin/analytics/tools", &admin_ck).await;
        assert!(tools["tools"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn day_bucket_and_iso_week_helpers() {
        assert_eq!(day_bucket(0), "1970-01-01");
        assert_eq!(day_bucket(86_400_000), "1970-01-02");
        // 2023-11-14 is a Tuesday; its ISO week starts Monday 2023-11-13.
        assert_eq!(iso_week_start("2023-11-14").unwrap(), "2023-11-13");
        assert_eq!(iso_week_start("2023-11-13").unwrap(), "2023-11-13");
        assert_eq!(iso_week_start("2024-01-01").unwrap(), "2024-01-01"); // Monday
        assert_eq!(classify_error("HTTP 429 rate limit"), "rate_limit");
        assert_eq!(classify_error("read timeout after 30s"), "timeout");
        assert_eq!(classify_error("something odd"), "other");
    }
}
