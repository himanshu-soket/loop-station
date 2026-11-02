//! Chat playground: conversation state + traced streaming inference.
//!
//! One `POST /api/chat` starts a real engine stream wrapped in
//! [`TracedRequest`]. Events flow to the browser over SSE (server-sent
//! events on a POST body via `fetch` reader). Stop-generation cancels the
//! real underlying request through `loop-ai`'s cancellation token.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::engine::params::InferenceParams;
use crate::engine::trace::{CancelHandle, TracedRequest, TracedRequestSpec};
use crate::engine::Engine;
use crate::persist::{ConversationRow, DashboardStore, MessageRow, RetentionConfig};

/// Errors surfaced to the chat API.
#[derive(Debug, thiserror::Error)]
pub enum PlaygroundError {
    /// Unknown provider/model.
    #[error("unknown model {provider}/{model}")]
    UnknownModel {
        /// Provider id.
        provider: String,
        /// Model id.
        model: String,
    },
    /// Invalid parameters.
    #[error("{0}")]
    InvalidParams(String),
    /// Unknown conversation.
    #[error("conversation not found")]
    UnknownConversation,
    /// Storage failure.
    #[error("storage error: {0}")]
    Storage(#[from] crate::persist::PersistError),
    /// Engine-level failure (message is already sanitized).
    #[error("{0}")]
    Engine(
        /// Sanitized message.
        String,
    ),
}

/// A chat request from the browser.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    /// Conversation id (created when absent).
    pub conversation_id: Option<String>,
    /// User message text.
    pub message: String,
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Inference parameters.
    #[serde(default)]
    pub params: InferenceParams,
}/// One SSE event sent to the browser.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    /// Stream established; ids for later correlation.
    Started {
        /// Conversation id.
        conversation_id: String,
        /// Trace id.
        trace_id: String,
        /// Request id.
        request_id: String,
        /// Persisted user message id.
        user_message_id: String,
        /// Streaming assistant message id.
        assistant_message_id: String,
    },
    /// A text delta.
    TextDelta {
        /// Fragment.
        delta: String,
    },
    /// A thinking delta.
    ThinkingDelta {
        /// Fragment.
        delta: String,
    },
    /// Terminal success.
    Done {
        /// Full assistant text.
        text: String,
        /// Input tokens, when reported.
        input_tokens: Option<u64>,
        /// Output tokens, when reported.
        output_tokens: Option<u64>,
        /// Time to first token (ms), when measured.
        ttft_ms: Option<u64>,
        /// Total duration (ms).
        duration_ms: u64,
    },
    /// Terminal failure.
    Failed {
        /// Sanitized error text.
        error: String,
        /// Partial text generated before the failure.
        partial_text: String,
    },
    /// Terminal cancellation (user stop or client disconnect).
    Cancelled {
        /// `user` | `client_disconnect`.
        reason: String,
        /// Partial text generated before cancellation.
        partial_text: String,
    },
}

/// Shared playground state.
pub struct Playground {
    /// Real inference engine.
    pub engine: Arc<Engine>,
    /// Durable store.
    pub store: Arc<DashboardStore>,
    /// Retention policy.
    pub retention: RetentionConfig,
    /// Live generations by request id, for user cancellation.
    live: Mutex<HashMap<String, CancelHandle>>,
}

impl Playground {
    /// New playground.
    pub fn new(engine: Arc<Engine>, store: Arc<DashboardStore>) -> Self {
        Self {
            engine,
            store,
            retention: RetentionConfig::default(),
            live: Mutex::new(HashMap::new()),
        }
    }

    /// How many generations are currently running.
    pub fn active_generations(&self) -> usize {
        self.live.lock().len()
    }

    /// Register/unregister helpers for the pump task.
    fn register_live(&self, request_id: &str, handle: CancelHandle) {
        self.live.lock().insert(request_id.to_string(), handle);
    }

    fn unregister_live(&self, request_id: &str) {
        self.live.lock().remove(request_id);
    }

    /// Stop a live generation by request id. Cancels the real engine request;
    /// returns whether a matching generation was found.
    pub fn cancel_generation(&self, request_id: &str) -> bool {
        let handle = self.live.lock().get(request_id).cloned();
        match handle {
            Some(h) => {
                h.cancel();
                true
            }
            None => false,
        }
    }

    /// Create a conversation owned by `owner_account_id`.
    ///
    /// The owner is the authenticated caller's account id, resolved from the
    /// verified server-side session by the HTTP layer — never from the
    /// request body.
    pub fn create_conversation(
        &self,
        owner_account_id: i64,
        title: &str,
        model: Option<&str>,
    ) -> Result<ConversationRow, PlaygroundError> {
        let now = chrono::Utc::now().timestamp_millis();
        let row = ConversationRow {
            conversation_id: uuid::Uuid::now_v7().to_string(),
            owner_account_id: Some(owner_account_id),
            title: title.chars().take(120).collect(),
            created_at_ms: now,
            updated_at_ms: now,
            model: model.map(String::from),
        };
        self.store.create_conversation(&row)?;
        Ok(row)
    }

    /// Ensure a conversation exists (creating it when the request carries no
    /// id) **and** belongs to `owner_account_id`.
    ///
    /// A conversation id that exists but belongs to somebody else is refused
    /// exactly like an unknown id: the caller learns nothing about whose
    /// conversation it is.
    fn ensure_conversation(
        &self,
        owner_account_id: i64,
        conversation_id: Option<&str>,
        first_message: &str,
        model: Option<&str>,
    ) -> Result<String, PlaygroundError> {
        if let Some(id) = conversation_id.filter(|s| !s.trim().is_empty()) {
            if self
                .store
                .get_conversation_for_owner(id, owner_account_id)?
                .is_some()
            {
                return Ok(id.to_string());
            }
            // Unknown or someone else's: indistinguishable by design.
            return Err(PlaygroundError::UnknownConversation);
        }
        // Title from the first words of the first message.
        let title: String = first_message.trim().chars().take(60).collect();
        let row = self.create_conversation(owner_account_id, &title, model)?;
        Ok(row.conversation_id)
    }

    /// Run one traced chat turn. Returns the SSE receiver after persisting the
    /// user message and starting the engine stream.
    ///
    /// `owner_account_id` is the authenticated caller's account id, resolved
    /// from the verified server-side session by the HTTP layer. Every write
    /// below is scoped to conversations owned by that account; a conversation
    /// id belonging to another user is rejected before anything is stored.
    ///
    /// The returned [`CancelHandle`] cancels the real engine request.
    pub async fn start_chat(
        self: &Arc<Self>,
        owner_account_id: i64,
        request: ChatRequest,
    ) -> Result<(mpsc::Receiver<ChatEvent>, CancelHandle), PlaygroundError> {
        // ---- validation (before any storage) ----
        if request.message.trim().is_empty() {
            return Err(PlaygroundError::InvalidParams(
                "message must not be empty".into(),
            ));
        }
        if request.message.chars().count() > 200_000 {
            return Err(PlaygroundError::InvalidParams(
                "message too long (max 200000 characters)".into(),
            ));
        }
        request
            .params
            .validate()
            .map_err(|e| PlaygroundError::InvalidParams(e.to_string()))?;

        let Some(model) = self.engine.get_model(&request.provider, &request.model) else {
            // Distinguish "engine has nothing at all" from "wrong id": the
            // provider registry itself tells us whether anything is wired
            // (keyless local servers count).
            if self.engine.provider_count() == 0 {
                return Err(PlaygroundError::Engine(
                    "no LLM provider is connected: run `loop` and use /login, or set an API key                      environment variable"
                        .into(),
                ));
            }
            return Err(PlaygroundError::UnknownModel {
                provider: request.provider.clone(),
                model: request.model.clone(),
            });
        };

        // ---- conversation + user message persistence (ownership-scoped) ----
        let conversation_id = self.ensure_conversation(
            owner_account_id,
            request.conversation_id.as_deref(),
            &request.message,
            Some(&request.model),
        )?;

        let now_ms = chrono::Utc::now().timestamp_millis();
        let existing = self.store.list_messages(&conversation_id)?;
        let next_seq = existing.last().map(|m| m.seq + 1).unwrap_or(1);
        let user_message_id = uuid::Uuid::now_v7().to_string();

        // Build context from persisted history (user + complete assistant only:
        // failed/cancelled turns are never misrepresented as successful context).
        let mut messages: Vec<loop_ai::Message> = Vec::new();
        for m in &existing {
            match m.role.as_str() {
                "user" => messages.push(loop_ai::Message::user_text(m.content.clone())),
                "assistant" if m.status == "complete" => {
                    messages.push(loop_ai::Message::Assistant(loop_ai::AssistantMessage {
                        content: vec![loop_ai::AssistantContent::Text(loop_ai::TextContent {
                            text: m.content.clone(),
                            text_signature: None,
                        })],
                        api: String::new(),
                        provider: String::new(),
                        model: m.model.clone().unwrap_or_default(),
                        response_model: None,
                        response_id: None,
                        usage: loop_ai::Usage::empty(),
                        stop_reason: loop_ai::StopReason::Stop,
                        error_message: None,
                        raw_stop_reason: None,
                        timestamp: m.created_at_ms,
                    }));
                }
                _ => {}
            }
        }
        messages.push(loop_ai::Message::user_text(request.message.clone()));

        let stored = self.store.append_message_for_owner(
            &MessageRow {
                conversation_id: conversation_id.clone(),
                message_id: user_message_id.clone(),
                seq: next_seq,
                role: "user".into(),
                content: request.message.clone(),
                created_at_ms: now_ms,
                status: "complete".into(),
                error_message: None,
                model: None,
                request_id: None,
                trace_id: None,
            },
            owner_account_id,
        )?;
        if !stored {
            // The conversation stopped belonging to this caller between the
            // check above and this insert (e.g. an administrator reassigned
            // it). Refuse rather than write into someone else's history.
            return Err(PlaygroundError::UnknownConversation);
        }

        // Assistant placeholder (streaming).
        let assistant_message_id = uuid::Uuid::now_v7().to_string();
        let stored = self.store.append_message_for_owner(
            &MessageRow {
                conversation_id: conversation_id.clone(),
                message_id: assistant_message_id.clone(),
                seq: next_seq + 1,
                role: "assistant".into(),
                content: String::new(),
                created_at_ms: now_ms,
                status: "streaming".into(),
                error_message: None,
                model: Some(request.model.clone()),
                request_id: None,
                trace_id: None,
            },
            owner_account_id,
        )?;
        if !stored {
            return Err(PlaygroundError::UnknownConversation);
        }

        // ---- traced engine call ----
        let cancel = CancelHandle::new();
        let mut options = loop_ai::SimpleStreamOptions::default();
        options.base.cancel = Some(cancel.token());
        options.base.temperature = request.params.temperature;
        options.base.max_tokens = request.params.max_tokens;
        options.reasoning = request.params.reasoning.and_then(|r| r.to_thinking());

        let mut traced = TracedRequest::begin(
            self.store.clone(),
            TracedRequestSpec {
                operation: "dashboard.chat",
                provider: request.provider.clone(),
                model: request.model.clone(),
                conversation_id: Some(conversation_id.clone()),
                params_json: request.params.trace_json(),
                retention: self.retention.clone(),
            },
        );

        // Link the assistant message to its trace/request.
        let _ = self.store.update_message(
            &conversation_id,
            &assistant_message_id,
            "",
            "streaming",
            None,
        );

        let validate_start = std::time::Instant::now();
        traced.record_phase(
            "request.validate",
            validate_start,
            serde_json::json!({ "messageChars": request.message.chars().count() }),
        );
        let resolve_start = std::time::Instant::now();
        traced.record_phase(
            "model.resolve",
            resolve_start,
            serde_json::json!({ "provider": request.provider, "model": request.model }),
        );

        let llm_span = traced.open_llm_span();
        let trace_id = traced.trace_id().to_string();
        let request_id = traced.request_id().to_string();

        // Backfill ids onto the assistant message row.
        let _ = self.store.with_conn(|c| {
            c.execute(
                "UPDATE conversation_messages SET request_id=?2, trace_id=?3
                 WHERE conversation_id=?1 AND message_id=?4",
                rusqlite::params![conversation_id, request_id, trace_id, assistant_message_id],
            )?;
            Ok(())
        });

        let (tx, rx) = mpsc::channel::<ChatEvent>(256);

        self.register_live(traced.request_id(), cancel.clone());

        let started = ChatEvent::Started {
            conversation_id: conversation_id.clone(),
            trace_id: trace_id.clone(),
            request_id: request_id.clone(),
            user_message_id,
            assistant_message_id: assistant_message_id.clone(),
        };
        // The channel is fresh; a failure here means the caller vanished instantly.
        let _ = tx.send(started).await;

        // ---- stream pump ----
        let pg = Arc::clone(self);
        let request_id_for_live = traced.request_id().to_string();
        let conversation_id_cl = conversation_id.clone();
        let assistant_id_cl = assistant_message_id.clone();
        let cancel_pump = cancel.clone();
        let pg_for_unregister = Arc::clone(&pg);
        tokio::spawn(async move {
            run_chat_pump(
                pg,
                model,
                messages,
                options,
                traced,
                llm_span,
                tx,
                conversation_id_cl,
                assistant_id_cl,
                cancel_pump,
            )
            .await;
            pg_for_unregister.unregister_live(&request_id_for_live);
        });

        Ok((rx, cancel))
    }
}

/// The stream pump, as a free async function so the registry cleanup is
/// guaranteed on every exit path.
#[allow(clippy::too_many_arguments)]
async fn run_chat_pump(
    pg: Arc<Playground>,
    model: loop_ai::Model,
    messages: Vec<loop_ai::Message>,
    options: loop_ai::SimpleStreamOptions,
    mut traced: TracedRequest,
    llm_span: String,
    tx: mpsc::Sender<ChatEvent>,
    conversation_id_cl: String,
    assistant_id_cl: String,
    cancel_pump: CancelHandle,
) {
    {
        let mut stream = pg.engine.models().stream_simple(
            &model,
            &loop_ai::Context {
                messages,
                ..Default::default()
            },
            options,
        );

        let mut text = String::new();
        let mut client_gone = false;
        let mut last_partial_usage: Option<loop_ai::Usage> = None;
        let started_at = std::time::Instant::now();

        while let Some(event) = stream.next().await {
            traced.observe_event(&event);
            match &event {
                loop_ai::AssistantMessageEvent::TextDelta { delta, .. } => {
                    text.push_str(delta);
                    if !client_gone
                        && tx
                            .send(ChatEvent::TextDelta {
                                delta: delta.clone(),
                            })
                            .await
                            .is_err()
                    {
                        // Browser disconnected: cancel the real engine
                        // request, keep draining so the model side
                        // finishes, then finalize as client-disconnect.
                        client_gone = true;
                        cancel_pump.cancel();
                    }
                }
                loop_ai::AssistantMessageEvent::ThinkingDelta { delta, .. } => {
                    if !client_gone {
                        let _ = tx
                            .send(ChatEvent::ThinkingDelta {
                                delta: delta.clone(),
                            })
                            .await;
                    }
                }
                loop_ai::AssistantMessageEvent::Done { message, .. } => {
                    let duration = started_at.elapsed().as_millis() as u64;
                    if client_gone {
                        // Engine finished after the browser left.
                        traced.finish_client_disconnected(Some(&message.usage), Some(&llm_span));
                        let _ = pg.store.update_message(
                            &conversation_id_cl,
                            &assistant_id_cl,
                            &text,
                            "cancelled",
                            None,
                        );
                        return;
                    }
                    traced.finish_success(message, &llm_span);
                    let full_text = message
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            loop_ai::AssistantContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let final_text = if full_text.is_empty() {
                        text.clone()
                    } else {
                        full_text
                    };
                    let _ = pg.store.update_message(
                        &conversation_id_cl,
                        &assistant_id_cl,
                        &final_text,
                        "complete",
                        None,
                    );
                    let _ = tx
                        .send(ChatEvent::Done {
                            text: final_text,
                            input_tokens: Some(message.usage.input),
                            output_tokens: Some(message.usage.output),
                            ttft_ms: traced.ttft_ms(),
                            duration_ms: duration,
                        })
                        .await;
                    return;
                }
                loop_ai::AssistantMessageEvent::Error { error, .. } => {
                    let aborted = error.stop_reason == loop_ai::StopReason::Aborted;
                    if client_gone {
                        traced.finish_client_disconnected(
                            last_partial_usage.as_ref(),
                            Some(&llm_span),
                        );
                        let _ = pg.store.update_message(
                            &conversation_id_cl,
                            &assistant_id_cl,
                            &text,
                            "cancelled",
                            None,
                        );
                        return;
                    }
                    if aborted {
                        // User pressed stop: the engine aborted the real
                        // underlying HTTP request.
                        traced.finish_cancelled(true, last_partial_usage.as_ref(), Some(&llm_span));
                        let _ = pg.store.update_message(
                            &conversation_id_cl,
                            &assistant_id_cl,
                            &text,
                            "cancelled",
                            None,
                        );
                        let _ = tx
                            .send(ChatEvent::Cancelled {
                                reason: "user".into(),
                                partial_text: text.clone(),
                            })
                            .await;
                        return;
                    }
                    let error_text = crate::engine::trace::sanitize_error(
                        error.error_message.as_deref().unwrap_or("inference failed"),
                    );
                    traced.finish_error(error, Some(&llm_span));
                    let _ = pg.store.update_message(
                        &conversation_id_cl,
                        &assistant_id_cl,
                        &text,
                        "error",
                        Some(&error_text),
                    );
                    let _ = tx
                        .send(ChatEvent::Failed {
                            error: error_text,
                            partial_text: text.clone(),
                        })
                        .await;
                    return;
                }
                _ => {}
            }
            if let Some(u) = partial_usage_of(&event) {
                last_partial_usage = Some(u);
            }
        }

        // Stream ended without a terminal event (should not happen:
        // Models wraps unclean ends into an Error) — record honestly.
        let error_text = "stream ended without completing";
        let _ = pg.store.update_message(
            &conversation_id_cl,
            &assistant_id_cl,
            &text,
            "error",
            Some(error_text),
        );
        let mut msg = loop_ai::AssistantMessage::pending(&model);
        msg.stop_reason = loop_ai::StopReason::Error;
        msg.error_message = Some(error_text.into());
        traced.finish_error(&msg, Some(&llm_span));
        let _ = tx
            .send(ChatEvent::Failed {
                error: error_text.into(),
                partial_text: text.clone(),
            })
            .await;
    }
}

fn partial_usage_of(event: &loop_ai::AssistantMessageEvent) -> Option<loop_ai::Usage> {
    let partial = match event {
        loop_ai::AssistantMessageEvent::Start { partial }
        | loop_ai::AssistantMessageEvent::TextStart { partial, .. }
        | loop_ai::AssistantMessageEvent::TextDelta { partial, .. }
        | loop_ai::AssistantMessageEvent::TextEnd { partial, .. }
        | loop_ai::AssistantMessageEvent::ThinkingStart { partial, .. }
        | loop_ai::AssistantMessageEvent::ThinkingDelta { partial, .. }
        | loop_ai::AssistantMessageEvent::ThinkingEnd { partial, .. }
        | loop_ai::AssistantMessageEvent::ToolcallStart { partial, .. }
        | loop_ai::AssistantMessageEvent::ToolcallDelta { partial, .. }
        | loop_ai::AssistantMessageEvent::ToolcallEnd { partial, .. } => partial,
        loop_ai::AssistantMessageEvent::Done { message, .. }
        | loop_ai::AssistantMessageEvent::Error { error: message, .. } => message,
    };
    (partial.usage.total_tokens > 0).then(|| partial.usage.clone())
}
