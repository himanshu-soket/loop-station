//! Inference integration tests.
//!
//! These run against the REAL `loop-ai` engine (not mocks): a
//! `FakeHttpServer` stands in for the model provider's HTTP endpoint, exactly
//! like `loop-ai`'s own integration tests. Requests genuinely flow through
//! `Models::stream_simple`, the OpenAI-completions adapter and SSE parsing.

use std::sync::Arc;

use futures::StreamExt;
use loop_ai::providers::{custom_provider, CustomModelSpec, CustomProviderConfig};
use loop_ai::{AssistantMessageEvent, Models};
use loop_test_support::{FakeHttpServer, FakeResponse};

use crate::engine::params::{InferenceParams, ReasoningLevel};
use crate::engine::trace::{CancelHandle, TracedRequest, TracedRequestSpec};
use crate::engine::Engine;
use crate::persist::{DashboardStore, RetentionConfig, TraceStatus};
use crate::playground::{ChatRequest, Playground};

/// Account id used as the conversation owner in these tests. The value is
/// arbitrary: what matters is that every conversation created through
/// `start_chat` is stamped with the caller's account and later reads are
/// scoped to it.
const TEST_OWNER_ACCOUNT: i64 = 1;

/// SSE body for a two-chunk text response with usage.
fn sse_text_response() -> String {
    [
        r#"data: {"id":"x","model":"tiny","choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"}}]}"#,
        "",
        r#"data: {"id":"x","model":"tiny","choices":[{"index":0,"delta":{"content":"lo!"}}]}"#,
        "",
        r#"data: {"id":"x","model":"tiny","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#,
        "",
        "data: [DONE]",
        "",
    ]
    .join("\n")
}

/// A playground wired to a fake OpenAI-compatible endpoint.
struct TestRig {
    _server: FakeHttpServer,
    store: Arc<DashboardStore>,
    playground: Arc<Playground>,
    current_trace: Option<String>,
}

impl TestRig {
    fn new(
        route: impl Fn(&loop_test_support::RecordedRequest) -> FakeResponse + Send + Sync + 'static,
    ) -> Self {
        let server = FakeHttpServer::start(route);
        let dir = tempfile::tempdir().unwrap();
        // Point the engine at the fake server via models.json.
        std::fs::write(
            dir.path().join("models.json"),
            format!(
                r#"{{"providers":[{{"id":"fakeprov","baseUrl":"{}/v1","models":["tiny"]}}]}}"#,
                server.base_url()
            ),
        )
        .unwrap();
        let store = Arc::new(DashboardStore::open(dir.path().join("store.db")).unwrap());
        let engine = Arc::new(Engine::load(crate::engine::EnginePaths {
            agent_dir: dir.path().to_path_buf(),
        }));
        // Keep the tempdir alive for the test's duration.
        std::mem::forget(dir);
        let playground = Arc::new(Playground::new(Arc::clone(&engine), Arc::clone(&store)));
        Self {
            _server: server,
            store,
            playground,
            current_trace: None,
        }
    }
}

#[tokio::test]
async fn successful_stream_is_traced_with_real_engine() {
    let mut rig = TestRig::new(|req| {
        assert!(
            req.path.contains("/chat/completions"),
            "must hit the real completions path"
        );
        FakeResponse::new(200, "text/event-stream", sse_text_response())
    });

    let (mut rx, _cancel) = rig
        .playground
        .start_chat(TEST_OWNER_ACCOUNT, ChatRequest {
            conversation_id: None,
            message: "say hi".into(),
            provider: "fakeprov".into(),
            model: "tiny".into(),
            params: InferenceParams {
                temperature: Some(0.5),
                max_tokens: Some(64),
                reasoning: None,
            },
        })
        .await
        .expect("chat starts");

    let mut text = String::new();
    let mut done = None;
    while let Some(event) = rx.recv().await {
        match event {
            crate::playground::ChatEvent::Started { trace_id, .. } => {
                rig.current_trace = Some(trace_id);
            }
            crate::playground::ChatEvent::TextDelta { delta } => text.push_str(&delta),
            crate::playground::ChatEvent::Done {
                input_tokens,
                ttft_ms,
                ..
            } => {
                done = Some((input_tokens, ttft_ms));
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    assert_eq!(text, "Hello!");
    let (input_tokens, ttft) = done.expect("terminal Done event");
    assert_eq!(input_tokens, Some(7));
    assert!(ttft.is_some(), "TTFT was measured");

    // The trace is durably persisted as success with real measurements.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let trace_id = rig.current_trace.clone().expect("trace id captured");
    let detail = rig
        .store
        .get_trace_detail(&trace_id)
        .unwrap()
        .expect("persisted");
    assert_eq!(detail.trace.status, TraceStatus::Success);
    assert_eq!(detail.trace.input_tokens, Some(7));
    assert_eq!(detail.trace.output_tokens, Some(3));
    assert!(detail.trace.ttft_ms.is_some());
    assert!(detail.trace.duration_ms.is_some());

    // Span tree: root + validate + resolve + llm.generate.
    let names: Vec<&str> = detail.spans.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"http.request"));
    assert!(names.contains(&"request.validate"));
    assert!(names.contains(&"model.resolve"));
    assert!(names.contains(&"llm.generate"));
    // Parent-child: every non-root span hangs off the root.
    let root_id = &detail.trace.root_span_id;
    for span in &detail.spans {
        if span.span_id != *root_id {
            assert_eq!(
                span.parent_span_id.as_deref(),
                Some(root_id.as_str()),
                "{} must be parented by the root",
                span.name
            );
        }
    }

    // Conversation persisted with the full exchange.
    let conversations = rig.store.list_conversations(10).unwrap();
    assert_eq!(conversations.len(), 1);
    let messages = rig
        .store
        .list_messages(&conversations[0].conversation_id)
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].status, "complete");
    assert_eq!(messages[1].content, "Hello!");
}

#[tokio::test]
async fn provider_error_is_traced_as_error() {
    let mut rig = TestRig::new(|_| {
        FakeResponse::new(
            500,
            "application/json",
            r#"{"error":{"message":"upstream exploded"}}"#,
        )
    });

    let (mut rx, _cancel) = rig
        .playground
        .start_chat(TEST_OWNER_ACCOUNT, ChatRequest {
            conversation_id: None,
            message: "hi".into(),
            provider: "fakeprov".into(),
            model: "tiny".into(),
            params: InferenceParams::default(),
        })
        .await
        .unwrap();

    let mut failed = None;
    while let Some(event) = rx.recv().await {
        match event {
            crate::playground::ChatEvent::Started { trace_id, .. } => {
                rig.current_trace = Some(trace_id)
            }
            crate::playground::ChatEvent::Failed { error, .. } => {
                failed = Some(error);
                break;
            }
            _ => {}
        }
    }
    assert!(failed.unwrap().contains("upstream exploded"));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let detail = rig
        .store
        .get_trace_detail(&rig.current_trace.clone().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(detail.trace.status, TraceStatus::Error);
    assert!(detail
        .trace
        .error_message
        .unwrap()
        .contains("upstream exploded"));

    // The assistant message is honestly marked errored.
    let conversations = rig.store.list_conversations(10).unwrap();
    let messages = rig
        .store
        .list_messages(&conversations[0].conversation_id)
        .unwrap();
    assert_eq!(messages[1].status, "error");
}

#[tokio::test]
async fn user_cancellation_cancels_the_real_request() {
    use std::time::Duration;
    // The server delays its body; we cancel before it arrives.
    let rig = TestRig::new(|_| {
        FakeResponse::new(200, "text/event-stream", sse_text_response())
            .with_body_delay(Duration::from_secs(30))
    });

    let (mut rx, _cancel) = rig
        .playground
        .start_chat(TEST_OWNER_ACCOUNT, ChatRequest {
            conversation_id: None,
            message: "slow please".into(),
            provider: "fakeprov".into(),
            model: "tiny".into(),
            params: InferenceParams::default(),
        })
        .await
        .unwrap();

    // Read Started to learn the ids, then stop.
    let mut trace_id = None;
    if let Some(crate::playground::ChatEvent::Started {
        trace_id: t,
        request_id,
        ..
    }) = rx.recv().await
    {
        trace_id = Some(t);
        // Cancel through the public API (the route handler's path).
        assert!(rig.playground.cancel_generation(&request_id));
    }

    let mut cancelled = false;
    while let Some(event) = rx.recv().await {
        if matches!(event, crate::playground::ChatEvent::Cancelled { .. }) {
            cancelled = true;
            break;
        }
    }
    assert!(cancelled, "user cancellation surfaced");

    tokio::time::sleep(Duration::from_millis(200)).await;
    let detail = rig
        .store
        .get_trace_detail(&trace_id.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(detail.trace.status, TraceStatus::CancelledUser);

    // The request row records why.
    let (requests, total) = rig
        .store
        .list_requests(&crate::persist::RequestFilter {
            page: 1,
            per_page: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(requests[0].status, TraceStatus::CancelledUser);
    assert_eq!(requests[0].cancel_reason.as_deref(), Some("user"));
}

#[tokio::test]
async fn parameters_reach_the_provider() {
    let server =
        FakeHttpServer::start(|_| FakeResponse::new(200, "text/event-stream", sse_text_response()));
    let models = Models::new();
    models.set_provider(custom_provider(CustomProviderConfig {
        id: "p".into(),
        name: None,
        base_url: format!("{}/v1", server.base_url()),
        api_key_env: vec![],
        models: vec![CustomModelSpec::new("tiny")],
        headers: None,
    }));
    let model = models.get_model("p", "tiny").unwrap();

    let cancel = CancelHandle::new();
    let mut options = loop_ai::SimpleStreamOptions::default();
    options.base.cancel = Some(cancel.token());
    options.base.temperature = Some(0.42);
    options.base.max_tokens = Some(777);
    options.reasoning = ReasoningLevel::High.to_thinking();

    let mut stream = models.stream_simple(
        &model,
        &loop_ai::Context {
            messages: vec![loop_ai::Message::user_text("hi")],
            ..Default::default()
        },
        options,
    );
    while let Some(ev) = stream.next().await {
        if matches!(ev, AssistantMessageEvent::Done { .. }) {
            break;
        }
    }

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("valid JSON payload");
    assert_eq!(
        body["temperature"], 0.42,
        "temperature reaches the provider"
    );
    assert_eq!(body["max_tokens"], 777, "max_tokens reaches the provider");
    assert!(
        body.get("top_p").is_none(),
        "no unsupported parameter is invented"
    );
    // No credential material in the outbound payload.
    let raw = String::from_utf8_lossy(&requests[0].body).to_lowercase();
    assert!(!raw.contains("authorization"));
    assert!(!raw.contains("api_key"));
}

#[tokio::test]
async fn traced_request_records_ttft_from_real_stream() {
    // Direct TracedRequest usage over a real engine stream (unit-level view
    // of what the playground does).
    let server =
        FakeHttpServer::start(|_| FakeResponse::new(200, "text/event-stream", sse_text_response()));
    let models = Models::new();
    models.set_provider(custom_provider(CustomProviderConfig {
        id: "p".into(),
        name: None,
        base_url: format!("{}/v1", server.base_url()),
        api_key_env: vec![],
        models: vec![CustomModelSpec::new("tiny")],
        headers: None,
    }));
    let model = models.get_model("p", "tiny").unwrap();

    let store = Arc::new(DashboardStore::open_in_memory().unwrap());
    let mut traced = TracedRequest::begin(
        store.clone(),
        TracedRequestSpec {
            operation: "dashboard.chat",
            provider: "p".into(),
            model: "tiny".into(),
            conversation_id: None,
            params_json: InferenceParams::default().trace_json(),
            retention: RetentionConfig {
                max_traces: 0,
                max_age_ms: None,
            },
        },
    );
    let llm_span = traced.open_llm_span();

    let mut stream = models.stream_simple(
        &model,
        &loop_ai::Context {
            messages: vec![loop_ai::Message::user_text("hi")],
            ..Default::default()
        },
        loop_ai::SimpleStreamOptions::default(),
    );
    while let Some(event) = stream.next().await {
        traced.observe_event(&event);
        if let AssistantMessageEvent::Done { message, .. } = event {
            traced.finish_success(&message, &llm_span);
            break;
        }
    }

    let detail = store.get_trace_detail(traced.trace_id()).unwrap().unwrap();
    assert_eq!(detail.trace.status, TraceStatus::Success);
    assert!(
        detail.trace.ttft_ms.is_some(),
        "TTFT measured from the first delta"
    );
    assert!(detail.trace.input_tokens.is_some());
    // Events recorded: stream.start + stream.done.
    let names: Vec<&str> = detail.events.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"stream.start"));
    assert!(names.contains(&"stream.done"));
}

#[tokio::test]
async fn multiple_conversations_do_not_cross_contaminate() {
    let rig = TestRig::new(|_| FakeResponse::new(200, "text/event-stream", sse_text_response()));

    let mut ids = Vec::new();
    for i in 0..3 {
        let (mut rx, _) = rig
            .playground
            .start_chat(TEST_OWNER_ACCOUNT, ChatRequest {
                conversation_id: None,
                message: format!("msg {i}"),
                provider: "fakeprov".into(),
                model: "tiny".into(),
                params: InferenceParams::default(),
            })
            .await
            .unwrap();
        while let Some(ev) = rx.recv().await {
            match ev {
                crate::playground::ChatEvent::Started {
                    conversation_id, ..
                } => ids.push(conversation_id),
                crate::playground::ChatEvent::Done { .. } => break,
                _ => {}
            }
        }
    }
    // Three distinct conversations, each with its own exchange.
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
    for id in &ids {
        let messages = rig.store.list_messages(id).unwrap();
        assert_eq!(
            messages.len(),
            2,
            "each conversation holds only its own turn"
        );
    }
}
