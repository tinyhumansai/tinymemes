//! End-to-end engine behaviour over scripted Jev, model, and meme backends.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyinference_decisions::{
    Answer, ChoiceAnswer, EvaluationRequest, EvaluationResponse, NoulAnswer, ScoreAnswer, Usage,
};
use tinymemes::{
    BoxError, ChatModel, Evaluator, Intent, Meme, MemeEngine, MemeSource, Region, Tier, Turn,
};

struct ScriptedJev {
    frankness: f64,
    playful: f64,
    serious: f64,
    intent: &'static str,
    /// Answer to `slang_enough`; flips after each call when scripted.
    slang_enough: Mutex<Vec<f64>>,
    slang_best: &'static str,
    seen: Mutex<Option<EvaluationRequest>>,
}

#[async_trait]
impl Evaluator for ScriptedJev {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationResponse, BoxError> {
        *self.seen.lock().unwrap() = Some(request.clone());
        let choice = |c: &str| {
            Answer::Choice(ChoiceAnswer {
                choice: c.to_owned(),
                probabilities: BTreeMap::new(),
                confidence: 0.9,
            })
        };
        Ok(EvaluationResponse {
            model: "jev-latest".into(),
            answers: BTreeMap::from([
                ("chat_intent".into(), choice(self.intent)),
                ("reply_intent".into(), choice(self.intent)),
                (
                    "frankness".into(),
                    Answer::Score(ScoreAnswer {
                        score: self.frankness * 4.0,
                        legend: BTreeMap::new(),
                        probabilities: BTreeMap::new(),
                        confidence: 0.8,
                    }),
                ),
                (
                    "playful".into(),
                    Answer::Noul(NoulAnswer { noul: self.playful }),
                ),
                (
                    "serious".into(),
                    Answer::Noul(NoulAnswer { noul: self.serious }),
                ),
            ]),
            usage: Usage::default(),
        })
        .map(|mut resp: EvaluationResponse| {
            // Slang verification asks one Noul per candidate term.
            for (id, q) in &request.questions {
                if id.starts_with('t') && id[1..].chars().all(|c| c.is_ascii_digit()) {
                    let fake = serde_json::to_string(q).unwrap().contains("fake");
                    let p = if fake { 0.1 } else { 0.9 };
                    resp.answers
                        .insert(id.clone(), Answer::Noul(NoulAnswer { noul: p }));
                }
            }
            if request.questions.contains_key("slang_enough") {
                let mut script = self.slang_enough.lock().unwrap();
                let p = if script.len() > 1 {
                    script.remove(0)
                } else {
                    script[0]
                };
                resp.answers
                    .insert("slang_enough".into(), Answer::Noul(NoulAnswer { noul: p }));
                resp.answers
                    .insert("slang_best".into(), choice(self.slang_best));
            }
            resp
        })
    }
}

fn jev(frankness: f64, playful: f64, serious: f64, intent: &'static str) -> Arc<ScriptedJev> {
    jev_slang(frankness, playful, serious, intent, &[0.9], "yaar")
}

/// A Jev whose `slang_enough` answers follow `enough` in order (the last one repeats).
fn jev_slang(
    frankness: f64,
    playful: f64,
    serious: f64,
    intent: &'static str,
    enough: &[f64],
    best: &'static str,
) -> Arc<ScriptedJev> {
    Arc::new(ScriptedJev {
        frankness,
        playful,
        serious,
        intent,
        slang_enough: Mutex::new(enough.to_vec()),
        slang_best: best,
        seen: Mutex::new(None),
    })
}

/// Answers the plan call with JSON and the rewrite call with a fixed remix.
struct ScriptedModel {
    rewrite: String,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ChatModel for ScriptedModel {
    async fn complete(&self, system: &str, user: &str) -> Result<String, BoxError> {
        self.calls.lock().unwrap().push(user.to_owned());
        if system.contains("meme searches") {
            Ok(r#"sure! {"queries": ["lets go", "success kid"]}"#.into())
        } else {
            Ok(self.rewrite.clone())
        }
    }
}

fn model(rewrite: &str) -> Arc<ScriptedModel> {
    Arc::new(ScriptedModel {
        rewrite: rewrite.into(),
        calls: Mutex::new(Vec::new()),
    })
}

struct FixedSource;

#[async_trait]
impl MemeSource for FixedSource {
    fn name(&self) -> &'static str {
        "fixed"
    }
    async fn search(
        &self,
        query: &str,
        _intent: Intent,
        _region: &Region,
        _limit: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        Ok(vec![Meme {
            title: format!("{query} meme"),
            url: format!("https://memes.example/{}.gif", query.replace(' ', "-")),
            source: "fixed".into(),
            meaning: None,
        }])
    }
}

struct BrokenSource;

#[async_trait]
impl MemeSource for BrokenSource {
    fn name(&self) -> &'static str {
        "broken"
    }
    async fn search(
        &self,
        _: &str,
        _: Intent,
        _region: &Region,
        _: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        Err("down".into())
    }
}

fn chat() -> Vec<Turn> {
    vec![
        Turn::user("yo the deploy finally went green lmaooo"),
        Turn::assistant("Nice. Want me to tag the release?"),
        Turn::user("ya send it"),
    ]
}

#[tokio::test]
async fn frank_chat_gets_slang_and_memes() {
    let jev = jev(0.9, 0.9, 0.02, "celebration");
    let model =
        model("W release fr 🎉\n[[meme:2]]\nTagged `v1.4.0` — notes at https://example.com/r");
    let engine = MemeEngine::builder(jev.clone(), model.clone())
        .source(Arc::new(BrokenSource))
        .source(Arc::new(FixedSource))
        .region(Region::global())
        .build();

    let out = engine
        .process(
            &chat(),
            "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.",
        )
        .await;

    let rating = out.rating.unwrap();
    assert_eq!(rating.tier, Tier::Unhinged);
    assert!(out.skipped.is_none(), "{:?}", out.skipped);
    let remix = out.remix.unwrap();
    assert_eq!(remix.queries, ["lets go", "success kid"]);
    assert!(remix.rewrite_kept);
    assert_eq!(remix.memes.len(), 1);
    assert_eq!(remix.memes[0].url, "https://memes.example/success-kid.gif");
    assert!(
        out.reply
            .contains("![success kid meme](https://memes.example/success-kid.gif)")
    );
    assert!(out.reply.starts_with("W release fr"));

    // Jev saw the whole chat and the reply in its shared state.
    let req = jev.seen.lock().unwrap().clone().unwrap();
    assert_eq!(req.model, "jev-latest");
    assert_eq!(req.state["conversation"].as_array().unwrap().len(), 3);
    assert!(
        req.state["assistant_reply"]
            .as_str()
            .unwrap()
            .contains("v1.4.0")
    );
    // Five reading questions plus the two slang-fit questions.
    assert_eq!(req.questions.len(), 7);
}

#[tokio::test]
async fn serious_chat_is_left_alone_without_calling_the_model() {
    let model = model("should never be used");
    let engine = MemeEngine::builder(jev(0.9, 0.9, 0.9, "bad_news"), model.clone())
        .source(Arc::new(FixedSource))
        .region(Region::global())
        .build();
    let reply = "I'm really sorry about your dad. Take all the time you need.";
    let out = engine
        .process(&[Turn::user("my dad passed away last night")], reply)
        .await;
    assert_eq!(out.reply, reply);
    assert_eq!(out.rating.unwrap().tier, Tier::Off);
    assert!(out.skipped.is_some());
    assert!(model.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rewrite_that_mangles_code_keeps_original_wording_without_memes() {
    let engine = MemeEngine::builder(
        jev(0.9, 0.8, 0.0, "banter"),
        model("just run the tests bestie [[meme:1]]"),
    )
    .source(Arc::new(FixedSource))
    .region(Region::global())
    .build();
    let reply = "Run `cargo test --all` and you're set.";
    let out = engine.process(&chat(), reply).await;
    let remix = out.remix.unwrap();
    assert!(!remix.rewrite_kept);
    assert!(out.reply.starts_with(reply));
    // The discarded rewrite held the meme marker; no meme is forced in.
    assert!(remix.memes.is_empty());
}

#[tokio::test]
async fn jev_failure_fails_open() {
    struct Down;
    #[async_trait]
    impl Evaluator for Down {
        async fn evaluate(&self, _: &EvaluationRequest) -> Result<EvaluationResponse, BoxError> {
            Err("503".into())
        }
    }
    let engine = MemeEngine::builder(Arc::new(Down), model("x"))
        .region(Region::global())
        .build();
    let out = engine.process(&chat(), "original").await;
    assert_eq!(out.reply, "original");
    assert!(out.skipped.unwrap().contains("jev reading failed"));
}

#[tokio::test]
async fn light_tier_rewrites_without_fetching_memes() {
    let model = model("ngl that's done");
    let engine = MemeEngine::builder(jev(0.5, 0.2, 0.0, "grind"), model.clone())
        .source(Arc::new(FixedSource))
        .region(Region::global())
        .build();
    let out = engine.process(&chat(), "That's done.").await;
    assert_eq!(out.rating.unwrap().tier, Tier::Light);
    assert_eq!(out.reply, "ngl that's done");
    assert!(out.remix.unwrap().memes.is_empty());
    // Only the rewrite call, no plan call.
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

// ---- India region and the learning slang index -----------------------------

use tinymemes::slang::Discovered;
use tinymemes::{IndexPolicy, SlangIndex, SlangResearcher};

struct ScriptedResearcher {
    queries: Mutex<Vec<String>>,
}

#[async_trait]
impl SlangResearcher for ScriptedResearcher {
    async fn research(&self, query: &str) -> Result<Vec<Discovered>, BoxError> {
        let n = {
            let mut q = self.queries.lock().unwrap();
            q.push(query.to_owned());
            q.len()
        };
        Ok(vec![Discovered {
            term: format!("naya lingo {n}"),
            meaning: "fresh hype word".into(),
            min_tier: Tier::Spicy,
            source_url: format!("https://slang.example/{n}"),
            verified: None,
        }])
    }
}

/// Records the rewrite prompt so tests can see what the model was offered.
struct PromptSpy {
    rewrite: String,
    systems: Mutex<Vec<String>>,
    users: Mutex<Vec<String>>,
}

#[async_trait]
impl ChatModel for PromptSpy {
    async fn complete(&self, system: &str, user: &str) -> Result<String, BoxError> {
        self.systems.lock().unwrap().push(system.to_owned());
        self.users.lock().unwrap().push(user.to_owned());
        if system.contains("meme searches") {
            Ok(r#"{"queries": ["moye moye"]}"#.into())
        } else {
            Ok(self.rewrite.clone())
        }
    }
}

fn spy(rewrite: &str) -> Arc<PromptSpy> {
    Arc::new(PromptSpy {
        rewrite: rewrite.into(),
        systems: Mutex::new(Vec::new()),
        users: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn india_uses_catalog_memes_with_meanings_and_hinglish_slang() {
    let model = spy(
        "Arre yaar, moye moye ho gaya 😅\n[[meme:1]]\nBuild fail hua because `serde` missing tha.",
    );
    let engine_memes =
        MemeEngine::builder(jev(0.9, 0.9, 0.0, "frustration"), model.clone()).build();

    let out = engine_memes
        .process(
            &[Turn::user("bhai build phir se fail ho gaya 😭")],
            "The build failed because `serde` is missing.",
        )
        .await;
    let remix = out.remix.unwrap();
    assert!(remix.rewrite_kept);
    assert_eq!(remix.memes[0].source, "catalog");
    assert!(remix.memes[0].url.starts_with("https://i.imgflip.com/"));
    assert!(remix.slang_used.contains(&"yaar".to_owned()));
    assert!(remix.slang_used.contains(&"arre".to_owned()));

    let systems = model.systems.lock().unwrap();
    let rewrite_system = systems.iter().find(|s| s.contains("Hard rules")).unwrap();
    assert!(rewrite_system.contains("Hinglish"));
    assert!(rewrite_system.contains("- yaar:"));
    let users = model.users.lock().unwrap();
    let rewrite_user = users.iter().find(|u| u.contains("Original reply")).unwrap();
    // The model is told what each candidate meme means, and what the user wrote.
    assert!(
        rewrite_user.contains("(posted when plans fall apart"),
        "{rewrite_user}"
    );
    assert!(rewrite_user.contains("bhai build phir se fail"));
}

#[tokio::test]
async fn gaali_in_rewrite_is_rejected() {
    let engine =
        MemeEngine::builder(jev(0.9, 0.9, 0.0, "banter"), model("bc kya scene hai 😂")).build();
    let out = engine.process(&chat(), "What's going on?").await;
    let remix = out.remix.unwrap();
    assert!(!remix.rewrite_kept);
    assert!(out.reply.starts_with("What's going on?"));
}

#[tokio::test]
async fn jev_decides_when_to_search_and_searches_taper_off() {
    let index = Arc::new(SlangIndex::new(IndexPolicy::default()));
    let researcher = Arc::new(ScriptedResearcher {
        queries: Mutex::new(Vec::new()),
    });
    let model = spy("bawaal ho gaya, naya lingo 1");
    // Jev: short of slang twice, then satisfied.
    let jev = jev_slang(0.9, 0.9, 0.0, "celebration", &[0.2, 0.3, 0.9], "yaar");
    let engine = MemeEngine::builder(jev.clone(), model.clone())
        .slang_index(index.clone())
        .researcher(researcher.clone())
        .build();
    let before = index.len("IN");

    let first = engine.process(&chat(), "Shipped.").await;
    assert_eq!(first.learned.as_ref().unwrap().added, 1);
    // The freshly learned term is offered in the very same rewrite.
    assert!(
        first
            .remix
            .unwrap()
            .slang_offered
            .contains(&"naya lingo 1".to_owned())
    );

    let second = engine.process(&chat(), "Shipped again.").await;
    assert_eq!(second.learned.as_ref().unwrap().added, 1);

    // Jev now says the index has enough: no search.
    let third = engine.process(&chat(), "And again.").await;
    assert!(third.learned.is_none());

    let queries = researcher.queries.lock().unwrap();
    assert_eq!(queries.len(), 2);
    assert_ne!(queries[0], queries[1]);
    // Jev-triggered searches are about the reply itself.
    assert!(queries[0].contains("\"Shipped.\""));
    assert!(queries[1].contains("\"Shipped again.\""));
    assert_eq!(index.len("IN"), before + 2);

    // Jev was shown the index's terms and asked to pick one or `none_fit`.
    let req = jev.seen.lock().unwrap().clone().unwrap();
    assert!(req.questions.contains_key("slang_best"));
    assert!(req.state["slang_candidates"].as_array().unwrap().len() >= 2);
    let best = serde_json::to_string(&req.questions["slang_best"]).unwrap();
    assert!(best.contains("none_fit") && best.contains("jugaad"));

    // Persisted snapshot carries the learned terms and their usage.
    let restored = SlangIndex::from_json(&index.to_json(), IndexPolicy::default()).unwrap();
    assert_eq!(restored.len("IN"), before + 2);
}

#[tokio::test]
async fn none_fit_triggers_search_even_when_jev_says_enough() {
    let researcher = Arc::new(ScriptedResearcher {
        queries: Mutex::new(Vec::new()),
    });
    let engine = MemeEngine::builder(
        jev_slang(0.9, 0.9, 0.0, "banter", &[0.95], "none_fit"),
        model("lol"),
    )
    .researcher(researcher.clone())
    .build();
    let out = engine.process(&chat(), "Fair.").await;
    assert!(out.reading.unwrap().slang_best.is_none());
    assert_eq!(researcher.queries.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn best_fit_term_is_passed_to_the_rewrite() {
    let model = spy("arre yaar");
    let engine = MemeEngine::builder(
        jev_slang(0.9, 0.9, 0.0, "banter", &[0.9], "jugaad"),
        model.clone(),
    )
    .build();
    engine.process(&chat(), "Use a workaround.").await;
    let users = model.users.lock().unwrap();
    assert!(
        users
            .iter()
            .any(|u| u
                .contains("Best-fitting slang for this reply (judged by a classifier): jugaad"))
    );
}

#[tokio::test]
async fn failing_researcher_does_not_block_the_reply() {
    struct Down;
    #[async_trait]
    impl SlangResearcher for Down {
        async fn research(&self, _: &str) -> Result<Vec<Discovered>, BoxError> {
            Err("search down".into())
        }
    }
    let engine = MemeEngine::builder(jev(0.9, 0.9, 0.0, "banter"), model("lol sahi hai"))
        .researcher(Arc::new(Down))
        .build();
    let out = engine.process(&chat(), "Fair point.").await;
    assert!(out.learned.is_none());
    assert_eq!(out.reply.lines().next().unwrap(), "lol sahi hai");
}

#[tokio::test]
async fn jev_verification_drops_terms_it_does_not_believe() {
    struct Mixed;
    #[async_trait]
    impl SlangResearcher for Mixed {
        async fn research(&self, _: &str) -> Result<Vec<Discovered>, BoxError> {
            let d = |term: &str| Discovered {
                term: term.into(),
                meaning: "hype".into(),
                min_tier: Tier::Spicy,
                source_url: "https://slang.example/x".into(),
                verified: None,
            };
            Ok(vec![d("full bawaal"), d("fake slang")])
        }
    }
    let index = Arc::new(SlangIndex::new(IndexPolicy::default()));
    let engine = MemeEngine::builder(
        jev_slang(0.9, 0.9, 0.0, "celebration", &[0.1], "yaar"),
        model("bawaal"),
    )
    .slang_index(index.clone())
    .researcher(Arc::new(Mixed))
    .build();
    let out = engine.process(&chat(), "Shipped.").await;
    let learned = out.learned.unwrap();
    assert_eq!((learned.added, learned.failed_verification), (1, 1));
    assert!(index.to_json().contains("full bawaal"));
    assert!(!index.to_json().contains("fake slang"));
}

#[tokio::test]
async fn daily_search_budget_caps_spend() {
    let index = Arc::new(SlangIndex::new(IndexPolicy {
        max_searches_per_day: 2,
        ..IndexPolicy::default()
    }));
    let researcher = Arc::new(ScriptedResearcher {
        queries: Mutex::new(Vec::new()),
    });
    let engine = MemeEngine::builder(
        jev_slang(0.9, 0.9, 0.0, "banter", &[0.1], "none_fit"),
        model("lol"),
    )
    .slang_index(index)
    .researcher(researcher.clone())
    .build();
    for i in 0..4 {
        engine.process(&chat(), &format!("reply {i}")).await;
    }
    assert_eq!(researcher.queries.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn the_same_reply_is_not_researched_twice() {
    let researcher = Arc::new(ScriptedResearcher {
        queries: Mutex::new(Vec::new()),
    });
    let engine = MemeEngine::builder(
        jev_slang(0.9, 0.9, 0.0, "banter", &[0.1], "none_fit"),
        model("lol"),
    )
    .researcher(researcher.clone())
    .build();
    engine.process(&chat(), "same reply").await;
    engine.process(&chat(), "same reply").await;
    assert_eq!(researcher.queries.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delivered_history_varies_voice_and_skips_recent_memes() {
    let model = spy("arre bhai [[meme:1]]");
    let engine = MemeEngine::builder(jev(0.9, 0.9, 0.0, "frustration"), model.clone()).build();
    // The previous reply was delivered remixed, with the Moye Moye meme.
    let history = vec![
        Turn::user("build fail ho gaya"),
        Turn::remixed("Arre yaar, moye moye 😅\n\n![Moye Moye](https://i.imgflip.com/82yaur.png)"),
        Turn::user("phir se fail 😭"),
    ];
    let out = engine
        .process(&history, "It failed again on the same test.")
        .await;
    let remix = out.remix.unwrap();
    assert!(
        remix.memes.iter().all(|m| m.title != "Moye Moye"),
        "{:?}",
        remix.memes
    );

    let users = model.users.lock().unwrap();
    let rewrite = users.iter().find(|u| u.contains("Original reply")).unwrap();
    assert!(rewrite.contains("Your recent replies"));
    assert!(
        rewrite.contains("Arre yaar, moye moye 😅 [meme: Moye Moye]")
            || rewrite.contains("[meme: Moye Moye]")
    );
    assert!(!rewrite.contains("1. Moye Moye"));
}

#[tokio::test]
async fn meme_cooldown_skips_memes_after_a_recent_one() {
    let model = spy("arre bhai [[meme:1]]");
    let engine = MemeEngine::builder(jev(0.9, 0.9, 0.0, "banter"), model.clone()).build();
    let history = vec![
        Turn::user("lol"),
        Turn::remixed("haha\n\n![Some Meme](https://x.example/m.png)"),
        Turn::user("sahi hai"),
    ];
    let out = engine.process(&history, "Glad you liked it.").await;
    assert_eq!(out.rating.unwrap().max_memes, 0);
    assert!(out.remix.unwrap().memes.is_empty());
    assert!(!out.reply.contains("!["));
}
