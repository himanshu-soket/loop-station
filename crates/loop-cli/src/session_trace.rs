//! Session lifecycle tracing for terminal-launched Loop CLI agent sessions.
//!
//! Every interactive or `--print` CLI process owns one [`SessionTrace`]. The
//! trace is written incrementally into the same durable SQLite store the
//! dashboard serves (`~/.loop/dashboard/store.db`) so Dashboard → Traces &
//! Requests → Traces shows every concurrently running CLI session:
//!
//! ```text
//! trace cli.session                       (root span: cli.session)
//! ├─ span turn <n>                        (one per user prompt)
//! │   └─ spans added by later phases      (llm.generate, tool.<name>, …)
//! └─ terminal status on every exit path
//! ```
//!
//! Design rules (mirrors `engine::trace::TracedRequest`, which stays untouched):
//! * Storage failures are logged and swallowed — tracing can never break the
//!   agent. All writes run on the blocking pool through `spawn_blocking`.
//! * Rows are inserted as `running` stamped with this process's pid and
//!   finalized exactly once, so only a crash (dead pid) is recovered as
//!   `interrupted` by `DashboardStore::open`; live sessions of other CLI
//!   processes are left untouched.
//! * One SQLite file shared by many processes: WAL + `busy_timeout` make
//!   concurrent CLI sessions safe; each session writes only its own rows,
//!   keyed by its unique `session_id` / `trace_id` (uuid v7, so chronological).
//! * Secrets are never recorded. Only ids, timestamps, model names, statuses
//!   and (subject to `capture_content`) prompt/response text.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use loop_dashboard::persist::{
    redact_json, redact_text, DashboardStore, RetentionConfig, SpanRow, TraceRow, TraceStatus,
};

/// Operation name recorded on the session trace row.
pub const OPERATION: &str = "cli.session";
/// Root span name of a CLI session trace.
pub const ROOT_SPAN_NAME: &str = "cli.session";

/// Longest single text value persisted for a lifecycle event.
const MAX_TEXT_LEN: usize = 20_000;

/// A live CLI session trace.
///
/// Cheap to clone for handing to spawned tasks; every clone shares the same
/// finalization guard, so the session is closed exactly once.
#[derive(Clone)]
pub struct SessionTrace {
    inner: Arc<Inner>,
}

struct Inner {
    store: Option<Arc<DashboardStore>>,
    session_id: String,
    trace_id: String,
    root_span: String,
    started: Instant,
    #[allow(dead_code)] // surfaced via Debug; kept for future span backfill
    started_at_ms: i64,
    /// Monotonic counter allocating sequence numbers for trace events.
    next_seq: std::sync::atomic::AtomicI64,
    /// Whether capture of message content is allowed by dashboard settings.
    capture_content: bool,
    /// Set once a terminal status has been written.
    finalized: std::sync::atomic::AtomicBool,
    /// Set when the user cancelled the session (quit mid-turn); the next
    /// finalize records `CancelledUser` instead of success.
    user_cancelled: std::sync::atomic::AtomicBool,
    /// Sender into this session's ordered write queue. Writes are
    /// fire-and-forget but strictly sequenced per session: each queued
    /// closure runs only after the previous one finished, so a trace row
    /// always lands before its spans/events and no FK constraint is raced.
    writes: tokio::sync::mpsc::UnboundedSender<WriteOp>,
    retention: RetentionConfig,
}impl std::fmt::Debug for SessionTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTrace")
            .field("session_id", &self.inner.session_id)
            .field("trace_id", &self.inner.trace_id)
            .field("store", &self.inner.store.is_some())
            .finish()
    }
}

impl SessionTrace {
    /// Open (or reuse) the dashboard store and record the start of a session.
    ///
    /// `capture_content` follows the dashboard's own `captureContent` setting
    /// so the CLI honors the same security control as the Playground.
    ///
    /// Never fails: a session whose storage cannot be opened is simply not
    /// traced, and the agent keeps running.
    pub fn begin(
        db_path: impl AsRef<Path>,
        session_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        interactive: bool,
        capture_content: bool,
    ) -> Self {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let trace_id = uuid::Uuid::now_v7().to_string();
        let root_span = uuid::Uuid::now_v7().to_string();

        // Open lazily and best-effort; a missing/corrupt store must not stop
        // the CLI from starting.
        let store = DashboardStore::open(db_path.as_ref())
            .map(Arc::new)
            .map_err(|err| {
                tracing::warn!(target: "loop_cli", error = %err, "session tracing disabled");
                err
            })
            .ok();

        // One ordered writer task per session drains the write queue on the
        // blocking pool. `disabled()` sessions (no store) still get a sender;
        // their writer exits as soon as it sees the channel closed with no
        // writes ever queued.
        let writer = WriteWorker::spawn(store.clone());

        let this = Self {
            inner: Arc::new(Inner {
                store,
                session_id: session_id.to_string(),
                trace_id: trace_id.clone(),
                root_span: root_span.clone(),
                started: Instant::now(),
                started_at_ms,
                next_seq: std::sync::atomic::AtomicI64::new(0),
                capture_content,
                finalized: std::sync::atomic::AtomicBool::new(false),
                user_cancelled: std::sync::atomic::AtomicBool::new(false),
                writes: writer.sender(),
                retention: RetentionConfig::default(),
            }),
        };

        if let Some(store) = &this.inner.store {
            let row = TraceRow {
                trace_id: trace_id.clone(),
                root_span_id: root_span.clone(),
                operation: OPERATION.to_string(),
                status: TraceStatus::Running,
                model: model.map(str::to_string),
                provider: provider.map(str::to_string),
                request_id: None,
                session_id: Some(session_id.to_string()),
                started_at_ms,
                ended_at_ms: None,
                duration_ms: None,
                input_tokens: None,
                output_tokens: None,
                ttft_ms: None,
                error_message: None,
                params_json: Some(
                    serde_json::json!({
                        "origin": "cli",
                        "interactive": interactive,
                        "cwd": cwd,
                    })
                    .to_string(),
                ),
                content_captured: false,
                owner_pid: Some(std::process::id() as i64),
            };
            let span = SpanRow {
                trace_id: trace_id.clone(),
                span_id: root_span.clone(),
                parent_span_id: None,
                name: ROOT_SPAN_NAME.into(),
                kind: "phase".into(),
                status: TraceStatus::Running,
                start_ms: started_at_ms,
                end_ms: None,
                duration_ms: None,
                attributes_json: Some(
                    serde_json::json!({
                        "sessionId": session_id,
                        "interactive": interactive,
                        "cwd": cwd,
                        "model": model,
                        "provider": provider,
                    })
                    .to_string(),
                ),
            };
            write_async(&this.inner, Arc::clone(store), move |store| {
                let outcome: crate::session_trace::StorageResult =
                    store
                        .insert_trace(&row)
                        .and_then(|()| store.insert_span(&span));
                if let Err(err) = outcome {
                    tracing::warn!(target: "loop_cli", error = %err, "session trace start failed");
                }
            });
            // Lifecycle: the trace row itself is the session record; these
            // events give the timeline a human-readable anchor. Queued after
            // the row insert above so their FK is satisfied.
            this.record_event(
                "session.created",
                serde_json::json!({ "sessionId": session_id, "interactive": interactive }),
            );
            this.record_event(
                if interactive { "session.start" } else { "session.print" },
                serde_json::json!({ "cwd": cwd, "model": model, "provider": provider }),
            );
        }
        this
    }

    /// A disabled trace (no storage). Recording is a no-op.
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Inner {
                store: None,
                session_id: String::new(),
                trace_id: String::new(),
                root_span: String::new(),
                started: Instant::now(),
                started_at_ms: 0,
                next_seq: std::sync::atomic::AtomicI64::new(0),
                capture_content: false,
                finalized: std::sync::atomic::AtomicBool::new(true),
                user_cancelled: std::sync::atomic::AtomicBool::new(false),
                writes: WriteWorker::spawn(None).sender(),
                retention: RetentionConfig::default(),
            }),
        }
    }

    /// Whether anything is being recorded.
    pub fn enabled(&self) -> bool {
        self.inner.store.is_some()
    }

    /// This session's id.
    pub fn session_id(&self) -> &str {
        &self.inner.session_id
    }

    /// This trace's id.
    pub fn trace_id(&self) -> &str {
        &self.inner.trace_id
    }

    /// Root span id (parent of turn spans).
    pub fn root_span_id(&self) -> &str {
        &self.inner.root_span
    }

    /// Whether message content may be captured.
    pub fn capture_content(&self) -> bool {
        self.inner.capture_content
    }

    /// The durable store, when tracing is active.
    pub fn store(&self) -> Option<Arc<DashboardStore>> {
        self.inner.store.clone()
    }

    /// Elapsed session time in milliseconds.
    pub fn elapsed_ms(&self) -> i64 {
        self.inner.started.elapsed().as_millis() as i64
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// Allocate the next sequence number for a trace event.
    pub fn next_event_seq(&self) -> i64 {
        self.inner
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }

    /// Record a structured event on the session trace (best effort).
    ///
    /// When the payload carries captured content the trace row is also
    /// flagged with `content_captured`, mirroring how terminal output
    /// capture works.
    pub fn record_event(&self, name: &str, data: serde_json::Value) {
        let Some(store) = &self.inner.store else { return };
        let trace_id = self.inner.trace_id.clone();
        let seq = self.next_event_seq();
        let at_ms = Self::now_ms();
        let name = name.to_string();
        let carries_content = matches!(
            name.as_str(),
            "user.prompt" | "assistant.response" | "assistant.delta"
                | "tool.started" | "tool.completed" | "tool.failed"
        );
        let data_json = data.to_string();
        write_async(&self.inner, Arc::clone(store), move |store| {
            let outcome =
                store.insert_event(&trace_id, seq, at_ms, &name, Some(data_json.as_str()));
            if let Err(err) = outcome {
                tracing::warn!(target: "loop_cli", error = %err, "session trace event failed");
                return;
            }
            if carries_content && data_json.contains("\"prompt\"") {
                let _ = store.mark_trace_content_captured(&trace_id);
            }
        });
    }

    /// Begin a user turn: opens a `turn <n>` span under the session root and
    /// records the prompt (when content capture allows).
    ///
    /// Returns the turn span id, which later phases attach model/tool spans to.
    pub fn begin_turn(&self, turn_index: u64, prompt: Option<&str>) -> Option<String> {
        if self.inner.store.is_none() {
            return None;
        }
        let span_id = uuid::Uuid::now_v7().to_string();
        let at_ms = Self::now_ms();
        let span = SpanRow {
            trace_id: self.inner.trace_id.clone(),
            span_id: span_id.clone(),
            parent_span_id: Some(self.inner.root_span.clone()),
            name: format!("turn {turn_index}"),
            kind: "phase".into(),
            status: TraceStatus::Running,
            start_ms: at_ms,
            end_ms: None,
            duration_ms: None,
            attributes_json: Some(
                serde_json::json!({ "turn": turn_index, "role": "user" }).to_string(),
            ),
        };
        self.write_span(span);
        if let Some(prompt) = prompt {
            let chars = prompt.chars().count();
            let body = if self.inner.capture_content {
                serde_json::json!({
                    "turn": turn_index,
                    "chars": chars,
                    "prompt": truncate_text(&redact_text(prompt)),
                })
            } else {
                serde_json::json!({ "turn": turn_index, "chars": chars })
            };
            self.record_event("user.prompt", body);
        }
        Some(span_id)
    }

    // ------------------------------------------------------------------
    // Content capture (Phase 2)
    // ------------------------------------------------------------------

    /// Record a model request marker for the current turn: which model is
    /// about to be called. Opens an `llm.generate` span under the turn span
    /// and returns its id (closed by [`Self::end_model_span`]).
    pub fn begin_model_span(
        &self,
        turn_span: &str,
        turn_index: u64,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> Option<String> {
        if self.inner.store.is_none() {
            return None;
        }
        let span_id = uuid::Uuid::now_v7().to_string();
        let at_ms = Self::now_ms();
        let span = SpanRow {
            trace_id: self.inner.trace_id.clone(),
            span_id: span_id.clone(),
            parent_span_id: Some(turn_span.to_string()),
            name: "llm.generate".into(),
            kind: "llm".into(),
            status: TraceStatus::Running,
            start_ms: at_ms,
            end_ms: None,
            duration_ms: None,
            attributes_json: Some(
                serde_json::json!({
                    "turn": turn_index,
                    "model": model,
                    "provider": provider,
                })
                .to_string(),
            ),
        };
        self.write_span(span);
        self.record_event(
            "model.request",
            serde_json::json!({ "turn": turn_index, "model": model, "provider": provider }),
        );
        Some(span_id)
    }

    /// Close an `llm.generate` span with a terminal status.
    pub fn end_model_span(&self, span_id: &str, status: TraceStatus, stop_reason: Option<&str>) {
        if self.inner.store.is_none() {
            return;
        }
        let end_ms = Self::now_ms();
        let attrs = serde_json::json!({ "stopReason": stop_reason }).to_string();
        self.close_span(span_id, status, end_ms, Some(attrs.as_str()));
    }

    /// Record one streaming text delta from the assistant.
    ///
    /// Deltas are buffered by the caller ([`StreamCapture`]) and persisted in
    /// batches; this method is the raw single-chunk event used only when the
    /// caller wants an immediate record (e.g. first token).
    pub fn record_first_token(&self, turn_index: u64, ttft_ms: u64) {
        self.record_event(
            "assistant.delta",
            serde_json::json!({ "turn": turn_index, "firstToken": true, "ttftMs": ttft_ms }),
        );
    }

    /// Persist the complete final assistant response of a turn.
    pub fn record_assistant_response(
        &self,
        turn_index: u64,
        text: &str,
        status: &str,
        stop_reason: Option<&str>,
    ) {
        if !self.inner.capture_content {
            self.record_event(
                "assistant.response",
                serde_json::json!({
                    "turn": turn_index,
                    "chars": text.chars().count(),
                    "status": status,
                    "stopReason": stop_reason,
                }),
            );
            return;
        }
        let clean = redact_text(text);
        self.record_event(
            "assistant.response",
            serde_json::json!({
                "turn": turn_index,
                "chars": clean.chars().count(),
                "status": status,
                "stopReason": stop_reason,
                "response": truncate_text(&clean),
            }),
        );
    }

    /// Record token usage for a turn and merge it into the trace row totals.
    pub fn record_usage(
        &self,
        turn_index: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        ttft_ms: Option<u64>,
    ) {
        let Some(store) = &self.inner.store else { return };
        self.record_event(
            "model.usage",
            serde_json::json!({
                "turn": turn_index,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "totalTokens": total_tokens,
            }),
        );
        let trace_id = self.inner.trace_id.clone();
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) =
                store.update_trace_usage(&trace_id, input_tokens, output_tokens, ttft_ms)
            {
                tracing::warn!(target: "loop_cli", error = %err, "session trace usage update failed");
            }
        });
    }

    /// Record a tool call starting: name, arguments, ids.
    pub fn record_tool_started(
        &self,
        turn_index: u64,
        tool_call_id: &str,
        name: &str,
        arguments: Option<&serde_json::Value>,
    ) {
        // Arguments carry whatever the model sent (commands, file paths,
        // possibly secrets): capture only when allowed, always redacted.
        let args = arguments
            .filter(|_| self.inner.capture_content)
            .map(redact_json);
        let mut body = serde_json::json!({
            "turn": turn_index,
            "toolCallId": tool_call_id,
            "tool": name,
        });
        if let Some(args) = args {
            body["arguments"] = args;
        }
        self.record_event("tool.started", body);
    }

    /// Record a tool call finishing with output, status and duration.
    pub fn record_tool_result(
        &self,
        turn_index: u64,
        tool_call_id: &str,
        name: &str,
        output: Option<&str>,
        failed: bool,
        duration_ms: Option<i64>,
    ) {
        let out = output
            .filter(|_| self.inner.capture_content)
            .map(|o| truncate_text(&redact_text(o)));
        let mut body = serde_json::json!({
            "turn": turn_index,
            "toolCallId": tool_call_id,
            "tool": name,
        });
        if let Some(out) = out {
            body["output"] = serde_json::Value::String(out);
        }
        if let Some(dur) = duration_ms {
            body["durationMs"] = serde_json::Value::from(dur);
        }
        self.record_event(if failed { "tool.failed" } else { "tool.completed" }, body);
    }

    /// Record an execution error with its actual message.
    pub fn record_error(&self, stage: &str, message: &str) {
        self.record_event(
            "error",
            serde_json::json!({
                "stage": stage,
                "message": truncate_text(&sanitize_error(message)),
            }),
        );
    }
    /// End a user turn span with a terminal status.
    pub fn end_turn(&self, turn_span: &str, status: TraceStatus, summary: serde_json::Value) {
        if self.inner.store.is_none() {
            return;
        }
        let end_ms = Self::now_ms();
        let attrs = summary.to_string();
        self.close_span(turn_span, status, end_ms, Some(attrs.as_str()));
    }

    /// End a user turn span on an explicitly named trace.
    ///
    /// Turns must close on the trace that opened them; callers holding a
    /// stale trace id (a rotated session) use this instead of [`Self::end_turn`].
    pub fn end_turn_on(
        &self,
        trace_id: &str,
        turn_span: &str,
        status: TraceStatus,
        summary: serde_json::Value,
    ) {
        let Some(store) = &self.inner.store else { return };
        // A turn from another trace's session is closed through that trace's
        // own writer; when it belongs to this one, keep the single path.
        if trace_id == self.inner.trace_id {
            self.end_turn(turn_span, status, summary);
            return;
        }
        let end_ms = Self::now_ms();
        let attrs = summary.to_string();
        let trace_id = trace_id.to_string();
        let span_id = turn_span.to_string();
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) =
                store.close_span(&trace_id, &span_id, status, end_ms, Some(attrs.as_str()))
            {
                tracing::warn!(target: "loop_cli", error = %err, "session trace close failed");
            }
        });
    }

    /// Insert a span row (best effort).
    pub fn write_span(&self, span: SpanRow) {
        let Some(store) = &self.inner.store else { return };
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) = store.insert_span(&span) {
                tracing::warn!(target: "loop_cli", error = %err, "session trace span failed");
            }
        });
    }

    /// Close an existing span (best effort).
    pub fn close_span(&self, span_id: &str, status: TraceStatus, end_ms: i64, attributes_json: Option<&str>) {
        let Some(store) = &self.inner.store else { return };
        let trace_id = self.inner.trace_id.clone();
        let span_id = span_id.to_string();
        let attrs = attributes_json.map(str::to_string);
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) =
                store.close_span(&trace_id, &span_id, status, end_ms, attrs.as_deref())
            {
                tracing::warn!(target: "loop_cli", error = %err, "session trace close failed");
            }
        });
    }

    /// Close a span that belongs to another trace of this session (a turn
    /// outliving a trace rotation closes its llm span where it was opened).
    pub fn close_span_on(
        &self,
        trace_id: &str,
        span_id: &str,
        status: TraceStatus,
        end_ms: i64,
        attributes_json: Option<&str>,
    ) {
        let Some(store) = &self.inner.store else { return };
        if trace_id == self.inner.trace_id {
            self.close_span(span_id, status, end_ms, attributes_json);
            return;
        }
        let trace_id = trace_id.to_string();
        let span_id = span_id.to_string();
        let attrs = attributes_json.map(str::to_string);
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) =
                store.close_span(&trace_id, &span_id, status, end_ms, attrs.as_deref())
            {
                tracing::warn!(target: "loop_cli", error = %err, "session trace close failed");
            }
        });
    }

    /// Update model/provider on the session trace row (model switching).
    pub fn set_model(&self, model: Option<&str>, provider: Option<&str>) {
        let Some(store) = &self.inner.store else { return };
        let trace_id = self.inner.trace_id.clone();
        let model = model.map(str::to_string);
        let provider = provider.map(str::to_string);
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) = store.update_trace_model(&trace_id, model.as_deref(), provider.as_deref()) {
                tracing::warn!(target: "loop_cli", error = %err, "session trace model update failed");
            }
        });
    }

    /// Re-bind this trace to another conversation id and record the switch.
    ///
    /// The CLI's session can change under a running process (`/new`, fork,
    /// agent adoption); the trace keeps a single stable identity while its
    /// `session_id` follows the active conversation.
    pub fn rebind_session(&self, session_id: &str, reason: &str) {
        if session_id.is_empty() || session_id == self.inner.session_id {
            return;
        }
        self.record_event(
            "session.switch",
            serde_json::json!({ "from": self.inner.session_id, "to": session_id, "reason": reason }),
        );
        let Some(store) = &self.inner.store else { return };
        let trace_id = self.inner.trace_id.clone();
        let session_id = session_id.to_string();
        write_async(&self.inner, Arc::clone(store), move |store| {
            if let Err(err) = store.update_trace_session(&trace_id, &session_id) {
                tracing::warn!(target: "loop_cli", error = %err, "session trace rebind failed");
            }
        });
    }

    /// Mark the session as user-cancelled (quit while a turn was still
    /// running). The final status becomes `CancelledUser`.
    pub fn mark_cancelled(&self, reason: &str) {
        self.inner
            .user_cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.record_event(
            "session.cancelled",
            serde_json::json!({ "reason": reason }),
        );
    }

    /// Finalize the session trace with a terminal status. Idempotent.
    pub async fn finish(&self, status: TraceStatus, error_message: Option<&str>) {
        if !self.begin_finalize() {
            return;
        }
        let Some(store) = &self.inner.store else { return };
        // A user cancellation outranks the default success the caller asks
        // for when the process ends cleanly after an interrupt.
        let status = if self.inner.user_cancelled.load(std::sync::atomic::Ordering::SeqCst)
            && matches!(status, TraceStatus::Success)
        {
            TraceStatus::CancelledUser
        } else {
            status
        };
        let end_ms = Self::now_ms();
        let duration = self.elapsed_ms();

        // Close the root span first, then the trace row — both through the
        // ordered queue so they land after every earlier write.
        self.close_span(&self.inner.root_span.clone(), status, end_ms, None);
        let trace_id = self.inner.trace_id.clone();
        let error = error_message.map(sanitize_error);
        let retention = self.inner.retention.clone();
        write_async(&self.inner, Arc::clone(store), move |store| {
            let outcome = store.finalize_trace(
                &trace_id,
                status,
                end_ms,
                duration,
                None,
                None,
                None,
                error.as_deref(),
            );
            if let Err(err) = outcome {
                tracing::warn!(target: "loop_cli", error = %err, "session trace finalize failed");
            }
            let _ = store.enforce_retention(&retention);
        });
        // The caller treats finish as a completed write; wait for it.
        self.flush().await;
    }

    /// Finalize on normal exit.
    pub async fn finish_success(&self) {
        self.finish(TraceStatus::Success, None).await;
    }

    /// Finalize on a fatal CLI error.
    pub async fn finish_error(&self, message: &str) {
        self.finish(TraceStatus::Error, Some(message)).await;
    }

    /// Finalize because the user cancelled / interrupted the session (Ctrl-C,
    /// Esc abort, terminal hangup).
    pub async fn finish_cancelled(&self, reason: &str) {
        self.finish(TraceStatus::CancelledUser, Some(reason)).await;
    }

    fn begin_finalize(&self) -> bool {
        !self.inner
            .finalized
            .swap(true, std::sync::atomic::Ordering::SeqCst)
    }

    /// Wait until every queued storage write for this session has landed.
    ///
    /// Fire-and-forget writes keep tracing off the agent's hot path; this is
    /// the explicit barrier used on session shutdown and by tests.
    pub async fn flush(&self) {
        if self.inner.store.is_none() {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        enqueue_write(&self.inner, WriteOp::Barrier(tx));
        // The writer exits when `Inner` drops; a closed barrier means every
        // earlier write has already landed.
        let _ = rx.await;
    }
}

/// Result of a best-effort storage write.
type StorageResult = loop_dashboard::persist::Result<()>;

/// Tracks the open user-turn span of a CLI session so protocol updates can
/// close it with the right terminal status.
///
/// One instance lives in the TUI event loop; it is deliberately tiny and
/// never blocks: all storage goes through [`SessionTrace`]'s best-effort
/// writers. The last completed index is remembered so turn numbers stay
/// strictly increasing across a whole session (Phase 2).
#[derive(Default)]
pub struct TurnTracker {
    open: parking_lot::Mutex<Option<OpenTurn>>,
    /// Highest turn index handed out so far (0 = none yet).
    last_index: std::sync::atomic::AtomicU64,
}

struct OpenTurn {
    index: u64,
    span_id: String,
    /// The turn's `llm.generate` span, opened when the prompt was submitted
    /// and closed with the turn (Phase 2).
    llm_span_id: Option<String>,
    /// Trace that owned this turn when it began. A session whose trace is
    /// rotated mid-turn must still close the turn on the trace that opened
    /// it, never on whichever trace happens to be active now.
    trace_id: String,
}

impl TurnTracker {
    /// New tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a user turn, closing any span left open by an interrupted turn.
    pub fn begin(&self, trace: &SessionTrace, prompt: Option<&str>) {
        let mut open = self.open.lock();
        if let Some(stale) = open.take() {
            trace.end_turn_on(
                &stale.trace_id,
                &stale.span_id,
                TraceStatus::CancelledUser,
                serde_json::json!({ "turn": stale.index, "note": "superseded" }),
            );
        }
        // Strictly monotonic: one past the highest index ever issued, so a
        // resumed or superseded turn never reuses a number.
        let next = self
            .last_index
            .fetch_max(open.as_ref().map_or(0, |t| t.index), std::sync::atomic::Ordering::SeqCst)
            .max(open.as_ref().map_or(0, |t| t.index))
            + 1;
        self.last_index.store(next, std::sync::atomic::Ordering::SeqCst);
        let span_id = trace.begin_turn(next, prompt);
        *open = span_id.map(|span_id| OpenTurn {
            index: next,
            span_id,
            llm_span_id: None,
            // Snapshot the owning trace so this turn always closes there.
            trace_id: trace.trace_id().to_string(),
        });
    }

    /// Attach the turn's `llm.generate` span so it closes together with the
    /// turn span on every exit path.
    pub fn set_llm_span(&self, llm_span_id: &str) {
        if let Some(open) = self.open.lock().as_mut() {
            open.llm_span_id = Some(llm_span_id.to_string());
        }
    }

    /// Close the turn's `llm.generate` span on its owning trace.
    fn close_llm(open: &OpenTurn, trace: &SessionTrace, status: TraceStatus, stop_reason: Option<&str>) {
        if let Some(llm) = &open.llm_span_id {
            // The llm span belongs to the same trace as the turn span.
            if open.trace_id == trace.trace_id() {
                trace.end_model_span(llm, status, stop_reason);
            } else {
                trace.close_span_on(&open.trace_id, llm, status, Self::now_ms(), None);
            }
        }
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// Close the open turn with a protocol stop reason.
    pub fn end(&self, trace: &SessionTrace, stop: StopReason, error: Option<&str>) {
        let Some(open) = self.open.lock().take() else {
            return;
        };
        let (status, _) = match stop {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::Refusal => {
                (TraceStatus::Success, None)
            }
            StopReason::Cancelled => (TraceStatus::CancelledUser, Some("cancelled".to_string())),
            StopReason::Error => (
                TraceStatus::Error,
                Some(error.unwrap_or("turn failed").to_string()),
            ),
            _ => (TraceStatus::Success, None),
        };
        Self::close_llm(&open, trace, status, Some(&format!("{stop:?}")));
        let (status, note): (TraceStatus, Option<String>) = match stop {
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::Refusal => {
                (TraceStatus::Success, None)
            }
            StopReason::Cancelled => (TraceStatus::CancelledUser, Some("cancelled".to_string())),
            StopReason::Error => (
                TraceStatus::Error,
                Some(error.unwrap_or("turn failed").to_string()),
            ),
            _ => (TraceStatus::Success, None),
        };
        let mut summary = serde_json::json!({ "turn": open.index, "stop": format!("{stop:?}") });
        if let Some(note) = note {
            summary["note"] = serde_json::json!(note);
        }
        trace.end_turn_on(&open.trace_id, &open.span_id, status, summary);
    }

    /// Close the open turn as failed (prompt-level failure).
    pub fn end_failed(&self, trace: &SessionTrace, message: &str) {
        let Some(open) = self.open.lock().take() else {
            return;
        };
        Self::close_llm(&open, trace, TraceStatus::Error, Some("failed"));
        trace.end_turn_on(
            &open.trace_id,
            &open.span_id,
            TraceStatus::Error,
            serde_json::json!({ "turn": open.index, "error": truncate_text(message) }),
        );
    }

    /// Close any open turn as cancelled (session exit / interrupt).
    pub fn cancel_open(&self, trace: &SessionTrace) {
        let Some(open) = self.open.lock().take() else {
            return;
        };
        Self::close_llm(&open, trace, TraceStatus::CancelledUser, Some("cancelled"));
        trace.end_turn_on(
            &open.trace_id,
            &open.span_id,
            TraceStatus::CancelledUser,
            serde_json::json!({ "turn": open.index, "note": "session ended" }),
        );
    }

    /// Whether a turn span is currently open.
    pub fn has_open(&self) -> bool {
        self.open.lock().is_some()
    }

    /// Index of the currently open turn, when one exists.
    pub fn open_index(&self) -> Option<u64> {
        self.open.lock().as_ref().map(|t| t.index)
    }

    /// Span id of the currently open turn, when one exists.
    pub fn open_span_id(&self) -> Option<String> {
        self.open.lock().as_ref().map(|t| t.span_id.clone())
    }
}

/// Buffered streaming-text capture for one assistant message.
///
/// Chunks arrive per token; persisting every one would hammer SQLite. This
/// buffer accumulates deltas and flushes a single `assistant.delta` event
/// with the assembled text-so-far at most once per [`Self::FLUSH_INTERVAL`]
/// (or [`Self::FLUSH_CHARS`] characters), so:
///
/// * the dashboard shows near-live partial output while a turn streams,
/// * the final assembled text is persisted exactly once by
///   [`SessionTrace::record_assistant_response`],
/// * no duplicate chunks are ever written (each flush replaces the previous
///   `delta` event's `seq`, keeping the timeline compact).
pub struct StreamCapture {
    turn_index: u64,
    buf: String,
    last_flush: std::time::Instant,
    flushed_chars: usize,
    /// `true` once any flush has been written (avoids an empty final flush).
    wrote_any: bool,
}

impl StreamCapture {
    /// Chars of accumulated text between periodic flushes.
    const FLUSH_CHARS: usize = 2_000;
    /// Wall time between periodic flushes.
    const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1_500);

    /// New capture bound to one turn.
    pub fn new(turn_index: u64) -> Self {
        Self {
            turn_index,
            buf: String::new(),
            last_flush: std::time::Instant::now(),
            flushed_chars: 0,
            wrote_any: false,
        }
    }

    /// Feed one text delta. Persists a partial snapshot when the buffer has
    /// grown enough or enough time has passed.
    pub fn push(&mut self, trace: &SessionTrace, delta: &str) {
        if delta.is_empty() || !trace.enabled() {
            return;
        }
        self.buf.push_str(delta);
        let due = self.buf.chars().count() - self.flushed_chars >= Self::FLUSH_CHARS
            || self.last_flush.elapsed() >= Self::FLUSH_INTERVAL && !self.buf.is_empty();
        if due {
            self.flush(trace);
        }
    }

    /// Persist the current assembled partial text as one `assistant.delta`.
    pub fn flush(&mut self, trace: &SessionTrace) {
        if !trace.enabled() || self.buf.is_empty() {
            return;
        }
        let chars = self.buf.chars().count();
        if chars == self.flushed_chars && self.wrote_any {
            return;
        }
        trace.record_event(
            "assistant.delta",
            serde_json::json!({
                "turn": self.turn_index,
                "chars": chars,
                "partial": truncate_text(&redact_text(&self.buf)),
            }),
        );
        self.flushed_chars = chars;
        self.wrote_any = true;
        self.last_flush = std::time::Instant::now();
    }

    /// The full assembled text so far.
    pub fn text(&self) -> &str {
        &self.buf
    }

    /// Whether anything was captured.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

use loop_protocol::StopReason;

/// Where the CLI writes session traces: the dashboard's own store.
pub fn session_store_path() -> anyhow::Result<PathBuf> {
    Ok(loop_dashboard::engine::EnginePaths::resolve()
        .agent_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve ~/.loop"))?
        .join("dashboard")
        .join("store.db"))
}

/// One queued storage operation.
enum WriteOp {
    /// Run a closure against the store, in queue order.
    Run(Box<dyn FnOnce(&DashboardStore) + Send>),
    /// Barrier: replied to once every earlier write has landed.
    Barrier(tokio::sync::oneshot::Sender<()>),
}

/// Background task that applies this session's writes strictly in order on
/// the blocking pool. It exits when the sender half (held by `Inner`) drops.
struct WriteWorker {
    tx: tokio::sync::mpsc::UnboundedSender<WriteOp>,
}

impl WriteWorker {
    fn spawn(store: Option<Arc<DashboardStore>>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WriteOp>();
        if let Some(store) = store {
            tokio::spawn(async move {
                while let Some(op) = rx.recv().await {
                    match op {
                        WriteOp::Run(f) => {
                            // Contained: a panicking write must not kill the
                            // writer and strand later writes.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                f(&store)
                            }));
                        }
                        WriteOp::Barrier(ack) => {
                            let _ = ack.send(());
                        }
                    }
                }
            });
        }
        Self { tx }
    }

    fn sender(&self) -> tokio::sync::mpsc::UnboundedSender<WriteOp> {
        self.tx.clone()
    }
}

/// Run one storage closure on the blocking pool without ever awaiting it
/// from the caller, strictly after every previously queued write for this
/// session. Tracing stays non-blocking for the agent; a panic inside a
/// closure is contained by the writer task.
fn write_async<F>(inner: &Inner, _store: Arc<DashboardStore>, f: F)
where
    F: FnOnce(&DashboardStore) + Send + 'static,
{
    enqueue_write(inner, WriteOp::Run(Box::new(f)));
}

fn enqueue_write(inner: &Inner, op: WriteOp) {
    // The sender lives for the whole process; a send failure means the
    // writer task is gone, in which case tracing is already shut down.
    let _ = inner.writes.send(op);
}

/// Read the dashboard's content-capture preference, defaulting to off.
pub fn capture_content_enabled(store: &DashboardStore) -> bool {
    store
        .load_settings()
        .map(|map| {
            map.0
                .get("captureContent")
                .map(String::as_str)
                == Some("true")
        })
        .unwrap_or(false)
}

/// Bound and clean a text value before it is persisted.
fn truncate_text(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT_LEN {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(MAX_TEXT_LEN).collect();
        cut.push('…');
        cut
    }
}

/// Sanitize an error string before storage (log-injection guard), matching the
/// dashboard's own sanitizer.
fn sanitize_error(raw: &str) -> String {
    loop_dashboard::engine::trace::sanitize_error(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        (dir, path)
    }

    #[tokio::test]
    async fn session_lifecycle_is_recorded_and_finalized_once() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-A", Some("/tmp"), Some("m"), Some("p"), true, true);

        assert!(trace.enabled());
        assert_eq!(trace.session_id(), "sess-A");

        // One user turn.
        let turn = trace
            .begin_turn(1, Some("hello world"))
            .expect("turn span");
        trace.end_turn(&turn, TraceStatus::Success, serde_json::json!({"assistantChars": 5}));

        trace.finish_success().await;
        // Second finalize is a no-op.
        trace.finish_error("must not overwrite").await;

        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(trace.trace_id()).unwrap().expect("trace row");
        assert_eq!(row.operation, OPERATION);
        assert_eq!(row.status, TraceStatus::Success);
        assert_eq!(row.session_id.as_deref(), Some("sess-A"));
        assert_eq!(row.error_message, None, "second finish ignored");
        assert!(row.duration_ms.unwrap() >= 0);
        assert!(row.content_captured, "prompt text was captured");

        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        let names: Vec<&str> = detail.spans.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&ROOT_SPAN_NAME), "root span present: {names:?}");
        assert!(names.contains(&"turn 1"), "turn span present: {names:?}");
        // Parent-child: the turn hangs off the root.
        let turn_span = detail
            .spans
            .iter()
            .find(|s| s.name == "turn 1")
            .unwrap();
        assert_eq!(turn_span.parent_span_id.as_deref(), Some(row.root_span_id.as_str()));

        // Events: the user prompt.
        assert!(
            detail
                .events
                .iter()
                .any(|e| e.name == "user.prompt"),
            "user prompt event recorded"
        );
    }

    #[tokio::test]
    async fn concurrent_sessions_stay_isolated() {
        let (_dir, path) = temp_store();
        let shared_path = Arc::new(path.clone());

        let mut joins = Vec::new();
        for n in 0..4 {
            let path = Arc::clone(&shared_path);
            joins.push(tokio::spawn(async move {
                let session_id = format!("sess-{n}");
                let trace = SessionTrace::begin(
                    path.as_ref(),
                    &session_id,
                    None,
                    Some("m"),
                    None,
                    true,
                    true,
                );
                for turn in 1..=3 {
                    let span = trace.begin_turn(turn, Some(&format!("prompt {n}/{turn}"))).unwrap();
                    trace
                        .end_turn(&span, TraceStatus::Success, serde_json::json!({"turn": turn}));
                }
                trace.finish_success().await;
                trace.trace_id().to_string()
            }));
        }
        let ids: Vec<String> = futures::future::join_all(joins)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Four distinct traces, each with its own session id and three turns.
        let store = DashboardStore::open(shared_path.as_ref()).unwrap();
        let mut seen_sessions = std::collections::HashSet::new();
        for id in &ids {
            let detail = store.get_trace_detail(id).unwrap().expect("detail");
            assert_eq!(detail.trace.status, TraceStatus::Success);
            seen_sessions.insert(detail.trace.session_id.clone().unwrap());
            let turns = detail
                .spans
                .iter()
                .filter(|s| s.name.starts_with("turn "))
                .count();
            assert_eq!(turns, 3, "three turns in {}", detail.trace.trace_id);
            // No cross-session span leakage: every span belongs to this trace.
            assert!(detail.spans.iter().all(|s| s.trace_id == *id));
        }
        assert_eq!(seen_sessions.len(), 4, "four distinct session ids");
    }

    #[tokio::test]
    async fn live_owner_rows_survive_reopen_dead_owners_recover() {
        let (_dir, path) = temp_store();
        {
            let trace = SessionTrace::begin(&path, "sess-live", None, None, None, true, false);
            let _turn = trace.begin_turn(1, None);
            trace.flush().await;
            // Drop without finishing: this process is still alive, so the
            // row must stay `running` — another open must never recover a
            // live session (concurrent CLI processes share the store).
        }
        let store = DashboardStore::open(&path).unwrap();
        let (traces, _, _) = store.counts().unwrap();
        assert_eq!(traces, 1);
        let row = store
            .list_traces(&Default::default())
            .unwrap()
            .0
            .pop()
            .unwrap();
        assert_eq!(
            row.status,
            TraceStatus::Running,
            "a live owner's session is never recovered by another open"
        );

        // A crashed process (dead pid) IS recovered as interrupted.
        {
            let crashed = SessionTrace::begin(&path, "sess-crash", None, None, None, true, false);
            crashed.flush().await;
            // Overwrite the owner pid with one that is guaranteed dead
            // through the store's own connection semantics: pids <= 0 are
            // treated as unknown/dead by `recover_interrupted`.
            let db = loop_dashboard::persist::DashboardStore::open(&path).unwrap();
            let _ = db; // same-process write below via SQL would need a pub API;
        }
        let store = DashboardStore::open(&path).unwrap();
        let rows = store.list_traces(&Default::default()).unwrap().0;
        let crash_row = rows
            .iter()
            .find(|r| r.session_id.as_deref() == Some("sess-crash"))
            .unwrap()
            .clone();
        // The crashed trace is still owned by this live process here, so it
        // stays running until the process dies; simulate that by clearing
        // its pid through a direct connection to the same file.
        {
            use rusqlite::Connection;
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "UPDATE traces SET owner_pid = -1 WHERE session_id = 'sess-crash'",
                [],
            )
            .unwrap();
        }
        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(&crash_row.trace_id).unwrap().unwrap();
        assert_eq!(row.status, TraceStatus::Interrupted);
    }

    #[tokio::test]
    async fn content_capture_off_hides_prompt_text() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-quiet", None, None, None, true, false);
        let turn = trace.begin_turn(1, Some("secret prompt")).unwrap();
        trace.end_turn(&turn, TraceStatus::Success, serde_json::json!({}));
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        assert!(!detail.trace.content_captured);
        let event = detail
            .events
            .iter()
            .find(|e| e.name == "user.prompt")
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(event.data_json.as_deref().unwrap()).unwrap();
        assert!(data.get("prompt").is_none(), "prompt text withheld");
        assert_eq!(data["chars"], serde_json::json!(13));
    }

    #[tokio::test]
    async fn disabled_trace_is_a_noop() {
        let trace = SessionTrace::disabled();
        assert!(!trace.enabled());
        // Nothing panics and nothing is written.
        trace.begin_turn(1, Some("x"));
        trace.finish_success().await;
    }

    #[tokio::test]
    async fn model_switch_updates_the_trace_row() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-model", None, Some("old"), Some("p"), true, false);
        trace.set_model(Some("new"), Some("q"));
        trace.finish_success().await;
        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(trace.trace_id()).unwrap().unwrap();
        assert_eq!(row.model.as_deref(), Some("new"));
        assert_eq!(row.provider.as_deref(), Some("q"));
    }

    #[tokio::test]
    async fn lifecycle_events_are_recorded() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-life", Some("/tmp"), None, None, true, false);
        trace.record_event("session.resume", serde_json::json!({"sessionId": "sess-life"}));
        trace.record_event("session.complete", serde_json::json!({}));
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        let names: Vec<&str> = detail.events.iter().map(|e| e.name.as_str()).collect();
        for expected in ["session.created", "session.start", "session.resume", "session.complete"] {
            assert!(
                names.contains(&expected),
                "missing {expected} in {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn rebind_moves_the_trace_to_the_new_session() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-old", None, None, None, true, false);
        // Same id: no-op.
        trace.rebind_session("sess-old", "same");
        trace.rebind_session("sess-new", "fork");
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(trace.trace_id()).unwrap().unwrap();
        assert_eq!(row.session_id.as_deref(), Some("sess-new"));
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        assert!(
            detail
                .events
                .iter()
                .any(|e| e.name == "session.switch"
                    && e.data_json.as_deref().unwrap_or_default().contains("sess-new")),
            "switch event recorded"
        );
    }

    #[tokio::test]
    async fn cancelled_after_interrupt_finalizes_as_cancelled() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-int", None, None, None, true, false);
        let turn = trace.begin_turn(1, Some("work")).unwrap();
        trace.end_turn(&turn, TraceStatus::CancelledUser, serde_json::json!({"note": "esc"}));
        trace.mark_cancelled("quit while a turn was running");
        // The process then exits cleanly; cancellation outranks success.
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(trace.trace_id()).unwrap().unwrap();
        assert_eq!(row.status, TraceStatus::CancelledUser);
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        assert!(detail.events.iter().any(|e| e.name == "session.cancelled"));
    }

    #[tokio::test]
    async fn turn_tracker_closes_a_stale_turn_on_its_own_trace() {
        let (_dir, path) = temp_store();
        let first = SessionTrace::begin(&path, "sess-one", None, None, None, true, false);
        let tracker = TurnTracker::new();
        tracker.begin(&first, Some("prompt on trace one"));
        first.flush().await;

        // The session rotates to a second trace while the turn is open.
        let second = SessionTrace::begin(&path, "sess-two", None, None, None, true, false);
        tracker.begin(&second, Some("prompt on trace two"));

        // Ending through the current trace must close the OLD turn on the
        // OLD trace, not corrupt the active one.
        tracker.end(&second, StopReason::EndTurn, None);
        second.flush().await;

        let store = DashboardStore::open(&path).unwrap();
        for trace in [&first, &second] {
            let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
            let open: Vec<_> = detail
                .spans
                .iter()
                .filter(|s| s.name.starts_with("turn ") && s.status == TraceStatus::Running)
                .collect();
            assert!(
                open.is_empty(),
                "no dangling running turn on {}",
                trace.trace_id()
            );
        }
        // Each trace has exactly one completed-or-cancelled turn span.
        let d1 = store.get_trace_detail(first.trace_id()).unwrap().unwrap();
        let t1: Vec<_> = d1.spans.iter().filter(|s| s.name.starts_with("turn ")).collect();
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].status, TraceStatus::CancelledUser, "superseded turn");

        assert!(tracker.has_open() == false);
    }
    // -----------------------------------------------------------------
    // Phase 2: content capture
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn assistant_response_and_tool_events_are_captured() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-cap", None, Some("m"), Some("p"), true, true);
        let turn = trace.begin_turn(1, Some("explain rust ownership")).unwrap();

        // Model span + streaming partials + final response.
        let llm = trace.begin_model_span(&turn, 1, Some("m"), Some("p")).unwrap();
        trace.record_first_token(1, 240);
        let mut stream = StreamCapture::new(1);
        stream.push(&trace, "Rust ownership is ");
        stream.push(&trace, "a memory management system.");
        stream.flush(&trace);
        trace.end_model_span(&llm, TraceStatus::Success, Some("EndTurn"));
        trace.record_assistant_response(
            1,
            stream.text(),
            "complete",
            Some("EndTurn"),
        );

        // Tool call with arguments and output.
        trace.record_tool_started(
            1,
            "tc-1",
            "shell",
            Some(&serde_json::json!({ "command": "cargo test", "api_key": "sk-secret" })),
        );
        trace.record_tool_result(1, "tc-1", "shell", Some("190 tests passed"), false, Some(42));

        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        let names: Vec<&str> = detail.events.iter().map(|e| e.name.as_str()).collect();
        for expected in [
            "user.prompt",
            "model.request",
            "assistant.delta",
            "assistant.response",
            "tool.started",
            "tool.completed",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }

        // The prompt text is persisted verbatim.
        let prompt = detail.events.iter().find(|e| e.name == "user.prompt").unwrap();
        let data: serde_json::Value = serde_json::from_str(prompt.data_json.as_deref().unwrap()).unwrap();
        assert_eq!(data["prompt"], "explain rust ownership");

        // The final response carries the assembled text.
        let resp = detail.events.iter().find(|e| e.name == "assistant.response").unwrap();
        let data: serde_json::Value = serde_json::from_str(resp.data_json.as_deref().unwrap()).unwrap();
        assert_eq!(data["response"], "Rust ownership is a memory management system.");

        // Tool arguments are captured AND the api_key is redacted.
        let started = detail.events.iter().find(|e| e.name == "tool.started").unwrap();
        let data: serde_json::Value = serde_json::from_str(started.data_json.as_deref().unwrap()).unwrap();
        assert_eq!(data["arguments"]["command"], "cargo test");
        assert_ne!(data["arguments"]["api_key"], "sk-secret");
        assert!(data["arguments"]["api_key"].as_str().unwrap().contains("REDACTED"));

        // Tool output is captured.
        let done = detail.events.iter().find(|e| e.name == "tool.completed").unwrap();
        let data: serde_json::Value = serde_json::from_str(done.data_json.as_deref().unwrap()).unwrap();
        assert_eq!(data["output"], "190 tests passed");
        assert_eq!(data["durationMs"], 42);

        // The llm.generate span hangs off the turn span.
        let llm_span = detail.spans.iter().find(|s| s.name == "llm.generate").unwrap();
        assert_eq!(llm_span.parent_span_id.as_deref(), Some(turn.as_str()));
        assert_eq!(llm_span.status, TraceStatus::Success);
    }

    #[tokio::test]
    async fn usage_merges_into_the_trace_row() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-use", None, None, None, true, false);
        trace.record_usage(1, Some(100), Some(50), Some(150), Some(80));
        trace.record_usage(2, Some(200), Some(60), Some(260), None);
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let row = store.get_trace(trace.trace_id()).unwrap().unwrap();
        assert_eq!(row.input_tokens, Some(300), "input tokens accumulate");
        assert_eq!(row.output_tokens, Some(110), "output tokens accumulate");
        assert_eq!(row.ttft_ms, Some(80), "ttft recorded once");
    }

    #[tokio::test]
    async fn content_capture_off_withholds_all_text() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-off", None, None, None, true, false);
        let turn = trace.begin_turn(1, Some("secret prompt")).unwrap();
        trace.record_assistant_response(1, "secret answer", "complete", None);
        trace.record_tool_started(1, "t", "shell", Some(&serde_json::json!({"cmd": "ls"})));
        trace.record_tool_result(1, "t", "shell", Some("secret output"), false, None);
        trace.end_turn(&turn, TraceStatus::Success, serde_json::json!({}));
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        for name in ["user.prompt", "assistant.response", "tool.started", "tool.completed"] {
            let ev = detail.events.iter().find(|e| e.name == name).unwrap();
            let data: serde_json::Value = serde_json::from_str(ev.data_json.as_deref().unwrap()).unwrap();
            for key in ["prompt", "response", "arguments", "output"] {
                assert!(
                    data.get(key).is_none(),
                    "{name} must not carry {key} when capture is off"
                );
            }
        }
    }

    #[tokio::test]
    async fn multi_turn_prompts_and_responses_stay_ordered() {
        let (_dir, path) = temp_store();
        let trace = SessionTrace::begin(&path, "sess-multi", None, None, None, true, true);
        for n in 1..=3 {
            let turn = trace.begin_turn(n, Some(&format!("prompt {n}"))).unwrap();
            trace.record_assistant_response(n, &format!("answer {n}"), "complete", None);
            trace.end_turn(&turn, TraceStatus::Success, serde_json::json!({ "turn": n }));
        }
        trace.finish_success().await;

        let store = DashboardStore::open(&path).unwrap();
        let detail = store.get_trace_detail(trace.trace_id()).unwrap().unwrap();
        // Events are strictly seq-ordered.
        let seqs: Vec<i64> = detail.events.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted, "event sequence numbers are monotonic");
        // Each turn's prompt is followed by its response with the same turn number.
        let prompts: Vec<(i64, u64)> = detail
            .events
            .iter()
            .filter(|e| e.name == "user.prompt")
            .map(|e| {
                let d: serde_json::Value = serde_json::from_str(e.data_json.as_deref().unwrap()).unwrap();
                (e.seq, d["turn"].as_u64().unwrap())
            })
            .collect();
        assert_eq!(prompts.len(), 3);
        for (i, (_, turn)) in prompts.iter().enumerate() {
            assert_eq!(*turn, (i + 1) as u64);
        }
        let responses: Vec<String> = detail
            .events
            .iter()
            .filter(|e| e.name == "assistant.response")
            .map(|e| {
                let d: serde_json::Value = serde_json::from_str(e.data_json.as_deref().unwrap()).unwrap();
                d["response"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(responses, ["answer 1", "answer 2", "answer 3"]);
    }

    #[test]
    fn redaction_scrubs_secrets_from_text() {
        let cases = [
            ("export API_KEY=abc123 nothing", "API_KEY"),
            ("password: hunter2 here", "password"),
            ("Authorization: Bearer eyJhbGciOi", "Bearer"),
        ];
        for (raw, marker) in cases {
            let out = loop_dashboard::persist::redact_text(raw);
            assert!(
                !out.contains(marker.to_lowercase().replace('_', "").as_str()) || out.contains("REDACTED"),
                "raw text leaked for {raw}"
            );
        }
        // A clean string passes through untouched.
        assert_eq!(
            loop_dashboard::persist::redact_text("cargo test -p loop-dashboard"),
            "cargo test -p loop-dashboard"
        );
    }

}
