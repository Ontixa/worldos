//! Model providers: pluggable LLM backends (BYOK).
//!
//! The runtime never hardcodes a vendor. `OpenAiCompatible` covers
//! OpenAI-style `/chat/completions` endpoints (OpenAI, Azure, local
//! servers like Ollama/LM Studio, enterprise gateways). Keys come from
//! environment variables — never committed.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider not configured: {0}")]
    NotConfigured(String),
    #[error("request failed: {0}")]
    Request(String),
    #[error("bad response: {0}")]
    Response(String),
}

/// A text-completion model endpoint. Deliberately minimal: planners send
/// one prompt, expect one text body back.
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn complete(&self, prompt: &str) -> Result<String, ProviderError>;
}

/// Provider configuration resolved from environment / config files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: String, // "openai-compatible" | "echo"
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// NAME of the env var holding the API key (never the key itself).
    pub api_key_env: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: std::env::var("WORLDOS_LLM_KIND").unwrap_or_else(|_| "echo".into()),
            base_url: std::env::var("WORLDOS_LLM_BASE_URL").ok(),
            model: std::env::var("WORLDOS_LLM_MODEL").ok(),
            api_key_env: std::env::var("WORLDOS_LLM_API_KEY_ENV")
                .ok()
                .or(Some("OPENAI_API_KEY".into())),
        }
    }
}

/// Offline deterministic provider — returns a fixed explanatory string.
/// Keeps the provider path exercised in tests without network access.
pub struct EchoProvider;

impl ModelProvider for EchoProvider {
    fn id(&self) -> &str {
        "echo"
    }
    fn complete(&self, prompt: &str) -> Result<String, ProviderError> {
        Ok(format!("echo: {} bytes of prompt received", prompt.len()))
    }
}

/// OpenAI-compatible chat completions provider.
#[cfg(feature = "llm")]
pub struct OpenAiCompatible {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[cfg(feature = "llm")]
impl OpenAiCompatible {
    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .or_else(|_| std::env::var("WORLDOS_LLM_API_KEY"))
            .map_err(|_| ProviderError::NotConfigured("set OPENAI_API_KEY".into()))?;
        Ok(Self {
            base_url: std::env::var("WORLDOS_LLM_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            model: std::env::var("WORLDOS_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
            api_key,
        })
    }
}

#[cfg(feature = "llm")]
impl ModelProvider for OpenAiCompatible {
    fn id(&self) -> &str {
        "openai-compatible"
    }
    fn complete(&self, prompt: &str) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.2,
        });
        let mut resp = ureq::post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let json: serde_json::Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Response(e.to_string()))?;
        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ProviderError::Response("missing content".into()))
    }
}
