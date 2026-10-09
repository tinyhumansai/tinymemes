//! The slang agent: plan meme searches, fetch candidates, rewrite the reply in
//! the region's slang with meme markers, and resolve the markers to images.

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::conversation::{Role, Turn, memes_as_text};
use crate::error::{Error, Result};
use crate::model::ChatModel;
use crate::rating::Rating;
use crate::reading::{Reading, UserLanguage};
use crate::region::{Region, SlangTerm};
use crate::slang::SlangIndex;
use crate::source::{Meme, MemeSource};

const MAX_QUERIES: usize = 3;
const PER_SEARCH: usize = 3;
const MAX_CANDIDATES: usize = 8;
/// Recent assistant replies shown to the rewrite so it varies its voice.
const RECENT_REPLIES: usize = 3;
/// Assistant turns scanned for memes that must not be sent again.
const MEME_MEMORY_TURNS: usize = 6;

/// What the agent produced.
#[derive(Clone, Debug, Serialize)]
pub struct Remix {
    /// Final reply text (markdown, images inlined as `![title](url)`).
    pub reply: String,
    /// Memes that ended up in the reply.
    pub memes: Vec<Meme>,
    /// Search queries the agent ran.
    pub queries: Vec<String>,
    /// Slang terms offered to the rewrite (from the index).
    pub slang_offered: Vec<String>,
    /// Indexed terms found in the final reply.
    pub slang_used: Vec<String>,
    /// False when the slang rewrite was discarded (it dropped code or links,
    /// ballooned, or used a blocklisted word) and the original wording was kept.
    pub rewrite_kept: bool,
}

/// Plans, fetches, rewrites, and inserts.
#[derive(Clone)]
pub struct SlangAgent {
    model: Arc<dyn ChatModel>,
    sources: Vec<Arc<dyn MemeSource>>,
    region: Arc<Region>,
    index: Arc<SlangIndex>,
}

impl std::fmt::Debug for SlangAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.sources.iter().map(|s| s.name()).collect();
        f.debug_struct("SlangAgent")
            .field("region", &self.region.code)
            .field("sources", &names)
            .finish_non_exhaustive()
    }
}

impl SlangAgent {
    /// An agent over a chat model, a region pack, and external meme sources.
    /// The region's own catalog is always searched first.
    pub fn new(
        model: Arc<dyn ChatModel>,
        sources: Vec<Arc<dyn MemeSource>>,
        region: Region,
        index: Arc<SlangIndex>,
    ) -> Self {
        index.seed(&region);
        Self {
            model,
            sources,
            region: Arc::new(region),
            index,
        }
    }

    /// The slang index in use.
    pub fn index(&self) -> &Arc<SlangIndex> {
        &self.index
    }

    /// The region pack in use.
    pub fn region(&self) -> &Region {
        &self.region
    }

    /// Remix `reply` at `rating`'s intensity. `conversation` is the chat as
    /// the user saw it. Only two things from it reach the rewrite: the user's
    /// latest message (language and script) and the slang terms used in recent
    /// replies (so the wording varies). Earlier messages themselves are never
    /// shown to the rewrite, so it cannot answer or blend them in. Memes sent
    /// recently are excluded by title.
    pub async fn run(
        &self,
        reply: &str,
        conversation: &[Turn],
        reading: &Reading,
        rating: Rating,
    ) -> Result<Remix> {
        let last_user = conversation
            .iter()
            .rev()
            .find(|t| t.role == Role::User)
            .map(|t| t.text.as_str());
        let assistant: Vec<&Turn> = conversation
            .iter()
            .rev()
            .filter(|t| t.role == Role::Assistant)
            .collect();
        let mut recent_slang: Vec<String> = Vec::new();
        for turn in assistant.iter().filter(|t| t.remixed).take(RECENT_REPLIES) {
            for term in self.index.terms_in(&self.region.code, &turn.text) {
                if !recent_slang.contains(&term) {
                    recent_slang.push(term);
                }
            }
        }
        let sent: Vec<_> = assistant
            .iter()
            .take(MEME_MEMORY_TURNS)
            .flat_map(|t| memes_as_text(&t.text).1)
            .collect();

        let has_sources = !self.sources.is_empty() || !self.region.memes.is_empty();
        let (queries, candidates) = if rating.max_memes > 0 && has_sources {
            let queries = self.plan(reply, reading).await;
            let candidates: Vec<Meme> = self
                .fetch(&queries, reading)
                .await
                .into_iter()
                .filter(|m| {
                    !sent.iter().any(|s| {
                        s.title.eq_ignore_ascii_case(&m.title) || s.url.as_deref() == Some(&m.url)
                    })
                })
                .collect();
            (queries, candidates)
        } else {
            (Vec::new(), Vec::new())
        };

        // Slang follows the user's language: the regional index for Hinglish or
        // Devanagari (or when Jev did not say), English internet slang for an
        // English writer, and nothing for any other language.
        let slang: Vec<SlangTerm> = match reading.user_language {
            Some(UserLanguage::English) => {
                Region::global().slang_for(rating.tier).cloned().collect()
            }
            Some(UserLanguage::Other) => Vec::new(),
            _ => self
                .index
                .terms_for(&self.region.code, reading.reply_intent, rating.tier),
        };
        let rewritten = self
            .model
            .complete(
                &rewrite_system(&self.region, &slang, rating, reading.user_language),
                &rewrite_user(
                    reply,
                    last_user,
                    &recent_slang,
                    reading,
                    &candidates,
                    rating.max_memes,
                ),
            )
            .await
            .map_err(Error::Model)?;
        let rewritten = strip_wrapping_fence(rewritten.trim());

        let blocked = self.region.blocked_words(rewritten);
        let (text, rewrite_kept) = if !blocked.is_empty() {
            tracing::warn!(
                ?blocked,
                "[tinymemes] rewrite used blocklisted words; keeping original wording"
            );
            (reply.to_owned(), false)
        } else if !preserves_protected(reply, rewritten) || !within_length(reply, rewritten) {
            tracing::warn!(
                "[tinymemes] rewrite dropped protected content or ballooned; keeping original wording"
            );
            (reply.to_owned(), false)
        } else {
            (rewritten.to_owned(), true)
        };
        let (reply, memes) = resolve_markers(&text, &candidates, rating.max_memes);
        let slang_used = self.index.record_used(&self.region.code, &reply);
        let slang_offered = slang.into_iter().map(|t| t.term).collect();
        Ok(Remix {
            reply,
            memes,
            queries,
            slang_offered,
            slang_used,
            rewrite_kept,
        })
    }

    /// Ask the model for short meme searches; fall back to intent defaults.
    async fn plan(&self, reply: &str, reading: &Reading) -> Vec<String> {
        #[derive(Deserialize)]
        struct Plan {
            queries: Vec<String>,
        }
        let mut system = format!(
            "You pick meme searches for an audience in {name}. Reply with JSON only: \
             {{\"queries\": [\"...\"]}}. Give 1-3 short (1-4 word) searches for reaction memes \
             or GIFs that match the mood of the reply. Draw on {culture}",
            name = self.region.name,
            culture = self.region.culture,
        );
        if !self.region.memes.is_empty() {
            system.push_str(
                "\nMemes people here know (use their names as searches when one fits):\n",
            );
            for m in &self.region.memes {
                system.push_str(&format!("- {}: {}\n", m.title, m.meaning));
            }
        }
        let user = format!(
            "Reply intent: {}\nChat intent: {}\n\nReply:\n{}",
            reading.reply_intent, reading.chat_intent, reply
        );
        let planned = match self.model.complete(&system, &user).await {
            Ok(text) => json_object(&text)
                .and_then(|j| serde_json::from_str::<Plan>(j).ok())
                .map(|p| p.queries)
                .unwrap_or_default(),
            Err(e) => {
                tracing::debug!(error = %e, "[tinymemes] plan call failed; using intent defaults");
                Vec::new()
            }
        };
        let mut queries: Vec<String> = planned
            .into_iter()
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty() && q.len() <= 60)
            .take(MAX_QUERIES)
            .collect();
        if queries.is_empty() {
            queries = reading
                .reply_intent
                .fallback_queries()
                .iter()
                .map(|q| (*q).to_owned())
                .collect();
        }
        queries
    }

    /// Search the region catalog, then every external source for every query
    /// concurrently. Catalog hits rank first; duplicates are dropped by URL.
    async fn fetch(&self, queries: &[String], reading: &Reading) -> Vec<Meme> {
        let intent = reading.reply_intent;
        let region = self.region.as_ref();
        let catalog = queries
            .iter()
            .flat_map(|q| region.search_catalog(q, intent, PER_SEARCH));
        let searches = self
            .sources
            .iter()
            .flat_map(|s| queries.iter().map(move |q| (s, q)))
            .map(|(s, q)| async move {
                match s.search(q, intent, region, PER_SEARCH).await {
                    Ok(hits) => hits,
                    Err(e) => {
                        tracing::debug!(source = s.name(), query = %q, error = %e, "[tinymemes] meme search failed");
                        Vec::new()
                    }
                }
            });
        let external = join_all(searches).await.into_iter().flatten();
        let mut seen = HashSet::new();
        catalog
            .chain(external)
            .filter(|m| m.url.starts_with("https://") && seen.insert(m.url.clone()))
            .take(MAX_CANDIDATES)
            .collect()
    }
}

fn rewrite_system(
    region: &Region,
    terms: &[SlangTerm],
    rating: Rating,
    language: Option<UserLanguage>,
) -> String {
    let mut slang = String::new();
    for t in terms {
        slang.push_str(&format!("- {}: {}\n", t.term, t.meaning));
    }
    if slang.is_empty() {
        slang.push_str("(none; keep the user's own register)\n");
    }
    let language_rule = match language {
        Some(UserLanguage::English) => {
            "The user writes in English: reply in English only. No Hindi or other non-English words."
        }
        Some(UserLanguage::Hinglish) => {
            "The user writes Hinglish: reply in Hinglish in Latin letters, at roughly their mix."
        }
        Some(UserLanguage::Devanagari) => {
            "The user writes Hindi in Devanagari: reply in Devanagari."
        }
        Some(UserLanguage::Other) => {
            "Reply in the same language and script as the user's message. Do not switch languages."
        }
        None => "Reply in the same language, script, and mix as the user's message.",
    };
    format!(
        "You remix an AI assistant's reply so it sounds like a fun friend in a group chat in {name}.\n\
         Voice: {voice}\n\
         Language: {language_rule}\n\
         Intensity: {style}\n\n\
         Slang you may use (pick what fits, use each the way its meaning says, only terms that \
         belong in the language the user is writing; do not invent regional slang that is not on \
         this list):\n{slang}\n\
         Hard rules:\n\
         - Remix only the original reply. Do not answer, repeat, or refer to earlier messages.\n\
         - Reply in the user's language and script. Never introduce a language the user did not use.\n\
         - Keep every fact, number, step, name, and caveat. Do not add new claims.\n\
         - Stay about as long as the original. Swap the voice, do not add explanations.\n\
         - Copy fenced code blocks, inline code, and URLs exactly, character for character.\n\
         - No profanity or gaali, no slurs, no punching down, nothing sexual, no politics or religion jokes.\n\
         - Insert at most {max} meme marker(s) of the form [[meme:N]] on their own line, \
           using only the numbered candidates given, where that meme's meaning fits the moment. \
           If there are no candidates, insert none.\n\
         - Output only the remixed reply. No preamble, no explanation.",
        name = region.name,
        voice = region.voice,
        style = rating.tier.style(),
        max = rating.max_memes,
    )
}

fn rewrite_user(
    reply: &str,
    last_user: Option<&str>,
    recent_slang: &[String],
    reading: &Reading,
    candidates: &[Meme],
    max: usize,
) -> String {
    let mut out = format!(
        "Chat intent: {}. Reply intent: {}.\n\n",
        reading.chat_intent, reading.reply_intent
    );
    let regional_slang = !matches!(
        reading.user_language,
        Some(UserLanguage::English | UserLanguage::Other)
    );
    if let Some(best) = reading.slang_best.as_ref().filter(|_| regional_slang) {
        out.push_str(&format!(
            "Best-fitting slang for this reply (judged by a classifier): {best}\n\n"
        ));
    }
    if let Some(u) = last_user {
        out.push_str(
            "User's latest message (reply in exactly this language, script, and mix of languages):\n",
        );
        out.push_str(u);
        out.push_str("\n\n");
    }
    if !recent_slang.is_empty() {
        out.push_str("Slang already used in your last few replies (prefer different words now): ");
        out.push_str(&recent_slang.join(", "));
        out.push_str("\n\n");
    }
    if max > 0 && !candidates.is_empty() {
        out.push_str("Meme candidates:\n");
        for (i, m) in candidates.iter().enumerate() {
            match &m.meaning {
                Some(meaning) => out.push_str(&format!("{}. {} ({})\n", i + 1, m.title, meaning)),
                None => out.push_str(&format!("{}. {}\n", i + 1, m.title)),
            }
        }
        out.push('\n');
    }
    out.push_str("Original reply:\n");
    out.push_str(reply);
    out
}

/// Replace `[[meme:N]]` markers with markdown images, enforcing `max` and
/// dropping unknown or repeated ids. A meme appears only where the model put
/// one; none is forced in.
pub(crate) fn resolve_markers(text: &str, candidates: &[Meme], max: usize) -> (String, Vec<Meme>) {
    let mut out = String::with_capacity(text.len());
    let mut used: Vec<usize> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[meme:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "[[meme:".len()..];
        let Some(end) = after.find("]]") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let pick = after[..end]
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1));
        if let Some(i) =
            pick.filter(|i| *i < candidates.len() && !used.contains(i) && used.len() < max)
        {
            used.push(i);
            out.push_str(&image(&candidates[i]));
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    let memes = used.into_iter().map(|i| candidates[i].clone()).collect();
    (collapse_blank_lines(&out), memes)
}

fn image(m: &Meme) -> String {
    let alt: String = m
        .title
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '(' | ')'))
        .collect();
    format!("\n\n![{}]({})\n\n", alt.trim(), m.url)
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.trim().lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_owned()
}

/// A remix may add some flavour, but not turn into a different, longer answer.
pub(crate) fn within_length(original: &str, rewritten: &str) -> bool {
    let (a, b) = (original.chars().count(), rewritten.chars().count());
    b <= a * 2 + 200
}

/// Every fenced block, inline code span, and URL in `original` must survive.
pub(crate) fn preserves_protected(original: &str, rewritten: &str) -> bool {
    protected_spans(original)
        .iter()
        .all(|s| rewritten.contains(s.as_str()))
}

fn protected_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut prose = String::new();
    let mut fence: Option<Vec<&str>> = None;
    for line in text.lines() {
        let is_fence = line.trim_start().starts_with("```");
        match (&mut fence, is_fence) {
            (None, true) => fence = Some(Vec::new()),
            (Some(body), true) => {
                spans.push(body.join("\n"));
                fence = None;
            }
            (Some(body), false) => body.push(line),
            (None, false) => {
                prose.push_str(line);
                prose.push('\n');
            }
        }
    }
    for (i, part) in prose.split('`').enumerate() {
        if i % 2 == 1 && !part.is_empty() {
            spans.push(part.to_owned());
        }
    }
    spans.extend(
        prose
            .split_whitespace()
            .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
            .map(|w| {
                w.trim_end_matches(['.', ',', ')', '!', '?', ';', ':'])
                    .to_owned()
            }),
    );
    spans.retain(|s| !s.trim().is_empty());
    spans
}

/// Models sometimes wrap the whole answer in a ```markdown fence.
fn strip_wrapping_fence(text: &str) -> &str {
    let Some(inner) = text
        .strip_prefix("```markdown")
        .or_else(|| text.strip_prefix("```md"))
    else {
        return text;
    };
    inner.strip_suffix("```").map_or(text, str::trim)
}

pub(crate) fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
