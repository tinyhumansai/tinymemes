use super::*;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;
use tinyinference_decisions::{EvaluationResponse, NoulAnswer, Usage};

fn gif(slug: &str, rating: &str, tags: &[&str]) -> FoundMeme {
    FoundMeme {
        title: format!("{slug} GIF"),
        page_url: format!("https://giphy.com/gifs/{slug}-ID{}", slug.len()),
        media_url: format!("https://media1.giphy.com/media/x/{slug}/200.webp"),
        rating: Some(rating.to_owned()),
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        slug_words: slug.split('-').map(str::to_owned).collect(),
    }
}

struct Found(Vec<FoundMeme>, StdMutex<usize>);

#[async_trait]
impl MemeResearcher for Found {
    async fn research(&self, _query: &str) -> Result<Vec<FoundMeme>, BoxError> {
        *self.1.lock().unwrap() += 1;
        Ok(self.0.clone())
    }
}

/// Describes candidates in order: n=1.. with a name from the slug.
struct Describer;

#[async_trait]
impl ChatModel for Describer {
    async fn complete(&self, _system: &str, user: &str) -> Result<String, BoxError> {
        let n = user.lines().filter(|l| l.contains(". title: ")).count();
        let memes: Vec<String> = (1..=n)
            .map(|i| {
                let name = if user.contains("dodgy") && i == n { "Dodgy Meme" } else { "Chup Ho Ja Satvi" };
                format!(r#"{{"n": {i}, "name": "{name}", "meaning": "when someone fails", "intents": ["banter"]}}"#)
            })
            .collect();
        Ok(format!("{{\"memes\": [{}]}}", memes.join(",")))
    }
}

/// Verifies everything except names containing "Dodgy".
struct Jev;

#[async_trait]
impl Evaluator for Jev {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationResponse, BoxError> {
        let answers = request
            .questions
            .iter()
            .map(|(id, q)| {
                let p = if serde_json::to_string(q).unwrap().contains("Dodgy") {
                    0.2
                } else {
                    0.9
                };
                (id.clone(), Answer::Noul(NoulAnswer { noul: p }))
            })
            .collect::<BTreeMap<_, _>>();
        Ok(EvaluationResponse {
            model: "jev".into(),
            answers,
            usage: Usage::default(),
        })
    }
}

#[tokio::test]
async fn vetting_keeps_only_rated_on_topic_verified_new_gifs() {
    let region = Region::india();
    let index = MemeIndex::new(MemeIndexPolicy::default());
    let researcher = Found(
        vec![
            gif(
                "tmkoc-jethalal-chup-ho-ja-satvi-fail",
                "g",
                &["tmkoc", "jethalal"],
            ),
            gif("spicy-dance", "r", &["dance"]),
            // The real political GIF from the 2026-10-09 test: rated g, tags give it away.
            gif(
                "cjp-protest-janter-manter",
                "g",
                &["shocked", "india", "modi", "narendra modi"],
            ),
            gif("tmkoc-jethalal-chup-ho-ja-satvi-fail", "g", &["tmkoc"]),
            gif("dodgy-clip", "pg", &["funny"]),
        ],
        StdMutex::new(0),
    );
    let report = index
        .learn(
            &researcher,
            &Describer,
            (&Jev, false),
            &region,
            Intent::Banter,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.found, 5);
    assert_eq!(report.rejected_rating, 1);
    assert_eq!(report.rejected_topic, 1);
    assert_eq!(report.duplicates, 1);
    assert_eq!(report.failed_verification, 1);
    assert_eq!(report.approved, 1);

    let approved = index.approved("IN");
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].title, "Chup Ho Ja Satvi");
    assert!(approved[0].url.ends_with("/200.webp"));
    assert!(index.find_approved("IN", "Chup Ho Ja Satvi").is_some());

    // Same query again: skipped until the refresh window passes.
    let again = index
        .learn(
            &researcher,
            &Describer,
            (&Jev, false),
            &region,
            Intent::Banter,
        )
        .await
        .unwrap();
    assert!(again.is_none());
    assert_eq!(*researcher.1.lock().unwrap(), 1);
}

#[tokio::test]
async fn review_mode_holds_new_gifs_until_approved() {
    let region = Region::india();
    let index = MemeIndex::new(MemeIndexPolicy {
        auto_approve: false,
        ..MemeIndexPolicy::default()
    });
    let researcher = Found(
        vec![gif("tmkoc-jethalal-chup-ho-ja-satvi-fail", "g", &["tmkoc"])],
        StdMutex::new(0),
    );
    let report = index
        .learn(
            &researcher,
            &Describer,
            (&Jev, false),
            &region,
            Intent::Banter,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!((report.approved, report.pending), (0, 1));
    assert!(index.approved("IN").is_empty());
    assert_eq!(index.pending("IN").len(), 1);
    assert!(index.approve("IN", "Chup Ho Ja Satvi"));
    assert_eq!(index.approved("IN").len(), 1);
    assert!(index.remove("IN", "Chup Ho Ja Satvi"));
    assert!(index.is_empty("IN"));
}

#[test]
fn titles_stay_unique_against_the_curated_catalog() {
    let region = Region::india();
    assert_eq!(unique_title(&[], &region, "Moye Moye"), "Moye Moye (2)");
    assert_eq!(unique_title(&[], &region, "Brand New"), "Brand New");
}

#[test]
fn snapshot_round_trips() {
    let index = MemeIndex::new(MemeIndexPolicy::default());
    let restored = MemeIndex::from_json(&index.to_json(), MemeIndexPolicy::default()).unwrap();
    assert!(restored.is_empty("IN"));
}
