//! Pure aggregation over parsed session entries.
//!
//! Everything here is derived from **stored** fields only:
//! * timestamps are the ones Loop recorded per entry/message;
//! * "turn latency" is *derived* (user message timestamp → following assistant
//!   message timestamp) and every place it is surfaced says so;
//! * no metric is invented or interpolated.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::payload::{AssistantContentBlock, AssistantMessage, KnownMessage, MessageBody, Usage};
use crate::store::{EntryRow, SessionRow};

/// Time-range selector parsed from the query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    /// All data.
    All,
    /// Last 24 hours.
    Day,
    /// Last 7 days.
    Week,
    /// Last 30 days.
    Month,
}

impl TimeRange {
    /// Parse from a query-string value (`all|24h|7d|30d`), defaulting to all.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim_ascii).filter(|s| !s.is_empty()) {
            Some("24h") => Self::Day,
            Some("7d") => Self::Week,
            Some("30d") => Self::Month,
            _ => Self::All,
        }
    }

    /// Inclusive lower bound (unix ms) relative to `now`.
    pub fn start_ms(&self, now: i64) -> Option<i64> {
        match self {
            Self::All => None,
            Self::Day => Some(now - 24 * 60 * 60 * 1000),
            Self::Week => Some(now - 7 * 24 * 60 * 60 * 1000),
            Self::Month => Some(now - 30 * 24 * 60 * 60 * 1000),
        }
    }
}

/// Status bucket for an assistant turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnStatus {
    /// Completed naturally (`stop`, `length`, `toolUse`).
    Success,
    /// Provider/runtime error or abort.
    Failed,
    /// Still streaming / unknown stop reason.
    Incomplete,
}

/// One LLM turn extracted from a session's entries.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TurnRecord {
    /// Session this turn belongs to.
    pub session_id: String,
    /// Model id (requested), when recorded.
    pub model: Option<String>,
    /// Response model id, when different.
    pub response_model: Option<String>,
    /// Provider id.
    pub provider: Option<String>,
    /// Turn end timestamp (assistant message timestamp or entry timestamp).
    pub ended_at: i64,
    /// Derived latency: preceding user message → this assistant message (ms).
    /// `None` when no user message precedes it in the stored branch.
    pub latency_ms: Option<u64>,
    /// Token usage + cost (zeroed when absent).
    pub usage: Usage,
    /// Status classification.
    pub status: TurnStatus,
    /// Stop-reason label as stored.
    pub stop_label: String,
    /// Error message when failed.
    pub error_message: Option<String>,
    /// Number of tool calls in this turn.
    pub tool_calls: u32,
}

/// Aggregated KPI block.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kpis {
    /// Sessions with at least one entry in range.
    pub sessions: u64,
    /// Total LLM turns in range.
    pub turns: u64,
    /// Successful turns.
    pub successful: u64,
    /// Failed turns (error/aborted).
    pub failed: u64,
    /// Incomplete/unknown turns.
    pub incomplete: u64,
    /// Turns with a derived latency value.
    pub latency_samples: u64,
    /// Mean derived latency (ms) across samples.
    pub avg_latency_ms: Option<f64>,
    /// Median derived latency (ms).
    pub p50_latency_ms: Option<f64>,
    /// 95th percentile derived latency (ms).
    pub p95_latency_ms: Option<f64>,
    /// Max derived latency (ms).
    pub max_latency_ms: Option<u64>,
    /// Summed usage.
    pub usage: Usage,
    /// Distinct models seen.
    pub distinct_models: u64,
    /// Tool calls issued.
    pub tool_calls: u64,
    /// Tool results flagged as errors.
    pub tool_errors: u64,
}

/// One point of a time series (bucketed).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    /// Bucket start (unix ms).
    pub t: i64,
    /// Turns in bucket.
    pub turns: u64,
    /// Failed turns in bucket.
    pub failed: u64,
    /// Successful turns in bucket.
    pub successful: u64,
    /// Total tokens in bucket.
    pub tokens: u64,
    /// Input tokens in bucket.
    pub input_tokens: u64,
    /// Output tokens in bucket.
    pub output_tokens: u64,
    /// Cache-read tokens in bucket.
    pub cache_read_tokens: u64,
    /// Mean derived latency (ms) among bucket samples.
    pub avg_latency_ms: Option<f64>,
    /// Sessions active in bucket.
    pub sessions: u64,
}

/// Per-model rollup row.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBreakdown {
    /// Model label (`model` or `responseModel` when only that exists).
    pub model: String,
    /// Providers seen for this model.
    pub providers: Vec<String>,
    /// Turns.
    pub turns: u64,
    /// Failed turns.
    pub failed: u64,
    /// Summed usage.
    pub usage: Usage,
    /// Mean derived latency (ms).
    pub avg_latency_ms: Option<f64>,
}

/// Full overview payload for `/api/overview`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// KPI block.
    pub kpis: Kpis,
    /// Volume/error/tokens/latency series.
    pub series: Vec<SeriesPoint>,
    /// Per-model rollups sorted by tokens desc.
    pub models: Vec<ModelBreakdown>,
    /// Data-window actually covered by stored entries.
    pub window: DataWindow,
}

/// Stored-data time window (not the requested filter).
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataWindow {
    /// Earliest entry timestamp (unix ms), when any.
    pub earliest: Option<i64>,
    /// Latest entry timestamp (unix ms), when any.
    pub latest: Option<i64>,
}

/// A session summary row for the trace table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// Session id.
    pub id: String,
    /// Display name (name or cwd basename or id prefix).
    pub title: String,
    /// Working directory.
    pub cwd: Option<String>,
    /// Created at (unix ms).
    pub created_at: i64,
    /// Last activity timestamp (unix ms).
    pub last_activity: i64,
    /// LLM turns in session.
    pub turns: u64,
    /// Failed turns.
    pub failed: u64,
    /// Incomplete turns.
    pub incomplete: u64,
    /// User messages.
    pub user_messages: u64,
    /// Tool calls.
    pub tool_calls: u64,
    /// Tool results flagged as errors.
    pub tool_errors: u64,
    /// Summed usage.
    pub usage: Usage,
    /// Models used (labels).
    pub models: Vec<String>,
    /// Dominant status for the session pill.
    pub status: TurnStatus,
    /// Mean derived latency (ms).
    pub avg_latency_ms: Option<f64>,
}

/// Extract LLM turns (assistant messages) from one session's entries.
///
/// Latency derivation: each assistant turn is measured against the timestamp
/// of the **nearest preceding message** (user prompt or tool result) on the
/// same session, using message-internal timestamps when present (entry
/// timestamps can be batch-append times). This yields:
/// * user → first response: perceived prompt latency;
/// * tool result → next LLM call: model turnaround inside the agent loop.
///
/// Turns with no measurable predecessor yield `None` — never a fabricated
/// number. Note this measures *message creation deltas*, not provider
/// time-to-first-token, which Loop does not store.
pub fn extract_turns(session_id: &str, entries: &[EntryRow]) -> Vec<TurnRecord> {
    let mut turns = Vec::new();
    let mut prev_msg_ts: Option<i64> = None;

    for e in entries.iter().filter(|e| e.payload.is_some()) {
        let payload = e.payload.as_ref().expect("checked");
        if payload.kind != "message" {
            continue;
        }
        let Some(message) = &payload.message else {
            continue;
        };
        match message {
            MessageBody::Known(KnownMessage::User(u)) => {
                // Prefer the message-internal timestamp (true creation time).
                let ts = u.timestamp.or(payload.timestamp).unwrap_or(0);
                if ts > 0 {
                    prev_msg_ts = Some(ts);
                }
            }
            MessageBody::Known(KnownMessage::ToolResult(tr)) => {
                let ts = tr.timestamp.or(payload.timestamp).unwrap_or(0);
                if ts > 0 {
                    prev_msg_ts = Some(ts);
                }
            }
            MessageBody::Known(KnownMessage::Assistant(a)) => {
                let ended_at = a.timestamp.or(payload.timestamp).unwrap_or(0);
                let latency_ms = match (prev_msg_ts, ended_at) {
                    (Some(start), end) if end >= start && start > 0 => Some((end - start) as u64),
                    _ => None,
                };
                let tool_calls = a
                    .content
                    .iter()
                    .filter(|b| matches!(b, AssistantContentBlock::ToolCall { .. }))
                    .count() as u32;
                turns.push(TurnRecord {
                    session_id: session_id.to_string(),
                    model: a.model.clone(),
                    response_model: a.response_model.clone(),
                    provider: a.provider.clone(),
                    ended_at,
                    latency_ms,
                    usage: a.usage.unwrap_or_default(),
                    status: classify(a),
                    stop_label: a.stop_label(),
                    error_message: a.error_message.clone(),
                    tool_calls,
                });
            }
            _ => {}
        }
    }
    turns
}

/// Classify an assistant message into a status bucket.
fn classify(a: &AssistantMessage) -> TurnStatus {
    if a.failed() {
        TurnStatus::Failed
    } else if a.succeeded() {
        TurnStatus::Success
    } else {
        TurnStatus::Incomplete
    }
}

/// Keep turns whose end timestamp falls within `[start, now]`.
fn in_range(turns: Vec<TurnRecord>, start: Option<i64>) -> Vec<TurnRecord> {
    match start {
        None => turns,
        Some(start) => turns.into_iter().filter(|t| t.ended_at >= start).collect(),
    }
}

/// Keep turns matching the model filter (exact label or substring-free equality).
fn by_model(turns: Vec<TurnRecord>, model: Option<&str>) -> Vec<TurnRecord> {
    match model
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "all")
    {
        None => turns,
        Some(want) => turns
            .into_iter()
            .filter(|t| {
                t.model.as_deref() == Some(want)
                    || t.response_model.as_deref() == Some(want)
                    || t.model
                        .as_deref()
                        .is_some_and(|m| m.eq_ignore_ascii_case(want))
            })
            .collect(),
    }
}

/// Model label for grouping: prefer requested model, fall back to response model.
fn turn_model_label(t: &TurnRecord) -> Option<String> {
    t.model
        .clone()
        .or_else(|| t.response_model.clone())
        .filter(|m| !m.trim().is_empty())
}

/// Percentile of a sorted-nonempty slice (linear interpolation).
fn percentile(sorted: &[u64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0] as f64;
    }
    let idx = pct * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted[lo] as f64
    } else {
        let frac = idx - lo as f64;
        sorted[lo] as f64 * (1.0 - frac) + sorted[hi] as f64 * frac
    }
}

/// Compute the KPI block from turns + tool stats.
fn compute_kpis(turns: &[TurnRecord], sessions: u64, tool_calls: u64, tool_errors: u64) -> Kpis {
    let mut kpis = Kpis {
        sessions,
        turns: turns.len() as u64,
        tool_calls,
        tool_errors,
        ..Default::default()
    };
    let mut latencies: Vec<u64> = Vec::new();
    let mut models: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for t in turns {
        match t.status {
            TurnStatus::Success => kpis.successful += 1,
            TurnStatus::Failed => kpis.failed += 1,
            TurnStatus::Incomplete => kpis.incomplete += 1,
        }
        kpis.usage.add(&t.usage);
        if let Some(l) = t.latency_ms {
            latencies.push(l);
        }
        if let Some(m) = turn_model_label(t) {
            models.insert(m);
        }
    }
    kpis.distinct_models = models.len() as u64;
    kpis.latency_samples = latencies.len() as u64;
    if !latencies.is_empty() {
        latencies.sort_unstable();
        let n = latencies.len();
        kpis.avg_latency_ms = Some(latencies.iter().sum::<u64>() as f64 / n as f64);
        kpis.p50_latency_ms = Some(percentile(&latencies, 0.50));
        kpis.p95_latency_ms = Some(percentile(&latencies, 0.95));
        kpis.max_latency_ms = latencies.last().copied();
    }
    kpis
}

/// Bucket size for the series given the span (aligned to round units).
fn bucket_ms(span: i64) -> i64 {
    if span <= 0 {
        return 60_000;
    }
    // aim for ~24-40 buckets
    let target = span / 28;
    const MIN: i64 = 60_000; // 1 minute
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 86_400_000;
    for step in [
        MIN,
        5 * MIN,
        15 * MIN,
        30 * MIN,
        HOUR,
        3 * HOUR,
        6 * HOUR,
        DAY,
    ] {
        if target <= step {
            return step;
        }
    }
    DAY
}

/// Build the time series from turns (already filtered).
fn build_series(turns: &[TurnRecord], window: DataWindow) -> Vec<SeriesPoint> {
    let Some(first) = window.earliest else {
        return Vec::new();
    };
    let last = window.latest.unwrap_or(first);
    let span = (last - first).max(1);
    let step = bucket_ms(span);

    let mut points: BTreeMap<i64, SeriesPoint> = BTreeMap::new();
    let mut start = first;
    while start <= last {
        points.insert(
            start,
            SeriesPoint {
                t: start,
                turns: 0,
                failed: 0,
                successful: 0,
                tokens: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                avg_latency_ms: None,
                sessions: 0,
            },
        );
        start += step;
    }

    let mut bucket_sessions: BTreeMap<i64, std::collections::BTreeSet<String>> = BTreeMap::new();
    let mut bucket_latencies: BTreeMap<i64, Vec<u64>> = BTreeMap::new();

    for t in turns {
        let key = points
            .range(..=t.ended_at)
            .next_back()
            .map(|(k, _)| *k)
            .unwrap_or(first);
        let Some(point) = points.get_mut(&key) else {
            continue;
        };
        point.turns += 1;
        match t.status {
            TurnStatus::Success => point.successful += 1,
            TurnStatus::Failed => point.failed += 1,
            TurnStatus::Incomplete => {}
        }
        let total = t.usage.total_or_sum();
        point.tokens += total;
        point.input_tokens += t.usage.prompt_tokens();
        point.output_tokens += t.usage.output;
        point.cache_read_tokens += t.usage.cache_read;
        bucket_sessions
            .entry(key)
            .or_default()
            .insert(t.session_id.clone());
        if let Some(l) = t.latency_ms {
            bucket_latencies.entry(key).or_default().push(l);
        }
    }

    for (key, sessions) in bucket_sessions {
        if let Some(p) = points.get_mut(&key) {
            p.sessions = sessions.len() as u64;
        }
    }
    for (key, lats) in bucket_latencies {
        if let Some(p) = points.get_mut(&key) {
            p.avg_latency_ms = Some(lats.iter().sum::<u64>() as f64 / lats.len() as f64);
        }
    }
    points.into_values().collect()
}

/// Per-model rollups.
fn build_models(turns: &[TurnRecord]) -> Vec<ModelBreakdown> {
    let mut map: BTreeMap<String, ModelBreakdown> = BTreeMap::new();
    for t in turns {
        let Some(label) = turn_model_label(t) else {
            continue;
        };
        let entry = map.entry(label.clone()).or_default();
        entry.model = label;
        if let Some(p) = &t.provider {
            if !entry.providers.contains(p) {
                entry.providers.push(p.clone());
            }
        }
        entry.turns += 1;
        if t.status == TurnStatus::Failed {
            entry.failed += 1;
        }
        entry.usage.add(&t.usage);
        if let Some(l) = t.latency_ms {
            let sum = entry.avg_latency_ms.unwrap_or(0.0) * ((entry.turns - 1) as f64);
            entry.avg_latency_ms = Some((sum + l as f64) / entry.turns as f64);
        }
    }
    let mut out: Vec<ModelBreakdown> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.usage
            .total_or_sum()
            .cmp(&a.usage.total_or_sum())
            .then_with(|| a.model.cmp(&b.model))
    });
    out
}

/// Tool-call statistics for one session's entries.
struct ToolStats {
    calls: u64,
    errors: u64,
}

fn tool_stats(entries: &[EntryRow]) -> ToolStats {
    let mut calls = 0u64;
    let mut errors = 0u64;
    for e in entries.iter().filter(|e| e.payload.is_some()) {
        let payload = e.payload.as_ref().expect("checked");
        let Some(message) = &payload.message else {
            continue;
        };
        match message {
            MessageBody::Known(KnownMessage::Assistant(a)) => {
                calls += a
                    .content
                    .iter()
                    .filter(|b| matches!(b, AssistantContentBlock::ToolCall { .. }))
                    .count() as u64;
            }
            MessageBody::Known(KnownMessage::ToolResult(tr)) if tr.is_error.unwrap_or(false) => {
                errors += 1;
            }
            MessageBody::Known(KnownMessage::ToolResult(_)) => {}
            _ => {}
        }
    }
    ToolStats { calls, errors }
}

/// Build session summaries (one per session with entries).
pub fn session_summaries(sessions: &[SessionRow], entries: &[EntryRow]) -> Vec<SessionSummary> {
    let mut by_session: BTreeMap<String, Vec<&EntryRow>> = BTreeMap::new();
    for e in entries {
        by_session.entry(e.session_id.clone()).or_default().push(e);
    }
    let mut out = Vec::new();
    for s in sessions {
        let Some(rows) = by_session.get(&s.id) else {
            continue;
        };
        let owned: Vec<EntryRow> = rows.iter().map(|r| (*r).clone()).collect();
        let turns = extract_turns(&s.id, &owned);
        let tools = tool_stats(&owned);
        let last_activity = rows
            .iter()
            .filter_map(|e| e.payload.as_ref().and_then(|p| p.timestamp))
            .max()
            .unwrap_or(s.created_at);
        let mut usage = Usage::default();
        let mut models: Vec<String> = Vec::new();
        let mut latencies: Vec<u64> = Vec::new();
        let mut failed = 0u64;
        let mut incomplete = 0u64;
        let mut user_messages = 0u64;
        for t in &turns {
            usage.add(&t.usage);
            if let Some(m) = turn_model_label(t) {
                if !models.contains(&m) {
                    models.push(m);
                }
            }
            if let Some(l) = t.latency_ms {
                latencies.push(l);
            }
            match t.status {
                TurnStatus::Failed => failed += 1,
                TurnStatus::Incomplete => incomplete += 1,
                TurnStatus::Success => {}
            }
        }
        for e in rows.iter().filter(|e| e.payload.is_some()) {
            if let Some(MessageBody::Known(KnownMessage::User(_))) =
                e.payload.as_ref().and_then(|p| p.message.as_ref())
            {
                user_messages += 1;
            }
        }
        let all_incomplete = turns.is_empty() || incomplete as usize == turns.len();
        let status = if failed > 0 {
            TurnStatus::Failed
        } else if all_incomplete {
            TurnStatus::Incomplete
        } else {
            TurnStatus::Success
        };
        out.push(SessionSummary {
            id: s.id.clone(),
            title: session_title(s),
            cwd: s.cwd.clone(),
            created_at: s.created_at,
            last_activity,
            turns: turns.len() as u64,
            failed,
            incomplete,
            user_messages,
            tool_calls: tools.calls,
            tool_errors: tools.errors,
            usage,
            models,
            status,
            avg_latency_ms: (!latencies.is_empty())
                .then(|| latencies.iter().sum::<u64>() as f64 / latencies.len() as f64),
        });
    }
    out
}

/// Human-friendly session title.
pub(crate) fn session_title_public(s: &SessionRow) -> String {
    session_title(s)
}

/// Human-friendly session title.
fn session_title(s: &SessionRow) -> String {
    if let Some(name) = s.name.as_deref().filter(|n| !n.trim().is_empty()) {
        return name.to_string();
    }
    if let Some(cwd) = s.cwd.as_deref().filter(|c| !c.trim().is_empty()) {
        let base = cwd.trim_end_matches('/');
        if let Some(last) = base.rsplit('/').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
        return cwd.to_string();
    }
    s.id.chars().take(8).collect()
}

/// Compute the full overview for a range + model filter.
pub fn overview(
    _sessions: &[SessionRow],
    entries: &[EntryRow],
    range: TimeRange,
    model: Option<&str>,
    now: i64,
) -> Overview {
    // Window from *stored* timestamps (pre-filter) so empty ranges still show
    // where the data lives.
    let mut window = DataWindow::default();
    for e in entries {
        if let Some(ts) = e.payload.as_ref().and_then(|p| p.timestamp) {
            window.earliest = Some(window.earliest.map_or(ts, |cur| cur.min(ts)));
            window.latest = Some(window.latest.map_or(ts, |cur| cur.max(ts)));
        }
    }

    let start = range.start_ms(now);
    let mut all_turns = Vec::new();
    let mut tool_calls = 0u64;
    let mut tool_errors = 0u64;
    let mut active_sessions = std::collections::BTreeSet::new();

    let mut by_session: BTreeMap<String, Vec<EntryRow>> = BTreeMap::new();
    for e in entries {
        by_session
            .entry(e.session_id.clone())
            .or_default()
            .push(e.clone());
    }
    for (sid, rows) in &by_session {
        let turns = extract_turns(sid, rows);
        let tools = tool_stats(rows);
        tool_calls += tools.calls;
        tool_errors += tools.errors;
        if turns.iter().any(|t| in_window(t.ended_at, start, now)) {
            active_sessions.insert(sid.clone());
        }
        all_turns.extend(turns);
    }

    let turns = by_model(in_range(all_turns, start), model);
    let filtered_sessions = if start.is_some() || model.is_some() {
        let keep: std::collections::BTreeSet<&str> =
            turns.iter().map(|t| t.session_id.as_str()).collect();
        active_sessions
            .iter()
            .filter(|s| keep.contains(s.as_str()))
            .count() as u64
    } else {
        by_session.len() as u64
    };

    let kpis = compute_kpis(&turns, filtered_sessions, tool_calls, tool_errors);
    let series = build_series(&turns, window);
    let models = build_models(&turns);
    Overview {
        kpis,
        series,
        models,
        window,
    }
}

/// Whether `ts` lies in `[start, now]`.
fn in_window(ts: i64, start: Option<i64>, now: i64) -> bool {
    ts <= now && start.is_none_or(|s| ts >= s)
}

/// Timeline event for the session detail view.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimelineEvent {
    /// User prompt.
    UserPrompt {
        /// Timestamp (unix ms).
        at: i64,
        /// Prompt text (first text block / string content), truncated server-side.
        preview: String,
    },
    /// Assistant turn (LLM call).
    AssistantTurn {
        /// Timestamp (unix ms).
        at: i64,
        /// Model label.
        model: Option<String>,
        /// Provider.
        provider: Option<String>,
        /// Stop label.
        stop: String,
        /// Status.
        status: TurnStatus,
        /// Derived latency (ms), when derivable.
        latency_ms: Option<u64>,
        /// Usage.
        usage: Usage,
        /// Text output blocks (truncated).
        text: Vec<String>,
        /// Thinking blocks (truncated).
        thinking: Vec<String>,
        /// Tool calls issued.
        tool_calls: Vec<ToolCallView>,
        /// Error message when failed.
        error: Option<String>,
    },
    /// Tool result.
    ToolResult {
        /// Timestamp (unix ms).
        at: i64,
        /// Tool name.
        tool: Option<String>,
        /// Matching call id.
        call_id: Option<String>,
        /// Result text (truncated).
        preview: String,
        /// Whether the tool reported an error.
        is_error: bool,
        /// Usage contributed by the tool.
        usage: Option<Usage>,
    },
    /// Model changed.
    ModelChange {
        /// Timestamp (unix ms).
        at: i64,
        /// Provider.
        provider: Option<String>,
        /// Model id.
        model: Option<String>,
    },
    /// Compaction happened.
    Compaction {
        /// Timestamp (unix ms).
        at: i64,
        /// Summary (truncated).
        preview: String,
    },
    /// Anything else (unknown/custom kinds).
    Other {
        /// Timestamp (unix ms).
        at: i64,
        /// Entry kind as stored.
        entry_kind: String,
    },
}

/// Tool-call view for the timeline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallView {
    /// Call id.
    pub id: Option<String>,
    /// Tool name.
    pub name: Option<String>,
    /// Arguments JSON (pretty, truncated).
    pub arguments: Option<String>,
}

/// Server-side truncation bound for sensitive content previews.
const PREVIEW_LIMIT: usize = 400;

/// Truncate a string for preview, marking the cut.
fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut out: String = s.chars().take(limit).collect();
    out.push('…');
    out
}

/// Extract display text from a user content value (string or blocks).
fn user_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Build the ordered timeline for one session.
pub fn timeline(entries: &[EntryRow]) -> Vec<TimelineEvent> {
    let mut events = Vec::new();
    // Timestamp of the nearest preceding message (user or tool result), used
    // to derive per-turn latency with the same semantics as `extract_turns`.
    let mut prev_msg_ts: Option<i64> = None;

    for e in entries.iter().filter(|e| e.payload.is_some()) {
        let payload = e.payload.as_ref().expect("checked");
        let at = payload.timestamp.unwrap_or(0);
        match payload.kind.as_str() {
            "model_change" | "modelChange" => events.push(TimelineEvent::ModelChange {
                at,
                provider: payload.provider.clone(),
                model: payload.model_id.clone(),
            }),
            "compaction" | "branch_summary" | "branchSummary" => {
                let preview = payload
                    .summary
                    .as_deref()
                    .map(|s| truncate(s, PREVIEW_LIMIT))
                    .unwrap_or_default();
                events.push(TimelineEvent::Compaction { at, preview });
            }
            "message" => {
                let Some(message) = &payload.message else {
                    continue;
                };
                match message {
                    MessageBody::Known(KnownMessage::User(u)) => {
                        let ts = u.timestamp.or(Some(at)).unwrap_or(0);
                        if ts > 0 {
                            prev_msg_ts = Some(ts);
                        }
                        events.push(TimelineEvent::UserPrompt {
                            at: ts,
                            preview: truncate(&user_text(&u.content), PREVIEW_LIMIT),
                        });
                    }
                    MessageBody::Known(KnownMessage::Assistant(a)) => {
                        let ended = a.timestamp.or(Some(at)).unwrap_or(0);
                        let latency_ms = match (prev_msg_ts, ended) {
                            (Some(start), end) if end >= start && start > 0 => {
                                Some((end - start) as u64)
                            }
                            _ => None,
                        };
                        events.push(TimelineEvent::AssistantTurn {
                            at: ended,
                            model: a.model.clone(),
                            provider: a.provider.clone(),
                            stop: a.stop_label(),
                            status: classify(a),
                            latency_ms,
                            usage: a.usage.unwrap_or_default(),
                            text: a
                                .content
                                .iter()
                                .filter_map(|b| match b {
                                    AssistantContentBlock::Text { text } => {
                                        Some(truncate(text, PREVIEW_LIMIT))
                                    }
                                    _ => None,
                                })
                                .collect(),
                            thinking: a
                                .content
                                .iter()
                                .filter_map(|b| match b {
                                    AssistantContentBlock::Thinking { thinking } => {
                                        Some(truncate(thinking, PREVIEW_LIMIT))
                                    }
                                    _ => None,
                                })
                                .collect(),
                            tool_calls: a
                                .content
                                .iter()
                                .filter_map(|b| match b {
                                    AssistantContentBlock::ToolCall {
                                        id,
                                        name,
                                        arguments,
                                    } => Some(ToolCallView {
                                        id: id.clone(),
                                        name: name.clone(),
                                        arguments: arguments.as_ref().map(|v| {
                                            truncate(
                                                &serde_json::to_string_pretty(v)
                                                    .unwrap_or_else(|_| v.to_string()),
                                                PREVIEW_LIMIT,
                                            )
                                        }),
                                    }),
                                    _ => None,
                                })
                                .collect(),
                            error: a.error_message.clone(),
                        });
                    }
                    MessageBody::Known(KnownMessage::ToolResult(tr)) => {
                        let result_ts = tr.timestamp.or(Some(at)).unwrap_or(0);
                        if result_ts > 0 {
                            prev_msg_ts = Some(result_ts);
                        }
                        let preview = tr
                            .content
                            .iter()
                            .filter_map(|b| match b {
                                crate::payload::ToolResultBlock::Text { text } => {
                                    Some(truncate(text, PREVIEW_LIMIT))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        events.push(TimelineEvent::ToolResult {
                            at: result_ts,
                            tool: tr.tool_name.clone(),
                            call_id: tr.tool_call_id.clone(),
                            preview: truncate(&preview, PREVIEW_LIMIT),
                            is_error: tr.is_error.unwrap_or(false),
                            usage: tr.usage,
                        });
                    }
                    MessageBody::Known(KnownMessage::BashExecution {
                        command,
                        output,
                        exit_code,
                        ..
                    }) => {
                        let _ = (command, output, exit_code);
                        // Rendered as a tool-style event with command context.
                        events.push(TimelineEvent::ToolResult {
                            at,
                            tool: Some("bash".to_string()),
                            call_id: None,
                            preview: truncate(command, PREVIEW_LIMIT),
                            is_error: exit_code.is_some_and(|c| c != 0),
                            usage: None,
                        });
                    }
                    MessageBody::Known(KnownMessage::BranchSummary { summary })
                    | MessageBody::Known(KnownMessage::CompactionSummary { summary }) => {
                        events.push(TimelineEvent::Compaction {
                            at,
                            preview: truncate(summary, PREVIEW_LIMIT),
                        });
                    }
                    MessageBody::Other { role } => events.push(TimelineEvent::Other {
                        at,
                        entry_kind: format!("message:{role}"),
                    }),
                }
            }
            other => events.push(TimelineEvent::Other {
                at,
                entry_kind: other.to_string(),
            }),
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::EntryPayload;
    use crate::store::EntryRow;

    fn entry(seq: i64, payload_json: &str) -> EntryRow {
        EntryRow {
            session_id: "s".into(),
            entry_id: format!("e{seq}"),
            parent_id: None,
            entry_seq: seq,
            payload: EntryPayload::parse(payload_json),
        }
    }

    #[test]
    fn tool_loop_latency_measures_adjacent_messages() {
        // user @1000 → assistant @4000 (3s prompt latency)
        // toolResult @4200 → assistant @6000 (1.8s model turnaround)
        let entries = vec![
            entry(1, r#"{"type":"message","timestamp":1000,
                "message":{"role":"user","content":"q","timestamp":1000}}"#),
            entry(2, r#"{"type":"message","timestamp":4000,
                "message":{"role":"assistant","model":"m","stopReason":"toolUse",
                "usage":{"input":10,"output":5},"timestamp":4000,
                "content":[{"type":"toolCall","id":"c1","name":"read","arguments":{}}]}}"#),
            entry(3, r#"{"type":"message","timestamp":4200,
                "message":{"role":"toolResult","toolCallId":"c1","toolName":"read",
                "isError":false,"timestamp":4200,
                "content":[{"type":"text","text":"ok"}]}}"#),
            entry(4, r#"{"type":"message","timestamp":6000,
                "message":{"role":"assistant","model":"m","stopReason":"stop",
                "usage":{"input":20,"output":7},"timestamp":6000,
                "content":[{"type":"text","text":"done"}]}}"#),
        ];
        let turns = extract_turns("s", &entries);
        assert_eq!(turns.len(), 2);
        // first turn measured from the user message
        assert_eq!(turns[0].latency_ms, Some(3000));
        // second turn measured from the tool result, NOT the original user msg
        assert_eq!(turns[1].latency_ms, Some(1800));
        assert_eq!(turns[0].tool_calls, 1);
        assert_eq!(turns[0].status, TurnStatus::Success);
    }

    #[test]
    fn no_predecessor_yields_none_not_fabricated() {
        let entries = vec![entry(1, r#"{"type":"message","timestamp":500,
            "message":{"role":"assistant","model":"m","stopReason":"stop",
            "usage":{"input":1,"output":1},"timestamp":500,"content":[]}}"#)];
        let turns = extract_turns("s", &entries);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].latency_ms, None, "must not invent latency");
    }

    #[test]
    fn prefers_message_timestamp_over_entry_timestamp() {
        // entry appended in a batch at t=9000, but message created at t=2000
        let entries = vec![
            entry(1, r#"{"type":"message","timestamp":9000,
                "message":{"role":"user","content":"q","timestamp":1000}}"#),
            entry(2, r#"{"type":"message","timestamp":9000,
                "message":{"role":"assistant","model":"m","stopReason":"stop",
                "usage":{"input":1,"output":1},"timestamp":2000,"content":[]}}"#),
        ];
        let turns = extract_turns("s", &entries);
        assert_eq!(turns[0].latency_ms, Some(1000), "uses message timestamps");
    }

    #[test]
    fn failed_and_incomplete_classification() {
        let mk = |stop: &str| {
            vec![entry(1, r#"{"type":"message","timestamp":100,
                "message":{"role":"user","content":"q","timestamp":100}}"#),
            entry(2, &format!(r#"{{"type":"message","timestamp":200,
                "message":{{"role":"assistant","model":"m","stopReason":"{stop}",
                "errorMessage":"boom","timestamp":200,"content":[]}}}}"#))]
        };
        let failed = extract_turns("s", &mk("error"));
        assert_eq!(failed[0].status, TurnStatus::Failed);
        assert_eq!(failed[0].error_message.as_deref(), Some("boom"));

        let pending = extract_turns("s", &mk("pending"));
        assert_eq!(pending[0].status, TurnStatus::Incomplete);
    }

    #[test]
    fn range_and_model_filters() {
        // Use a realistic epoch so 24h windows are meaningful.
        let now = 1_800_000_000_000;
        let entries = vec![
            entry(1, r#"{"type":"message","timestamp":100,
                "message":{"role":"user","content":"q","timestamp":100}}"#),
            entry(2, r#"{"type":"message","timestamp":200,
                "message":{"role":"assistant","model":"old-model","stopReason":"stop",
                "usage":{"input":1,"output":1},"timestamp":200,"content":[]}}"#),
            entry(3, &format!(
                r#"{{"type":"message","timestamp":{},
                "message":{{"role":"user","content":"q2","timestamp":{}}}}}"#,
                now - 1000,
                now - 1000
            )),
            entry(4, &format!(
                r#"{{"type":"message","timestamp":{},
                "message":{{"role":"assistant","model":"new-model","stopReason":"stop",
                "usage":{{"input":2,"output":2}},"timestamp":{},"content":[]}}}}"#,
                now - 500,
                now - 500
            )),
        ];
        let sessions = vec![SessionRow {
            id: "s".into(),
            cwd: Some("/w".into()),
            name: None,
            parent_session_id: None,
            created_at: 100,
        }];

        // all range: both turns
        let ov = overview(&sessions, &entries, TimeRange::All, None, now);
        assert_eq!(ov.kpis.turns, 2);

        // 24h range (start = now - 86400000 = 136000): only the new turn
        let ov2 = overview(&sessions, &entries, TimeRange::Day, None, now);
        assert_eq!(ov2.kpis.turns, 1);
        assert_eq!(ov2.kpis.usage.input, 2);

        // model filter narrows to old-model only
        let ov3 = overview(&sessions, &entries, TimeRange::All, Some("old-model"), now);
        assert_eq!(ov3.kpis.turns, 1);
        assert_eq!(ov3.kpis.distinct_models, 1);
    }

    #[test]
    fn time_range_parsing() {
        assert_eq!(TimeRange::parse(None), TimeRange::All);
        assert_eq!(TimeRange::parse(Some("")), TimeRange::All);
        assert_eq!(TimeRange::parse(Some("24h")), TimeRange::Day);
        assert_eq!(TimeRange::parse(Some("7d")), TimeRange::Week);
        assert_eq!(TimeRange::parse(Some("30d")), TimeRange::Month);
        assert_eq!(TimeRange::parse(Some("garbage")), TimeRange::All);
    }

    #[test]
    fn percentile_interpolates() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[10], 0.5), 10.0);
        assert_eq!(percentile(&[10, 20], 0.5), 15.0);
        assert_eq!(percentile(&[10, 20, 30, 40], 0.5), 25.0);
        assert_eq!(percentile(&[10, 20, 30, 40], 0.95), 38.5);
    }

    #[test]
    fn truncation_marks_the_cut() {
        let long = "x".repeat(500);
        let out = truncate(&long, 400);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 401);
        assert_eq!(truncate("short", 400), "short");
    }

    #[test]
    fn session_title_prefers_name_then_cwd_basename() {
        let mk = |name: Option<&str>, cwd: Option<&str>| SessionRow {
            id: "abcdef123456".into(),
            cwd: cwd.map(Into::into),
            name: name.map(Into::into),
            parent_session_id: None,
            created_at: 0,
        };
        assert_eq!(session_title(&mk(Some("my session"), None)), "my session");
        assert_eq!(session_title(&mk(None, Some("/a/b/proj"))), "proj");
        assert_eq!(session_title(&mk(None, Some("/trailing/"))), "trailing");
        assert_eq!(
            session_title(&mk(None, None)),
            "abcdef12",
            "falls back to id prefix"
        );
    }

    #[test]
    fn timeline_renders_all_event_kinds() {
        let entries = vec![
            entry(1, r#"{"type":"model_change","timestamp":50,"provider":"soket","modelId":"GLM-5.3"}"#),
            entry(2, r#"{"type":"message","timestamp":100,
                "message":{"role":"user","content":"hello","timestamp":100}}"#),
            entry(3, r#"{"type":"message","timestamp":300,
                "message":{"role":"assistant","model":"GLM-5.3","provider":"soket",
                "stopReason":"toolUse","usage":{"input":5,"output":2},"timestamp":300,
                "content":[{"type":"thinking","thinking":"hmm"},
                           {"type":"text","text":"answer"},
                           {"type":"toolCall","id":"c1","name":"bash","arguments":{"cmd":"ls"}}]}}"#),
            entry(4, r#"{"type":"message","timestamp":400,
                "message":{"role":"toolResult","toolCallId":"c1","toolName":"bash",
                "isError":true,"timestamp":400,
                "content":[{"type":"text","text":"exit 1"}]}}"#),
            entry(5, r#"{"type":"compaction","timestamp":500,"summary":"compacted"}"#),
            entry(6, r#"{"type":"some_future_kind","timestamp":600}"#),
        ];
        let events = timeline(&entries);
        assert_eq!(events.len(), 6);
        assert!(matches!(events[0], TimelineEvent::ModelChange { .. }));
        assert!(matches!(events[1], TimelineEvent::UserPrompt { .. }));
        match &events[2] {
            TimelineEvent::AssistantTurn {
                latency_ms,
                tool_calls,
                thinking,
                text,
                ..
            } => {
                assert_eq!(*latency_ms, Some(200));
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name.as_deref(), Some("bash"));
                assert_eq!(thinking.len(), 1);
                assert_eq!(text.len(), 1);
            }
            other => panic!("expected AssistantTurn, got {other:?}"),
        }
        match &events[3] {
            TimelineEvent::ToolResult { is_error, tool, .. } => {
                assert!(is_error);
                assert_eq!(tool.as_deref(), Some("bash"));
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
        assert!(matches!(events[4], TimelineEvent::Compaction { .. }));
        match &events[5] {
            TimelineEvent::Other { entry_kind, .. } => {
                assert_eq!(entry_kind, "some_future_kind");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn sensitive_content_is_truncated_server_side() {
        let secret = "S".repeat(2000);
        let entries = vec![entry(1, &format!(
            r#"{{"type":"message","timestamp":1,
                "message":{{"role":"user","content":"{secret}","timestamp":1}}}}"#
        ))];
        let events = timeline(&entries);
        match &events[0] {
            TimelineEvent::UserPrompt { preview, .. } => {
                assert!(preview.chars().count() <= PREVIEW_LIMIT + 1);
                assert!(preview.ends_with('…'));
            }
            other => panic!("expected UserPrompt, got {other:?}"),
        }
    }
}
