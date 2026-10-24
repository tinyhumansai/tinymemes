//! Builds one engine for one remix: the host callbacks as the engine's chat
//! model, Jev evaluator and web search, with the `TINYMEMES_*` environment
//! overrides applied on top.
//!
//! The module is loaded into the host process, so it reads its own overrides
//! from the process environment (see `tinymemes::env`). Credentials in those
//! variables therefore never cross the bus.
//!
//! | piece | default (the host's callbacks) | override |
//! | --- | --- | --- |
//! | chat model | `Complete` | `TINYMEMES_OPENROUTER_KEY` (+ `TINYMEMES_MODEL`) |
//! | Jev | `Evaluate`; the chat model answers Jev's questions when the host has no Jev | `TINYMEMES_JEV` (+ its key variables) |
//! | slang research | `Search` + the chat model | `TINYMEMES_OPENROUTER_KEY` (OpenRouter web plugin) |
//! | meme research | `Search` + GIPHY pages | none |

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tinymemes::decisions::{EvaluationRequest, EvaluationResponse};
use tinymemes::{
    BoxError, ChatModel, EnvConfig, Evaluator, GiphyPageResearcher, MemeEngine, MemeIndex,
    MemeSource, RatingPolicy, Region, SearchHit, SearchResearcher, SlangIndex, SlangResearcher,
    WebSearch,
};
use tinymemes_bus::{CompleteRequest, HostFailure, RemixRequest, SearchRequest};

use crate::host::SharedHost;

pub(crate) struct HostModel(pub(crate) SharedHost);
#[async_trait]
impl ChatModel for HostModel {
    async fn complete(&self, system: &str, user: &str) -> Result<String, BoxError> {
        let request = CompleteRequest {
            correlation: String::new(),
            system: system.into(),
            user: user.into(),
        };
        self.0
            .complete(request)
            .await
            .map_err(|e| format!("host complete failed: {e:?}").into())
    }
}

/// Jev through the host, or the chat model when the host has none.
///
/// A host without a Jev route answers `Evaluate` with `Unsupported`; the
/// engine then still needs typed answers, so the same questions go to the chat
/// model (`LlmEvaluator`). Any other failure is the host failing, and fails
/// the reading, which fails open to the original reply.
pub(crate) struct HostEvaluator {
    host: SharedHost,
    fallback: Arc<dyn Evaluator>,
    unsupported: AtomicBool,
}

impl HostEvaluator {
    pub(crate) fn new(host: SharedHost, model: Arc<dyn ChatModel>) -> Self {
        Self {
            host,
            fallback: tinymemes::env::llm_jev(model),
            unsupported: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Evaluator for HostEvaluator {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationResponse, BoxError> {
        if !self.unsupported.load(Ordering::Relaxed) {
            let body = serde_json::to_value(request)?;
            match self.host.evaluate(body).await {
                Ok(reply) => return Ok(serde_json::from_value(reply)?),
                Err(HostFailure::Unsupported) => {
                    tracing::debug!("[tinymemes] host has no jev; the chat model answers instead");
                    self.unsupported.store(true, Ordering::Relaxed);
                }
                Err(error) => return Err(format!("host evaluate failed: {error:?}").into()),
            }
        }
        self.fallback.evaluate(request).await
    }
}

pub(crate) struct HostSearch(pub(crate) SharedHost);
#[async_trait]
impl WebSearch for HostSearch {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchHit>, BoxError> {
        let request = SearchRequest {
            correlation: String::new(),
            query: query.into(),
            max_results: u32::try_from(max_results).unwrap_or(u32::MAX),
        };
        let hits = self
            .0
            .search(request)
            .await
            .map_err(|e| format!("host search failed: {e:?}"))?;
        Ok(hits
            .into_iter()
            .map(|h| SearchHit {
                title: h.title,
                url: h.url,
                snippet: h.snippet,
            })
            .collect())
    }
}

fn http() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

/// What a request is built from beyond the host: the environment overrides and
/// the external meme source. Tests supply their own.
pub(crate) struct Deps {
    pub(crate) env: EnvConfig,
    pub(crate) source: Arc<dyn MemeSource>,
}

impl Deps {
    /// The process environment and Imgflip.
    pub(crate) fn production() -> Self {
        Self {
            env: EnvConfig::from_env(),
            source: Arc::new(tinymemes::source::Imgflip::new(http())),
        }
    }
}

/// The engine for one request and the indexes it shares.
pub(crate) struct Built {
    pub(crate) engine: MemeEngine,
    pub(crate) slang: Arc<SlangIndex>,
    pub(crate) memes: Arc<MemeIndex>,
}

/// Build the engine for `request` over `host`, restoring `slang` and `memes`.
pub(crate) fn build(
    host: &SharedHost,
    request: &RemixRequest,
    deps: &Deps,
    slang: SlangIndex,
    memes: MemeIndex,
) -> Built {
    let env = &deps.env;
    let http = http();
    let host_model: Arc<dyn ChatModel> = Arc::new(HostModel(host.clone()));
    let (chat, chat_label): (Arc<dyn ChatModel>, String) = match env.chat_model(&http) {
        Some(model) => (model, format!("env:openrouter/{}", env.model_id())),
        None => (host_model, "host".to_owned()),
    };

    // Jev: the host's, unless TINYMEMES_JEV overrides it; the chat model
    // answers when neither exists.
    let (jev, jev_label): (Arc<dyn Evaluator>, &str) = if env.forces_llm_jev() {
        (
            tinymemes::env::llm_jev(chat.clone()),
            "llm (TINYMEMES_JEV=llm)",
        )
    } else {
        let overridden = match env.jev {
            Some(_) => env.jev().unwrap_or_else(|e| {
                tracing::warn!("[tinymemes] TINYMEMES_JEV override unusable, using the host: {e}");
                None
            }),
            None => None,
        };
        match overridden {
            Some((jev, label)) => (jev, label),
            None => (
                Arc::new(HostEvaluator::new(host.clone(), chat.clone())),
                "host (llm fallback)",
            ),
        }
    };

    let search: Option<Arc<dyn WebSearch>> = request
        .options
        .research
        .then(|| Arc::new(HostSearch(host.clone())) as Arc<dyn WebSearch>);
    let (researcher, research_label): (Option<Arc<dyn SlangResearcher>>, &str) =
        match env.web_researcher(&http) {
            Some(r) => (Some(Arc::new(r)), "env:openrouter-web"),
            None => match search.clone() {
                Some(search) => (
                    Some(Arc::new(SearchResearcher::new(search, chat.clone()))),
                    "host-search",
                ),
                None => (None, "off"),
            },
        };

    let region = match request.region.as_deref() {
        Some(code) if code.eq_ignore_ascii_case("global") => Region::global(),
        _ => Region::default(),
    };
    let slang = Arc::new(slang);
    let memes = Arc::new(memes);
    let mut policy = RatingPolicy::default();
    if let Some(turns) = request.options.meme_cooldown_turns {
        policy.meme_cooldown_turns = turns as usize;
    }
    let mut builder = MemeEngine::builder(jev, chat)
        .region(region)
        .policy(policy)
        .slang_index(slang.clone())
        .meme_index(memes.clone())
        .source(deps.source.clone())
        // Research runs in the background after delivery, never on the
        // reply's critical path.
        .learn_inline(None);
    if let Some(researcher) = researcher {
        builder = builder.researcher(researcher);
    }
    let meme_research_label = match search {
        Some(search) => {
            builder = builder.meme_researcher(Arc::new(GiphyPageResearcher::new(search, http)));
            "giphy-pages"
        }
        None => "off",
    };
    tracing::debug!(
        correlation = %request.correlation,
        chat = %chat_label,
        jev = jev_label,
        research = research_label,
        meme_research = meme_research_label,
        "[tinymemes] engine ready"
    );
    Built {
        engine: builder.build(),
        slang,
        memes,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
