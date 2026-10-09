//! One Jev call that reads the chat: intent, frankness, playfulness, and a
//! seriousness gate.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;
use tinyinference_decisions::{
    Answer, Choice, EvaluationRequest, EvaluationResponse, Noul, NoulCriteria, Question, Score,
};

use crate::conversation::{Turn, Window, jev_state};
use crate::error::{BoxError, Error, Result};
use crate::intent::Intent;

const CHAT_INTENT: &str = "chat_intent";
const REPLY_INTENT: &str = "reply_intent";
const FRANKNESS: &str = "frankness";
const PLAYFUL: &str = "playful";
const SERIOUS: &str = "serious";

/// Frankness levels, lowest first. Jev returns a probability-weighted position
/// along this scale.
const FRANKNESS_LEVELS: [&str; 5] = [
    "formal and guarded: polite, professional, no slang or jokes",
    "neutral and businesslike",
    "relaxed and conversational",
    "casual and blunt: slang, jokes, says exactly what they think",
    "unfiltered: very candid and irreverent; swearing, roasting, memes welcome",
];

/// Anything that can answer a Jev evaluation. Implemented for the real
/// [`tinyinference_decisions::Client`]; tests and hosts can supply their own.
#[async_trait]
pub trait Evaluator: Send + Sync {
    /// Evaluate one request.
    async fn evaluate(
        &self,
        request: &EvaluationRequest,
    ) -> std::result::Result<EvaluationResponse, BoxError>;
}

#[async_trait]
impl Evaluator for tinyinference_decisions::Client {
    async fn evaluate(
        &self,
        request: &EvaluationRequest,
    ) -> std::result::Result<EvaluationResponse, BoxError> {
        tinyinference_decisions::Client::evaluate(self, request)
            .await
            .map(|r| r.response)
            .map_err(Into::into)
    }
}

/// What Jev concluded about the chat.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reading {
    /// Intent of the whole conversation.
    pub chat_intent: Intent,
    /// Intent of the reply being remixed; memes are fetched for this one.
    pub reply_intent: Intent,
    /// How concentrated Jev's reply-intent distribution was (0..1).
    pub reply_intent_confidence: f64,
    /// Frankness of the chat, normalized to 0..1.
    pub frankness: f64,
    /// Probability the user would enjoy a joking, meme-filled reply.
    pub playful: f64,
    /// Probability the chat is about something serious or sensitive.
    pub serious: f64,
}

/// Build the Jev request for one chat and reply.
pub fn reading_request(
    conversation: &[Turn],
    reply: &str,
    region: &str,
    window: Window,
    openjev: bool,
) -> EvaluationRequest {
    let questions = BTreeMap::from([
        (
            CHAT_INTENT.to_owned(),
            Question::Choice(Choice {
                instructions: json!(
                    "What is the overall intent of the user across this conversation?"
                ),
                criteria: Intent::criteria(),
            }),
        ),
        (
            REPLY_INTENT.to_owned(),
            Question::Choice(Choice {
                instructions: json!(
                    "What is the intent of `assistant_reply`, the message about to be sent?"
                ),
                criteria: Intent::criteria(),
            }),
        ),
        (
            FRANKNESS.to_owned(),
            Question::Score(Score {
                instructions: json!(
                    "How frank and casual is the user's tone in this conversation?"
                ),
                criteria: FRANKNESS_LEVELS.iter().map(|l| json!(l)).collect(),
            }),
        ),
        (
            PLAYFUL.to_owned(),
            Question::Noul(Noul {
                instructions: json!(
                    "The user would enjoy a reply full of slang and memes right now."
                ),
                criteria: None,
            }),
        ),
        (
            SERIOUS.to_owned(),
            Question::Noul(Noul {
                instructions: json!(
                    "The conversation involves a serious or sensitive matter where jokes would be inappropriate."
                ),
                criteria: Some(NoulCriteria {
                    r#true: json!(
                        "grief, health, mental distress, safety, legal trouble, layoffs, large financial loss"
                    ),
                    r#false: json!("everyday work, tech, hobbies, banter, or minor annoyances"),
                }),
            }),
        ),
    ]);
    let state = jev_state(conversation, reply, region, window);
    if openjev {
        EvaluationRequest::openjev(state, questions)
    } else {
        EvaluationRequest::jev(state, questions)
    }
}

/// Decode Jev's answers into a [`Reading`].
pub fn parse_reading(response: &EvaluationResponse) -> Result<Reading> {
    let choice = |key: &'static str| match response.answers.get(key) {
        Some(Answer::Choice(c)) => Intent::from_label(&c.choice)
            .map(|i| (i, c.confidence))
            .ok_or(Error::MissingAnswer(key)),
        _ => Err(Error::MissingAnswer(key)),
    };
    let noul = |key: &'static str| match response.answers.get(key) {
        Some(Answer::Noul(n)) => Ok(n.noul.clamp(0.0, 1.0)),
        _ => Err(Error::MissingAnswer(key)),
    };
    let frankness = match response.answers.get(FRANKNESS) {
        Some(Answer::Score(s)) => (s.score / (FRANKNESS_LEVELS.len() - 1) as f64).clamp(0.0, 1.0),
        _ => return Err(Error::MissingAnswer(FRANKNESS)),
    };
    let (chat_intent, _) = choice(CHAT_INTENT)?;
    let (reply_intent, reply_intent_confidence) = choice(REPLY_INTENT)?;
    Ok(Reading {
        chat_intent,
        reply_intent,
        reply_intent_confidence,
        frankness,
        playful: noul(PLAYFUL)?,
        serious: noul(SERIOUS)?,
    })
}

/// Read a chat with Jev.
pub async fn read(
    jev: &dyn Evaluator,
    conversation: &[Turn],
    reply: &str,
    region: &str,
    window: Window,
    openjev: bool,
) -> Result<Reading> {
    let request = reading_request(conversation, reply, region, window, openjev);
    let response = jev.evaluate(&request).await.map_err(Error::Reading)?;
    parse_reading(&response)
}
