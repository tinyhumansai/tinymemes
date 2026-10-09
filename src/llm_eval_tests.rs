use super::*;
use crate::conversation::{Turn, Window};
use crate::rating::Tier;
use crate::reading::{parse_reading, reading_request};
use crate::region::SlangTerm;

struct Fixed(&'static str);

#[async_trait]
impl ChatModel for Fixed {
    async fn complete(&self, _system: &str, user: &str) -> Result<String, BoxError> {
        assert!(user.contains("Questions:"));
        Ok(self.0.to_owned())
    }
}

fn slang() -> Vec<SlangTerm> {
    vec![
        SlangTerm {
            term: "yaar".into(),
            meaning: "mate".into(),
            min_tier: Tier::Light,
        },
        SlangTerm {
            term: "jugaad".into(),
            meaning: "hack".into(),
            min_tier: Tier::Light,
        },
    ]
}

#[tokio::test]
async fn llm_answers_drive_a_full_reading() {
    let reply = r#"Sure! ```json
    {"chat_intent": {"choice": "Celebration"}, "reply_intent": {"choice": "celebration"},
     "frankness": {"level": 3}, "playful": {"p": 0.8}, "serious": {"p": 0.05},
     "slang_best": {"choice": "jugaad"}, "slang_enough": {"p": 0.7}}
    ```"#;
    let eval = LlmEvaluator::new(Arc::new(Fixed(reply)));
    let request = reading_request(
        &[Turn::user("got the job!!")],
        "Congrats!",
        "India",
        &slang(),
        Window::default(),
        false,
    );
    let response = eval.evaluate(&request).await.unwrap();
    let reading = parse_reading(&response).unwrap();
    assert_eq!(reading.chat_intent.as_str(), "celebration");
    assert!((reading.frankness - 0.75).abs() < 1e-9);
    assert_eq!(reading.slang_best.as_deref(), Some("jugaad"));
    assert_eq!(reading.slang_enough, Some(0.7));
}

#[test]
fn invalid_answers_are_dropped_not_invented() {
    let request = reading_request(
        &[Turn::user("hi")],
        "hello",
        "India",
        &slang(),
        Window::default(),
        false,
    );
    let raw: BTreeMap<String, Value> = serde_json::from_value(json!({
        "chat_intent": {"choice": "party_time"},
        "frankness": {"level": 9},
        "playful": {"p": 1.7},
        "serious": {"p": "low"},
    }))
    .unwrap();
    let answers = decode(&request, &raw);
    assert!(!answers.contains_key("chat_intent"));
    // Out-of-range levels clamp to the scale's end.
    match answers.get("frankness") {
        Some(Answer::Score(s)) => assert_eq!(s.score, 4.0),
        other => panic!("{other:?}"),
    }
    assert!(!answers.contains_key("serious"));
    match answers.get("playful") {
        Some(Answer::Noul(n)) => assert_eq!(n.noul, 1.0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn tolerant_shapes_are_accepted() {
    let request = reading_request(
        &[Turn::user("hi")],
        "hello",
        "India",
        &slang(),
        Window::default(),
        false,
    );
    let raw: BTreeMap<String, Value> = serde_json::from_value(json!({
        "chat_intent": {"answer": "banter"},
        "frankness": {"score": "2"},
        "playful": 0.4,
        "serious": {"probability": "0.1"},
    }))
    .unwrap();
    let answers = decode(&request, &raw);
    assert!(matches!(answers.get("chat_intent"), Some(Answer::Choice(c)) if c.choice == "banter"));
    assert!(matches!(answers.get("frankness"), Some(Answer::Score(s)) if s.score == 2.0));
    assert!(matches!(answers.get("playful"), Some(Answer::Noul(n)) if n.noul == 0.4));
    assert!(matches!(answers.get("serious"), Some(Answer::Noul(n)) if (n.noul - 0.1).abs() < 1e-9));
}
