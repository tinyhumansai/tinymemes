//! Slang research over a host-provided web search.
//!
//! [`SearchResearcher`] lets a host (OpenHuman) supply its own search stack
//! and chat model instead of the OpenRouter web plugin: it searches, shows the
//! results to the model, and keeps only terms whose `source_url` is one of the
//! result URLs, the same grounding rule as [`crate::slang::OpenRouterWebResearcher`].

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::BoxError;
use crate::model::ChatModel;
use crate::rating::Tier;
use crate::slang::{Discovered, SlangResearcher, norm_url};

/// One web search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Page title.
    pub title: String,
    /// Page URL.
    pub url: String,
    /// Result snippet, when the provider returns one.
    pub snippet: Option<String>,
}

/// A web search backend supplied by the host.
#[async_trait]
pub trait WebSearch: Send + Sync {
    /// Up to `max_results` results for `query`.
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchHit>, BoxError>;
}

/// Researches slang with a host search backend and a chat model.
#[derive(Clone)]
pub struct SearchResearcher {
    search: Arc<dyn WebSearch>,
    model: Arc<dyn ChatModel>,
    max_results: usize,
}

impl std::fmt::Debug for SearchResearcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchResearcher").finish_non_exhaustive()
    }
}

impl SearchResearcher {
    /// A researcher over `search` and `model`, reading up to 6 results.
    pub fn new(search: Arc<dyn WebSearch>, model: Arc<dyn ChatModel>) -> Self {
        Self {
            search,
            model,
            max_results: 6,
        }
    }
}

const CONDENSE: &str = "Turn the request into one short web search query (at most 8 words) that \
would find pages listing the slang it asks for. Reply with the query only.";

const EXTRACT: &str = "From the search results below only, list slang terms that real people \
currently use in chat and that fit the request. Return JSON only: {\"terms\": [{\"term\": \"\", \
\"meaning\": \"\", \"intensity\": \"light|spicy|unhinged\", \"source_url\": \"\"}]} with up to 10 \
terms. `meaning` says what it means and when to use it. `intensity`: light = mild and widely \
understood, spicy = clearly slangy, unhinged = loud or over the top. `source_url` must be the URL \
of the result the term appears in. Exclude profanity, slurs, sexual terms, and anything about \
religion, caste, or politics. If the results contain no usable slang, return {\"terms\": []}.";

#[async_trait]
impl SlangResearcher for SearchResearcher {
    async fn research(&self, query: &str) -> Result<Vec<Discovered>, BoxError> {
        #[derive(Deserialize)]
        struct Found {
            terms: Vec<Raw>,
        }
        #[derive(Deserialize)]
        struct Raw {
            term: String,
            meaning: String,
            intensity: String,
            source_url: String,
        }

        // Research requests can be a whole reply; search engines want a few words.
        let search_query = if query.chars().count() > 80 {
            let condensed = self.model.complete(CONDENSE, query).await?;
            condensed
                .trim()
                .trim_matches('"')
                .lines()
                .next()
                .unwrap_or("")
                .to_owned()
        } else {
            query.to_owned()
        };
        if search_query.is_empty() {
            return Ok(Vec::new());
        }
        let hits = self.search.search(&search_query, self.max_results).await?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let urls: HashSet<String> = hits.iter().map(|h| norm_url(&h.url)).collect();
        let mut results = String::new();
        for (i, h) in hits.iter().enumerate() {
            results.push_str(&format!(
                "{}. {}\nURL: {}\n{}\n\n",
                i + 1,
                h.title,
                h.url,
                h.snippet.as_deref().unwrap_or("")
            ));
        }
        let user = format!("Request: {query}\n\nSearch results:\n{results}");
        let text = self.model.complete(EXTRACT, &user).await?;
        let json = crate::agent::json_object(&text).ok_or("research reply had no JSON")?;
        let found: Found = serde_json::from_str(json)?;
        Ok(found
            .terms
            .into_iter()
            .filter(|r| urls.contains(&norm_url(&r.source_url)))
            .map(|r| Discovered {
                term: r.term,
                meaning: r.meaning,
                min_tier: match r.intensity.as_str() {
                    "unhinged" => Tier::Unhinged,
                    "spicy" => Tier::Spicy,
                    _ => Tier::Light,
                },
                source_url: r.source_url,
                verified: None,
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "research_tests.rs"]
mod tests;
