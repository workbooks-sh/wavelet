//! Concrete [`LlmBackend`] impl over `fal-ai/any-llm` (text-only).
//!
//! Routes the system + user prompt through Fal's model-router endpoint
//! at `https://fal.run/fal-ai/any-llm`. Two routed models are
//! pre-defined: Gemini 2.5 Pro and Claude Opus 4.7. Gemini is the
//! default (cheaper, JSON-mode-friendlier in our smokes); Claude is
//! the fallback the CLI exposes via `--model claude`.
//!
//! Wire format (verified live 2026-05-18):
//! ```text
//! POST https://fal.run/fal-ai/any-llm
//! { "prompt": "...", "system_prompt": "...", "model": "google/gemini-2.5-pro" }
//! → { "output": "{...}", "reasoning": null, "partial": false, "error": null }
//! ```
//!
//! Cost: ~$0.01–0.03 per synthesis call depending on shot count + model.

use serde::{Deserialize, Serialize};

use crate::backends::fal::FalClient;
use crate::backends::BackendError;

use super::creative_director::LlmBackend;

/// `fal-ai/any-llm` text endpoint path.
pub const ANY_LLM_PATH: &str = "fal-ai/any-llm";

/// Gemini 2.5 Pro routed-model identifier. Primary default — best
/// JSON adherence in director smokes.
pub const MODEL_GEMINI_PRO: &str = "google/gemini-2.5-pro";

/// Claude Opus 4.7 routed-model identifier. Fallback when Gemini is
/// degraded or the agent wants a second opinion.
pub const MODEL_CLAUDE_OPUS: &str = "anthropic/claude-opus-4-7";

/// FAL any-llm backend for the creative director.
#[derive(Debug, Clone)]
pub struct FalAnyLlmBackend {
    client: FalClient,
    model: String,
}

impl FalAnyLlmBackend {
    /// Build with an explicit routed model.
    pub fn new(client: FalClient, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }

    /// Build the Gemini 2.5 Pro variant.
    pub fn gemini_pro(client: FalClient) -> Self {
        Self::new(client, MODEL_GEMINI_PRO)
    }

    /// Build the Claude Opus 4.7 variant.
    pub fn claude_opus(client: FalClient) -> Self {
        Self::new(client, MODEL_CLAUDE_OPUS)
    }

    /// Routed-model id this adapter sends.
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl LlmBackend for FalAnyLlmBackend {
    fn complete(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        retry_followup: Option<&str>,
    ) -> Result<String, BackendError> {
        // any-llm is single-turn; concatenate the retry message onto
        // the user prompt with an explicit divider so the model can't
        // miss the correction.
        let prompt = match retry_followup {
            None => user_prompt.to_string(),
            Some(retry) => format!(
                "{user_prompt}\n\n---\nFollow-up correction:\n{retry}"
            ),
        };
        let body = AnyLlmBody {
            prompt,
            system_prompt: system_prompt.to_string(),
            model: self.model.clone(),
        };
        let resp: AnyLlmResponse = self.client.post_sync(ANY_LLM_PATH, &body)?;
        if let Some(err) = resp.error.as_ref() {
            return Err(BackendError::HttpStatus {
                status: 502,
                body: err.clone(),
            });
        }
        Ok(resp.output)
    }
}

#[derive(Debug, Serialize)]
struct AnyLlmBody {
    prompt: String,
    system_prompt: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct AnyLlmResponse {
    #[serde(default)]
    output: String,
    #[serde(default)]
    error: Option<String>,
}

/// Resolve the `--model` CLI flag (`gemini` | `claude`) to a routed
/// model identifier. Defaults to Gemini.
pub fn resolve_model_flag(flag: &str) -> &'static str {
    match flag.trim().to_ascii_lowercase().as_str() {
        "claude" | "opus" | "anthropic" => MODEL_CLAUDE_OPUS,
        _ => MODEL_GEMINI_PRO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_defaults_to_gemini() {
        assert_eq!(resolve_model_flag(""), MODEL_GEMINI_PRO);
        assert_eq!(resolve_model_flag("gemini"), MODEL_GEMINI_PRO);
        assert_eq!(resolve_model_flag("GEMINI"), MODEL_GEMINI_PRO);
        assert_eq!(resolve_model_flag("unknown"), MODEL_GEMINI_PRO);
    }

    #[test]
    fn flag_resolves_claude_aliases() {
        assert_eq!(resolve_model_flag("claude"), MODEL_CLAUDE_OPUS);
        assert_eq!(resolve_model_flag("opus"), MODEL_CLAUDE_OPUS);
        assert_eq!(resolve_model_flag("anthropic"), MODEL_CLAUDE_OPUS);
        assert_eq!(resolve_model_flag("  Claude  "), MODEL_CLAUDE_OPUS);
    }

    #[test]
    fn adapter_records_model_choice() {
        let tmp = std::env::temp_dir().join("wavelet-director-model");
        let client = FalClient::with_key("id:secret", tmp);
        let g = FalAnyLlmBackend::gemini_pro(client.clone());
        let c = FalAnyLlmBackend::claude_opus(client);
        assert_eq!(g.model(), MODEL_GEMINI_PRO);
        assert_eq!(c.model(), MODEL_CLAUDE_OPUS);
    }
}
