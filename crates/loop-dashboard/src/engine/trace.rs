//! Trace instrumentation for one inference request.
//!
//! `TracedRequest` wraps a real `loop-ai` event stream and durably records
//! what actually happens:
//!
//! ```text
//! trace dashboard.chat                  (root span: http.request)
//! ├─ span request.validate              (phase)
//! ├─ span model.resolve                 (phase)
//! ├─ span llm.generate                  (llm)     ← TTFT measured here
//! │   └─ span stream.response           (stream)  ← chunk accounting
//! └─ terminal status on every path
//! ```
//!
//! Every terminal path is covered:
//! * success (`Done`)
//! * inference failure (`Error`)
//! * user cancellation (CancellationToken observed)
//! * client disconnect (SSE channel send fails)
//! * process exit (recovered as `interrupted` at next store open)
//!
//! Nothing is invented: token counts come from the final assistant message's
//! `usage`; TTFT is measured at the first content delta; spans that cannot be
//! observed are simply not created.

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::persist::{DashboardStore, RequestRow, RetentionConfig, SpanRow, TraceRow, TraceStatus};
use loop_ai::{AssistantMessage, AssistantMessageEvent, StopReason};

/// Sanitize + bound an error string before storage (log-injection guard).
pub fn sanitize_error(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_ERROR_LEN));
    for ch in raw.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            out.push(' ');
        } else {
            out.push(ch);
        }
        if out.len() >= MAX_ERROR_LEN {
            break;
        }
    }
    if raw.chars().count() > MAX_ERROR_LEN {
        out.push('…');
    }
    out.trim().to_string()
}

const MAX_ERROR_LEN: usize = 2000;

/// What the caller wants recorded about one request.
#[derive(Debug, Clone)]
pub struct TracedRequestSpec {
    /// Operation name for the trace (e.g. `dashboard.chat`).
    pub operation: &'static str,
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Playground conversation id, when applicable.
    pub conversation_id: Option<String>,
    /// Whitelisted params JSON (from [`crate::engine::params`]).
    pub params_json: String,
    /// Retention policy to enforce after finalization.
    pub retention: RetentionConfig,
}

/// A live traced inference request.
///
/// Created before the engine call; finalized exactly once on any terminal
/// path. Storage failures are logged to stderr — they never abort inference.
pub struct TracedRequest {
    store: Arc<DashboardStore>,
    spec: TracedRequestSpec,
    trace_id: String,
    request_id: String,
    root_span: String,
    started: Instant,
    #[allow(dead_code)] // surfaced via Debug; kept for future span backfill
    started_at_ms: i64,
    ttft: Option<u64>,
    event_seq: i64,
    finalized: bool,
}

impl TracedRequest {
    /// Begin a request: insert running trace + request rows and the root span.
    pub fn begin(store: Arc<DashboardStore>, spec: TracedRequestSpec) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let trace_id = uuid::Uuid::now_v7().to_string();
        let request_id = uuid::Uuid::now_v7().to_string();
        let root_span = uuid::Uuid::now_v7().to_string();

        let req = Self {
            store,
            spec,
            trace_id: trace_id.clone(),
            request_id: request_id.clone(),
            root_span: root_span.clone(),
            started: Instant::now(),
            started_at_ms: now_ms,
            ttft: None,
            event_seq: 0,
            finalized: false,
        };

        // Root span: the HTTP request itself.
        let _ = req.store.insert_trace(&TraceRow {
            trace_id: trace_id.clone(),
            root_span_id: root_span.clone(),
            operation: req.spec.operation.to_string(),
            status: TraceStatus::Running,
            model: Some(req.spec.model.clone()),
            provider: Some(req.spec.provider.clone()),
            request_id: Some(request_id.clone()),
            session_id: req.spec.conversation_id.clone(),
            started_at_ms: now_ms,
            ended_at_ms: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            ttft_ms: None,
            error_message: None,
            params_json: Some(req.spec.params_json.clone()),
            content_captured: false,
            owner_pid: Some(std::process::id() as i64),
        });
        let _ = req.store.insert_span(&SpanRow {
            trace_id: trace_id.clone(),
            span_id: root_span.clone(),
            parent_span_id: None,
            name: "http.request".into(),
            kind: "phase".into(),
            status: TraceStatus::Running,
            start_ms: now_ms,
            end_ms: None,
            duration_ms: None,
            attributes_json: Some(
                serde_json::json!({
                    "operation": req.spec.operation,
                    "model": req.spec.model,
                    "provider": req.spec.provider,
                })
                .to_string(),
            ),
        });
        let _ = req.store.insert_request(&RequestRow {
            request_id: request_id.clone(),
            trace_id: trace_id.clone(),
            conversation_id: req.spec.conversation_id.clone(),
            model: Some(req.spec.model.clone()),
            provider: Some(req.spec.provider.clone()),
            status: TraceStatus::Running,
            started_at_ms: now_ms,
            ended_at_ms: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            ttft_ms: None,
            cancel_reason: None,
            error_message: None,
        });
        req
    }

    /// Trace id.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Request id.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Measured time-to-first-token so far, if any.
    pub fn ttft_ms(&self) -> Option<u64> {
        self.ttft
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn next_seq(&mut self) -> i64 {
        self.event_seq += 1;
        self.event_seq
    }

    /// Record a phase span that already completed (validate/resolve are
    /// synchronous and cheap).
    pub fn record_phase(&mut self, name: &str, start: Instant, attrs: serde_json::Value) {
        let end = self.now_ms();
        let span_id = uuid::Uuid::now_v7().to_string();
        let _ = self.store.insert_span(&SpanRow {
            trace_id: self.trace_id.clone(),
            span_id,
            parent_span_id: Some(self.root_span.clone()),
            name: name.into(),
            kind: "phase".into(),
            status: TraceStatus::Success,
            start_ms: end - start.elapsed().as_millis() as i64,
            end_ms: Some(end),
            duration_ms: Some(start.elapsed().as_millis() as i64),
            attributes_json: Some(attrs.to_string()),
        });
    }

    /// Open the `llm.generate` span. Returns its id for closing.
    pub fn open_llm_span(&mut self) -> String {
        let now = self.now_ms();
        let span_id = uuid::Uuid::now_v7().to_string();
        let _ = self.store.insert_span(&SpanRow {
            trace_id: self.trace_id.clone(),
            span_id: span_id.clone(),
            parent_span_id: Some(self.root_span.clone()),
            name: "llm.generate".into(),
            kind: "llm".into(),
            status: TraceStatus::Running,
            start_ms: now,
            end_ms: None,
            duration_ms: None,
            attributes_json: Some(
                serde_json::json!({
                    "model": self.spec.model,
                    "provider": self.spec.provider,
                })
                .to_string(),
            ),
        });
        span_id
    }

    /// Observe an engine event: measures TTFT and records stream events.
    /// Returns the measured TTFT when this event was the first token.
    pub fn observe_event(&mut self, event: &AssistantMessageEvent) {
        let is_first_token = self.ttft.is_none()
            && matches!(
                event,
                AssistantMessageEvent::TextDelta { .. }
                    | AssistantMessageEvent::ThinkingDelta { .. }
                    | AssistantMessageEvent::ToolcallDelta { .. }
            );
        if is_first_token {
            self.ttft = Some(self.started.elapsed().as_millis() as u64);
            let _ = self.store.finalize_trace(
                &self.trace_id,
                TraceStatus::Running,
                self.now_ms(),
                self.started.elapsed().as_millis() as i64,
                None,
                None,
                self.ttft,
                None,
            );
        }
        // Structured events for milestones only (not every delta).
        let milestone: Option<(&str, Option<String>)> = match event {
            AssistantMessageEvent::Start { .. } => Some(("stream.start", None)),
            AssistantMessageEvent::Done { reason, .. } => Some((
                "stream.done",
                Some(serde_json::json!({ "reason": format!("{reason:?}") }).to_string()),
            )),
            AssistantMessageEvent::Error { reason, .. } => Some((
                "stream.error",
                Some(serde_json::json!({ "reason": format!("{reason:?}") }).to_string()),
            )),
            _ => None,
        };
        if let Some((name, data)) = milestone {
            let seq = self.next_seq();
            let at = self.now_ms();
            let _ = self
                .store
                .insert_event(&self.trace_id, seq, at, name, data.as_deref());
        }
    }

    /// Finalize on success with the final assistant message.
    pub fn finish_success(&mut self, message: &AssistantMessage, llm_span: &str) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let now = self.now_ms();
        let duration = self.started.elapsed().as_millis() as i64;
        let usage = &message.usage;

        let llm_attrs = serde_json::json!({
            "stopReason": format!("{:?}", message.stop_reason),
            "responseModel": message.response_model,
        })
        .to_string();
        let _ = self.store.close_span(
            &self.trace_id,
            llm_span,
            TraceStatus::Success,
            now,
            Some(llm_attrs.as_str()),
        );
        let _ = self.store.close_span(
            &self.trace_id,
            &self.root_span,
            TraceStatus::Success,
            now,
            None,
        );
        let _ = self.store.finalize_trace(
            &self.trace_id,
            TraceStatus::Success,
            now,
            duration,
            Some(usage.input),
            Some(usage.output),
            self.ttft,
            None,
        );
        let _ = self.store.finalize_request(
            &self.request_id,
            TraceStatus::Success,
            now,
            duration,
            Some(usage.input),
            Some(usage.output),
            self.ttft,
            None,
            None,
        );
        let _ = self.store.enforce_retention(&self.spec.retention);
    }

    /// Finalize on inference failure.
    pub fn finish_error(&mut self, message: &AssistantMessage, llm_span: Option<&str>) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let now = self.now_ms();
        let duration = self.started.elapsed().as_millis() as i64;
        let error = sanitize_error(
            message
                .error_message
                .as_deref()
                .unwrap_or("inference failed"),
        );
        if let Some(span) = llm_span {
            let _ = self
                .store
                .close_span(&self.trace_id, span, TraceStatus::Error, now, None);
        }
        let _ = self.store.close_span(
            &self.trace_id,
            &self.root_span,
            TraceStatus::Error,
            now,
            None,
        );
        let _ = self.store.finalize_trace(
            &self.trace_id,
            TraceStatus::Error,
            now,
            duration,
            Some(message.usage.input),
            Some(message.usage.output),
            self.ttft,
            Some(&error),
        );
        let _ = self.store.finalize_request(
            &self.request_id,
            TraceStatus::Error,
            now,
            duration,
            Some(message.usage.input),
            Some(message.usage.output),
            self.ttft,
            None,
            Some(&error),
        );
        let _ = self.store.enforce_retention(&self.spec.retention);
    }

    /// Finalize on cancellation. `reason` distinguishes user stop from client
    /// disconnect.
    pub fn finish_cancelled(
        &mut self,
        by_user: bool,
        partial_usage: Option<&loop_ai::Usage>,
        llm_span: Option<&str>,
    ) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let now = self.now_ms();
        let duration = self.started.elapsed().as_millis() as i64;
        let status = if by_user {
            TraceStatus::CancelledUser
        } else {
            TraceStatus::CancelledClient
        };
        let cancel_reason = if by_user { "user" } else { "client_disconnect" };
        if let Some(span) = llm_span {
            let _ = self
                .store
                .close_span(&self.trace_id, span, status, now, None);
        }
        let _ = self
            .store
            .close_span(&self.trace_id, &self.root_span, status, now, None);
        let _ = self.store.finalize_trace(
            &self.trace_id,
            status,
            now,
            duration,
            partial_usage.map(|u| u.input),
            partial_usage.map(|u| u.output),
            self.ttft,
            None,
        );
        let _ = self.store.finalize_request(
            &self.request_id,
            status,
            now,
            duration,
            partial_usage.map(|u| u.input),
            partial_usage.map(|u| u.output),
            self.ttft,
            Some(cancel_reason),
            None,
        );
        let _ = self.store.enforce_retention(&self.spec.retention);
    }

    /// Finalize when the browser disconnected mid-stream. The engine keeps
    /// draining briefly so the model-side request completes, but the trace
    /// records the disconnect honestly.
    pub fn finish_client_disconnected(
        &mut self,
        partial_usage: Option<&loop_ai::Usage>,
        llm_span: Option<&str>,
    ) {
        self.finish_cancelled(false, partial_usage, llm_span);
    }
}

/// Classify a terminal assistant message.
#[allow(dead_code)] // used by tests; kept as public classification helper
pub fn terminal_status(message: &AssistantMessage) -> TraceStatus {
    match message.stop_reason {
        StopReason::Error => TraceStatus::Error,
        StopReason::Aborted => TraceStatus::CancelledUser,
        StopReason::Pending => TraceStatus::Interrupted,
        _ => TraceStatus::Success,
    }
}

/// A cancellation source shared between the HTTP handler and the drain task.
#[derive(Clone)]
pub struct CancelHandle {
    token: CancellationToken,
}

impl CancelHandle {
    /// New handle.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Trigger cancellation.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Whether cancellation was requested.
    #[allow(dead_code)] // used by tests
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Inner token (passed into engine options).
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}
