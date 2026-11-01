//! Inference parameters the engine actually supports, validated on both ends.
//!
//! `loop-ai`'s OpenAI-completions builder sends exactly: `temperature`,
//! `max_tokens` / `max_completion_tokens`, and reasoning effort. There is no
//! `top_p` path, so no `top_p` control is exposed — a control that does
//! nothing would be dishonest.

use serde::{Deserialize, Serialize};

/// Supported inference parameters.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InferenceParams {
    /// Sampling temperature (0.0–2.0).
    pub temperature: Option<f64>,
    /// Maximum output tokens (1–200_000).
    pub max_tokens: Option<u32>,
    /// Reasoning effort level.
    pub reasoning: Option<ReasoningLevel>,
}

/// Reasoning effort levels understood by `loop-ai`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    /// Off.
    Off,
    /// Minimal.
    Minimal,
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
    /// Extra-high.
    Xhigh,
    /// Maximum.
    Max,
}

impl ReasoningLevel {
    /// Convert to loop-ai's `ThinkingLevel`.
    pub fn to_thinking(self) -> Option<loop_ai::ThinkingLevel> {
        Some(match self {
            Self::Off => return None,
            Self::Minimal => loop_ai::ThinkingLevel::Minimal,
            Self::Low => loop_ai::ThinkingLevel::Low,
            Self::Medium => loop_ai::ThinkingLevel::Medium,
            Self::High => loop_ai::ThinkingLevel::High,
            Self::Xhigh => loop_ai::ThinkingLevel::XHigh,
            Self::Max => loop_ai::ThinkingLevel::Max,
        })
    }
}

/// Validation error for parameters.
#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    /// Temperature out of range.
    #[error("temperature must be between 0.0 and 2.0")]
    Temperature,
    /// max_tokens out of range.
    #[error("maxTokens must be between 1 and 200000")]
    MaxTokens,
}

impl InferenceParams {
    /// Validate; returns a human-readable error when invalid.
    pub fn validate(&self) -> Result<(), ParamsError> {
        if let Some(t) = self.temperature {
            if !(0.0..=2.0).contains(&t) || t.is_nan() {
                return Err(ParamsError::Temperature);
            }
        }
        if let Some(m) = self.max_tokens {
            if m == 0 || m > 200_000 {
                return Err(ParamsError::MaxTokens);
            }
        }
        Ok(())
    }

    /// Whitelisted JSON for trace storage (no secrets, bounded size).
    pub fn trace_json(self) -> String {
        serde_json::json!({
            "temperature": self.temperature,
            "maxTokens": self.max_tokens,
            "reasoning": self.reasoning.map(|r| serde_json::to_value(r).unwrap_or_default()),
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ranges() {
        assert!(InferenceParams::default().validate().is_ok());
        assert!(InferenceParams {
            temperature: Some(1.5),
            ..Default::default()
        }
        .validate()
        .is_ok());
        assert!(InferenceParams {
            temperature: Some(2.5),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(InferenceParams {
            temperature: Some(f64::NAN),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(InferenceParams {
            max_tokens: Some(0),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(InferenceParams {
            max_tokens: Some(200_001),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(InferenceParams {
            max_tokens: Some(4096),
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn trace_json_is_whitelisted() {
        let p = InferenceParams {
            temperature: Some(0.7),
            max_tokens: Some(1024),
            reasoning: Some(ReasoningLevel::High),
        };
        let json = p.trace_json();
        assert!(json.contains("temperature"));
        assert!(json.contains("maxTokens"));
        assert!(json.contains("reasoning"));
        // No secret-shaped keys ever appear.
        assert!(!json.to_lowercase().contains("key"));
        assert!(!json.to_lowercase().contains("authorization"));
    }

    #[test]
    fn reasoning_maps_to_thinking_level() {
        assert_eq!(ReasoningLevel::Off.to_thinking(), None);
        assert_eq!(
            ReasoningLevel::High.to_thinking(),
            Some(loop_ai::ThinkingLevel::High)
        );
        assert_eq!(
            ReasoningLevel::Xhigh.to_thinking(),
            Some(loop_ai::ThinkingLevel::XHigh)
        );
    }
}
