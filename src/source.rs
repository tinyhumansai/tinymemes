//! Where memes come from. Every source is optional and best-effort: a source
//! that fails returns an error and the agent carries on with the others.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::error::BoxError;
use crate::intent::Intent;

/// One meme image the agent may insert.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Meme {
    /// Human-readable title, used as the image alt text.
    pub title: String,
    /// Direct image URL.
    pub url: String,
    /// Source name, e.g. `imgflip`.
    pub source: String,
}

/// A searchable meme catalog.
#[async_trait]
pub trait MemeSource: Send + Sync {
    /// Short stable name for logs and [`Meme::source`].
    fn name(&self) -> &'static str;

    /// Up to `limit` memes for `query`, with `intent` as a mood hint.
    async fn search(
        &self,
        query: &str,
        intent: Intent,
        limit: usize,
    ) -> Result<Vec<Meme>, BoxError>;
}

/// Imgflip's public top-100 template list. Needs no key; the list is fetched
/// once and matched locally by intent hints and query words.
#[derive(Debug)]
pub struct Imgflip {
    http: reqwest::Client,
    templates: OnceCell<Vec<Meme>>,
}

impl Imgflip {
    /// A client that fetches templates on first use.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            templates: OnceCell::new(),
        }
    }

    /// A client over a fixed template list (tests, offline hosts).
    pub fn with_templates(templates: Vec<Meme>) -> Self {
        Self {
            http: reqwest::Client::new(),
            templates: OnceCell::new_with(Some(templates)),
        }
    }

    async fn templates(&self) -> Result<&[Meme], BoxError> {
        #[derive(Deserialize)]
        struct Resp {
            success: bool,
            data: Data,
        }
        #[derive(Deserialize)]
        struct Data {
            memes: Vec<Template>,
        }
        #[derive(Deserialize)]
        struct Template {
            name: String,
            url: String,
        }
        let list = self
            .templates
            .get_or_try_init(|| async {
                let resp: Resp = self
                    .http
                    .get("https://api.imgflip.com/get_memes")
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if !resp.success {
                    return Err::<_, BoxError>("imgflip reported failure".into());
                }
                Ok(resp
                    .data
                    .memes
                    .into_iter()
                    .map(|t| Meme {
                        title: t.name,
                        url: t.url,
                        source: "imgflip".to_owned(),
                    })
                    .collect())
            })
            .await?;
        Ok(list)
    }
}

#[async_trait]
impl MemeSource for Imgflip {
    fn name(&self) -> &'static str {
        "imgflip"
    }

    async fn search(
        &self,
        query: &str,
        intent: Intent,
        limit: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        let templates = self.templates().await?;
        let words: Vec<String> = tokens(query).filter(|w| w.len() > 2).collect();
        let mut scored: Vec<(usize, &Meme)> = templates
            .iter()
            .filter_map(|m| {
                let name = m.title.to_lowercase();
                let hint = intent.template_hints().iter().any(|h| name.contains(h));
                let overlap = tokens(&name).filter(|t| words.contains(t)).count();
                let score = usize::from(hint) * 2 + overlap;
                (score > 0).then_some((score, m))
            })
            .collect();
        // Stable sort keeps Imgflip's popularity order among equal scores.
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, m)| m.clone())
            .collect())
    }
}

/// GIPHY search. Requires an API key.
#[derive(Debug)]
pub struct Giphy {
    http: reqwest::Client,
    api_key: String,
}

impl Giphy {
    /// A GIPHY client.
    pub fn new(http: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
        }
    }
}

#[async_trait]
impl MemeSource for Giphy {
    fn name(&self) -> &'static str {
        "giphy"
    }

    async fn search(
        &self,
        query: &str,
        _intent: Intent,
        limit: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        #[derive(Deserialize)]
        struct Resp {
            data: Vec<Gif>,
        }
        #[derive(Deserialize)]
        struct Gif {
            title: String,
            images: Images,
        }
        #[derive(Deserialize)]
        struct Images {
            downsized: Image,
        }
        #[derive(Deserialize)]
        struct Image {
            url: String,
        }
        let limit = limit.to_string();
        let resp: Resp = self
            .http
            .get("https://api.giphy.com/v1/gifs/search")
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("q", query),
                ("limit", limit.as_str()),
                ("rating", "pg-13"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .data
            .into_iter()
            .map(|g| Meme {
                title: g.title,
                url: g.images.downsized.url,
                source: "giphy".to_owned(),
            })
            .collect())
    }
}

/// Tenor v2 search. Requires a Google API key.
#[derive(Debug)]
pub struct Tenor {
    http: reqwest::Client,
    api_key: String,
}

impl Tenor {
    /// A Tenor client.
    pub fn new(http: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
        }
    }
}

#[async_trait]
impl MemeSource for Tenor {
    fn name(&self) -> &'static str {
        "tenor"
    }

    async fn search(
        &self,
        query: &str,
        _intent: Intent,
        limit: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        #[derive(Deserialize)]
        struct Resp {
            results: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            content_description: String,
            media_formats: Formats,
        }
        #[derive(Deserialize)]
        struct Formats {
            tinygif: Format,
        }
        #[derive(Deserialize)]
        struct Format {
            url: String,
        }
        let limit = limit.to_string();
        let resp: Resp = self
            .http
            .get("https://tenor.googleapis.com/v2/search")
            .query(&[
                ("key", self.api_key.as_str()),
                ("q", query),
                ("limit", limit.as_str()),
                ("contentfilter", "medium"),
                ("media_filter", "tinygif"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .results
            .into_iter()
            .map(|i| Meme {
                title: i.content_description,
                url: i.media_formats.tinygif.url,
                source: "tenor".to_owned(),
            })
            .collect())
    }
}

fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str) -> Meme {
        Meme {
            title: name.to_owned(),
            url: format!("https://i.imgflip.com/{}.jpg", name.len()),
            source: "imgflip".into(),
        }
    }

    #[tokio::test]
    async fn imgflip_prefers_intent_hints_then_query_words() {
        let src = Imgflip::with_templates(vec![
            template("Drake Hotline Bling"),
            template("This Is Fine"),
            template("Fine Dining Cat"),
            template("Two Buttons"),
        ]);
        let hits = src
            .search("everything is fine", Intent::Frustration, 5)
            .await
            .unwrap();
        let names: Vec<_> = hits.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(names, ["This Is Fine", "Fine Dining Cat"]);
    }
}
