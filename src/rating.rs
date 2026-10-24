//! Turn a Jev reading into a 0–10 meme rating. Pure policy: no I/O.

use serde::Serialize;

use crate::intent::Intent;
use crate::reading::Reading;

pub use tinymemes_bus::Tier;

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
    /// No meme when one was sent in this many most recent assistant replies.
    /// 0 disables the cooldown.
    pub meme_cooldown_turns: usize,
    /// When Jev's `reply_matches_user` is at least this, skip the rewrite and
    /// only attach a meme (the agent already sounds like the user).
    pub meme_only_above: f64,
    /// A meme Jev picks with less probability than this counts as `none_fit`:
    /// no meme is sent, and hosts may research a better one.
    pub meme_min_p: f64,
}

impl Default for RatingPolicy {
    fn default() -> Self {
        Self {
            serious_cutoff: 0.35,
            light_at: 3,
            spicy_at: 5,
            unhinged_at: 8,
            spicy_memes: 0,
            unhinged_memes: 1,
            meme_cooldown_turns: 3,
            meme_only_above: 0.6,
            meme_min_p: 0.5,
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
    /// The default policy with `meme_cooldown_turns` overridden by the numeric
    /// value of environment variable `var`, when it is set and parses.
    pub fn with_cooldown_from_env(var: &str) -> Self {
        let mut policy = Self::default();
        if let Some(turns) = std::env::var(var)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            policy.meme_cooldown_turns = turns;
        }
        policy
    }

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
#[path = "rating_tests.rs"]
mod tests;
