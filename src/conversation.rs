//! The conversation shape the engine reads.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Who wrote a turn.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The human.
    User,
    /// The agent.
    Assistant,
}

/// One message in the chat.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Turn {
    /// Author of the message.
    pub role: Role,
    /// Message text, as the user saw it.
    pub text: String,
    /// True for an assistant turn that tinymemes remixed. Jev is told that
    /// its slang is the bot's style, not evidence of the user's tone.
    #[serde(default)]
    pub remixed: bool,
}

impl Turn {
    /// A user turn.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            remixed: false,
        }
    }

    /// An assistant turn.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            remixed: false,
        }
    }

    /// An assistant turn that was delivered remixed.
    pub fn remixed(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            remixed: true,
        }
    }
}

/// Bounds on how much of the chat is sent to Jev, so a long session does not
/// turn one meme decision into an expensive call.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    /// Most recent turns kept.
    pub max_turns: usize,
    /// Characters kept per turn (head of the message).
    pub max_chars_per_turn: usize,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            max_turns: 20,
            max_chars_per_turn: 1500,
        }
    }
}

/// Build the shared Jev state: the windowed conversation plus the reply being remixed.
pub(crate) fn jev_state(conversation: &[Turn], reply: &str, region: &str, window: Window) -> Value {
    let start = conversation.len().saturating_sub(window.max_turns);
    let turns: Vec<Value> = conversation[start..]
        .iter()
        .map(|t| {
            let text = clip(&memes_as_text(&t.text).0, window.max_chars_per_turn);
            if t.remixed {
                json!({
                    "role": t.role,
                    "text": text,
                    "note": "the assistant's own playful remix voice; not evidence of the user's tone",
                })
            } else {
                json!({ "role": t.role, "text": text })
            }
        })
        .collect();
    json!({
        "conversation": turns,
        "assistant_reply": clip(reply, window.max_chars_per_turn * 2),
        "audience_region": region,
    })
}

/// A meme found in a delivered message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentMeme {
    /// Meme title (the image alt text).
    pub title: String,
    /// Image URL, when the message carried the image itself.
    pub url: Option<String>,
}

/// Replace markdown images with `[meme: title]` and list the memes found.
/// Already-normalized `[meme: title]` markers are listed too.
pub fn memes_as_text(text: &str) -> (String, Vec<SentMeme>) {
    let mut out = String::with_capacity(text.len());
    let mut memes = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let parsed = after.find("](").and_then(|mid| {
            let url_part = &after[mid + 2..];
            url_part.find(')').map(|end| (mid, &url_part[..end], end))
        });
        match parsed {
            Some((mid, url, end)) if !after[..mid].contains('\n') => {
                let title = after[..mid].trim().to_owned();
                out.push_str(&format!("[meme: {title}]"));
                memes.push(SentMeme {
                    title,
                    url: Some(url.trim().to_owned()),
                });
                rest = &after[mid + 2 + end + 1..];
            }
            _ => {
                out.push_str("![");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let mut scan = out.as_str();
    while let Some(start) = scan.find("[meme: ") {
        let after = &scan[start + 7..];
        let Some(end) = after.find(']') else { break };
        let title = after[..end].trim().to_owned();
        if !memes.iter().any(|m| m.title == title) {
            memes.push(SentMeme { title, url: None });
        }
        scan = &after[end..];
    }
    (out, memes)
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
