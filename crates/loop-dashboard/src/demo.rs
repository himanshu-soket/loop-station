//! Clearly-labeled demo data adapter.
//!
//! Demo mode exists **only** so the dashboard can be demonstrated on machines
//! without a populated session database (e.g. a fresh install or a judge's
//! laptop). It is opt-in via `--demo`, is announced in `/api/health`, shows a
//! persistent banner in the UI, and **never** touches or mixes with the real
//! store: when `--demo` is passed the server serves synthetic data exclusively.

use std::path::PathBuf;

use crate::store::{EntryRow, SessionRow, StoreError};

/// A trace store that returns synthetic sessions instead of reading SQLite.
pub struct DemoStore {
    /// Fake db path label for health display.
    pub label: PathBuf,
}

impl DemoStore {
    /// Build the demo store.
    pub fn new() -> Self {
        Self {
            label: PathBuf::from("demo://synthetic-session-data"),
        }
    }

    /// Same error surface as the real store; demo always succeeds.
    pub fn counts(&self) -> Result<(u64, u64), StoreError> {
        Ok((self.list_sessions()?.len() as u64, 42))
    }

    /// Synthetic sessions.
    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, StoreError> {
        Ok(demo_sessions())
    }

    /// Synthetic entries for one session.
    pub fn list_entries(&self, session_id: &str) -> Result<Vec<EntryRow>, StoreError> {
        Ok(demo_entries(session_id))
    }

    /// All synthetic entries across sessions.
    pub fn list_all_entries(&self) -> Result<Vec<crate::store::RawEntry>, StoreError> {
        let mut out = Vec::new();
        for s in demo_sessions() {
            for e in demo_entries(&s.id) {
                let entry_id = e.entry_id.clone();
                let ts = e.payload.as_ref().and_then(|p| p.timestamp);
                let message = demo_message_body(e.payload.as_ref());
                out.push(crate::store::RawEntry {
                    session_id: e.session_id,
                    entry_id,
                    parent_id: e.parent_id,
                    entry_seq: e.entry_seq,
                    payload: serde_json::to_string(&serde_json::json!({
                        "type": "message",
                        "id": e.entry_id,
                        "timestamp": ts,
                        "message": message,
                    }))
                    .unwrap_or_default(),
                });
            }
        }
        Ok(out)
    }

    /// One synthetic session.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>, StoreError> {
        Ok(demo_sessions().into_iter().find(|s| s.id == id))
    }
}

impl Default for DemoStore {
    fn default() -> Self {
        Self::new()
    }
}

fn demo_message_body(payload: Option<&crate::payload::EntryPayload>) -> serde_json::Value {
    // The demo store serializes entries back through the same wire shapes the
    // real payloads use, so parsing round-trips exactly like production data.
    let Some(p) = payload else {
        return serde_json::Value::Null;
    };
    let Some(m) = &p.message else {
        return serde_json::Value::Null;
    };
    match m {
        crate::payload::MessageBody::Known(crate::payload::KnownMessage::User(u)) => {
            serde_json::json!({ "role": "user", "content": u.content, "timestamp": u.timestamp })
        }
        crate::payload::MessageBody::Known(crate::payload::KnownMessage::Assistant(a)) => {
            serde_json::json!({
                "role": "assistant",
                "content": a.content.iter().map(|b| match b {
                    crate::payload::AssistantContentBlock::Text { text } =>
                        serde_json::json!({ "type": "text", "text": text }),
                    crate::payload::AssistantContentBlock::Thinking { thinking } =>
                        serde_json::json!({ "type": "thinking", "thinking": thinking }),
                    crate::payload::AssistantContentBlock::ToolCall { id, name, arguments } =>
                        serde_json::json!({ "type": "toolCall", "id": id, "name": name, "arguments": arguments }),
                    crate::payload::AssistantContentBlock::Other =>
                        serde_json::json!({ "type": "other" }),
                }).collect::<Vec<_>>(),
                "api": a.api,
                "provider": a.provider,
                "model": a.model,
                "responseModel": a.response_model,
                "responseId": a.response_id,
                "usage": a.usage.as_ref().map(|u| serde_json::json!({
                    "input": u.input, "output": u.output, "cacheRead": u.cache_read,
                    "cacheWrite": u.cache_write, "reasoning": u.reasoning,
                    "totalTokens": u.total_tokens,
                    "cost": { "input": u.cost.input, "output": u.cost.output,
                              "cacheRead": u.cost.cache_read, "cacheWrite": u.cost.cache_write,
                              "total": u.cost.total },
                })),
                "stopReason": a.stop_reason,
                "errorMessage": a.error_message,
                "rawStopReason": a.raw_stop_reason,
                "timestamp": a.timestamp,
            })
        }
        crate::payload::MessageBody::Known(crate::payload::KnownMessage::ToolResult(t)) => {
            serde_json::json!({
                "role": "toolResult",
                "toolCallId": t.tool_call_id,
                "toolName": t.tool_name,
                "isError": t.is_error,
                "content": t.content.iter().map(|b| match b {
                    crate::payload::ToolResultBlock::Text { text } =>
                        serde_json::json!({ "type": "text", "text": text }),
                    _ => serde_json::json!({ "type": "other" }),
                }).collect::<Vec<_>>(),
                "timestamp": t.timestamp,
            })
        }
        crate::payload::MessageBody::Known(_) | crate::payload::MessageBody::Other { .. } => {
            serde_json::Value::Null
        }
    }
}

fn base_time() -> i64 {
    // Fixed recent-ish window so demo charts look alive but deterministic.
    chrono::Utc::now().timestamp_millis() - 36 * 3_600_000
}

fn demo_sessions() -> Vec<SessionRow> {
    let t0 = base_time();
    (0..6)
        .map(|i| SessionRow {
            id: format!("demo-{i:04}-session"),
            cwd: Some(match i % 3 {
                0 => "/home/dev/api-server".into(),
                1 => "/home/dev/web-app".into(),
                _ => "/home/dev/etl-pipeline".into(),
            }),
            name: Some(format!("Demo session {i:02}")),
            parent_session_id: None,
            created_at: t0 + i as i64 * 900_000,
        })
        .collect()
}

fn demo_entries(session_id: &str) -> Vec<EntryRow> {
    let idx: usize = session_id
        .strip_prefix("demo-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let t0 = base_time() + idx as i64 * 900_000;
    let turns = 2 + (idx % 4);
    let mut entries = Vec::new();
    let mut seq = 1i64;
    let mut parent: Option<String> = None;

    let mk_entry = |seq: i64,
                    parent: &Option<String>,
                    ts: i64,
                    message: Option<crate::payload::MessageBody>,
                    kind: &str|
     -> EntryRow {
        EntryRow {
            session_id: session_id.to_string(),
            entry_id: format!("{session_id}-e{seq}"),
            parent_id: parent.clone(),
            entry_seq: seq,
            payload: Some(crate::payload::EntryPayload {
                kind: kind.to_string(),
                id: Some(format!("{session_id}-e{seq}")),
                parent_id: parent.clone(),
                timestamp: Some(ts),
                message,
                model_id: None,
                provider: None,
                summary: None,
                custom_type: None,
            }),
        }
    };

    for turn in 0..turns {
        let user_ts = t0 + turn as i64 * 1_800_000;
        let user = crate::payload::MessageBody::Known(crate::payload::KnownMessage::User(
            crate::payload::UserMessage {
                content: serde_json::json!(format!(
                    "Demo prompt #{turn}: summarize the failing test output"
                )),
                timestamp: Some(user_ts),
            },
        ));
        entries.push(mk_entry(seq, &parent, user_ts, Some(user), "message"));
        parent = Some(format!("{session_id}-e{seq}"));
        seq += 1;

        // Every 3rd session gets a failed turn; others succeed.
        let failed = idx % 3 == 2 && turn == 1;
        let latency = 900 + (idx * 700 + turn * 450) as i64;
        let assistant_ts = user_ts + latency;
        let stop = if failed { "error" } else { "stop" };
        let usage = crate::payload::Usage {
            input: 1_200 + (idx as u64 * 300) + turn as u64 * 150,
            output: 80 + turn as u64 * 40,
            cache_read: idx as u64 * 120,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: Some(turn as u64 * 25),
            total_tokens: 0,
            cost: crate::payload::Cost {
                input: 0.002,
                output: 0.001,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.003,
            },
        };
        let tool_call = crate::payload::AssistantContentBlock::ToolCall {
            id: Some(format!("call-{turn}")),
            name: Some("read".to_string()),
            arguments: Some(serde_json::json!({"path": "/tmp/demo.log"})),
        };
        let assistant = crate::payload::MessageBody::Known(
            crate::payload::KnownMessage::Assistant(crate::payload::AssistantMessage {
                content: vec![
                    crate::payload::AssistantContentBlock::Thinking {
                        thinking: "Demo reasoning trace about the test output.".into(),
                    },
                    crate::payload::AssistantContentBlock::Text {
                        text: "Demo answer: two tests fail due to a missing fixture.".into(),
                    },
                    tool_call,
                ],
                api: Some("openai-completions".into()),
                provider: Some(
                    if idx.is_multiple_of(2) {
                        "soket"
                    } else {
                        "openrouter"
                    }
                    .into(),
                ),
                model: Some(if idx.is_multiple_of(2) {
                    "GLM-5.3".into()
                } else {
                    "qwen3-8-27b".into()
                }),
                response_model: None,
                response_id: Some(format!("resp-{idx}-{turn}")),
                usage: Some(usage),
                stop_reason: Some(serde_json::json!(stop)),
                error_message: failed.then(|| "provider 502: upstream timeout".into()),
                raw_stop_reason: Some(stop.into()),
                timestamp: Some(assistant_ts),
            }),
        );
        entries.push(mk_entry(
            seq,
            &parent,
            assistant_ts,
            Some(assistant),
            "message",
        ));
        parent = Some(format!("{session_id}-e{seq}"));
        seq += 1;

        // Tool result for the call above.
        let result_ts = assistant_ts + 250;
        let tool_result = crate::payload::MessageBody::Known(
            crate::payload::KnownMessage::ToolResult(crate::payload::ToolResultMessage {
                tool_call_id: Some(format!("call-{turn}")),
                tool_name: Some("read".into()),
                content: vec![crate::payload::ToolResultBlock::Text {
                    text: "file contents (demo)".into(),
                }],
                details: None,
                usage: None,
                is_error: Some(false),
                timestamp: Some(result_ts),
            }),
        );
        entries.push(mk_entry(
            seq,
            &parent,
            result_ts,
            Some(tool_result),
            "message",
        ));
        parent = Some(format!("{session_id}-e{seq}"));
        seq += 1;
    }
    entries
}
