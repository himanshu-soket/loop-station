//! Typed rows and filters for the durable dashboard store.

use rusqlite::Row;
use serde::{Deserialize, Serialize};

/// Terminal or in-flight state of a trace / span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    /// Still executing in this process.
    Running,
    /// Completed normally (stop / length / toolUse).
    Success,
    /// Inference or transport failure.
    Error,
    /// The user pressed stop.
    CancelledUser,
    /// The browser disconnected mid-stream.
    CancelledClient,
    /// The process died before finalization; recovered at next open.
    Interrupted,
}

impl TraceStatus {
    /// Wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Error => "error",
            Self::CancelledUser => "cancelled_user",
            Self::CancelledClient => "cancelled_client",
            Self::Interrupted => "interrupted",
        }
    }

    /// Parse from wire string.
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "running" => Self::Running,
            "success" => Self::Success,
            "error" => Self::Error,
            "cancelled_user" => Self::CancelledUser,
            "cancelled_client" => Self::CancelledClient,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }

    /// Whether this is a terminal state.
    #[allow(dead_code)] // used by tests
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// One trace row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceRow {
    /// Unique trace id (uuid v7).
    pub trace_id: String,
    /// Root span id.
    pub root_span_id: String,
    /// Operation name, e.g. `dashboard.chat`.
    pub operation: String,
    /// Status.
    pub status: TraceStatus,
    /// Requested model id.
    pub model: Option<String>,
    /// Provider id.
    pub provider: Option<String>,
    /// Associated request id.
    pub request_id: Option<String>,
    /// Playground conversation id when applicable.
    pub session_id: Option<String>,
    /// Start (unix ms).
    pub started_at_ms: i64,
    /// End (unix ms), `None` while running.
    pub ended_at_ms: Option<i64>,
    /// Duration (ms).
    pub duration_ms: Option<i64>,
    /// Input tokens, when reported.
    pub input_tokens: Option<u64>,
    /// Output tokens, when reported.
    pub output_tokens: Option<u64>,
    /// Time to first token (ms), when measured.
    pub ttft_ms: Option<u64>,
    /// Error message on failure.
    pub error_message: Option<String>,
    /// Whitelisted inference parameters (JSON).
    pub params_json: Option<String>,
    /// Whether message content was captured into this trace.
    pub content_captured: bool,
    /// Process that owns this trace while it runs; used by crash recovery to
    /// tell live sessions from abandoned ones. `None` on legacy rows means
    /// "owner unknown" and is recovered as interrupted.
    #[serde(skip)]
    pub owner_pid: Option<i64>,
}

impl TraceRow {
    /// Map a SQL row.
    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            trace_id: r.get(0)?,
            root_span_id: r.get(1)?,
            operation: r.get(2)?,
            status: TraceStatus::parse(&r.get::<_, String>(3)?).unwrap_or(TraceStatus::Interrupted),
            model: r.get(4)?,
            provider: r.get(5)?,
            request_id: r.get(6)?,
            session_id: r.get(7)?,
            started_at_ms: r.get(8)?,
            ended_at_ms: r.get(9)?,
            duration_ms: r.get(10)?,
            input_tokens: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
            output_tokens: r.get::<_, Option<i64>>(12)?.map(|v| v as u64),
            ttft_ms: r.get::<_, Option<i64>>(13)?.map(|v| v as u64),
            error_message: r.get(14)?,
            params_json: r.get(15)?,
            content_captured: r.get::<_, i64>(16)? != 0,
            owner_pid: None,
        })
    }
}

/// One span row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanRow {
    /// Owning trace.
    pub trace_id: String,
    /// Span id.
    pub span_id: String,
    /// Parent span id (`None` for the root).
    pub parent_span_id: Option<String>,
    /// Operation name, e.g. `llm.generate`.
    pub name: String,
    /// `phase` | `llm` | `stream`.
    pub kind: String,
    /// Status.
    pub status: TraceStatus,
    /// Start (unix ms).
    pub start_ms: i64,
    /// End (unix ms).
    pub end_ms: Option<i64>,
    /// Duration (ms).
    pub duration_ms: Option<i64>,
    /// Attributes JSON (non-sensitive).
    pub attributes_json: Option<String>,
}

impl SpanRow {
    /// Map a SQL row.
    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            trace_id: r.get(0)?,
            span_id: r.get(1)?,
            parent_span_id: r.get(2)?,
            name: r.get(3)?,
            kind: r.get(4)?,
            status: TraceStatus::parse(&r.get::<_, String>(5)?).unwrap_or(TraceStatus::Interrupted),
            start_ms: r.get(6)?,
            end_ms: r.get(7)?,
            duration_ms: r.get(8)?,
            attributes_json: r.get(9)?,
        })
    }
}

/// A structured event on a trace.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceEvent {
    /// Sequence within the trace.
    pub seq: i64,
    /// When (unix ms).
    pub at_ms: i64,
    /// Event name.
    pub name: String,
    /// Optional payload JSON.
    pub data_json: Option<String>,
}

/// Full trace detail.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceDetail {
    /// The trace row.
    pub trace: TraceRow,
    /// All spans.
    pub spans: Vec<SpanRow>,
    /// Recorded events.
    pub events: Vec<TraceEvent>,
}

/// Filter/pagination for trace queries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceFilter {
    /// Free-text search over id/operation/model.
    pub q: String,
    /// Status filter (`all` handled by the caller).
    pub statuses: Option<Vec<TraceStatus>>,
    /// Model filter.
    pub models: Option<Vec<String>>,
    /// Range start (unix ms, inclusive).
    pub from_ms: Option<i64>,
    /// Range end (unix ms, inclusive).
    pub to_ms: Option<i64>,
    /// Minimum duration (ms).
    pub min_duration_ms: Option<i64>,
    /// Maximum duration (ms).
    pub max_duration_ms: Option<i64>,
    /// Sort column: `started` (default) | `duration`.
    pub sort: Option<String>,
    /// `asc` | `desc` (default).
    pub dir: Option<String>,
    /// 1-based page.
    pub page: i64,
    /// Page size (capped).
    pub per_page: i64,
}

/// Request status mirrors trace status.
pub type RequestStatus = TraceStatus;

/// One request row (request-oriented view).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRow {
    /// Unique request id.
    pub request_id: String,
    /// Associated trace.
    pub trace_id: String,
    /// Conversation id when issued from the playground.
    pub conversation_id: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Provider.
    pub provider: Option<String>,
    /// Status.
    pub status: RequestStatus,
    /// Start (unix ms).
    pub started_at_ms: i64,
    /// End (unix ms).
    pub ended_at_ms: Option<i64>,
    /// Duration (ms).
    pub duration_ms: Option<i64>,
    /// Input tokens.
    pub input_tokens: Option<u64>,
    /// Output tokens.
    pub output_tokens: Option<u64>,
    /// Time to first token (ms).
    pub ttft_ms: Option<u64>,
    /// Why it was cancelled: `user` | `client_disconnect` | `timeout`.
    pub cancel_reason: Option<String>,
    /// Error message.
    pub error_message: Option<String>,
}

impl RequestRow {
    /// Map a SQL row.
    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            request_id: r.get(0)?,
            trace_id: r.get(1)?,
            conversation_id: r.get(2)?,
            model: r.get(3)?,
            provider: r.get(4)?,
            status: TraceStatus::parse(&r.get::<_, String>(5)?).unwrap_or(TraceStatus::Interrupted),
            started_at_ms: r.get(6)?,
            ended_at_ms: r.get(7)?,
            duration_ms: r.get(8)?,
            input_tokens: r.get::<_, Option<i64>>(9)?.map(|v| v as u64),
            output_tokens: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
            ttft_ms: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
            cancel_reason: r.get(12)?,
            error_message: r.get(13)?,
        })
    }
}

/// Filter for request queries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestFilter {
    /// Free-text search.
    pub q: String,
    /// Status filter.
    pub statuses: Option<Vec<RequestStatus>>,
    /// Range start.
    pub from_ms: Option<i64>,
    /// Range end.
    pub to_ms: Option<i64>,
    /// 1-based page.
    pub page: i64,
    /// Page size.
    pub per_page: i64,
}

/// Conversation metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    /// Conversation id.
    pub conversation_id: String,
    /// Display title.
    pub title: String,
    /// Created (unix ms).
    pub created_at_ms: i64,
    /// Last updated (unix ms).
    pub updated_at_ms: i64,
    /// Last selected model.
    pub model: Option<String>,
}

impl ConversationRow {
    /// Map a SQL row.
    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            conversation_id: r.get(0)?,
            title: r.get(1)?,
            created_at_ms: r.get(2)?,
            updated_at_ms: r.get(3)?,
            model: r.get(4)?,
        })
    }
}

/// Conversation list item with message count.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    /// Conversation id.
    pub conversation_id: String,
    /// Title.
    pub title: String,
    /// Created (unix ms).
    pub created_at_ms: i64,
    /// Updated (unix ms).
    pub updated_at_ms: i64,
    /// Last model.
    pub model: Option<String>,
    /// Message count.
    pub message_count: u64,
}

/// One persisted chat message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    /// Owning conversation.
    pub conversation_id: String,
    /// Message id.
    pub message_id: String,
    /// Order within the conversation.
    pub seq: i64,
    /// `user` | `assistant`.
    pub role: String,
    /// Markdown content.
    pub content: String,
    /// Created (unix ms).
    pub created_at_ms: i64,
    /// `complete` | `streaming` | `error` | `cancelled`.
    pub status: String,
    /// Error text when status is error.
    pub error_message: Option<String>,
    /// Model that produced an assistant message.
    pub model: Option<String>,
    /// Originating request id.
    pub request_id: Option<String>,
    /// Originating trace id.
    pub trace_id: Option<String>,
}

impl MessageRow {
    /// Map a SQL row.
    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            conversation_id: r.get(0)?,
            message_id: r.get(1)?,
            seq: r.get(2)?,
            role: r.get(3)?,
            content: r.get(4)?,
            created_at_ms: r.get(5)?,
            status: r.get(6)?,
            error_message: r.get(7)?,
            model: r.get(8)?,
            request_id: r.get(9)?,
            trace_id: r.get(10)?,
        })
    }
}

/// Retention configuration.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    /// Maximum number of traces to keep (0 = unlimited).
    pub max_traces: u64,
    /// Maximum age in ms (None = unlimited).
    pub max_age_ms: Option<i64>,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_traces: 20_000,
            max_age_ms: None,
        }
    }
}

/// Dashboard settings map (string key/value).
#[derive(Debug, Clone, Default)]
pub struct SettingsMap(pub std::collections::BTreeMap<String, String>);
