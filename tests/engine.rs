//! End-to-end engine behaviour over scripted Jev, model, and meme backends.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyinference_decisions::{
    Answer, ChoiceAnswer, EvaluationRequest, EvaluationResponse, NoulAnswer, ScoreAnswer, Usage,
};
use tinymemes::{BoxError, ChatModel, Evaluator, Intent, Meme, MemeEngine, MemeSource, Tier, Turn};

struct ScriptedJev {
    frankness: f64,
    playful: f64,
    serious: f64,
    intent: &'static str,
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
    }
}

fn jev(frankness: f64, playful: f64, serious: f64, intent: &'static str) -> Arc<ScriptedJev> {
    Arc::new(ScriptedJev {
        frankness,
        playful,
        serious,
        intent,
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
        _limit: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        Ok(vec![Meme {
            title: format!("{query} meme"),
            url: format!("https://memes.example/{}.gif", query.replace(' ', "-")),
            source: "fixed".into(),
        }])
    }
}

struct BrokenSource;

#[async_trait]
impl MemeSource for BrokenSource {
    fn name(&self) -> &'static str {
        "broken"
    }
    async fn search(&self, _: &str, _: Intent, _: usize) -> Result<Vec<Meme>, BoxError> {
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
    assert_eq!(req.questions.len(), 5);
}

#[tokio::test]
async fn serious_chat_is_left_alone_without_calling_the_model() {
    let model = model("should never be used");
    let engine = MemeEngine::builder(jev(0.9, 0.9, 0.9, "bad_news"), model.clone())
        .source(Arc::new(FixedSource))
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
async fn rewrite_that_mangles_code_keeps_original_wording_but_still_memes() {
    let engine = MemeEngine::builder(
        jev(0.9, 0.8, 0.0, "banter"),
        model("just run the tests bestie [[meme:1]]"),
    )
    .source(Arc::new(FixedSource))
    .build();
    let reply = "Run `cargo test --all` and you're set.";
    let out = engine.process(&chat(), reply).await;
    let remix = out.remix.unwrap();
    assert!(!remix.rewrite_kept);
    assert!(out.reply.starts_with(reply));
    assert_eq!(remix.memes.len(), 1);
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
    let engine = MemeEngine::builder(Arc::new(Down), model("x")).build();
    let out = engine.process(&chat(), "original").await;
    assert_eq!(out.reply, "original");
    assert!(out.skipped.unwrap().contains("jev reading failed"));
}

#[tokio::test]
async fn light_tier_rewrites_without_fetching_memes() {
    let model = model("ngl that's done");
    let engine = MemeEngine::builder(jev(0.5, 0.2, 0.0, "grind"), model.clone())
        .source(Arc::new(FixedSource))
        .build();
    let out = engine.process(&chat(), "That's done.").await;
    assert_eq!(out.rating.unwrap().tier, Tier::Light);
    assert_eq!(out.reply, "ngl that's done");
    assert!(out.remix.unwrap().memes.is_empty());
    // Only the rewrite call, no plan call.
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}
