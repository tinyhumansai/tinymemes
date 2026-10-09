//! Turn a Jev reading into a 0–10 meme rating. Pure policy: no I/O.

use serde::{Deserialize, Serialize};

use crate::intent::Intent;
use crate::reading::Reading;

/// How hard the reply gets remixed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Leave the reply alone.
    Off,
    /// A light sprinkle of slang, no images.
    Light,
    /// Casual slang and one meme.
    Spicy,
    /// Full internet voice and up to two memes.
    Unhinged,
}

impl Tier {
    /// Whether this tier runs the agent at all.
    pub fn remixes(self) -> bool {
        self != Tier::Off
    }

    /// Intensity instruction handed to the slang agent. The words themselves
    /// come from the region's lexicon.
    pub fn style(self) -> &'static str {
        match self {
            Tier::Off => "Do not change the reply.",
            Tier::Light => {
                "Keep the reply mostly as written; swap in two or three casual words. Stay clear and readable."
            }
            Tier::Spicy => {
                "Rewrite in a casual, slangy voice. Keep it punchy and fun but every point must still land."
            }
            Tier::Unhinged => {
                "Go full group-chat: heavy slang, playful exaggeration, a bit of roasting if the user is roasting. \
                 Still never drop or change a fact, step, number, or link."
            }
        }
    }
}

/// Knobs for the rating policy.
#[derive(Clone, Copy, Debug)]
pub struct RatingPolicy {
    /// Seriousness probability at or above which memes are switched off.
    pub serious_cutoff: f64,
    /// Lowest rating that gets [`Tier::Light`].
    pub light_at: u8,
    /// Lowest rating that gets [`Tier::Spicy`].
    pub spicy_at: u8,
    /// Lowest rating that gets [`Tier::Unhinged`].
    pub unhinged_at: u8,
    /// Memes allowed at [`Tier::Spicy`].
    pub spicy_memes: usize,
    /// Memes allowed at [`Tier::Unhinged`].
    pub unhinged_memes: usize,
}

impl Default for RatingPolicy {
    fn default() -> Self {
        Self {
            serious_cutoff: 0.35,
            light_at: 3,
            spicy_at: 5,
            unhinged_at: 8,
            spicy_memes: 1,
            unhinged_memes: 2,
        }
    }
}

/// The verdict for one reply.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rating {
    /// 0 (keep it straight) to 10 (meme it up).
    pub score: u8,
    /// Remix intensity derived from the score.
    pub tier: Tier,
    /// Memes the agent may insert.
    pub max_memes: usize,
}

impl RatingPolicy {
    /// Rate a reading.
    ///
    /// Frankness carries 60% of the score and playfulness 40%. A serious chat
    /// zeroes it outright, and bad news is capped so a sad moment never gets
    /// more than a light touch.
    pub fn rate(&self, reading: &Reading) -> Rating {
        let score = if reading.serious >= self.serious_cutoff {
            0
        } else {
            let mut raw = reading.frankness * 6.0 + reading.playful * 4.0;
            raw += match reading.reply_intent {
                Intent::Banter | Intent::Celebration => 1.0,
                Intent::Grind | Intent::Confusion => -0.5,
                _ => 0.0,
            };
            let mut score = raw.round().clamp(0.0, 10.0) as u8;
            if reading.chat_intent == Intent::BadNews || reading.reply_intent == Intent::BadNews {
                score = score.min(self.light_at);
            }
            score
        };
        let tier = match score {
            s if s >= self.unhinged_at => Tier::Unhinged,
            s if s >= self.spicy_at => Tier::Spicy,
            s if s >= self.light_at => Tier::Light,
            _ => Tier::Off,
        };
        let max_memes = match tier {
            Tier::Unhinged => self.unhinged_memes,
            Tier::Spicy => self.spicy_memes,
            Tier::Light | Tier::Off => 0,
        };
        Rating {
            score,
            tier,
            max_memes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(frankness: f64, playful: f64, serious: f64, intent: Intent) -> Reading {
        Reading {
            chat_intent: intent,
            reply_intent: intent,
            reply_intent_confidence: 0.9,
            frankness,
            playful,
            serious,
        }
    }

    #[test]
    fn serious_chat_is_off_however_frank() {
        let r = RatingPolicy::default().rate(&reading(1.0, 1.0, 0.8, Intent::Banter));
        assert_eq!((r.score, r.tier, r.max_memes), (0, Tier::Off, 0));
    }

    #[test]
    fn frank_playful_banter_is_unhinged() {
        let r = RatingPolicy::default().rate(&reading(0.9, 0.9, 0.05, Intent::Banter));
        assert_eq!(r.tier, Tier::Unhinged);
        assert_eq!(r.max_memes, 2);
    }

    #[test]
    fn formal_chat_stays_off() {
        let r = RatingPolicy::default().rate(&reading(0.1, 0.2, 0.0, Intent::Grind));
        assert_eq!(r.tier, Tier::Off);
    }

    #[test]
    fn bad_news_is_capped_at_light() {
        let r = RatingPolicy::default().rate(&reading(1.0, 1.0, 0.2, Intent::BadNews));
        assert_eq!(r.tier, Tier::Light);
        assert_eq!(r.max_memes, 0);
    }

    #[test]
    fn middling_chat_is_spicy() {
        let r = RatingPolicy::default().rate(&reading(0.6, 0.5, 0.1, Intent::Curiosity));
        assert_eq!(r.tier, Tier::Spicy);
    }
}
