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
    /// Message text.
    pub text: String,
}

impl Turn {
    /// A user turn.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
        }
    }

    /// An assistant turn.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
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
        .map(|t| json!({ "role": t.role, "text": clip(&t.text, window.max_chars_per_turn) }))
        .collect();
    json!({
        "conversation": turns,
        "assistant_reply": clip(reply, window.max_chars_per_turn * 2),
        "audience_region": region,
    })
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keeps_latest_turns_and_clips() {
        let convo: Vec<Turn> = (0..30)
            .map(|i| Turn::user(format!("msg {i} {}", "x".repeat(50))))
            .collect();
        let state = jev_state(
            &convo,
            "reply",
            "India",
            Window {
                max_turns: 3,
                max_chars_per_turn: 10,
            },
        );
        let turns = state["conversation"].as_array().unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0]["text"], "msg 27 xxx…");
        assert_eq!(state["assistant_reply"], "reply");
    }
}
