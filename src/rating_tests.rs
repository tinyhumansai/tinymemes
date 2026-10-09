use super::*;

fn reading(frankness: f64, playful: f64, serious: f64, intent: Intent) -> Reading {
    Reading {
        chat_intent: intent,
        reply_intent: intent,
        reply_intent_confidence: 0.9,
        frankness,
        playful,
        serious,
        slang_best: None,
        slang_enough: None,
        user_language: None,
        meme: crate::reading::MemePick::Unasked,
        reply_matches_user: None,
        meme_p: None,
        meme_weak: None,
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
    assert_eq!(r.max_memes, 1);
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
