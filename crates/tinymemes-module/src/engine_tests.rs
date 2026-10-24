use super::*;
use crate::host::Host;
use std::sync::Mutex;
use tinymemes::Turn;
use tinymemes::decisions::{Answer, ChoiceAnswer, Usage};
use tinymemes_bus::{DeliverRequest, HostResult, IndexKind, SearchResult};

/// A host whose `Evaluate` and `Complete` behaviour is scripted.
struct Scripted {
    evaluate: HostResult<serde_json::Value>,
    calls: Mutex<Vec<&'static str>>,
}

impl Scripted {
    fn new(evaluate: HostResult<serde_json::Value>) -> Arc<Self> {
        Arc::new(Self {
            evaluate,
            calls: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Host for Scripted {
    async fn complete(&self, _: CompleteRequest) -> HostResult<String> {
        self.calls.lock().unwrap().push("complete");
        Ok(r#"{"chat_intent": {"choice": "celebration"}, "reply_intent": {"choice": "celebration"},
            "frankness": {"level": 3}, "playful": {"p": 0.8}, "serious": {"p": 0.05}}"#
            .into())
    }
    async fn evaluate(&self, _: serde_json::Value) -> HostResult<serde_json::Value> {
        self.calls.lock().unwrap().push("evaluate");
        self.evaluate.clone()
    }
    async fn search(&self, _: SearchRequest) -> HostResult<Vec<SearchResult>> {
        Ok(Vec::new())
    }
    async fn load_index(&self, _: IndexKind) -> HostResult<Option<String>> {
        Ok(None)
    }
    async fn save_index(&self, _: IndexKind, _: String) -> HostResult<()> {
        Ok(())
    }
    async fn deliver(&self, _: DeliverRequest) -> HostResult<()> {
        Ok(())
    }
    async fn end(&self) -> HostResult<()> {
        Ok(())
    }
}

fn question_request() -> EvaluationRequest {
    tinymemes::reading::reading_request(
        &[Turn::user("got the job!!")],
        "Congrats!",
        "India",
        &[],
        &[],
        tinymemes::Window::default(),
        false,
    )
}

fn evaluator(host: &Arc<Scripted>) -> HostEvaluator {
    let shared: SharedHost = host.clone();
    let model: Arc<dyn ChatModel> = Arc::new(HostModel(shared.clone()));
    HostEvaluator::new(shared, model)
}

#[tokio::test]
async fn a_host_without_jev_is_answered_by_the_chat_model() {
    let host = Scripted::new(Err(HostFailure::Unsupported));
    let jev = evaluator(&host);
    let response = jev.evaluate(&question_request()).await.unwrap();
    assert_eq!(response.model, "llm-fallback");
    assert!(
        matches!(response.answers["chat_intent"], Answer::Choice(ChoiceAnswer { ref choice, .. }) if choice == "celebration")
    );
    // The host is asked once; afterwards the fallback is used directly.
    jev.evaluate(&question_request()).await.unwrap();
    let calls = host.calls.lock().unwrap().clone();
    assert_eq!(calls.iter().filter(|c| **c == "evaluate").count(), 1);
    assert_eq!(calls.iter().filter(|c| **c == "complete").count(), 2);
}

#[tokio::test]
async fn a_failing_host_jev_is_not_papered_over_by_the_chat_model() {
    let host = Scripted::new(Err(HostFailure::Backend));
    let error = evaluator(&host)
        .evaluate(&question_request())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("host evaluate failed"));
    assert!(!host.calls.lock().unwrap().contains(&"complete"));
}

#[tokio::test]
async fn the_host_jev_answer_is_used_when_it_has_one() {
    let answer = EvaluationResponse {
        model: "jev-latest".into(),
        answers: [(
            "chat_intent".to_owned(),
            Answer::Choice(ChoiceAnswer {
                choice: "celebration".into(),
                probabilities: Default::default(),
                confidence: 0.9,
            }),
        )]
        .into(),
        usage: Usage::default(),
    };
    let host = Scripted::new(Ok(serde_json::to_value(&answer).unwrap()));
    let response = evaluator(&host)
        .evaluate(&question_request())
        .await
        .unwrap();
    assert_eq!(response.model, "jev-latest");
    assert!(!host.calls.lock().unwrap().contains(&"complete"));
}
