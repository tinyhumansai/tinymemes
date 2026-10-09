//! The slang agent: plan meme searches, fetch candidates, rewrite the reply in
//! the region's slang with meme markers, and resolve the markers to images.

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::join_all;
use serde::Serialize;

use crate::conversation::{Role, Turn, memes_as_text};
use crate::error::{Error, Result};
use crate::meme_index::MemeIndex;
use crate::model::ChatModel;
use crate::rating::Rating;
use crate::reading::{MemePick, Reading, UserLanguage};
use crate::region::{Region, SlangTerm};
use crate::slang::SlangIndex;
use crate::source::{Meme, MemeSource};

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
    /// ballooned, or added a blocklisted word) and the original wording was kept.
    pub rewrite_kept: bool,
    /// Whether the reply was rewritten or only had a meme attached.
    pub mode: RemixMode,
    /// Lines removed from the rewrite because it repeated them.
    pub duplicates_removed: usize,
}

/// How a reply was remixed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemixMode {
    /// The rewrite LLM restyled the reply.
    Rewrite,
    /// The reply already matched the user; only a meme was attached.
    MemeOnly,
}

/// Plans, fetches, rewrites, and inserts.
#[derive(Clone)]
pub struct SlangAgent {
    model: Arc<dyn ChatModel>,
    sources: Vec<Arc<dyn MemeSource>>,
    region: Arc<Region>,
    index: Arc<SlangIndex>,
    memes: Arc<MemeIndex>,
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
        memes: Arc<MemeIndex>,
    ) -> Self {
        index.seed(&region);
        Self {
            model,
            sources,
            region: Arc::new(region),
            index,
            memes,
        }
    }

    /// The learned-meme index in use.
    pub fn meme_index(&self) -> &Arc<MemeIndex> {
        &self.memes
    }

    /// The chat model the agent rewrites with.
    pub fn model(&self) -> &Arc<dyn ChatModel> {
        &self.model
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
        meme_only: bool,
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
        // Memes: Jev already chose from the region catalog in the reading call.
        // With no catalog (or no answer), search external sources by intent.
        // There is no LLM call for this step.
        let not_sent = |m: &Meme| {
            !sent
                .iter()
                .any(|s| s.title.eq_ignore_ascii_case(&m.title) || s.url.as_deref() == Some(&m.url))
        };
        let (queries, candidates): (Vec<String>, Vec<Meme>) = if rating.max_memes == 0 {
            (Vec::new(), Vec::new())
        } else {
            match &reading.meme {
                MemePick::Pick(title) => {
                    // Curated catalog first, then approved learned GIFs.
                    let picked = self
                        .region
                        .memes
                        .iter()
                        .find(|m| &m.title == title)
                        .map(|m| m.to_meme())
                        .or_else(|| self.memes.find_approved(&self.region.code, title));
                    (
                        vec![format!("jev:{title}")],
                        picked.into_iter().filter(not_sent).collect(),
                    )
                }
                MemePick::NoneFit => (vec!["jev:none_fit".to_owned()], Vec::new()),
                MemePick::Unasked if has_sources => {
                    let queries: Vec<String> = reading
                        .reply_intent
                        .fallback_queries()
                        .iter()
                        .map(|q| (*q).to_owned())
                        .collect();
                    let found = self.fetch(&queries, reading).await;
                    (queries, found.into_iter().filter(not_sent).collect())
                }
                MemePick::Unasked => (Vec::new(), Vec::new()),
            }
        };

        // Meme-only mode: the agent's reply already matches the user's register,
        // so a rewrite would add little and risk artifacts. Keep the wording and
        // only attach Jev's meme.
        if meme_only {
            let (reply, memes) = attach_meme(reply, &candidates, rating.max_memes);
            memes.iter().for_each(|m| self.memes.record_used(&m.url));
            let slang_used = self.index.record_used(&self.region.code, &reply);
            return Ok(Remix {
                reply,
                memes,
                queries,
                slang_offered: Vec::new(),
                slang_used,
                rewrite_kept: true,
                mode: RemixMode::MemeOnly,
                duplicates_removed: 0,
            });
        }

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
        // Repair, don't discard: a line the rewrite copied twice loses its extra copies.
        let (rewritten, duplicates_removed) = repair_duplicates(reply, rewritten);
        if duplicates_removed > 0 {
            tracing::debug!(
                duplicates_removed,
                "[tinymemes] removed duplicated lines from rewrite"
            );
        }

        // Only gaali the rewrite adds counts; words already in the agent's reply
        // are the agent's, and sending the original would not remove them.
        let added = self.region.blocked_added(reply, &rewritten);
        let (text, rewrite_kept) = if !added.is_empty() {
            tracing::warn!(
                ?added,
                "[tinymemes] rewrite added blocklisted words; keeping original wording"
            );
            (reply.to_owned(), false)
        } else if !preserves_protected(reply, &rewritten) || !within_length(reply, &rewritten) {
            tracing::warn!(
                "[tinymemes] rewrite dropped protected content or ballooned; keeping original wording"
            );
            (reply.to_owned(), false)
        } else {
            (rewritten, true)
        };
        let (reply, memes) = resolve_markers(&text, &candidates, rating.max_memes);
        memes.iter().for_each(|m| self.memes.record_used(&m.url));
        let slang_used = self.index.record_used(&self.region.code, &reply);
        let slang_offered = slang.into_iter().map(|t| t.term).collect();
        Ok(Remix {
            reply,
            memes,
            queries,
            slang_offered,
            slang_used,
            rewrite_kept,
            mode: RemixMode::Rewrite,
            duplicates_removed,
        })
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

/// Put the first allowed meme after the reply's first paragraph (or at the
/// end of a one-paragraph reply). Used when the wording is kept as is.
pub(crate) fn attach_meme(reply: &str, candidates: &[Meme], max: usize) -> (String, Vec<Meme>) {
    let Some(meme) = candidates.first().filter(|_| max > 0) else {
        return (reply.to_owned(), Vec::new());
    };
    let reply = reply.trim_end();
    let out = match reply.find("\n\n") {
        Some(cut) => format!(
            "{}{}{}",
            &reply[..cut],
            image(meme),
            reply[cut..].trim_start()
        ),
        None => format!("{reply}{}", image(meme)),
    };
    (collapse_blank_lines(&out), vec![meme.clone()])
}

/// Words of a line, lowercased, for comparing lines that differ only in
/// punctuation, emoji, bullets, or casing.
fn line_words(line: &str) -> Vec<String> {
    crate::region::tokens(line).collect()
}

/// How much of the shorter line's words the longer one contains.
fn overlap(a: &[String], b: &[String]) -> f64 {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.is_empty() {
        return 0.0;
    }
    let hits = short.iter().filter(|w| long.contains(w)).count();
    hits as f64 / short.len() as f64
}

/// Shared words over all distinct words of both lines.
fn jaccard(a: &[String], b: &[String]) -> f64 {
    let mut all: Vec<&String> = a.iter().chain(b.iter()).collect();
    all.sort();
    all.dedup();
    if all.is_empty() {
        return 0.0;
    }
    let shared = all
        .iter()
        .filter(|w| a.contains(w) && b.contains(w))
        .count();
    shared as f64 / all.len() as f64
}

const DUP_MIN_WORDS: usize = 6;
const DUP_OVERLAP: f64 = 0.8;

/// Remove lines the rewrite repeated. A substantial line (6+ words) that
/// near-copies another rewrite line is a duplicate when the original did not
/// have it twice; of the copies, the one closest to an original line is kept.
pub(crate) fn repair_duplicates(original: &str, rewritten: &str) -> (String, usize) {
    let orig: Vec<Vec<String>> = original
        .lines()
        .map(line_words)
        .filter(|w| w.len() >= DUP_MIN_WORDS)
        .collect();
    let lines: Vec<&str> = rewritten.lines().collect();
    let words: Vec<Vec<String>> = lines.iter().map(|l| line_words(l)).collect();
    // How close each rewrite line is to its best-matching original line.
    // Jaccard, so extra words count against a copy: of two near-copies, the
    // one that mirrors the original line most exactly is kept.
    let closeness: Vec<f64> = words
        .iter()
        .map(|w| orig.iter().map(|o| jaccard(w, o)).fold(0.0, f64::max))
        .collect();
    let mut drop = vec![false; lines.len()];
    for i in 0..lines.len() {
        if drop[i] || words[i].len() < DUP_MIN_WORDS {
            continue;
        }
        for j in (i + 1)..lines.len() {
            if drop[j]
                || words[j].len() < DUP_MIN_WORDS
                || overlap(&words[i], &words[j]) < DUP_OVERLAP
            {
                continue;
            }
            // The original repeating it too means the repeat is intended.
            let in_original = orig
                .iter()
                .filter(|o| overlap(o, &words[i]) >= DUP_OVERLAP)
                .count();
            if in_original >= 2 {
                continue;
            }
            if closeness[j] > closeness[i] {
                drop[i] = true;
                break;
            }
            drop[j] = true;
        }
    }
    let removed = drop.iter().filter(|d| **d).count();
    if removed == 0 {
        return (rewritten.to_owned(), 0);
    }
    let kept: Vec<&str> = lines
        .iter()
        .zip(&drop)
        .filter(|(_, d)| !**d)
        .map(|(l, _)| *l)
        .collect();
    (collapse_blank_lines(&kept.join("\n")), removed)
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
