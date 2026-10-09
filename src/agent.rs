//! The slang agent: plan meme searches, fetch candidates, rewrite the reply in
//! slang with meme markers, and resolve the markers to images.

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::intent::Intent;
use crate::model::ChatModel;
use crate::rating::Rating;
use crate::reading::Reading;
use crate::source::{Meme, MemeSource};

const MAX_QUERIES: usize = 3;
const PER_SEARCH: usize = 3;
const MAX_CANDIDATES: usize = 8;

/// What the agent produced.
#[derive(Clone, Debug, Serialize)]
pub struct Remix {
    /// Final reply text (markdown, images inlined as `![title](url)`).
    pub reply: String,
    /// Memes that ended up in the reply.
    pub memes: Vec<Meme>,
    /// Search queries the agent ran.
    pub queries: Vec<String>,
    /// False when the slang rewrite was discarded for dropping protected
    /// content (code, links) and the original wording was kept.
    pub rewrite_kept: bool,
}

/// Plans, fetches, rewrites, and inserts.
#[derive(Clone)]
pub struct SlangAgent {
    model: Arc<dyn ChatModel>,
    sources: Vec<Arc<dyn MemeSource>>,
}

impl std::fmt::Debug for SlangAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.sources.iter().map(|s| s.name()).collect();
        f.debug_struct("SlangAgent")
            .field("sources", &names)
            .finish_non_exhaustive()
    }
}

impl SlangAgent {
    /// An agent over a chat model and meme sources (searched in order).
    pub fn new(model: Arc<dyn ChatModel>, sources: Vec<Arc<dyn MemeSource>>) -> Self {
        Self { model, sources }
    }

    /// Remix `reply` at `rating`'s intensity.
    pub async fn run(&self, reply: &str, reading: &Reading, rating: Rating) -> Result<Remix> {
        let (queries, candidates) = if rating.max_memes > 0 && !self.sources.is_empty() {
            let queries = self.plan(reply, reading).await;
            let candidates = self.fetch(&queries, reading.reply_intent).await;
            (queries, candidates)
        } else {
            (Vec::new(), Vec::new())
        };

        let rewritten = self
            .model
            .complete(
                &rewrite_system(rating),
                &rewrite_user(reply, reading, &candidates, rating.max_memes),
            )
            .await
            .map_err(Error::Model)?;
        let rewritten = strip_wrapping_fence(rewritten.trim());

        let (text, rewrite_kept) = if preserves_protected(reply, rewritten)
            && within_length(reply, rewritten)
        {
            (rewritten.to_owned(), true)
        } else {
            tracing::warn!(
                "[tinymemes] rewrite dropped protected content or ballooned; keeping original wording"
            );
            (reply.to_owned(), false)
        };
        let (reply, memes) = resolve_markers(&text, &candidates, rating.max_memes);
        Ok(Remix {
            reply,
            memes,
            queries,
            rewrite_kept,
        })
    }

    /// Ask the model for short meme searches; fall back to intent defaults.
    async fn plan(&self, reply: &str, reading: &Reading) -> Vec<String> {
        #[derive(Deserialize)]
        struct Plan {
            queries: Vec<String>,
        }
        let system = "You pick meme searches. Reply with JSON only: {\"queries\": [\"...\"]}. \
                      Give 1-3 short (1-4 word) searches for reaction memes or GIFs that match the \
                      mood of the reply. Prefer well-known meme names when one fits.";
        let user = format!(
            "Reply intent: {}\nChat intent: {}\n\nReply:\n{}",
            reading.reply_intent, reading.chat_intent, reply
        );
        let planned = match self.model.complete(system, &user).await {
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

    /// Search every source for every query concurrently; dedupe by URL.
    async fn fetch(&self, queries: &[String], intent: Intent) -> Vec<Meme> {
        let searches = self
            .sources
            .iter()
            .flat_map(|s| queries.iter().map(move |q| (s, q)))
            .map(|(s, q)| async move {
                match s.search(q, intent, PER_SEARCH).await {
                    Ok(hits) => hits,
                    Err(e) => {
                        tracing::debug!(source = s.name(), query = %q, error = %e, "[tinymemes] meme search failed");
                        Vec::new()
                    }
                }
            });
        let mut seen = HashSet::new();
        join_all(searches)
            .await
            .into_iter()
            .flatten()
            .filter(|m| m.url.starts_with("https://") && seen.insert(m.url.clone()))
            .take(MAX_CANDIDATES)
            .collect()
    }
}

fn rewrite_system(rating: Rating) -> String {
    format!(
        "You remix an AI assistant's reply so it sounds like a fun friend in a group chat.\n\
         Style: {style}\n\n\
         Hard rules:\n\
         - Keep every fact, number, step, name, and caveat. Do not add new claims.\n\
         - Stay about as long as the original. Swap the voice, do not add explanations.\n\
         - Copy fenced code blocks, inline code, and URLs exactly, character for character.\n\
         - Keep the same language as the original reply.\n\
         - No slurs, no punching down, nothing sexual.\n\
         - Insert at most {max} meme marker(s) of the form [[meme:N]] on their own line, \
           using only the numbered candidates given, where a reaction image lands best. \
           If there are no candidates, insert none.\n\
         - Output only the remixed reply. No preamble, no explanation.",
        style = rating.tier.style(),
        max = rating.max_memes,
    )
}

fn rewrite_user(reply: &str, reading: &Reading, candidates: &[Meme], max: usize) -> String {
    let mut out = format!(
        "Chat intent: {}. Reply intent: {}.\n\n",
        reading.chat_intent, reading.reply_intent
    );
    if max > 0 && !candidates.is_empty() {
        out.push_str("Meme candidates:\n");
        for (i, m) in candidates.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, m.title));
        }
        out.push('\n');
    }
    out.push_str("Original reply:\n");
    out.push_str(reply);
    out
}

/// Replace `[[meme:N]]` markers with markdown images, enforcing `max` and
/// dropping unknown or repeated ids. If the model placed none but memes were
/// allowed, the best candidate goes at the end.
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
    if used.is_empty() && max > 0 && !candidates.is_empty() {
        out.push_str("\n\n");
        out.push_str(&image(&candidates[0]));
        used.push(0);
    }
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

fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meme(n: usize) -> Meme {
        Meme {
            title: format!("Meme {n}"),
            url: format!("https://m.example/{n}.gif"),
            source: "test".into(),
        }
    }

    #[test]
    fn markers_resolve_with_cap_and_dedupe() {
        let c = [meme(1), meme(2), meme(3)];
        let (out, used) = resolve_markers(
            "yo\n[[meme:2]]\nmid [[meme:2]] [[meme:9]] [[meme:3]] [[meme:1]]\nend",
            &c,
            2,
        );
        assert_eq!(used, vec![meme(2), meme(3)]);
        assert!(out.contains("![Meme 2](https://m.example/2.gif)"));
        assert!(out.contains("![Meme 3](https://m.example/3.gif)"));
        assert!(!out.contains("[[meme"));
        assert!(!out.contains("\n\n\n"));
    }

    #[test]
    fn no_marker_appends_best_candidate() {
        let (out, used) = resolve_markers("all good fam", &[meme(1)], 1);
        assert_eq!(used.len(), 1);
        assert!(out.ends_with("![Meme 1](https://m.example/1.gif)"));
    }

    #[test]
    fn zero_budget_strips_markers() {
        let (out, used) = resolve_markers("hey [[meme:1]] there", &[meme(1)], 0);
        assert!(used.is_empty());
        assert_eq!(out, "hey  there");
    }

    #[test]
    fn protected_content_must_survive() {
        let original = "Run `cargo test` then see https://docs.rs/x.\n```rust\nfn main() {}\n```";
        assert!(preserves_protected(
            original,
            "ngl just `cargo test` fr, peep https://docs.rs/x\n```rust\nfn main() {}\n```"
        ));
        assert!(!preserves_protected(
            original,
            "just run the tests bestie https://docs.rs/x\n```rust\nfn main() {}\n```"
        ));
        assert!(!preserves_protected(
            original,
            "`cargo test` https://docs.rs/x\n```rust\nfn main(){}\n```"
        ));
    }

    #[test]
    fn ballooned_rewrite_is_rejected() {
        let original = "x".repeat(300);
        assert!(within_length(&original, &"y".repeat(700)));
        assert!(!within_length(&original, &"y".repeat(900)));
    }

    #[test]
    fn wrapping_fence_is_stripped() {
        assert_eq!(strip_wrapping_fence("```markdown\nyo\n```"), "yo");
        assert_eq!(
            strip_wrapping_fence("```rust\nfn x(){}\n```"),
            "```rust\nfn x(){}\n```"
        );
    }
}
