//! A slang index that grows from web research.
//!
//! The index starts from a region's curated terms. Each time a reply is
//! remixed for an intent whose bucket is still thin (or stale), the engine runs
//! the next unused research query for that bucket, vets what comes back, and
//! merges it in. A term found by several searches gains corroboration; a term
//! the agent actually uses gains usage. Both raise its rank, so the vocabulary
//! sharpens as traffic grows.

use std::collections::HashSet;
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::BoxError;
use crate::intent::Intent;
use crate::rating::Tier;
use crate::reading::Evaluator;
use crate::region::{Region, SlangTerm, tokens};
use tinyinference_decisions::{Answer, EvaluationRequest, Noul, Question};

/// A term reported by a researcher, before vetting.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Discovered {
    /// The term as written.
    pub term: String,
    /// Meaning and when to use it.
    pub meaning: String,
    /// Lowest tier the term suits.
    pub min_tier: Tier,
    /// Page the term was found on. Must be a page the search actually cited.
    pub source_url: String,
    /// Jev's probability that this is genuine, inoffensive regional slang
    /// with the stated meaning. `None` until verified.
    #[serde(default)]
    pub verified: Option<f64>,
}

/// Finds slang on the web.
#[async_trait]
pub trait SlangResearcher: Send + Sync {
    /// Run one research query and return grounded terms.
    async fn research(&self, query: &str) -> Result<Vec<Discovered>, BoxError>;
}

/// Where a term came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Shipped in the region pack.
    Curated,
    /// Learned from web research.
    Web,
}

/// One indexed term.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LearnedTerm {
    /// Region code the term belongs to.
    pub region: String,
    /// The term as written.
    pub term: String,
    /// Meaning and when to use it.
    pub meaning: String,
    /// Lowest tier the term suits.
    pub min_tier: Tier,
    /// Intents it was found for. Empty means any intent.
    pub intents: Vec<Intent>,
    /// Pages it was found on.
    pub sources: Vec<String>,
    /// How many separate searches surfaced it.
    pub seen: u32,
    /// How many remixed replies used it.
    pub used: u32,
    /// Curated or learned.
    pub origin: Origin,
    /// Jev's verification probability for web terms.
    #[serde(default)]
    pub verified: Option<f64>,
}

/// One research query that ran.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SearchRecord {
    /// Region code.
    pub region: String,
    /// Intent bucket the search was for.
    pub intent: Intent,
    /// The query text.
    pub query: String,
    /// Unix seconds.
    pub at: u64,
    /// Terms accepted from it.
    pub accepted: usize,
}

/// What one learning step did.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LearnReport {
    /// The query that ran.
    pub query: String,
    /// New terms added.
    pub added: usize,
    /// Existing terms corroborated.
    pub corroborated: usize,
    /// Terms rejected by the shape, blocklist, or source checks.
    pub rejected: usize,
    /// Terms Jev judged not to be genuine, inoffensive regional slang.
    pub failed_verification: usize,
}

/// When to research.
#[derive(Clone, Copy, Debug)]
pub struct IndexPolicy {
    /// Search when Jev's `slang_enough` probability falls below this.
    pub search_below: f64,
    /// Terms shown to Jev for the fit check (Choice allows up to 254 + `none_fit`).
    pub jev_candidates: usize,
    /// Re-run a query after this many seconds.
    pub refresh_after_secs: u64,
    /// Terms offered to the rewrite per reply.
    pub offer: usize,
    /// Most web searches per region per rolling 24 hours.
    pub max_searches_per_day: usize,
    /// Minimum Jev verification probability for a web term to be kept.
    pub min_verified: f64,
}

impl Default for IndexPolicy {
    fn default() -> Self {
        Self {
            search_below: 0.5,
            jev_candidates: 40,
            refresh_after_secs: 7 * 24 * 3600,
            max_searches_per_day: 50,
            offer: 18,
            min_verified: 0.6,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Snapshot {
    terms: Vec<LearnedTerm>,
    searches: Vec<SearchRecord>,
}

/// The in-memory slang index. Share one per process; snapshot it to persist.
#[derive(Debug, Default)]
pub struct SlangIndex {
    data: RwLock<Snapshot>,
    in_flight: Mutex<HashSet<String>>,
    policy: IndexPolicy,
}

impl SlangIndex {
    /// An empty index.
    pub fn new(policy: IndexPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// Restore from a JSON snapshot.
    pub fn from_json(json: &str, policy: IndexPolicy) -> serde_json::Result<Self> {
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
    pub fn policy(&self) -> IndexPolicy {
        self.policy
    }

    /// Add a region's curated terms (idempotent).
    pub fn seed(&self, region: &Region) {
        let mut data = self.data.write().unwrap();
        for t in &region.slang {
            if find(&data.terms, &region.code, &t.term).is_none() {
                data.terms.push(LearnedTerm {
                    region: region.code.clone(),
                    term: t.term.clone(),
                    meaning: t.meaning.clone(),
                    min_tier: t.min_tier,
                    intents: Vec::new(),
                    sources: Vec::new(),
                    seen: 1,
                    used: 0,
                    origin: Origin::Curated,
                    verified: None,
                });
            }
        }
    }

    /// Number of terms for a region.
    pub fn len(&self, region: &str) -> usize {
        self.data
            .read()
            .unwrap()
            .terms
            .iter()
            .filter(|t| t.region == region)
            .count()
    }

    /// Whether a region has no terms.
    pub fn is_empty(&self, region: &str) -> bool {
        self.len(region) == 0
    }

    /// The best terms to offer the rewrite for this intent and tier.
    pub fn terms_for(&self, region: &str, intent: Intent, tier: Tier) -> Vec<SlangTerm> {
        let data = self.data.read().unwrap();
        let mut ranked: Vec<(u32, &LearnedTerm)> = data
            .terms
            .iter()
            .filter(|t| t.region == region && t.min_tier <= tier)
            .filter(|t| t.intents.is_empty() || t.intents.contains(&intent))
            .map(|t| {
                let fit = if t.intents.contains(&intent) { 6 } else { 0 };
                let curated = u32::from(t.origin == Origin::Curated) * 2;
                (fit + curated + t.seen.min(5) * 2 + t.used.min(10), t)
            })
            .collect();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.0));
        ranked
            .into_iter()
            .take(self.policy.offer)
            .map(|(_, t)| SlangTerm {
                term: t.term.clone(),
                meaning: t.meaning.clone(),
                min_tier: t.min_tier,
            })
            .collect()
    }

    /// Terms of `region` that appear in `text`; bumps their usage.
    pub fn record_used(&self, region: &str, text: &str) -> Vec<String> {
        let haystack = format!(" {} ", tokens(text).collect::<Vec<_>>().join(" "));
        let mut data = self.data.write().unwrap();
        let mut used = Vec::new();
        for t in data.terms.iter_mut().filter(|t| t.region == region) {
            let needle = format!(" {} ", tokens(&t.term).collect::<Vec<_>>().join(" "));
            if needle.trim().is_empty() || !haystack.contains(&needle) {
                continue;
            }
            t.used += 1;
            used.push(t.term.clone());
        }
        used
    }

    /// The next research query for this bucket, if it needs one.
    /// The next query to run for this bucket: never-run templates first, then
    /// the stalest expired one. `None` when every template ran recently, which
    /// caps search spend however often Jev asks for more.
    pub fn next_query(&self, region: &Region, intent: Intent, now: u64) -> Option<String> {
        let data = self.data.read().unwrap();
        let last_run = |q: &str| {
            data.searches
                .iter()
                .filter(|s| s.region == region.code && s.query == q)
                .map(|s| s.at)
                .max()
        };
        region_queries(region, intent, now)
            .into_iter()
            .filter(|q| {
                last_run(q)
                    .is_none_or(|at| now.saturating_sub(at) >= self.policy.refresh_after_secs)
            })
            .min_by_key(|q| last_run(q).unwrap_or(0))
    }

    /// The region's best terms regardless of intent, for Jev to judge against
    /// a reply.
    pub fn top_terms(&self, region: &str, limit: usize) -> Vec<SlangTerm> {
        let data = self.data.read().unwrap();
        let mut ranked: Vec<(u32, &LearnedTerm)> = data
            .terms
            .iter()
            .filter(|t| t.region == region)
            .map(|t| {
                let curated = u32::from(t.origin == Origin::Curated) * 2;
                (curated + t.seen.min(5) * 2 + t.used.min(10), t)
            })
            .collect();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.0));
        let mut seen = HashSet::new();
        ranked
            .into_iter()
            .filter(|(_, t)| seen.insert(t.term.to_lowercase()))
            .take(limit)
            .map(|(_, t)| SlangTerm {
                term: t.term.clone(),
                meaning: t.meaning.clone(),
                min_tier: t.min_tier,
            })
            .collect()
    }

    /// Vet and merge research results.
    pub fn absorb(
        &self,
        region: &Region,
        intent: Intent,
        query: &str,
        found: Vec<Discovered>,
        now: u64,
    ) -> LearnReport {
        let mut report = LearnReport {
            query: query.to_owned(),
            added: 0,
            corroborated: 0,
            rejected: 0,
            failed_verification: 0,
        };
        let mut data = self.data.write().unwrap();
        let mut seen_this_query = HashSet::new();
        let mut per_source: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for d in found {
            let term = clean(&d.term, 40);
            let meaning = clean(&d.meaning, 160);
            let ok = !term.is_empty()
                && !meaning.is_empty()
                && term.split_whitespace().count() <= 4
                && !term.contains(['(', ')', '/', ':', ',', '.', '?', '!', ';', '…', '"'])
                && per_source.get(d.source_url.as_str()).copied().unwrap_or(0) < MAX_PER_SOURCE
                && d.source_url.starts_with("http")
                && !shady_source(&d.source_url)
                && region.blocked_words(&term).is_empty()
                && region.blocked_words(&meaning).is_empty()
                && seen_this_query.insert(term.to_lowercase());
            if !ok {
                report.rejected += 1;
                continue;
            }
            *per_source.entry(d.source_url.clone()).or_insert(0) += 1;
            match find(&data.terms, &region.code, &term) {
                Some(i) => {
                    let t = &mut data.terms[i];
                    t.seen += 1;
                    if !t.intents.is_empty() && !t.intents.contains(&intent) {
                        t.intents.push(intent);
                    }
                    if !t.sources.contains(&d.source_url) {
                        t.sources.push(d.source_url);
                    }
                    report.corroborated += 1;
                }
                None => {
                    data.terms.push(LearnedTerm {
                        region: region.code.clone(),
                        term,
                        meaning,
                        min_tier: d.min_tier.max(Tier::Light),
                        intents: vec![intent],
                        sources: vec![d.source_url],
                        seen: 1,
                        used: 0,
                        origin: Origin::Web,
                        verified: d.verified,
                    });
                    report.added += 1;
                }
            }
        }
        data.searches.push(SearchRecord {
            region: region.code.clone(),
            intent,
            query: query.to_owned(),
            at: now,
            accepted: report.added + report.corroborated,
        });
        report
    }

    /// Warm a bucket from the region's generic query templates (the next one
    /// that has not run recently). Hosts can call this in the background.
    pub async fn learn(
        &self,
        researcher: &dyn SlangResearcher,
        verifier: Option<(&dyn Evaluator, bool)>,
        region: &Region,
        intent: Intent,
    ) -> Result<Option<LearnReport>, BoxError> {
        let now = unix_now();
        let Some(query) = self.next_query(region, intent, now) else {
            return Ok(None);
        };
        self.run_query(researcher, verifier, region, intent, query, now)
            .await
    }

    /// Search for slang that fits one specific reply. The engine calls this
    /// when Jev judges that the index has nothing suited to the reply.
    pub async fn learn_for_reply(
        &self,
        researcher: &dyn SlangResearcher,
        verifier: Option<(&dyn Evaluator, bool)>,
        region: &Region,
        intent: Intent,
        reply: &str,
    ) -> Result<Option<LearnReport>, BoxError> {
        let now = unix_now();
        let excerpt = clean(reply, 240);
        if excerpt.is_empty() {
            return Ok(None);
        }
        let query = region.reply_query.replace("{reply}", &excerpt);
        self.run_query(researcher, verifier, region, intent, query, now)
            .await
    }

    /// Searches in the last 24 hours for a region.
    pub fn searches_today(&self, region: &str, now: u64) -> usize {
        let data = self.data.read().unwrap();
        data.searches
            .iter()
            .filter(|s| s.region == region && now.saturating_sub(s.at) < 24 * 3600)
            .count()
    }

    async fn run_query(
        &self,
        researcher: &dyn SlangResearcher,
        verifier: Option<(&dyn Evaluator, bool)>,
        region: &Region,
        intent: Intent,
        query: String,
        now: u64,
    ) -> Result<Option<LearnReport>, BoxError> {
        if self.searches_today(&region.code, now) >= self.policy.max_searches_per_day {
            tracing::debug!("[tinymemes] daily slang search budget spent");
            return Ok(None);
        }
        let repeat = self.data.read().unwrap().searches.iter().any(|s| {
            s.region == region.code
                && s.query == query
                && now.saturating_sub(s.at) < self.policy.refresh_after_secs
        });
        if repeat {
            return Ok(None);
        }
        let key = format!("{}/{}", region.code, query);
        if !self.in_flight.lock().unwrap().insert(key.clone()) {
            return Ok(None);
        }
        let result = async {
            let found = researcher.research(&query).await?;
            match verifier {
                Some((jev, openjev)) if !found.is_empty() => {
                    verify(jev, openjev, region, found).await
                }
                _ => Ok(found),
            }
        }
        .await;
        self.in_flight.lock().unwrap().remove(&key);
        let found = result?;
        let (kept, failed): (Vec<_>, Vec<_>) = found
            .into_iter()
            .partition(|d| d.verified.is_none_or(|p| p >= self.policy.min_verified));
        let mut report = self.absorb(region, intent, &query, kept, now);
        report.failed_verification = failed.len();
        tracing::debug!(?report, "[tinymemes] slang research absorbed");
        Ok(Some(report))
    }
}

/// Ask Jev, in one call, whether each candidate is real regional slang.
async fn verify(
    jev: &dyn Evaluator,
    openjev: bool,
    region: &Region,
    mut found: Vec<Discovered>,
) -> Result<Vec<Discovered>, BoxError> {
    let questions: std::collections::BTreeMap<String, Question> = found
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let q = Question::Noul(Noul {
                instructions: json!(format!(
                    "\"{}\" is genuine slang that people in {} actually use in casual chat today, it \
                     means roughly \"{}\", and it is not vulgar, sexual, or offensive.",
                    clean(&d.term, 40),
                    region.name,
                    clean(&d.meaning, 160),
                )),
                criteria: None,
            });
            (format!("t{i:02}"), q)
        })
        .collect();
    let state =
        json!({ "region": region.name, "task": "vetting slang candidates found on the web" });
    let request = if openjev {
        EvaluationRequest::openjev(state, questions)
    } else {
        EvaluationRequest::jev(state, questions)
    };
    let response = jev.evaluate(&request).await?;
    for (i, d) in found.iter_mut().enumerate() {
        d.verified = Some(match response.answers.get(&format!("t{i:02}")) {
            Some(Answer::Noul(n)) => n.noul.clamp(0.0, 1.0),
            _ => 0.0,
        });
    }
    Ok(found)
}

/// One listicle should not dominate a bucket.
const MAX_PER_SOURCE: usize = 3;

/// Sources we never learn from.
fn shady_source(url: &str) -> bool {
    let host = url
        .split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    [
        "fap", "porn", "xxx", "sex", "nude", "adult", "escort", "casino", "nsfw",
    ]
    .iter()
    .any(|w| host.contains(w))
}

fn find(terms: &[LearnedTerm], region: &str, term: &str) -> Option<usize> {
    let key = term.trim().to_lowercase();
    terms
        .iter()
        .position(|t| t.region == region && t.term.to_lowercase() == key)
}

/// One line, no markup, bounded: web text goes straight into a prompt.
fn clean(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '`' | '[' | ']' | '{' | '}' | '<' | '>'))
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max_chars).collect()
}

fn region_queries(region: &Region, intent: Intent, now: u64) -> Vec<String> {
    let year = (1970 + now / 31_556_952).to_string();
    region
        .slang_queries
        .iter()
        .map(|q| {
            q.replace("{intent}", intent.phrase())
                .replace("{year}", &year)
        })
        .collect()
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Researches slang with OpenRouter's web search plugin and keeps only terms
/// whose source is one of the pages the search actually cited.
#[derive(Clone)]
pub struct OpenRouterWebResearcher {
    http: reqwest::Client,
    api_key: String,
    model: String,
    max_results: u32,
}

impl std::fmt::Debug for OpenRouterWebResearcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterWebResearcher")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OpenRouterWebResearcher {
    /// A researcher using `model` with up to 5 web results per query.
    pub fn new(
        http: reqwest::Client,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http,
            api_key: api_key.into(),
            model: model.into(),
            max_results: 5,
        }
    }
}

#[async_trait]
impl SlangResearcher for OpenRouterWebResearcher {
    async fn research(&self, query: &str) -> Result<Vec<Discovered>, BoxError> {
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Msg,
        }
        #[derive(Deserialize)]
        struct Msg {
            content: Option<String>,
            #[serde(default)]
            annotations: Vec<Annotation>,
        }
        #[derive(Deserialize)]
        struct Annotation {
            url_citation: Option<Citation>,
        }
        #[derive(Deserialize)]
        struct Citation {
            url: String,
        }
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
        let prompt = format!(
            "Search the web: {query}\n\nReturn JSON only: {{\"terms\": [{{\"term\": \"\", \"meaning\": \"\", \
             \"intensity\": \"light|spicy|unhinged\", \"source_url\": \"\"}}]}} with up to 10 terms that \
             real people currently use in chat. `meaning` says what it means and when to use it. \
             `intensity`: light = mild and widely understood, spicy = clearly slangy, unhinged = loud or \
             over the top. `source_url` must be the page you found it on. Exclude profanity, slurs, sexual \
             terms, and anything about religion, caste, or politics."
        );
        let body = json!({
            "model": self.model,
            "temperature": 0.2,
            "plugins": [{ "id": "web", "max_results": self.max_results }],
            "messages": [{ "role": "user", "content": prompt }],
        });
        let resp: Resp = self
            .http
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let msg = resp.choices.into_iter().next().ok_or("no choices")?.message;
        let cited: HashSet<String> = msg
            .annotations
            .into_iter()
            .filter_map(|a| a.url_citation)
            .map(|c| norm_url(&c.url))
            .collect();
        let content = msg.content.unwrap_or_default();
        let json = crate::agent::json_object(&content).ok_or("research reply had no JSON")?;
        let found: Found = serde_json::from_str(json)?;
        Ok(found
            .terms
            .into_iter()
            .filter(|r| cited.contains(&norm_url(&r.source_url)))
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

fn norm_url(url: &str) -> String {
    let url = url.split('#').next().unwrap_or(url);
    let url = url.split("?utm_").next().unwrap_or(url);
    url.trim_end_matches('/').to_lowercase()
}

#[cfg(test)]
#[path = "slang_tests.rs"]
mod tests;
