//! Region packs: the slang the agent may use, the memes people there actually
//! know (with what each one means), and the words it must never produce.
//!
//! A pack is plain data, so a host can extend or replace any part of it, for
//! example by appending trending terms it learned elsewhere.

use serde::{Deserialize, Serialize};

use crate::intent::Intent;
use crate::rating::Tier;
use crate::source::Meme;

/// One slang term with what it means, so the model uses it correctly.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SlangTerm {
    /// The term as written.
    pub term: String,
    /// Meaning and when to use it.
    pub meaning: String,
    /// Lowest tier at which the term may appear.
    pub min_tier: Tier,
}

/// A curated meme template with its meaning in that culture.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CatalogMeme {
    /// Template name, usually the catchphrase.
    pub title: String,
    /// Direct image URL.
    pub url: String,
    /// What the meme means and when people post it.
    pub meaning: String,
    /// Intents it fits.
    pub intents: Vec<Intent>,
}

impl CatalogMeme {
    pub(crate) fn to_meme(&self) -> Meme {
        Meme {
            title: self.title.clone(),
            url: self.url.clone(),
            source: "catalog".to_owned(),
            meaning: Some(self.meaning.clone()),
        }
    }
}

/// Everything region-specific.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Region {
    /// ISO 3166-1 alpha-2 code, e.g. `IN`.
    pub code: String,
    /// Display name, also shown to Jev.
    pub name: String,
    /// How the voice should sound here (language mixing, script).
    pub voice: String,
    /// Pop culture the meme planner should draw on.
    pub culture: String,
    /// Curated slang that seeds the [`crate::slang::SlangIndex`].
    pub slang: Vec<SlangTerm>,
    /// Web research query templates for growing the index. `{intent}` and
    /// `{year}` are filled in per bucket.
    pub slang_queries: Vec<String>,
    /// Memes searched before any external source.
    pub memes: Vec<CatalogMeme>,
    /// Words that must never appear in a rewrite (matched as whole words,
    /// case-insensitively). A rewrite containing one is discarded.
    pub blocklist: Vec<String>,
    /// GIPHY `lang` parameter.
    pub giphy_lang: Option<String>,
    /// Tenor `locale` parameter.
    pub tenor_locale: Option<String>,
}

impl Default for Region {
    fn default() -> Self {
        Self::india()
    }
}

fn term(term: &str, meaning: &str, min_tier: Tier) -> SlangTerm {
    SlangTerm {
        term: term.to_owned(),
        meaning: meaning.to_owned(),
        min_tier,
    }
}

fn meme(title: &str, url: &str, meaning: &str, intents: &[Intent]) -> CatalogMeme {
    CatalogMeme {
        title: title.to_owned(),
        url: url.to_owned(),
        meaning: meaning.to_owned(),
        intents: intents.to_vec(),
    }
}

/// Profanity common in English-language chat. Shared by every pack.
const ENGLISH_BLOCKLIST: &[&str] = &[
    "fuck", "fucking", "shit", "bitch", "cunt", "dick", "pussy", "retard", "retarded",
];

impl Region {
    /// India: Hinglish voice, Bollywood and OTT memes.
    pub fn india() -> Self {
        use Intent::*;
        use Tier::*;
        Self {
            code: "IN".to_owned(),
            name: "India".to_owned(),
            voice: "Write the way young Indians text: Hinglish (English mixed with Hindi words) in \
                    Latin script. If the user writes in Devanagari, reply in Devanagari; if they write \
                    plain English, keep it mostly English with a little Hinglish. Never translate \
                    technical terms, commands, or names."
                .to_owned(),
            culture: "Indian internet culture: Bollywood (Hera Pheri, Welcome, Gully Boy), OTT shows \
                      (Sacred Games, Panchayat, Mirzapur), TV (Taarak Mehta Ka Ooltah Chashmah), \
                      cricket, desi parents, chai, engineering and IT-job life."
                .to_owned(),
            slang: vec![
                term("yaar", "mate / dude; softens a sentence", Light),
                term("bhai", "bro; friendly address", Light),
                term("arre", "oh / hey; mild surprise", Light),
                term("ekdum", "totally, absolutely", Light),
                term("pakka", "for sure, guaranteed", Light),
                term("mast", "great, nice", Light),
                term("sorted", "handled, done", Light),
                term("jugaad", "a clever hack or workaround", Light),
                term("kya baat hai", "wow, well done", Light),
                term("full on", "completely, to the max", Light),
                term("scene kya hai", "what's the situation", Spicy),
                term("scene set hai", "everything is arranged", Spicy),
                term("lite le", "take it easy, don't stress", Spicy),
                term("chill maar", "relax", Spicy),
                term("bindaas", "carefree; go ahead without worry", Spicy),
                term("jhakaas", "awesome (Mumbai)", Spicy),
                term("ek number", "top-notch", Spicy),
                term("kadak", "strong, excellent (also strong chai)", Spicy),
                term("khatarnak", "literally dangerous; means insanely good", Spicy),
                term("bawaal", "epic chaos; something wild", Spicy),
                term("OP", "overpowered; amazing", Spicy),
                term("faltu", "useless, pointless", Spicy),
                term("ghanta", "sarcastic 'yeah right' / 'not happening'", Spicy),
                term("pro max", "the extreme version of something", Spicy),
                term("bas kar bhai", "stop it bro (playful, after something outrageous)", Unhinged),
                term("dimaag ka dahi", "my brain is curd; mentally fried", Unhinged),
                term("kat gaya", "got fooled or ripped off", Unhinged),
                term("moye moye", "when a plan falls apart (ironic sad)", Unhinged),
            ],
            slang_queries: [
                "Hinglish slang people use when {intent}",
                "Indian Gen Z slang for {intent} {year}",
                "Hindi meme phrases used for {intent}",
                "Mumbai and Delhi street slang for {intent}",
            ]
            .map(String::from)
            .to_vec(),
            memes: vec![
                meme("Apna Time Aayega", "https://i.imgflip.com/2ubf5u.jpg",
                     "Gully Boy: 'my time will come'; underdog hype after grinding", &[Celebration, Grind]),
                meme("Kabhi Kabhi Lagta Hai Apun Hi Bhagwan Hai", "https://i.imgflip.com/37ph4r.jpg",
                     "Sacred Games: 'sometimes I feel I'm god'; feeling unstoppable after pulling something off",
                     &[Celebration, Banter]),
                meme("Yeh Hamari Pawri Ho Rahi Hai", "https://i.imgflip.com/4ympde.jpg",
                     "viral 'this is our party' clip; good vibes, celebrating", &[Celebration, Banter]),
                meme("Utha Le Re Baba", "https://i.imgflip.com/86znte.png",
                     "Hera Pheri, Baburao: 'take me away, god'; exasperated, done with everything",
                     &[Frustration]),
                meme("Jethalal Thinking", "https://i.imgflip.com/57ergj.png",
                     "Taarak Mehta: Jethalal scheming; thinking hard, figuring something out",
                     &[Confusion, Curiosity, Grind]),
                meme("Majnu Bhai Painting", "https://i.imgflip.com/6zxkqi.jpg",
                     "Welcome: Majnu bhai's absurd painting; something chaotic presented proudly",
                     &[Confusion, Banter]),
                meme("Moye Moye", "https://i.imgflip.com/82yaur.png",
                     "posted when plans fall apart or something fails; ironic, light sadness",
                     &[BadNews, Frustration, Banter]),
                meme("Dekh Raha Hai Na Binod", "https://i.imgflip.com/6mj1dl.png",
                     "'you seeing this, Binod?'; pointing out how something sneaky works",
                     &[Banter, Curiosity]),
                meme("Kya Gunda Banega Re Tu", "https://i.imgflip.com/2v9onc.png",
                     "Phir Hera Pheri: 'you think you're a gangster?'; teasing someone acting tough",
                     &[Banter]),
                meme("Mast Plan Hai", "https://i.imgflip.com/40psld.jpg",
                     "Hera Pheri: 'great plan'; approving a plan, often ironically", &[Grind, Curiosity]),
                meme("Sab Moh Maya Hai", "https://i.imgflip.com/af9zk2.png",
                     "'it's all illusion'; philosophical shrug after a loss", &[BadNews, Frustration]),
                meme("Pain Dukh Dard Aansu", "https://i.imgflip.com/5dh6ff.jpg",
                     "four-panel mock suffering; dramatic over a small pain", &[BadNews, Frustration]),
                meme("Mast Thodi Der So Jata Hu", "https://i.imgflip.com/4c26e0.png",
                     "Hera Pheri, Baburao: 'I'll take a nice little nap'; work done, time to relax",
                     &[Celebration, Grind, Gratitude]),
            ],
            blocklist: ENGLISH_BLOCKLIST
                .iter()
                .chain(&[
                    "bc", "mc", "bkl", "bsdk", "chutiya", "chutiye", "madarchod", "behenchod", "bhenchod",
                    "bhosdike", "bhosdi", "gandu", "lund", "lauda", "randi", "harami", "kamina",
                ])
                .map(|w| (*w).to_owned())
                .collect(),
            giphy_lang: Some("hi".to_owned()),
            tenor_locale: Some("en_IN".to_owned()),
        }
    }

    /// Generic English internet voice, no curated catalog.
    pub fn global() -> Self {
        use Tier::*;
        Self {
            code: "XX".to_owned(),
            name: "Global".to_owned(),
            voice: "Write in casual English internet voice. Keep the user's language if it is not English."
                .to_owned(),
            culture: "widely known English-language reaction memes".to_owned(),
            slang: vec![
                term("ngl", "not gonna lie", Light),
                term("lowkey", "somewhat, quietly", Light),
                term("tbh", "to be honest", Light),
                term("fr", "for real", Spicy),
                term("no cap", "no lie, seriously", Spicy),
                term("bet", "okay, deal", Spicy),
                term("it's giving", "it has the vibe of", Spicy),
                term("W", "a win", Spicy),
                term("L", "a loss", Spicy),
                term("built different", "exceptionally capable", Unhinged),
                term("main character energy", "acting like the star of the story", Unhinged),
            ],
            slang_queries: [
                "internet slang people use when {intent} {year}",
                "Gen Z slang for {intent}",
            ]
            .map(String::from)
            .to_vec(),
            memes: Vec::new(),
            blocklist: ENGLISH_BLOCKLIST.iter().map(|w| (*w).to_owned()).collect(),
            giphy_lang: None,
            tenor_locale: None,
        }
    }

    /// Slang allowed at `tier`.
    pub fn slang_for(&self, tier: Tier) -> impl Iterator<Item = &SlangTerm> {
        self.slang.iter().filter(move |t| t.min_tier <= tier)
    }

    /// Catalog memes ranked for a query and intent.
    pub(crate) fn search_catalog(&self, query: &str, intent: Intent, limit: usize) -> Vec<Meme> {
        let words: Vec<String> = tokens(query).filter(|w| w.len() > 2).collect();
        let mut scored: Vec<(usize, &CatalogMeme)> = self
            .memes
            .iter()
            .filter_map(|m| {
                let text = format!("{} {}", m.title, m.meaning);
                let overlap = tokens(&text).filter(|t| words.contains(t)).count();
                let score = usize::from(m.intents.contains(&intent)) * 2 + overlap;
                (score > 0).then_some((score, m))
            })
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, m)| m.to_meme())
            .collect()
    }

    /// Blocklisted words present in `text`.
    pub fn blocked_words(&self, text: &str) -> Vec<String> {
        let words: Vec<String> = tokens(text).collect();
        self.blocklist
            .iter()
            .filter(|b| words.contains(&b.to_lowercase()))
            .cloned()
            .collect()
    }
}

pub(crate) fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn india_catalog_ranks_by_intent_then_words() {
        let r = Region::india();
        let hits = r.search_catalog("plan fell apart", Intent::BadNews, 3);
        assert_eq!(hits[0].title, "Moye Moye");
        assert!(
            hits.iter()
                .all(|m| m.meaning.is_some() && m.url.starts_with("https://i.imgflip.com/"))
        );
    }

    #[test]
    fn every_intent_has_an_india_meme() {
        let r = Region::india();
        for intent in Intent::ALL {
            assert!(
                r.memes.iter().any(|m| m.intents.contains(&intent)),
                "{intent} has no India meme"
            );
        }
    }

    #[test]
    fn blocklist_matches_whole_words_only() {
        let r = Region::india();
        assert_eq!(r.blocked_words("arre bc kya hua"), ["bc"]);
        assert!(r.blocked_words("abc mcq bcrypt").is_empty());
    }

    #[test]
    fn tier_filters_slang() {
        let r = Region::india();
        assert!(r.slang_for(Tier::Light).all(|t| t.min_tier == Tier::Light));
        assert!(r.slang_for(Tier::Unhinged).count() == r.slang.len());
    }
}
