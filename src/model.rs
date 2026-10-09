//! The chat model behind the slang agent.

use std::fmt;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::error::BoxError;

/// A single-shot chat completion. OpenHuman plugs its own provider in here;
/// [`OpenAiCompatible`] covers OpenRouter and any OpenAI-style endpoint.
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// Complete one system + user exchange and return the assistant text.
    async fn complete(&self, system: &str, user: &str) -> Result<String, BoxError>;
}

/// An OpenAI-compatible `/chat/completions` client.
#[derive(Clone)]
pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    /// Send OpenRouter's `reasoning: {enabled: false}`. A rewrite needs no
    /// thinking, and reasoning models otherwise spend ~10s and 1k+ hidden
    /// tokens per call.
    disable_reasoning: bool,
}

impl fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatible")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OpenAiCompatible {
    /// A client for `base_url` (without the `/chat/completions` suffix).
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            disable_reasoning: false,
        }
    }

    /// A client for OpenRouter.
    pub fn openrouter(
        http: reqwest::Client,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            disable_reasoning: true,
            ..Self::new(http, "https://openrouter.ai/api/v1", api_key, model)
        }
    }
}

#[async_trait]
impl ChatModel for OpenAiCompatible {
    async fn complete(&self, system: &str, user: &str) -> Result<String, BoxError> {
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<ChoiceMsg>,
        }
        #[derive(Deserialize)]
        struct ChoiceMsg {
            message: Msg,
        }
        #[derive(Deserialize)]
        struct Msg {
            content: Option<String>,
        }
        let mut body = json!({
            "model": self.model,
            "temperature": 0.8,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        if self.disable_reasoning {
            body["reasoning"] = json!({ "enabled": false });
        }
        let resp: Resp = self
            .http
            .post(format!(
                "{}/chat/completions",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        resp.choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| "model returned no content".into())
    }
}
