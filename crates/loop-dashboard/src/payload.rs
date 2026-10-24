//! Tolerant read-only mirrors of Loop session payloads stored in SQLite.
//!
//! These types mirror [`loop_agent`] / [`loop_ai`] wire formats **as they are
//! serialized into `session_entries.payload`** (camelCase JSON). They are
//! deliberately decoupled from those crates so the dashboard never links agent
//! internals, and every field is optional or defaulted: a malformed or
//! forward-incompatible payload degrades to "unknown" instead of failing the
//! whole request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level `session_entries.payload` document (`{"type": "...", ...}`).
///
/// Some fields exist only to mirror the stored schema faithfully; they are
/// read by tests and future features.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct EntryPayload {
    /// Discriminator, snake_case as serialized by the harness
    /// (e.g. `"message"`, `"model_change"`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Entry id.
    #[serde(default)]
    pub id: Option<String>,
    /// Parent entry id (tree link), when present.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Entry timestamp (unix ms).
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Message body for `kind == "message"`.
    #[serde(default)]
    pub message: Option<MessageBody>,
    /// New model id for `kind == "model_change"` (camelCase on the wire).
    #[serde(default, rename = "modelId")]
    pub model_id: Option<String>,
    /// Provider for `kind == "model_change"`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Summary text for compaction/branch-summary entries.
    #[serde(default)]
    pub summary: Option<String>,
    /// Custom type label for `kind == "custom"`.
    #[serde(default, rename = "customType")]
    pub custom_type: Option<String>,
}

impl EntryPayload {
    /// Parse one payload JSON string; `None` when it is not valid JSON.
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }
}

/// `payload.message` — an LLM or custom-role message.
///
/// Serde tag is `"role"`; unknown roles (future harness messages) fall back to
/// [`MessageBody::other`] so they still render in the timeline.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant, reason = "mirrors the stored wire format")]
pub enum MessageBody {
    /// A known-shape message.
    Known(KnownMessage),
    /// Anything else: keep role + raw JSON for display.
    Other {
        /// Role string as stored.
        role: String,
    },
}

/// Known message shapes (user / assistant / toolResult / custom variants).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum KnownMessage {
    /// User turn.
    User(UserMessage),
    /// Assistant turn (one LLM call).
    Assistant(AssistantMessage),
    /// Tool result.
    #[serde(rename = "toolResult")]
    ToolResult(ToolResultMessage),
    /// Bash execution display message.
    #[serde(rename = "bashExecution")]
    #[allow(dead_code)]
    BashExecution {
        /// Command that was run.
        command: String,
        /// Captured output text.
        output: String,
        /// Exit code if known.
        #[serde(default)]
        exit_code: Option<i32>,
        /// Unix epoch ms.
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Branch summary injected into context.
    #[serde(rename = "branchSummary")]
    BranchSummary {
        /// Summary text.
        summary: String,
    },
    /// Compaction summary injected into context.
    #[serde(rename = "compactionSummary")]
    CompactionSummary {
        /// Summary text.
        summary: String,
    },
}

#[allow(dead_code)]
impl MessageBody {
    /// Role string for grouping/display (used by tests + future filters).
    pub fn role(&self) -> &str {
        match self {
            Self::Known(KnownMessage::User(_)) => "user",
            Self::Known(KnownMessage::Assistant(_)) => "assistant",
            Self::Known(KnownMessage::ToolResult(_)) => "toolResult",
            Self::Known(KnownMessage::BashExecution { .. }) => "bashExecution",
            Self::Known(KnownMessage::BranchSummary { .. }) => "branchSummary",
            Self::Known(KnownMessage::CompactionSummary { .. }) => "compactionSummary",
            Self::Other { role } => role,
        }
    }
}

/// User message (`content` is a plain string or multimodal blocks).
#[derive(Debug, Clone, Deserialize)]
pub struct UserMessage {
    /// Content: plain string or content blocks.
    #[serde(default)]
    pub content: Value,
    /// Unix epoch ms.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

/// Assistant message — one recorded LLM call with usage and stop reason.
#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    /// Content blocks (text / thinking / toolCall).
    #[serde(default)]
    pub content: Vec<AssistantContentBlock>,
    /// Wire API used, e.g. `"openai-completions"`.
    #[serde(default)]
    pub api: Option<String>,
    /// Provider id, e.g. `"soket"`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Requested model id.
    #[serde(default)]
    pub model: Option<String>,
    /// Response model id if different.
    #[serde(default, rename = "responseModel")]
    pub response_model: Option<String>,
    /// Provider response id.
    #[serde(default, rename = "responseId")]
    pub response_id: Option<String>,
    /// Token usage and cost.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Stop reason (camelCase: stop / length / toolUse / error / aborted …).
    #[serde(default, rename = "stopReason")]
    pub stop_reason: Option<Value>,
    /// Error message when stopped with error/aborted.
    #[serde(default, rename = "errorMessage")]
    pub error_message: Option<String>,
    /// Raw provider stop reason.
    #[serde(default, rename = "rawStopReason")]
    pub raw_stop_reason: Option<String>,
    /// Unix epoch ms.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

impl AssistantMessage {
    /// Normalized stop-reason label, lowercased; `"unknown"` when absent.
    pub fn stop_label(&self) -> String {
        match &self.stop_reason {
            Some(Value::String(s)) => s.to_lowercase(),
            _ => "unknown".to_string(),
        }
    }

    /// Whether this turn ended successfully (natural stop, length cap or a
    /// tool-use handoff — all three mean the stream completed).
    pub fn succeeded(&self) -> bool {
        matches!(self.stop_label().as_str(), "stop" | "length" | "tooluse")
    }

    /// Whether this turn failed (provider/runtime error or abort).
    pub fn failed(&self) -> bool {
        matches!(self.stop_label().as_str(), "error" | "aborted")
    }
}

/// One assistant content block.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantContentBlock {
    /// Text output.
    Text {
        /// Text.
        text: String,
    },
    /// Reasoning trace.
    Thinking {
        /// Thinking text.
        thinking: String,
    },
    /// Tool invocation.
    #[serde(rename = "toolCall")]
    ToolCall {
        /// Provider-assigned call id.
        #[serde(default)]
        id: Option<String>,
        /// Tool name.
        #[serde(default)]
        name: Option<String>,
        /// Parsed arguments object.
        #[serde(default)]
        arguments: Option<Value>,
    },
    /// Unknown future block type: keep raw JSON.
    #[serde(other)]
    Other,
}

/// Token usage + dollar cost recorded on a turn.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Input / prompt tokens.
    #[serde(default)]
    pub input: u64,
    /// Output / completion tokens.
    #[serde(default)]
    pub output: u64,
    /// Cache-read tokens.
    #[serde(default, rename = "cacheRead")]
    pub cache_read: u64,
    /// Cache-write tokens.
    #[serde(default, rename = "cacheWrite")]
    pub cache_write: u64,
    /// Optional 1h cache-write tokens.
    #[serde(default, rename = "cacheWrite1h")]
    pub cache_write_1h: Option<u64>,
    /// Reasoning tokens (subset of output when reported).
    #[serde(default)]
    pub reasoning: Option<u64>,
    /// Total tokens.
    #[serde(default, rename = "totalTokens")]
    pub total_tokens: u64,
    /// Dollar cost.
    #[serde(default)]
    pub cost: Cost,
}

impl Usage {
    /// Prompt volume: input + cache read + cache write.
    pub fn prompt_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    /// All token components (prompt + output).
    pub fn total_or_sum(&self) -> u64 {
        let summed = self.prompt_tokens().saturating_add(self.output);
        if self.total_tokens > 0 {
            self.total_tokens.max(summed)
        } else {
            summed
        }
    }

    /// Fold another usage record into this one (saturating).
    pub fn add(&mut self, other: &Usage) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.cache_write_1h = match (self.cache_write_1h, other.cache_write_1h) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (None, Some(b)) => Some(b),
            (a, None) => a,
        };
        self.reasoning = match (self.reasoning, other.reasoning) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (None, Some(b)) => Some(b),
            (a, None) => a,
        };
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.cost.add(&other.cost);
    }
}

/// Dollar cost breakdown.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// Input token cost.
    #[serde(default)]
    pub input: f64,
    /// Output token cost.
    #[serde(default)]
    pub output: f64,
    /// Cache-read cost.
    #[serde(default)]
    pub cache_read: f64,
    /// Cache-write cost.
    #[serde(default)]
    pub cache_write: f64,
    /// Total cost.
    #[serde(default)]
    pub total: f64,
}

impl Cost {
    /// Fold another cost in.
    pub fn add(&mut self, other: &Cost) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.total += other.total;
    }
}

/// Tool result message.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ToolResultMessage {
    /// Matching tool call id.
    #[serde(default, rename = "toolCallId")]
    pub tool_call_id: Option<String>,
    /// Tool name.
    #[serde(default, rename = "toolName")]
    pub tool_name: Option<String>,
    /// Result content blocks.
    #[serde(default)]
    pub content: Vec<ToolResultBlock>,
    /// Optional structured details.
    #[serde(default)]
    pub details: Option<Value>,
    /// Optional usage contributed by the tool.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Whether the tool failed.
    #[serde(default, rename = "isError")]
    pub is_error: Option<bool>,
    /// Unix epoch ms.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

/// One tool-result content block.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolResultBlock {
    /// Text output.
    Text {
        /// Text.
        text: String,
    },
    /// Image output (source kept for schema fidelity).
    #[allow(dead_code)]
    Image {
        /// Image source descriptor.
        #[serde(default)]
        source: Option<Value>,
    },
    /// Unknown future block type.
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_assistant_payload() {
        let raw = r#"{
          "type":"message","id":"e1","parent_id":"e0","timestamp":1792656485356,
          "message":{
            "role":"assistant",
            "content":[
              {"type":"thinking","thinking":"Consider the greeting."},
              {"type":"text","text":"Hlo Himanshu!"},
              {"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"/tmp/x"}}
            ],
            "api":"openai-completions","provider":"soket","model":"qwen3-8-27b",
            "responseModel":"qwen3-8-27b","responseId":"resp-9",
            "usage":{"input":4985,"output":71,"cacheRead":0,"cacheWrite":0,"reasoning":50,
                     "totalTokens":5056,"cost":{"input":0.0,"output":0.0,"cacheRead":0.0,
                     "cacheWrite":0.0,"total":0.0}},
            "stopReason":"stop","rawStopReason":"stop","timestamp":1792656482772
          }
        }"#;
        let p = EntryPayload::parse(raw).expect("parses");
        assert_eq!(p.kind, "message");
        let msg = p.message.expect("message present");
        let MessageBody::Known(KnownMessage::Assistant(a)) = &msg else {
            panic!("expected assistant");
        };
        assert_eq!(a.model.as_deref(), Some("qwen3-8-27b"));
        assert!(a.succeeded());
        assert_eq!(a.usage.as_ref().unwrap().input, 4985);
        assert_eq!(a.content.len(), 3);
        assert!(matches!(
            a.content[0],
            AssistantContentBlock::Thinking { .. }
        ));
    }

    #[test]
    fn parses_user_string_content_and_blocks() {
        let raw = r#"{"type":"message","id":"u1","timestamp":1,
          "message":{"role":"user","content":"hello there","timestamp":1}}"#;
        let p = EntryPayload::parse(raw).expect("parses");
        let MessageBody::Known(KnownMessage::User(u)) = p.message.as_ref().unwrap() else {
            panic!("expected user");
        };
        assert_eq!(u.content, serde_json::json!("hello there"));

        let raw_blocks = r#"{"type":"message","message":{"role":"user",
          "content":[{"type":"text","text":"hi"},{"type":"image","source":{"x":1}}]}}"#;
        let p2 = EntryPayload::parse(raw_blocks).expect("parses blocks");
        let MessageBody::Known(KnownMessage::User(u2)) = p2.message.as_ref().unwrap() else {
            panic!("expected user blocks");
        };
        assert!(u2.content.is_array());
    }

    #[test]
    fn parses_tool_result() {
        let raw = r#"{"type":"message","message":{"role":"toolResult",
          "toolCallId":"call_1","toolName":"read","isError":false,
          "content":[{"type":"text","text":"file contents"}],"timestamp":5}}"#;
        let p = EntryPayload::parse(raw).expect("parses");
        let MessageBody::Known(KnownMessage::ToolResult(t)) = p.message.as_ref().unwrap() else {
            panic!("expected toolResult");
        };
        assert_eq!(t.tool_name.as_deref(), Some("read"));
        assert!(!t.is_error.unwrap_or(false));
    }

    #[test]
    fn unknown_role_falls_back_to_other() {
        let raw = r#"{"type":"message","message":{"role":"someFutureRole","x":1}}"#;
        let p = EntryPayload::parse(raw).expect("parses");
        let m = p.message.as_ref().unwrap();
        assert_eq!(m.role(), "someFutureRole");
    }

    #[test]
    fn unknown_block_type_is_other() {
        let raw = r#"{"type":"message","message":{"role":"assistant",
          "content":[{"type":"brandNewBlock","data":1}],"stopReason":"stop"}}"#;
        let p = EntryPayload::parse(raw).expect("parses");
        let MessageBody::Known(KnownMessage::Assistant(a)) = p.message.as_ref().unwrap() else {
            panic!("expected assistant");
        };
        assert!(matches!(a.content[0], AssistantContentBlock::Other));
    }

    #[test]
    fn stop_reasons_classify() {
        let mk = |reason: &str| {
            EntryPayload::parse(&format!(
                r#"{{"type":"message","message":{{"role":"assistant",
                   "content":[],"stopReason":"{reason}"}}}}"#
            ))
            .unwrap()
        };
        for ok in ["stop", "length", "toolUse"] {
            let p = mk(ok);
            let MessageBody::Known(KnownMessage::Assistant(a)) = p.message.as_ref().unwrap() else {
                panic!()
            };
            assert!(a.succeeded(), "{ok} should be success");
            assert!(!a.failed());
        }
        for bad in ["error", "aborted"] {
            let p = mk(bad);
            let MessageBody::Known(KnownMessage::Assistant(a)) = p.message.as_ref().unwrap() else {
                panic!()
            };
            assert!(a.failed(), "{bad} should fail");
            assert!(!a.succeeded());
        }
        // pending / missing are neither success nor failure
        let p = mk("pending");
        let MessageBody::Known(KnownMessage::Assistant(a)) = p.message.as_ref().unwrap() else {
            panic!()
        };
        assert!(!a.succeeded() && !a.failed());
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(EntryPayload::parse("{not json").is_none());
    }

    #[test]
    fn usage_math_saturates_and_totals() {
        let mut a = Usage {
            input: u64::MAX - 5,
            output: 10,
            total_tokens: 100,
            ..Default::default()
        };
        let b = Usage {
            input: 10,
            output: 5,
            total_tokens: 15,
            cost: Cost {
                total: 0.25,
                ..Default::default()
            },
            ..Default::default()
        };
        a.add(&b);
        assert_eq!(a.input, u64::MAX, "saturating add");
        assert_eq!(a.output, 15);
        assert_eq!(a.cost.total, 0.25);
        assert_eq!(a.prompt_tokens(), u64::MAX);
    }

    #[test]
    fn parses_model_change_entry() {
        // Harness serializes entry tags snake_case, fields camelCase.
        let raw = r#"{"type":"model_change","id":"m1","timestamp":9,
          "provider":"soket","modelId":"GLM-5.3"}"#;
        let p = EntryPayload::parse(raw).expect("parses");
        assert_eq!(p.kind, "model_change");
        assert_eq!(p.model_id.as_deref(), Some("GLM-5.3"));
        assert_eq!(p.provider.as_deref(), Some("soket"));
    }
}
