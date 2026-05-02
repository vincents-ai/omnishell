//! LLM integration layer for OmniShell.
//!
//! Provides a unified interface to vincents-ai/llm with:
//! - Global tokio runtime for async LLM calls
//! - System prompt templates per profile (Kids tutor, Agent executor)
//! - Graceful degradation (--no-llm flag, network failures)
//! - Structured response types

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Global tokio runtime for async LLM operations.
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Get or initialize the global tokio runtime.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// LLM configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Whether LLM features are enabled.
    pub enabled: bool,
    /// Model to use (provider-specific).
    #[serde(default = "default_model")]
    pub model: String,
    /// Temperature for generation (0.0 - 2.0).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Maximum tokens to generate.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_model() -> String {
    "default".to_string()
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> u32 {
    1024
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: default_model(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
        }
    }
}

/// LLM response.
#[derive(Debug, Clone)]
pub enum LlmResponse {
    /// Successful response with content.
    Success(String),
    /// LLM is disabled (--no-llm or config).
    Disabled(String),
    /// Network or provider error.
    Error(String),
}

/// System prompt for the given mode.
pub fn system_prompt(mode: Mode) -> String {
    match mode {
        Mode::Kids => {
            "You are OmniTutor, a friendly and encouraging AI assistant for children aged 5-9 \
             who are learning to use the terminal. \
             \n\nRules:\
             \n- Be patient, warm, and encouraging.\
             \n- Explain concepts simply with analogies children understand.\
             \n- NEVER execute commands or suggest dangerous operations.\
             \n- Use emojis and fun examples.\
             \n- If a child asks about something dangerous, gently redirect them.\
             \n- Keep responses short (2-3 sentences max).".to_string()
        }
        Mode::Agent => {
            "You are an AI coding agent operating within OmniShell. \
             \n\nRules:\
             \n- Return responses in JSON format.\
             \n- When generating commands, use the schema: {\"command\": \"...\", \"explanation\": \"...\"}.\
             \n- Be precise and efficient.\
             \n- Prefer idempotent commands.\
             \n- Never suggest sudo or destructive operations.\
             \n- Include error handling in multi-step operations.".to_string()
        }
        Mode::Admin => {
            "You are an AI assistant for an experienced system administrator. \
             \nBe concise and technical. Assume expertise.".to_string()
        }
    }
}

/// The LLM client wrapper.
pub struct LlmClient {
    config: LlmConfig,
    mode: Mode,
}

impl LlmClient {
    /// Create a new LLM client.
    pub fn new(config: LlmConfig, mode: Mode) -> Self {
        Self { config, mode }
    }

    /// Check if LLM is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Send a prompt to the LLM.
    pub async fn query(&self, prompt: &str) -> LlmResponse {
        if !self.config.enabled {
            return LlmResponse::Disabled(
                "LLM is disabled. Use --no-llm to suppress this message.".to_string(),
            );
        }

        let system = system_prompt(self.mode);

        // Build the vincents-llm request
        use vincents_llm::{ProviderManager, provider_manager::ProviderManagerConfig, ChatCompletionRequest, ChatMessage};

        let pm_config = ProviderManagerConfig {
            default_provider: self.config.model.clone(),
            ..Default::default()
        };
        let manager = ProviderManager::new(pm_config);
        let provider = match manager.get_default_provider() {
            Some(p) => p,
            None => {
                // No provider configured — graceful degradation
                return LlmResponse::Error(
                    "No LLM provider configured. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or run Ollama locally.".to_string()
                );
            }
        };

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage::System { content: system, name: None },
                ChatMessage::User { content: prompt.to_string(), name: None },
            ],
            temperature: Some(self.config.temperature as f64),
            max_tokens: Some(self.config.max_tokens),
            ..Default::default()
        };

        match provider.chat_completion(request).await {
            Ok(response) => {
                let content = response.choices.first()
                    .and_then(|c| match &c.message {
                        ChatMessage::Assistant { content, .. } => content.clone(),
                        _ => None,
                    })
                    .unwrap_or_default();
                LlmResponse::Success(content)
            },
            Err(e) => LlmResponse::Error(format!("LLM error: {e}")),
        }
    }

    /// Send a query synchronously (blocks on the global runtime).
    pub fn query_sync(&self, prompt: &str) -> LlmResponse {
        runtime().block_on(self.query(prompt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_kids() {
        let prompt = system_prompt(Mode::Kids);
        assert!(prompt.contains("OmniTutor"));
        assert!(prompt.contains("children"));
        assert!(prompt.contains("NEVER execute"));
    }

    #[test]
    fn test_system_prompt_agent() {
        let prompt = system_prompt(Mode::Agent);
        assert!(prompt.contains("JSON format"));
        assert!(prompt.contains("coding agent"));
    }

    #[test]
    fn test_system_prompt_admin() {
        let prompt = system_prompt(Mode::Admin);
        assert!(prompt.contains("system administrator"));
    }

    #[test]
    fn test_llm_client_disabled() {
        let config = LlmConfig {
            enabled: false,
            ..Default::default()
        };
        let client = LlmClient::new(config, Mode::Admin);
        let response = client.query_sync("test prompt");
        assert!(matches!(response, LlmResponse::Disabled(_)));
    }

    #[test]
    fn test_llm_client_graceful_degradation() {
        let config = LlmConfig::default();
        let client = LlmClient::new(config, Mode::Agent);
        let response = client.query_sync("build the project");

        // Should get a graceful error (not a panic)
        match response {
            LlmResponse::Error(msg) => assert!(
                msg.contains("No LLM provider") || msg.contains("LLM error"),
                "Expected error about missing provider, got: {msg}"
            ),
            _ => panic!("Expected Error response for unconfigured LLM"),
        }
    }

    #[test]
    fn test_llm_config_default() {
        let config = LlmConfig::default();
        assert!(config.enabled);
        assert_eq!(config.temperature, 0.7);
        assert_eq!(config.max_tokens, 1024);
    }

    #[test]
    fn test_runtime_initialization() {
        let rt = runtime();
        let _guard = rt.enter();
        // Should not panic
    }
}
