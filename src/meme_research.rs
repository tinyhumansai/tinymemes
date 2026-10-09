//! Find new reaction GIFs on the web through a host search backend.
//!
//! Search engines return pages, not media, so [`GiphyPageResearcher`] searches
//! for GIPHY GIF pages and reads each page for what GIPHY itself publishes
//! there: the media link (`og:image`), GIPHY's own content rating, and the
//! GIF's tags. No GIPHY API key is involved. The media link is switched to
//! GIPHY's small `200.webp` rendition, so chat bubbles do not load megabytes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::BoxError;
use crate::research::WebSearch;

/// Hosts a learned meme's media may come from.
pub const ALLOWED_MEDIA_HOSTS: &[&str] = &[
    "media.giphy.com",
    "media0.giphy.com",
    "media1.giphy.com",
    "media2.giphy.com",
    "media3.giphy.com",
    "media4.giphy.com",
    "i.giphy.com",
];

/// Largest media file accepted for a chat bubble.
const MAX_MEDIA_BYTES: u64 = 1_500_000;

/// A GIF found on the web, before vetting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundMeme {
    /// Page title, cleaned of the host's boilerplate.
    pub title: String,
    /// The GIF's own page.
    pub page_url: String,
    /// Direct media link, small rendition.
    pub media_url: String,
    /// The host's content rating for the GIF (`g`, `pg`, `pg-13`, `r`).
    pub rating: Option<String>,
    /// The host's tags for the GIF.
    pub tags: Vec<String>,
    /// Words of the page URL's slug (a strong hint at what the GIF shows).
    pub slug_words: Vec<String>,
}

/// Finds GIFs for a research query.
#[async_trait]
pub trait MemeResearcher: Send + Sync {
    /// Run one query and return candidate GIFs.
    async fn research(&self, query: &str) -> Result<Vec<FoundMeme>, BoxError>;
}

/// Searches for GIPHY GIF pages and reads each page for its media and metadata.
#[derive(Clone)]
pub struct GiphyPageResearcher {
    search: Arc<dyn WebSearch>,
    http: reqwest::Client,
    max_results: usize,
}

impl std::fmt::Debug for GiphyPageResearcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GiphyPageResearcher")
            .finish_non_exhaustive()
    }
}

impl GiphyPageResearcher {
    /// A researcher over a host search backend, reading up to 8 results.
    pub fn new(search: Arc<dyn WebSearch>, http: reqwest::Client) -> Self {
        Self {
            search,
            http,
            max_results: 8,
        }
    }

    async fn read_page(&self, page_url: &str) -> Option<FoundMeme> {
        let html = self
            .http
            .get(page_url)
            .header("user-agent", "Mozilla/5.0 (compatible; tinymemes/0.1)")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;
        let found = parse_giphy_page(page_url, &html)?;
        let head = self
            .http
            .head(&found.media_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .ok()?;
        let is_image = head
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("image/"));
        let small_enough = head
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_none_or(|n| n <= MAX_MEDIA_BYTES);
        (head.status().is_success() && is_image && small_enough).then_some(found)
    }
}

#[async_trait]
impl MemeResearcher for GiphyPageResearcher {
    async fn research(&self, query: &str) -> Result<Vec<FoundMeme>, BoxError> {
        let hits = self.search.search(query, self.max_results).await?;
        let pages: Vec<String> = hits
            .into_iter()
            .map(|h| h.url)
            .filter(|u| giphy_gif_id(u).is_some())
            .collect();
        let read = join_all(pages.iter().map(|p| self.read_page(p))).await;
        Ok(read.into_iter().flatten().collect())
    }
}

/// The GIF id of a GIPHY GIF page URL (`https://giphy.com/gifs/<slug>-<id>`).
pub fn giphy_gif_id(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://giphy.com/gifs/")
        .or_else(|| url.strip_prefix("https://www.giphy.com/gifs/"))?;
    let segment = rest.split(['/', '?', '#']).next()?;
    let id = segment.rsplit('-').next()?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then_some(id)
}

/// Read a GIPHY GIF page: media link, rating, tags, title. `None` when the
/// page is not a usable GIF page.
pub fn parse_giphy_page(page_url: &str, html: &str) -> Option<FoundMeme> {
    let id = giphy_gif_id(page_url)?;
    let media = meta(html, "og:image").or_else(|| meta(html, "twitter:image"))?;
    let media_url = small_rendition(&media)?;
    let slug = page_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(id)
        .trim_end_matches('-');
    let slug_words: Vec<String> = slug
        .split('-')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let title = meta(html, "og:title")
        .unwrap_or_default()
        .replace(" - Find & Share on GIPHY", "")
        .trim()
        .to_owned();
    Some(FoundMeme {
        title,
        page_url: page_url.to_owned(),
        media_url,
        rating: rating_for(html, id),
        tags: tags_for(html, &slug_words),
        slug_words,
    })
}

fn meta(html: &str, property: &str) -> Option<String> {
    for pattern in [
        format!("property=\"{property}\""),
        format!("name=\"{property}\""),
    ] {
        let Some(at) = html.find(&pattern) else {
            continue;
        };
        let tag_start = html[..at].rfind('<')?;
        let tag_end = at + html[at..].find('>')?;
        let tag = &html[tag_start..tag_end];
        let content = tag.split("content=\"").nth(1)?.split('"').next()?;
        return Some(content.replace("&amp;", "&"));
    }
    None
}

/// GIPHY's small animated rendition (`200.webp`), from an allowed host.
fn small_rendition(media: &str) -> Option<String> {
    let host = media.strip_prefix("https://")?.split('/').next()?;
    if !ALLOWED_MEDIA_HOSTS.contains(&host) {
        return None;
    }
    let path = media.split(['?', '#']).next()?;
    let (dir, _file) = path.rsplit_once('/')?;
    Some(format!("{dir}/200.webp"))
}

/// The rating recorded next to this GIF's id in the page's embedded data.
fn rating_for(html: &str, id: &str) -> Option<String> {
    let html = html.replace("\\\"", "\"");
    let anchor = html.find(&format!("\"id\":\"{id}\"")).unwrap_or(0);
    let window = &html[anchor..html.len().min(anchor + 4000)];
    let after = window
        .split("\"rating\":\"")
        .nth(1)
        .or_else(|| html.split("\"rating\":\"").nth(1))?;
    let rating = after.split('"').next()?.to_ascii_lowercase();
    (!rating.is_empty() && rating.len() <= 6).then_some(rating)
}

/// The tag list that belongs to this GIF: the page embeds tags for related
/// GIFs too, so pick the list that shares the most words with the slug.
fn tags_for(html: &str, slug_words: &[String]) -> Vec<String> {
    let html = html.replace("\\\"", "\"");
    let lists: Vec<Vec<String>> = html
        .split("\"tags\":[")
        .skip(1)
        .filter_map(|chunk| chunk.split(']').next())
        .map(|inner| {
            inner
                .split('"')
                .enumerate()
                .filter(|(i, s)| i % 2 == 1 && !s.is_empty())
                .map(|(_, s)| s.to_lowercase())
                .collect()
        })
        .collect();
    let score = |tags: &Vec<String>| {
        tags.iter()
            .flat_map(|t| t.split(' '))
            .filter(|w| slug_words.iter().any(|s| s == w))
            .count()
    };
    lists
        .iter()
        .max_by_key(|tags| score(tags))
        .filter(|tags| score(tags) > 0)
        .or_else(|| lists.first())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "meme_research_tests.rs"]
mod tests;
