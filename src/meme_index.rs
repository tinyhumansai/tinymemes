//! Reaction GIFs learned from the web, vetted before Jev may pick them.
//!
//! Research runs when a reply was allowed a meme but Jev found nothing in the
//! catalog that fits (`none_fit`). The search is for the moment the user is in:
//! the chat model turns their message into a short, generic meme concept (a
//! known catchphrase such as "Sharma ji ka beta", never personal details), so
//! what is learned is the meme that was actually missing. Each candidate then
//! passes, in order:
//!
//! 1. **Host rating**: GIPHY's own content rating must be G or PG. This is the
//!    image-level check (GIPHY moderates the media itself).
//! 2. **Topic filter**: slug, title, and tags must not touch the region's
//!    blocked topics (politics, religion, sex, violence) or its gaali list.
//! 3. **Meaning**: the chat model names each GIF and says when people post it.
//! 4. **Jev**: verifies it is a widely recognised reaction GIF with that
//!    meaning and is not political, religious, sexual, violent, or mocking.
//!
//! Survivors join the catalog Jev picks from. There is no approval step: the
//! filters are the gate.

use std::collections::HashSet;
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tinyinference_decisions::{Answer, EvaluationRequest, Noul, Question};

use crate::error::BoxError;
use crate::intent::Intent;
use crate::meme_research::{FoundMeme, MemeResearcher};
use crate::model::ChatModel;
use crate::reading::Evaluator;
use crate::region::{CatalogMeme, Region, tokens};
use crate::slang::unix_now;
use crate::source::Meme;

/// One learned GIF.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LearnedMeme {
    /// Region code.
    pub region: String,
    /// Short name, unique within the region; Jev's option label.
    pub title: String,
    /// When people post it.
    pub meaning: String,
    /// Direct media link (small rendition).
    pub url: String,
    /// The GIF's page.
    pub page_url: String,
    /// Intents it fits.
    pub intents: Vec<Intent>,
    /// The host's tags.
    pub tags: Vec<String>,
    /// The host's content rating.
    pub rating: String,
    /// Jev's verification probability.
    pub verified: f64,
    /// Times it went out in a reply.
    pub used: u32,
    /// Unix seconds.
    pub added_at: u64,
}

impl LearnedMeme {
    fn to_catalog(&self) -> CatalogMeme {
        CatalogMeme {
            title: self.title.clone(),
            url: self.url.clone(),
            meaning: self.meaning.clone(),
            intents: self.intents.clone(),
        }
    }
}

/// One research query that ran.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MemeSearchRecord {
    /// Region code.
    pub region: String,
    /// Intent the search was for.
    pub intent: Intent,
    /// The query text.
    pub query: String,
    /// Unix seconds.
    pub at: u64,
    /// Memes added from it.
    pub added: usize,
}

/// What one research step did.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MemeLearnReport {
    /// The query that ran.
    pub query: String,
    /// GIF pages read successfully.
    pub found: usize,
    /// Added to the catalog.
    pub added: usize,
    /// Rejected by the host rating.
    pub rejected_rating: usize,
    /// Rejected by the topic filter.
    pub rejected_topic: usize,
    /// Already known.
    pub duplicates: usize,
    /// Jev did not verify them.
    pub failed_verification: usize,
}

/// When and how to research.
#[derive(Clone, Copy, Debug)]
pub struct MemeIndexPolicy {
    /// Re-run a query after this many seconds.
    pub refresh_after_secs: u64,
    /// Most searches per region per rolling 24 hours.
    pub max_searches_per_day: usize,
    /// Minimum Jev verification probability.
    pub min_verified: f64,
    /// Most learned memes offered to Jev per reply.
    pub offer: usize,
}

impl Default for MemeIndexPolicy {
    fn default() -> Self {
        Self {
            refresh_after_secs: 3 * 24 * 3600,
            max_searches_per_day: 20,
            min_verified: 0.75,
            offer: 30,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Snapshot {
    memes: Vec<LearnedMeme>,
    searches: Vec<MemeSearchRecord>,
}

/// Learned memes. Share one per process; snapshot it to persist.
#[derive(Debug, Default)]
pub struct MemeIndex {
    data: RwLock<Snapshot>,
    in_flight: Mutex<HashSet<String>>,
    policy: MemeIndexPolicy,
}

/// Host ratings accepted for a chat bubble.
const ACCEPTED_RATINGS: &[&str] = &["g", "pg"];

impl MemeIndex {
    /// An empty index.
    pub fn new(policy: MemeIndexPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// Restore from a JSON snapshot.
    pub fn from_json(json: &str, policy: MemeIndexPolicy) -> serde_json::Result<Self> {
        Ok(Self {
            data: RwLock::new(serde_json::from_str(json)?),
            policy,
            ..Self::default()
        })
    }

    /// Serialize to a JSON snapshot.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&*self.data.read().unwrap()).unwrap_or_default()
    }

    /// The policy in use.
    pub fn policy(&self) -> MemeIndexPolicy {
        self.policy
    }

    /// Learned memes for a region, most used first, capped.
    pub fn catalog(&self, region: &str) -> Vec<CatalogMeme> {
        let data = self.data.read().unwrap();
        let mut memes: Vec<&LearnedMeme> =
            data.memes.iter().filter(|m| m.region == region).collect();
        memes.sort_by_key(|m| std::cmp::Reverse((m.used, m.added_at)));
        memes
            .into_iter()
            .take(self.policy.offer)
            .map(LearnedMeme::to_catalog)
            .collect()
    }

    /// Remove a learned meme by title. Returns whether one was removed.
    pub fn remove(&self, region: &str, title: &str) -> bool {
        let mut data = self.data.write().unwrap();
        let before = data.memes.len();
        data.memes
            .retain(|m| !(m.region == region && m.title == title));
        data.memes.len() != before
    }

    /// A learned meme by title, as a sendable [`Meme`].
    pub fn find(&self, region: &str, title: &str) -> Option<Meme> {
        let data = self.data.read().unwrap();
        data.memes
            .iter()
            .find(|m| m.region == region && m.title == title)
            .map(|m| Meme {
                title: m.title.clone(),
                url: m.url.clone(),
                source: "learned".to_owned(),
                meaning: Some(m.meaning.clone()),
            })
    }

    /// Bump usage of a learned meme that went out.
    pub fn record_used(&self, url: &str) {
        let mut data = self.data.write().unwrap();
        if let Some(m) = data.memes.iter_mut().find(|m| m.url == url) {
            m.used += 1;
        }
    }

    /// Number of learned memes for a region (any status).
    pub fn len(&self, region: &str) -> usize {
        self.data
            .read()
            .unwrap()
            .memes
            .iter()
            .filter(|m| m.region == region)
            .count()
    }

    /// Whether a region has no learned memes.
    pub fn is_empty(&self, region: &str) -> bool {
        self.len(region) == 0
    }

    fn searches_today(&self, region: &str, now: u64) -> usize {
        let data = self.data.read().unwrap();
        data.searches
            .iter()
            .filter(|s| s.region == region && now.saturating_sub(s.at) < 24 * 3600)
            .count()
    }

    fn ran_recently(&self, region: &str, query: &str, now: u64) -> bool {
        let data = self.data.read().unwrap();
        data.searches.iter().any(|s| {
            s.region == region
                && s.query == query
                && now.saturating_sub(s.at) < self.policy.refresh_after_secs
        })
    }

    /// Research GIFs for an intent, vet them, and add the survivors.
    ///
    /// `describer` writes each GIF's name, meaning, and intents; `verifier`
    /// is Jev (or the LLM fallback). `Ok(None)` means nothing ran (budget,
    /// a recent identical query, or a concurrent step for the same intent).
    pub async fn learn(
        &self,
        researcher: &dyn MemeResearcher,
        describer: &dyn ChatModel,
        verifier: (&dyn Evaluator, bool),
        region: &Region,
        intent: Intent,
        moment: &str,
    ) -> Result<Option<MemeLearnReport>, BoxError> {
        let now = unix_now();
        if self.searches_today(&region.code, now) >= self.policy.max_searches_per_day {
            return Ok(None);
        }
        // Search for the moment the user is in, not the broad intent bucket.
        let concept = meme_concept(describer, region, intent, moment)
            .await
            .unwrap_or_else(|e| {
                tracing::debug!(error = %e, "[tinymemes] meme concept failed; using the intent");
                intent.phrase().to_owned()
            });
        let query = region.meme_query.replace("{moment}", &concept);
        if self.ran_recently(&region.code, &query, now) {
            return Ok(None);
        }
        let key = format!("{}/{}", region.code, query);
        if !self.in_flight.lock().unwrap().insert(key.clone()) {
            return Ok(None);
        }
        let result = self
            .research_and_vet(
                researcher, describer, verifier, region, &concept, &query, now,
            )
            .await;
        self.in_flight.lock().unwrap().remove(&key);
        let report = result?;
        self.data.write().unwrap().searches.push(MemeSearchRecord {
            region: region.code.clone(),
            intent,
            query,
            at: now,
            added: report.added,
        });
        tracing::debug!(?report, "[tinymemes] meme research absorbed");
        Ok(Some(report))
    }

    #[allow(clippy::too_many_arguments)]
    async fn research_and_vet(
        &self,
        researcher: &dyn MemeResearcher,
        describer: &dyn ChatModel,
        verifier: (&dyn Evaluator, bool),
        region: &Region,
        concept: &str,
        query: &str,
        now: u64,
    ) -> Result<MemeLearnReport, BoxError> {
        let mut report = MemeLearnReport {
            query: query.to_owned(),
            ..Default::default()
        };
        let found = researcher.research(query).await?;
        report.found = found.len();

        // 1-2. Host rating and topic filter; drop what we already have.
        let mut candidates: Vec<FoundMeme> = Vec::new();
        for f in found {
            let known = {
                let data = self.data.read().unwrap();
                data.memes
                    .iter()
                    .any(|m| m.url == f.media_url || m.page_url == f.page_url)
            } || candidates.iter().any(|c| c.media_url == f.media_url);
            if known {
                report.duplicates += 1;
                continue;
            }
            if !f
                .rating
                .as_deref()
                .is_some_and(|r| ACCEPTED_RATINGS.contains(&r))
            {
                report.rejected_rating += 1;
                continue;
            }
            if touches_blocked_topic(region, &f) {
                report.rejected_topic += 1;
                continue;
            }
            candidates.push(f);
        }
        if candidates.is_empty() {
            return Ok(report);
        }

        // 3. Name and meaning from the model.
        let described = describe(describer, region, concept, &candidates).await?;
        if described.is_empty() {
            return Ok(report);
        }

        // 4. Jev verification, one batched call.
        let scores = verify(verifier, region, &described).await?;
        let mut data = self.data.write().unwrap();
        for (d, score) in described.into_iter().zip(scores) {
            if score < self.policy.min_verified {
                report.failed_verification += 1;
                continue;
            }
            report.added += 1;
            let title = unique_title(&data.memes, region, &d.name);
            data.memes.push(LearnedMeme {
                region: region.code.clone(),
                title,
                meaning: d.meaning,
                url: d.found.media_url,
                page_url: d.found.page_url,
                intents: d.intents,
                tags: d.found.tags,
                rating: d.found.rating.unwrap_or_default(),
                verified: score,
                used: 0,
                added_at: now,
            });
        }
        Ok(report)
    }
}

/// Words that keep a GIF out regardless of rating: politics, religion, sex,
/// violence, plus the region's gaali list.
fn touches_blocked_topic(region: &Region, f: &FoundMeme) -> bool {
    let text = format!(
        "{} {} {}",
        f.title,
        f.slug_words.join(" "),
        f.tags.join(" ")
    );
    let words: Vec<String> = tokens(&text).collect();
    region
        .meme_topic_blocklist
        .iter()
        .chain(region.blocklist.iter())
        .any(|b| words.iter().any(|w| w == &b.to_lowercase()))
}

/// Turn the user's message into a short, generic meme concept to search for.
/// Only the concept leaves the process: no names, places, numbers, or the
/// user's own words beyond a widely known catchphrase.
async fn meme_concept(
    model: &dyn ChatModel,
    region: &Region,
    intent: Intent,
    moment: &str,
) -> Result<String, BoxError> {
    #[derive(Deserialize)]
    struct Out {
        concept: String,
    }
    let system = format!(
        "Name the reaction-meme moment in a chat message, for a GIF search in {region}. Reply \
         with JSON only: {{\"concept\": \"...\"}}, 2-6 words. Use a widely known meme \
         catchphrase or trope if the message invokes one (for example Sharma ji ka beta, emotional \
         damage, leg day); otherwise describe the situation generically. Never include names of \
         private people, workplaces, places, numbers, or other personal details from the message.",
        region = region.name,
    );
    let user = format!("Mood: {}\nMessage: {}", intent.phrase(), clean(moment, 400));
    let text = model.complete(&system, &user).await?;
    let json = crate::agent::json_object(&text).ok_or("meme concept reply had no JSON")?;
    let concept = clean(&serde_json::from_str::<Out>(json)?.concept, 60);
    if concept.split_whitespace().count() == 0 {
        return Err("empty meme concept".into());
    }
    Ok(concept)
}

struct Described {
    found: FoundMeme,
    name: String,
    meaning: String,
    intents: Vec<Intent>,
}

async fn describe(
    model: &dyn ChatModel,
    region: &Region,
    concept: &str,
    candidates: &[FoundMeme],
) -> Result<Vec<Described>, BoxError> {
    #[derive(Deserialize)]
    struct Out {
        memes: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        n: usize,
        name: String,
        meaning: String,
        #[serde(default)]
        intents: Vec<String>,
    }
    let intents: Vec<&str> = Intent::ALL.iter().map(|i| i.as_str()).collect();
    let system = format!(
        "You catalogue reaction GIFs for an audience in {region}. For each numbered GIF, using only \
         its title, URL slug, and tags, give a short recognisable name (2-6 words, e.g. the show \
         and character or the catchphrase), what it means and when people post it (one line), and \
         which of these intents it fits: {intents}. Skip a GIF if you cannot tell what it shows. \
         Reply with JSON only: {{\"memes\": [{{\"n\": 1, \"name\": \"\", \"meaning\": \"\", \
         \"intents\": [\"\"]}}]}}",
        region = region.name,
        intents = intents.join(", "),
    );
    let mut user = format!("Looking for GIFs for the moment: {concept}\n\n");
    for (i, c) in candidates.iter().enumerate() {
        user.push_str(&format!(
            "{}. title: {} | slug: {} | tags: {}\n",
            i + 1,
            c.title,
            c.slug_words.join(" "),
            c.tags.join(", ")
        ));
    }
    let text = model.complete(&system, &user).await?;
    let json = crate::agent::json_object(&text).ok_or("meme description reply had no JSON")?;
    let out: Out = serde_json::from_str(json)?;
    let mut seen = HashSet::new();
    Ok(out
        .memes
        .into_iter()
        .filter_map(|item| {
            let found = candidates.get(item.n.checked_sub(1)?)?.clone();
            let name = clean(&item.name, 48);
            let meaning = clean(&item.meaning, 160);
            let intents: Vec<Intent> = item
                .intents
                .iter()
                .filter_map(|i| Intent::from_label(i))
                .collect();
            (!name.is_empty() && !meaning.is_empty() && !intents.is_empty() && seen.insert(item.n))
                .then_some(Described {
                    found,
                    name,
                    meaning,
                    intents,
                })
        })
        .collect())
}

async fn verify(
    verifier: (&dyn Evaluator, bool),
    region: &Region,
    described: &[Described],
) -> Result<Vec<f64>, BoxError> {
    let (jev, openjev) = verifier;
    let questions = described
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let q = Question::Noul(Noul {
                instructions: json!(format!(
                    "\"{}\", a GIF titled \"{}\" and tagged [{}], is a widely recognised reaction \
                     meme or GIF from {} internet culture that people post {}, and it is not \
                     political, religious, sexual, violent, mocking a community, or satire of a \
                     real politician, journalist, or public figure.",
                    d.name,
                    d.found.title,
                    d.found.tags.join(", "),
                    region.name,
                    d.meaning,
                )),
                criteria: None,
            });
            (format!("m{i:02}"), q)
        })
        .collect();
    let state = json!({ "region": region.name, "task": "vetting reaction GIFs found on the web" });
    let request = if openjev {
        EvaluationRequest::openjev(state, questions)
    } else {
        EvaluationRequest::jev(state, questions)
    };
    let response = jev.evaluate(&request).await?;
    Ok((0..described.len())
        .map(|i| match response.answers.get(&format!("m{i:02}")) {
            Some(Answer::Noul(n)) => n.noul.clamp(0.0, 1.0),
            _ => 0.0,
        })
        .collect())
}

fn unique_title(existing: &[LearnedMeme], region: &Region, name: &str) -> String {
    let taken = |t: &str| {
        existing
            .iter()
            .any(|m| m.region == region.code && m.title.eq_ignore_ascii_case(t))
            || region.memes.iter().any(|m| m.title.eq_ignore_ascii_case(t))
    };
    if !taken(name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|t| !taken(t))
        .unwrap_or_default()
}

fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '`' | '[' | ']' | '{' | '}' | '<' | '>' | '(' | ')'))
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

#[cfg(test)]
#[path = "meme_index_tests.rs"]
mod tests;
