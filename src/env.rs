//! `TINYMEMES_*` environment configuration.
//!
//! Standalone, these settings are the whole configuration. Inside a host such
//! as OpenHuman they are overrides: anything set here wins over the host's own
//! inference and Jev settings, and anything unset falls through to the host.
//!
//! | variable | meaning |
//! | --- | --- |
//! | `TINYMEMES_OPENROUTER_KEY` | use OpenRouter for the rewrite model and web research (falls back to `OPENROUTER_API_KEY` only in standalone mode) |
//! | `TINYMEMES_MODEL` | rewrite model id (default `deepseek/deepseek-v4-flash`) |
//! | `TINYMEMES_JEV` | `auto` (default), `typesafe`, `openrouter`, `openjev`, or `llm` (answer Jev's questions with the chat model) |
//! | `TINYMEMES_TYPESAFE_KEY` / `TYPESAFE_API_KEY` | TypeSafe System One key |
//! | `TINYMEMES_OPENJEV_KEY` / `OPENJEV_API_KEY` | OpenJEV key |

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::llm_eval::LlmEvaluator;
use crate::model::{ChatModel, OpenAiCompatible};
use crate::reading::Evaluator;
use crate::slang::OpenRouterWebResearcher;

/// Default rewrite model.
pub const DEFAULT_MODEL: &str = "deepseek/deepseek-v4-flash";

/// How Jev is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JevMode {
    /// Use the first route that has a credential; fall back to the LLM.
    Auto,
    /// TypeSafe's first-party System One API.
    TypeSafe,
    /// OpenRouter's System One API.
    OpenRouter,
    /// OpenJEV's System One API.
    OpenJev,
    /// No Jev: the chat model answers the same questions ([`LlmEvaluator`]).
    Llm,
}

impl JevMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Self::Auto),
            "typesafe" => Some(Self::TypeSafe),
            "openrouter" => Some(Self::OpenRouter),
            "openjev" | "open_jev" => Some(Self::OpenJev),
            "llm" | "off" | "none" => Some(Self::Llm),
            _ => None,
        }
    }
}

/// Parsed `TINYMEMES_*` settings. `None` means "not set; defer to the host".
#[derive(Clone, Default)]
pub struct EnvConfig {
    /// OpenRouter key for the rewrite model and web research.
    pub openrouter_key: Option<String>,
    /// Rewrite model id.
    pub model: Option<String>,
    /// Jev route.
    pub jev: Option<JevMode>,
    /// TypeSafe key.
    pub typesafe_key: Option<String>,
    /// OpenJEV key.
    pub openjev_key: Option<String>,
}

impl std::fmt::Debug for EnvConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvConfig")
            .field(
                "openrouter_key",
                &self.openrouter_key.as_ref().map(|_| "<set>"),
            )
            .field("model", &self.model)
            .field("jev", &self.jev)
            .field("typesafe_key", &self.typesafe_key.as_ref().map(|_| "<set>"))
            .field("openjev_key", &self.openjev_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

impl EnvConfig {
    /// Read the process environment (host-override semantics: only
    /// `TINYMEMES_*` names, plus the standard Jev key names).
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from any lookup (tests, hosts with their own env layer).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let get = |k: &str| {
            get(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let jev = get("TINYMEMES_JEV").map(|v| {
            JevMode::parse(&v).unwrap_or_else(|| {
                tracing::warn!("[tinymemes] unknown TINYMEMES_JEV value; using auto");
                JevMode::Auto
            })
        });
        Self {
            openrouter_key: get("TINYMEMES_OPENROUTER_KEY"),
            model: get("TINYMEMES_MODEL"),
            jev,
            typesafe_key: get("TINYMEMES_TYPESAFE_KEY").or_else(|| get("TYPESAFE_API_KEY")),
            openjev_key: get("TINYMEMES_OPENJEV_KEY").or_else(|| get("OPENJEV_API_KEY")),
        }
    }

    /// The rewrite model id to use.
    pub fn model_id(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    /// An OpenRouter chat model, when a key is set.
    pub fn chat_model(&self, http: &reqwest::Client) -> Option<Arc<dyn ChatModel>> {
        self.openrouter_key.as_ref().map(|key| {
            Arc::new(OpenAiCompatible::openrouter(
                http.clone(),
                key,
                self.model_id(),
            )) as Arc<dyn ChatModel>
        })
    }

    /// The OpenRouter web researcher, when a key is set.
    pub fn web_researcher(&self, http: &reqwest::Client) -> Option<OpenRouterWebResearcher> {
        self.openrouter_key
            .as_ref()
            .map(|key| OpenRouterWebResearcher::new(http.clone(), key, self.model_id()))
    }

    /// A Jev client from env credentials, if the mode and keys allow one.
    /// `Ok(None)` means "no env Jev; use the host's, or the LLM fallback".
    /// Returns the client and a short route label for logs.
    pub fn jev(&self) -> Result<Option<(Arc<dyn Evaluator>, &'static str)>> {
        use tinyinference_decisions::{Client, ClientConfig};
        let build = |config: ClientConfig, label: &'static str| {
            Client::new(config)
                .map(|c| Some((Arc::new(c) as Arc<dyn Evaluator>, label)))
                .map_err(|e| Error::Reading(Box::new(e)))
        };
        match self.jev {
            Some(JevMode::Llm) => Ok(None),
            Some(JevMode::TypeSafe) => match &self.typesafe_key {
                Some(k) => build(ClientConfig::new(k), "typesafe"),
                None => Err(Error::Reading(
                    "TINYMEMES_JEV=typesafe but no TypeSafe key".into(),
                )),
            },
            Some(JevMode::OpenJev) => match &self.openjev_key {
                Some(k) => build(ClientConfig::openjev(k), "openjev"),
                None => Err(Error::Reading(
                    "TINYMEMES_JEV=openjev but no OpenJEV key".into(),
                )),
            },
            Some(JevMode::OpenRouter) => match &self.openrouter_key {
                Some(k) => build(ClientConfig::openrouter(k), "openrouter"),
                None => Err(Error::Reading(
                    "TINYMEMES_JEV=openrouter but no OpenRouter key".into(),
                )),
            },
            // Auto: only env credentials count here; the host decides the rest.
            Some(JevMode::Auto) | None => {
                if let Some(k) = &self.typesafe_key {
                    build(ClientConfig::new(k), "typesafe")
                } else if let Some(k) = &self.openjev_key {
                    build(ClientConfig::openjev(k), "openjev")
                } else if self.jev == Some(JevMode::Auto) {
                    match &self.openrouter_key {
                        Some(k) => build(ClientConfig::openrouter(k), "openrouter"),
                        None => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Whether the env forces the LLM fallback instead of Jev.
    pub fn forces_llm_jev(&self) -> bool {
        self.jev == Some(JevMode::Llm)
    }
}

/// The LLM fallback evaluator over `model`.
pub fn llm_jev(model: Arc<dyn ChatModel>) -> Arc<dyn Evaluator> {
    Arc::new(LlmEvaluator::new(model))
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
