//! Live run: Jev over OpenRouter, a chat model over OpenRouter, Imgflip memes
//! (plus GIPHY / Tenor when their keys are set).
//!
//! ```sh
//! OPENROUTER_API_KEY=… cargo run --example remix
//! OPENROUTER_API_KEY=… cargo run --example remix -- chat.json
//! ```
//!
//! `chat.json` is `{"conversation": [{"role": "user", "text": "…"}], "reply": "…"}`.
//! Spends one Jev call and up to two chat-model calls.

use std::sync::Arc;

use serde::Deserialize;
use tinyinference_decisions::{Client, ClientConfig};
use tinymemes::model::OpenAiCompatible;
use tinymemes::source::{Giphy, Imgflip, Tenor};
use tinymemes::{MemeEngine, Turn};

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

    let mut builder = MemeEngine::builder(
        Arc::new(Client::new(ClientConfig::openrouter(&key))?),
        Arc::new(OpenAiCompatible::openrouter(http.clone(), &key, model)),
    )
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
    if let Some(remix) = &out.remix {
        eprintln!(
            "queries={:?} memes={} rewrite_kept={}",
            remix.queries,
            remix.memes.len(),
            remix.rewrite_kept
        );
    }
    if let Some(why) = &out.skipped {
        eprintln!("skipped: {why}");
    }
    println!("{}", out.reply);
    Ok(())
}
