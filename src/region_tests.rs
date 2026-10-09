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

#[test]
fn only_gaali_the_rewrite_adds_is_flagged() {
    let r = Region::india();
    assert!(
        r.blocked_added("Bc stadium ka scene", "bc stadium ka scene yaar")
            .is_empty()
    );
    assert_eq!(
        r.blocked_added("stadium ka scene", "bc stadium ka scene"),
        ["bc"]
    );
    assert_eq!(r.blocked_added("bc once", "bc bc twice"), ["bc"]);
}
