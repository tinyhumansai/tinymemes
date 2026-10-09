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
pub mod region;
pub mod slang;
pub mod source;

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

pub use agent::{Remix, SlangAgent};
pub use conversation::{Role, SentMeme, Turn, Window, memes_as_text};
pub use error::{BoxError, Error, Result};
pub use intent::Intent;
pub use rating::{Rating, RatingPolicy, Tier};
pub use reading::{Evaluator, Reading};
pub use region::{CatalogMeme, Region, SlangTerm};
pub use slang::{IndexPolicy, LearnReport, SlangIndex, SlangResearcher};
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
    /// Slang research that ran for this reply, if any.
    pub learned: Option<LearnReport>,
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
    researcher: Option<Arc<dyn SlangResearcher>>,
    learn_inline: Option<Duration>,
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
    /// An engine builder wired to OpenRouter for everything: Jev (System
    /// One), the rewrite model `model`, web slang research, and Imgflip memes.
    /// The region defaults to India; add a slang index or other options on the
    /// returned builder.
    pub fn openrouter(api_key: &str, model: &str) -> Result<MemeEngineBuilder> {
        let http = reqwest::Client::new();
        let jev = tinyinference_decisions::Client::new(
            tinyinference_decisions::ClientConfig::openrouter(api_key),
        )
        .map_err(|e| Error::Reading(Box::new(e)))?;
        Ok(Self::builder(
            Arc::new(jev),
            Arc::new(model::OpenAiCompatible::openrouter(
                http.clone(),
                api_key,
                model,
            )),
        )
        .researcher(Arc::new(slang::OpenRouterWebResearcher::new(
            http.clone(),
            api_key,
            model,
        )))
        .source(Arc::new(source::Imgflip::new(http))))
    }

    /// Start building an engine from a Jev evaluator and the agent's chat model.
    pub fn builder(jev: Arc<dyn Evaluator>, model: Arc<dyn ChatModel>) -> MemeEngineBuilder {
        MemeEngineBuilder {
            jev,
            model,
            sources: Vec::new(),
            region: Region::default(),
            index: None,
            researcher: None,
            learn_inline: Some(Duration::from_secs(25)),
            policy: RatingPolicy::default(),
            window: Window::default(),
            openjev: false,
        }
    }

    /// The slang index, to snapshot or inspect.
    pub fn slang_index(&self) -> &Arc<SlangIndex> {
        self.agent.index()
    }

    /// Run one slang research step for `intent` (the next query template that
    /// has not run recently). The engine calls this when Jev judges the index
    /// short of slang for a reply; hosts that turn off inline learning can call
    /// it in the background when [`Reading::wants_more_slang`] says so.
    pub async fn learn_slang(&self, intent: Intent) -> Result<Option<LearnReport>> {
        let Some(researcher) = &self.researcher else {
            return Ok(None);
        };
        self.agent
            .index()
            .learn(
                researcher.as_ref(),
                Some((self.jev.as_ref(), self.openjev)),
                self.agent.region(),
                intent,
            )
            .await
            .map_err(Error::Research)
    }

    /// Search for slang that fits `reply`, verify it with Jev, and add it to
    /// the index. For hosts that research in the background when
    /// [`Reading::wants_more_slang`] is true instead of inline.
    pub async fn learn_for_reply(
        &self,
        intent: Intent,
        reply: &str,
    ) -> Result<Option<LearnReport>> {
        let Some(researcher) = &self.researcher else {
            return Ok(None);
        };
        self.agent
            .index()
            .learn_for_reply(
                researcher.as_ref(),
                Some((self.jev.as_ref(), self.openjev)),
                self.agent.region(),
                intent,
                reply,
            )
            .await
            .map_err(Error::Research)
    }

    /// Read and rate only; no rewrite. Useful for hosts that want the signal.
    ///
    /// The same Jev call also judges the index's top slang against the reply
    /// (`slang_best`, `slang_enough`), which decides whether to search.
    pub async fn rate(&self, conversation: &[Turn], reply: &str) -> Result<(Reading, Rating)> {
        let index = self.agent.index();
        let candidates = index.top_terms(&self.agent.region().code, index.policy().jev_candidates);
        let reading = reading::read(
            self.jev.as_ref(),
            conversation,
            reply,
            &self.agent.region().name,
            &candidates,
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
                learned: None,
                skipped: Some(format!(
                    "rating {} is below the remix threshold",
                    rating.score
                )),
            });
        }
        let wants_search = reading.wants_more_slang(self.agent.index().policy().search_below);
        let learned = match (self.learn_inline, &self.researcher) {
            (Some(budget), Some(researcher)) if wants_search => {
                let search = self.agent.index().learn_for_reply(
                    researcher.as_ref(),
                    Some((self.jev.as_ref(), self.openjev)),
                    self.agent.region(),
                    reading.reply_intent,
                    reply,
                );
                match tokio::time::timeout(budget, async { search.await.map_err(Error::Research) })
                    .await
                {
                    Ok(Ok(report)) => report,
                    Ok(Err(e)) => {
                        tracing::debug!(error = %e, "[tinymemes] slang research failed; using current index");
                        None
                    }
                    Err(_) => {
                        tracing::debug!(
                            "[tinymemes] slang research timed out; using current index"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        let remix = self
            .agent
            .run(reply, conversation, &reading, rating)
            .await?;
        Ok(Outcome {
            reply: remix.reply.clone(),
            reading: Some(reading),
            rating: Some(rating),
            remix: Some(remix),
            learned,
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
                    learned: None,
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
    region: Region,
    index: Option<Arc<SlangIndex>>,
    researcher: Option<Arc<dyn SlangResearcher>>,
    learn_inline: Option<Duration>,
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

    /// Set the region pack (slang, meme catalog, blocklist). Defaults to
    /// [`Region::india`].
    pub fn region(mut self, region: Region) -> Self {
        self.region = region;
        self
    }

    /// Share a slang index (e.g. one restored from a snapshot). Defaults to a
    /// fresh in-memory index seeded from the region.
    pub fn slang_index(mut self, index: Arc<SlangIndex>) -> Self {
        self.index = Some(index);
        self
    }

    /// Grow the slang index with web research.
    pub fn researcher(mut self, researcher: Arc<dyn SlangResearcher>) -> Self {
        self.researcher = Some(researcher);
        self
    }

    /// How long a reply may wait for one research step (default 25s). `None`
    /// never researches inline; call [`MemeEngine::learn_slang`] yourself.
    pub fn learn_inline(mut self, budget: Option<Duration>) -> Self {
        self.learn_inline = budget;
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
            agent: SlangAgent::new(
                self.model,
                self.sources,
                self.region,
                self.index.unwrap_or_default(),
            ),
            policy: self.policy,
            window: self.window,
            openjev: self.openjev,
            researcher: self.researcher,
            learn_inline: self.learn_inline,
        }
    }
}
