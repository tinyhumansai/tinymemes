//! TinyMemes makes agent replies more fun, when the chat can take it.
//!
//! For each reply the engine:
//!
//! 1. **Reads** the whole conversation with one Jev call: the chat's intent,
//!    the reply's intent, how frank the user is, whether they want jokes, and
//!    whether the topic is too serious for any of this.
//! 2. **Rates** the reply 0–10 with a pure policy ([`RatingPolicy`]) and maps
//!    the score to a [`Tier`].
//! 3. **Remixes** (only above [`Tier::Off`]) with a small agent that plans meme
//!    searches for the reply's intent, fetches candidates, rewrites the reply in
//!    slang, and inlines the chosen memes as markdown images.
//!
//! The engine fails open: any error returns the original reply untouched, and a
//! rewrite that drops code, inline code, or links is discarded in favour of the
//! original wording.
//!
//! ```no_run
//! use std::sync::Arc;
//! use tinymemes::{MemeEngine, Turn, model::OpenAiCompatible, source::Imgflip};
//! use tinyinference_decisions::{Client, ClientConfig};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let http = reqwest::Client::new();
//! let key = std::env::var("OPENROUTER_API_KEY")?;
//! let engine = MemeEngine::builder(
//!     Arc::new(Client::new(ClientConfig::openrouter(&key))?),
//!     Arc::new(OpenAiCompatible::openrouter(http.clone(), &key, "google/gemini-2.5-flash")),
//! )
//! .source(Arc::new(Imgflip::new(http)))
//! .build();
//!
//! let chat = [Turn::user("bro my build finally passed after 3 hours lmao")];
//! let out = engine.process(&chat, "Congrats! The fix was the missing feature flag.").await;
//! println!("{} (rating {})", out.reply, out.rating.map_or(0, |r| r.score));
//! # Ok(())
//! # }
//! ```

pub mod agent;
pub mod conversation;
mod error;
pub mod intent;
pub mod model;
pub mod rating;
pub mod reading;
pub mod source;

use std::sync::Arc;

use serde::Serialize;

pub use agent::{Remix, SlangAgent};
pub use conversation::{Role, Turn, Window};
pub use error::{BoxError, Error, Result};
pub use intent::Intent;
pub use rating::{Rating, RatingPolicy, Tier};
pub use reading::{Evaluator, Reading};
pub use source::{Meme, MemeSource};

/// Everything the engine decided about one reply.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    /// The reply to send: remixed, or the original.
    pub reply: String,
    /// Jev's reading, when it succeeded.
    pub reading: Option<Reading>,
    /// The rating, when a reading succeeded.
    pub rating: Option<Rating>,
    /// The remix, when the agent ran.
    pub remix: Option<Remix>,
    /// Why the original reply was returned unchanged, if it was.
    pub skipped: Option<String>,
}

/// Reads, rates, and remixes agent replies.
#[derive(Clone)]
pub struct MemeEngine {
    jev: Arc<dyn Evaluator>,
    agent: SlangAgent,
    policy: RatingPolicy,
    window: Window,
    openjev: bool,
}

impl std::fmt::Debug for MemeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemeEngine")
            .field("agent", &self.agent)
            .field("policy", &self.policy)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl MemeEngine {
    /// Start building an engine from a Jev evaluator and the agent's chat model.
    pub fn builder(jev: Arc<dyn Evaluator>, model: Arc<dyn ChatModel>) -> MemeEngineBuilder {
        MemeEngineBuilder {
            jev,
            model,
            sources: Vec::new(),
            policy: RatingPolicy::default(),
            window: Window::default(),
            openjev: false,
        }
    }

    /// Read and rate only; no rewrite. Useful for hosts that want the signal.
    pub async fn rate(&self, conversation: &[Turn], reply: &str) -> Result<(Reading, Rating)> {
        let reading = reading::read(
            self.jev.as_ref(),
            conversation,
            reply,
            self.window,
            self.openjev,
        )
        .await?;
        let rating = self.policy.rate(&reading);
        Ok((reading, rating))
    }

    /// Read, rate, and remix, returning errors instead of falling back.
    pub async fn try_process(&self, conversation: &[Turn], reply: &str) -> Result<Outcome> {
        let (reading, rating) = self.rate(conversation, reply).await?;
        tracing::debug!(
            chat = %reading.chat_intent,
            reply = %reading.reply_intent,
            frankness = reading.frankness,
            playful = reading.playful,
            serious = reading.serious,
            score = rating.score,
            tier = ?rating.tier,
            "[tinymemes] rated reply"
        );
        if !rating.tier.remixes() {
            return Ok(Outcome {
                reply: reply.to_owned(),
                reading: Some(reading),
                rating: Some(rating),
                remix: None,
                skipped: Some(format!(
                    "rating {} is below the remix threshold",
                    rating.score
                )),
            });
        }
        let remix = self.agent.run(reply, &reading, rating).await?;
        Ok(Outcome {
            reply: remix.reply.clone(),
            reading: Some(reading),
            rating: Some(rating),
            remix: Some(remix),
            skipped: None,
        })
    }

    /// Read, rate, and remix. Never fails: on any error the original reply is
    /// returned with the reason in [`Outcome::skipped`].
    pub async fn process(&self, conversation: &[Turn], reply: &str) -> Outcome {
        match self.try_process(conversation, reply).await {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::warn!(error = %e, "[tinymemes] remix failed; sending original reply");
                Outcome {
                    reply: reply.to_owned(),
                    reading: None,
                    rating: None,
                    remix: None,
                    skipped: Some(e.to_string()),
                }
            }
        }
    }
}

pub use model::ChatModel;

/// Builder for [`MemeEngine`].
pub struct MemeEngineBuilder {
    jev: Arc<dyn Evaluator>,
    model: Arc<dyn ChatModel>,
    sources: Vec<Arc<dyn MemeSource>>,
    policy: RatingPolicy,
    window: Window,
    openjev: bool,
}

impl std::fmt::Debug for MemeEngineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemeEngineBuilder").finish_non_exhaustive()
    }
}

impl MemeEngineBuilder {
    /// Add a meme source. Sources are searched concurrently.
    pub fn source(mut self, source: Arc<dyn MemeSource>) -> Self {
        self.sources.push(source);
        self
    }

    /// Override the rating policy.
    pub fn policy(mut self, policy: RatingPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Override how much of the chat Jev sees.
    pub fn window(mut self, window: Window) -> Self {
        self.window = window;
        self
    }

    /// Use OpenJEV's `openjev` model id instead of `jev-latest`.
    pub fn openjev(mut self, on: bool) -> Self {
        self.openjev = on;
        self
    }

    /// Finish building.
    pub fn build(self) -> MemeEngine {
        MemeEngine {
            jev: self.jev,
            agent: SlangAgent::new(self.model, self.sources),
            policy: self.policy,
            window: self.window,
            openjev: self.openjev,
        }
    }
}
