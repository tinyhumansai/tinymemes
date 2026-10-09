//! Live run: Jev over OpenRouter, a chat model over OpenRouter, Imgflip memes
//! (plus GIPHY / Tenor when their keys are set).
//!
//! ```sh
//! OPENROUTER_API_KEY=… cargo run --example remix
//! OPENROUTER_API_KEY=… cargo run --example remix -- chat.json
//! ```
//!
//! The slang index is saved to `$TINYMEMES_SLANG_INDEX` (default
//! `slang-index.json`) and grows with each run.
//!
//! `chat.json` is `{"conversation": [{"role": "user", "text": "…"}], "reply": "…"}`.
//! Spends one Jev call and up to two chat-model calls.

use std::sync::Arc;

use serde::Deserialize;
use tinyinference_decisions::{Client, ClientConfig};
use tinymemes::model::OpenAiCompatible;
use tinymemes::slang::OpenRouterWebResearcher;
use tinymemes::source::{Giphy, Imgflip, Tenor};
use tinymemes::{IndexPolicy, MemeEngine, SlangIndex, Turn};

#[derive(Deserialize)]
struct Input {
    conversation: Vec<Turn>,
    reply: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENROUTER_API_KEY")?;
    let model =
        std::env::var("TINYMEMES_MODEL").unwrap_or_else(|_| "google/gemini-2.5-flash".to_owned());
    let http = reqwest::Client::new();

    // The slang index persists between runs, so it grows with every query.
    let index_path =
        std::env::var("TINYMEMES_SLANG_INDEX").unwrap_or_else(|_| "slang-index.json".to_owned());
    let index = Arc::new(match std::fs::read_to_string(&index_path) {
        Ok(json) => SlangIndex::from_json(&json, IndexPolicy::default())?,
        Err(_) => SlangIndex::new(IndexPolicy::default()),
    });

    let mut builder = MemeEngine::builder(
        Arc::new(Client::new(ClientConfig::openrouter(&key))?),
        Arc::new(OpenAiCompatible::openrouter(http.clone(), &key, model)),
    )
    .slang_index(index.clone())
    .researcher(Arc::new(OpenRouterWebResearcher::new(
        http.clone(),
        &key,
        "google/gemini-2.5-flash",
    )))
    .source(Arc::new(Imgflip::new(http.clone())));
    if let Ok(k) = std::env::var("GIPHY_API_KEY") {
        builder = builder.source(Arc::new(Giphy::new(http.clone(), k)));
    }
    if let Ok(k) = std::env::var("TENOR_API_KEY") {
        builder = builder.source(Arc::new(Tenor::new(http.clone(), k)));
    }
    let engine = builder.build();

    let input = match std::env::args().nth(1) {
        Some(path) => serde_json::from_str::<Input>(&std::fs::read_to_string(path)?)?,
        None => Input {
            conversation: vec![
                Turn::user("bro i've been fighting this borrow checker for 3 hours 💀"),
                Turn::assistant("Paste the error and the function and I'll take a look."),
                Turn::user("ok here. it says `cannot borrow *self as mutable more than once` lmao help"),
            ],
            reply: "You're holding `let item = self.items.get(i)` while calling `self.push(...)`. \
                    Clone the item first (`let item = self.items[i].clone();`) or restructure so the \
                    borrow ends before the mutable call. That should compile."
                .to_owned(),
        },
    };

    let out = engine
        .try_process(&input.conversation, &input.reply)
        .await?;
    if let (Some(reading), Some(rating)) = (&out.reading, &out.rating) {
        eprintln!(
            "chat={} reply={} frankness={:.2} playful={:.2} serious={:.2} -> rating {}/10 ({:?})",
            reading.chat_intent,
            reading.reply_intent,
            reading.frankness,
            reading.playful,
            reading.serious,
            rating.score,
            rating.tier,
        );
    }
    if let Some(reading) = &out.reading {
        eprintln!(
            "jev slang check: best={:?} enough={:?} -> {}",
            reading.slang_best,
            reading.slang_enough,
            match (reading.wants_more_slang(0.5), out.learned.is_some()) {
                (false, _) => "index has enough, no search",
                (true, true) => "searched",
                (true, false) => "wanted a search, but it timed out, failed, or hit the budget",
            },
        );
    }
    if let Some(remix) = &out.remix {
        eprintln!(
            "queries={:?} memes={} rewrite_kept={}",
            remix.queries,
            remix.memes.len(),
            remix.rewrite_kept
        );
    }
    if let Some(learned) = &out.learned {
        eprintln!(
            "learned: +{} new, {} corroborated, {} rejected, {} failed Jev check <- {:?} (index now {} terms)",
            learned.added,
            learned.corroborated,
            learned.rejected,
            learned.failed_verification,
            learned.query,
            index.len("IN"),
        );
    }
    if let Some(remix) = &out.remix {
        eprintln!("slang used: {:?}", remix.slang_used);
    }
    std::fs::write(&index_path, index.to_json())?;
    if let Some(why) = &out.skipped {
        eprintln!("skipped: {why}");
    }
    println!("{}", out.reply);
    Ok(())
}
