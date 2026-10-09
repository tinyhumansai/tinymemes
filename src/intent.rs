//! The closed set of chat intents Jev chooses between.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// What a conversation (or a reply) is doing, emotionally.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// Sharing a win or good news.
    Celebration,
    /// Venting; something is broken or going wrong.
    Frustration,
    /// Lost, asking how or why something works.
    Confusion,
    /// Joking, teasing, playful chit-chat.
    Banter,
    /// Heads-down work: getting a task done, planning, executing.
    Grind,
    /// A loss, failure, or disappointment.
    BadNews,
    /// Exploring an idea or asking for opinions.
    Curiosity,
    /// Thanking or appreciating.
    Gratitude,
}

impl Intent {
    /// Every intent, in a stable order.
    pub const ALL: [Intent; 8] = [
        Intent::Celebration,
        Intent::Frustration,
        Intent::Confusion,
        Intent::Banter,
        Intent::Grind,
        Intent::BadNews,
        Intent::Curiosity,
        Intent::Gratitude,
    ];

    /// Wire label used as the Jev Choice option.
    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Celebration => "celebration",
            Intent::Frustration => "frustration",
            Intent::Confusion => "confusion",
            Intent::Banter => "banter",
            Intent::Grind => "grind",
            Intent::BadNews => "bad_news",
            Intent::Curiosity => "curiosity",
            Intent::Gratitude => "gratitude",
        }
    }

    /// Parse a wire label.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == label)
    }

    /// Description that distinguishes this option for Jev.
    pub fn description(self) -> &'static str {
        match self {
            Intent::Celebration => "sharing a win, good news, hype, excitement",
            Intent::Frustration => "venting or annoyed; something is broken, slow, or going wrong",
            Intent::Confusion => "lost or stuck; asking how or why something works",
            Intent::Banter => "joking, teasing, roasting, playful chit-chat",
            Intent::Grind => "focused work: getting a task done, planning, step-by-step execution",
            Intent::BadNews => "a loss, failure, rejection, or disappointment",
            Intent::Curiosity => "exploring an idea, asking for opinions or recommendations",
            Intent::Gratitude => "thanking or appreciating someone",
        }
    }

    /// Short phrase used in slang research queries.
    pub fn phrase(self) -> &'static str {
        match self {
            Intent::Celebration => "celebrating a win",
            Intent::Frustration => "being frustrated or annoyed",
            Intent::Confusion => "being confused",
            Intent::Banter => "teasing friends and joking around",
            Intent::Grind => "working hard and getting things done",
            Intent::BadNews => "something going wrong or a letdown",
            Intent::Curiosity => "being curious or impressed",
            Intent::Gratitude => "thanking someone",
        }
    }

    /// Imgflip template names that usually fit this mood. Matched by
    /// case-insensitive substring, so a renamed template just stops matching.
    pub fn template_hints(self) -> &'static [&'static str] {
        match self {
            Intent::Celebration => &[
                "success kid",
                "leonardo dicaprio cheers",
                "buff doge",
                "epic handshake",
            ],
            Intent::Frustration => &[
                "this is fine",
                "disaster girl",
                "woman yelling at cat",
                "hide the pain harold",
            ],
            Intent::Confusion => &[
                "is this a pigeon",
                "futurama fry",
                "surprised pikachu",
                "monkey puppet",
            ],
            Intent::Banter => &[
                "drake hotline bling",
                "distracted boyfriend",
                "change my mind",
                "uno draw 25",
            ],
            Intent::Grind => &[
                "gru's plan",
                "expanding brain",
                "two buttons",
                "left exit 12",
            ],
            Intent::BadNews => &[
                "sad pablo escobar",
                "hide the pain harold",
                "waiting skeleton",
                "panik kalm",
            ],
            Intent::Curiosity => &[
                "roll safe",
                "expanding brain",
                "change my mind",
                "always has been",
            ],
            Intent::Gratitude => &["epic handshake", "leonardo dicaprio cheers", "success kid"],
        }
    }

    /// Search queries used when the agent's own plan is unusable.
    pub fn fallback_queries(self) -> &'static [&'static str] {
        match self {
            Intent::Celebration => &["lets go", "celebration"],
            Intent::Frustration => &["this is fine", "facepalm"],
            Intent::Confusion => &["confused", "mind blown"],
            Intent::Banter => &["no u", "laughing"],
            Intent::Grind => &["lets get to work", "big brain"],
            Intent::BadNews => &["its okay", "hug"],
            Intent::Curiosity => &["thinking", "big brain"],
            Intent::Gratitude => &["thank you", "respect"],
        }
    }

    /// Jev Choice criteria covering every intent.
    pub(crate) fn criteria() -> std::collections::BTreeMap<String, Option<Value>> {
        Self::ALL
            .into_iter()
            .map(|i| (i.as_str().to_owned(), Some(json!(i.description()))))
            .collect()
    }
}

impl std::fmt::Display for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
