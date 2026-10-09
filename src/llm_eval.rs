//! Answer Jev-shaped questions with an ordinary chat model.
//!
//! When no Jev route is available (no TinyHumans session, no TypeSafe or
//! OpenRouter key), the engine still needs the same typed answers: a choice
//! from a closed set, a position on a scale, a yes/no probability. This
//! evaluator renders the [`EvaluationRequest`] into one prompt, asks the
//! host's chat model for JSON, and validates the reply against the request
//! the same way a Jev response would be: unknown options, out-of-range levels,
//! and missing answers are dropped, so callers see a missing answer rather
//! than an invented one.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tinyinference_decisions::{
    Answer, ChoiceAnswer, EvaluationRequest, EvaluationResponse, NoulAnswer, Question, ScoreAnswer,
    Usage,
};

use crate::error::BoxError;
use crate::model::ChatModel;
use crate::reading::Evaluator;

/// Confidence reported for LLM answers. A chat model gives no distribution,
/// so this is a fixed, honest "moderate" rather than a measured concentration.
const LLM_CONFIDENCE: f64 = 0.6;

/// [`Evaluator`] backed by a chat model instead of Jev.
#[derive(Clone)]
pub struct LlmEvaluator {
    model: Arc<dyn ChatModel>,
}

impl std::fmt::Debug for LlmEvaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmEvaluator").finish_non_exhaustive()
    }
}

impl LlmEvaluator {
    /// An evaluator that asks `model`.
    pub fn new(model: Arc<dyn ChatModel>) -> Self {
        Self { model }
    }
}

const SYSTEM: &str = "You answer classification questions about the given state. \
Reply with JSON only: an object mapping each question id to its answer.\n\
- choice: {\"choice\": \"<one of the listed options, exactly as written>\"}\n\
- score: {\"level\": <0-based index of the best-fitting level>}\n\
- noul: {\"p\": <probability from 0 to 1 that the statement is true>}\n\
Answer every question. No prose.";

#[async_trait]
impl Evaluator for LlmEvaluator {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationResponse, BoxError> {
        let user = render(request);
        let text = self.model.complete(SYSTEM, &user).await?;
        let json = crate::agent::json_object(&text).ok_or("evaluator reply had no JSON object")?;
        let raw: BTreeMap<String, Value> = serde_json::from_str(json)?;
        Ok(EvaluationResponse {
            model: "llm-fallback".to_owned(),
            answers: decode(request, &raw),
            usage: Usage::default(),
        })
    }
}

fn render(request: &EvaluationRequest) -> String {
    let questions: BTreeMap<&str, Value> = request
        .questions
        .iter()
        .map(|(id, q)| {
            let rendered = match q {
                Question::Choice(c) => json!({
                    "type": "choice",
                    "question": c.instructions,
                    "options": c.criteria,
                }),
                Question::Score(s) => json!({
                    "type": "score",
                    "question": s.instructions,
                    "levels": s.criteria.iter().enumerate()
                        .map(|(i, l)| json!({ "level": i, "means": l }))
                        .collect::<Vec<_>>(),
                }),
                Question::Noul(n) => json!({
                    "type": "noul",
                    "statement": n.instructions,
                    "criteria": n.criteria,
                }),
            };
            (id.as_str(), rendered)
        })
        .collect();
    format!(
        "State:\n{}\n\nQuestions:\n{}",
        serde_json::to_string_pretty(&request.state).unwrap_or_default(),
        serde_json::to_string_pretty(&questions).unwrap_or_default(),
    )
}

/// Keep only answers that are valid for the request.
pub(crate) fn decode(
    request: &EvaluationRequest,
    raw: &BTreeMap<String, Value>,
) -> BTreeMap<String, Answer> {
    let mut answers = BTreeMap::new();
    for (id, question) in &request.questions {
        let Some(value) = raw.get(id) else { continue };
        let answer = match question {
            Question::Choice(c) => pick(value, &["choice", "answer", "option"])
                .and_then(|v| v.as_str().map(str::to_owned))
                .as_deref()
                .map(str::trim)
                .and_then(|picked| c.criteria.keys().find(|k| k.eq_ignore_ascii_case(picked)))
                .map(|picked| {
                    Answer::Choice(ChoiceAnswer {
                        choice: picked.clone(),
                        probabilities: BTreeMap::from([(picked.clone(), 1.0)]),
                        confidence: LLM_CONFIDENCE,
                    })
                }),
            Question::Score(s) => pick(value, &["level", "score", "index"])
                .and_then(number)
                // A level just past either end still says "extreme"; clamp it.
                .map(|l| l.clamp(0.0, (s.criteria.len() - 1) as f64))
                .map(|level| {
                    Answer::Score(ScoreAnswer {
                        score: level,
                        legend: BTreeMap::new(),
                        probabilities: BTreeMap::new(),
                        confidence: LLM_CONFIDENCE,
                    })
                }),
            Question::Noul(_) => pick(value, &["p", "probability", "noul"])
                .and_then(number)
                .map(|p| {
                    Answer::Noul(NoulAnswer {
                        noul: p.clamp(0.0, 1.0),
                    })
                }),
        };
        if let Some(answer) = answer {
            answers.insert(id.clone(), answer);
        }
    }
    answers
}

/// The first present key, or the value itself when the model answered with a
/// bare scalar instead of an object.
fn pick<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    if !value.is_object() {
        return Some(value);
    }
    keys.iter().find_map(|k| value.get(*k))
}

/// A finite number from a JSON number or numeric string.
fn number(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    n.is_finite().then_some(n)
}

#[cfg(test)]
#[path = "llm_eval_tests.rs"]
mod tests;
