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
use crate::region::{CatalogMeme, SlangTerm};

const CHAT_INTENT: &str = "chat_intent";
const REPLY_INTENT: &str = "reply_intent";
const FRANKNESS: &str = "frankness";
const PLAYFUL: &str = "playful";
const SERIOUS: &str = "serious";
const SLANG_BEST: &str = "slang_best";
const SLANG_ENOUGH: &str = "slang_enough";
const REPLY_MATCHES: &str = "reply_matches_user";
const USER_LANGUAGE: &str = "user_language";
const MEME_BEST: &str = "meme_best";

/// Jev's meme decision for this reply.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemePick {
    /// No catalog was offered, or Jev did not answer.
    Unasked,
    /// Jev judged that no offered meme fits.
    NoneFit,
    /// The title of the catalog meme Jev picked.
    Pick(String),
}

/// The language and script the user writes in, as Jev judged it. The rewrite
/// mirrors it, and the slang offered follows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserLanguage {
    /// English only (internet slang and emoji count as English).
    English,
    /// An Indian language mixed with English, in Latin letters.
    Hinglish,
    /// Hindi in Devanagari script.
    Devanagari,
    /// Any other language.
    Other,
}

impl UserLanguage {
    const ALL: [UserLanguage; 4] = [
        UserLanguage::English,
        UserLanguage::Hinglish,
        UserLanguage::Devanagari,
        UserLanguage::Other,
    ];

    /// Wire label used as the Jev Choice option.
    pub fn as_str(self) -> &'static str {
        match self {
            UserLanguage::English => "english",
            UserLanguage::Hinglish => "hinglish",
            UserLanguage::Devanagari => "devanagari",
            UserLanguage::Other => "other",
        }
    }

    fn description(self) -> &'static str {
        match self {
            UserLanguage::English => {
                "English only; internet slang and emoji still count as English"
            }
            UserLanguage::Hinglish => {
                "Hindi or another Indian language mixed with English, written in Latin letters"
            }
            UserLanguage::Devanagari => "Hindi written in Devanagari script",
            UserLanguage::Other => "any other language",
        }
    }
}

/// Choice option meaning no indexed term fits the reply.
pub const NONE_FIT: &str = "none_fit";

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
    /// Jev's best-fitting indexed slang term for this reply, or `None` when
    /// it chose [`NONE_FIT`] or no terms were offered.
    pub slang_best: Option<String>,
    /// Probability the offered slang already has enough terms that fit this
    /// reply. `None` when no terms were offered.
    pub slang_enough: Option<f64>,
    /// The user's language and script. `None` when Jev did not answer.
    pub user_language: Option<UserLanguage>,
    /// Probability the agent's reply already sounds as casual and slangy as
    /// the user writes. High means a rewrite would add little.
    pub reply_matches_user: Option<f64>,
    /// Which catalog meme (if any) fits this reply.
    pub meme: MemePick,
}

impl Reading {
    /// Whether Jev thinks the index lacks slang for this reply, so a web
    /// search is worth running. `threshold` applies to [`Reading::slang_enough`].
    pub fn wants_more_slang(&self, threshold: f64) -> bool {
        // The index holds regional (Hinglish) slang; researching more of it
        // does nothing for a user who writes English or another language.
        if matches!(
            self.user_language,
            Some(UserLanguage::English | UserLanguage::Other)
        ) {
            return false;
        }
        match self.slang_enough {
            Some(enough) => enough < threshold || self.slang_best.is_none(),
            None => true,
        }
    }
}

/// Build the Jev request for one chat and reply.
pub fn reading_request(
    conversation: &[Turn],
    reply: &str,
    region: &str,
    slang: &[SlangTerm],
    memes: &[CatalogMeme],
    window: Window,
    openjev: bool,
) -> EvaluationRequest {
    let mut questions = BTreeMap::from([
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
    questions.insert(
        REPLY_MATCHES.to_owned(),
        Question::Noul(Noul {
            instructions: json!(
                "`assistant_reply` already sounds as casual, slangy, and playful as the user writes, \
                 in the user's own language, so restyling it would add little."
            ),
            criteria: None,
        }),
    );
    questions.insert(
        USER_LANGUAGE.to_owned(),
        Question::Choice(Choice {
            instructions: json!("In what language and script does the user write their messages?"),
            criteria: UserLanguage::ALL
                .into_iter()
                .map(|l| (l.as_str().to_owned(), Some(json!(l.description()))))
                .collect(),
        }),
    );
    if !memes.is_empty() {
        let mut criteria: BTreeMap<String, Option<serde_json::Value>> = memes
            .iter()
            .map(|m| (m.title.clone(), Some(json!(m.meaning))))
            .collect();
        criteria.insert(
            NONE_FIT.to_owned(),
            Some(json!("no meme here fits the moment of this reply")),
        );
        questions.insert(
            MEME_BEST.to_owned(),
            Question::Choice(Choice {
                instructions: json!(
                    "Which reaction meme, by its meaning, fits the moment of `assistant_reply` best? \
                     Choose none_fit unless one clearly matches."
                ),
                criteria,
            }),
        );
    }
    let mut state = jev_state(conversation, reply, region, window);
    if slang.len() >= 2 {
        let mut criteria: BTreeMap<String, Option<serde_json::Value>> = slang
            .iter()
            .map(|t| (t.term.clone(), Some(json!(t.meaning))))
            .collect();
        criteria.insert(
            NONE_FIT.to_owned(),
            Some(json!(
                "none of these terms would fit naturally in this reply"
            )),
        );
        questions.insert(
            SLANG_BEST.to_owned(),
            Question::Choice(Choice {
                instructions: json!(format!(
                    "Which slang term is most specific to the topic and moment of \
                     `assistant_reply` if it were rewritten in a casual {region} voice? Generic \
                     fillers that fit any message do not count; choose none_fit if nothing specific fits."
                )),
                criteria,
            }),
        );
        questions.insert(
            SLANG_ENOUGH.to_owned(),
            Question::Noul(Noul {
                instructions: json!(format!(
                    "`slang_candidates` already contains slang suited to the specific topic and \
                     moment of `assistant_reply` (not just generic fillers like bro or dude that fit \
                     any message), enough to make it sound current and hip for a {region} audience."
                )),
                criteria: None,
            }),
        );
        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "slang_candidates".to_owned(),
                json!(slang.iter().map(|t| &t.term).collect::<Vec<_>>()),
            );
        }
    }
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
        slang_best: match response.answers.get(SLANG_BEST) {
            Some(Answer::Choice(c)) if c.choice != NONE_FIT => Some(c.choice.clone()),
            _ => None,
        },
        slang_enough: noul(SLANG_ENOUGH).ok(),
        reply_matches_user: noul(REPLY_MATCHES).ok(),
        meme: match response.answers.get(MEME_BEST) {
            Some(Answer::Choice(c)) if c.choice == NONE_FIT => MemePick::NoneFit,
            Some(Answer::Choice(c)) => MemePick::Pick(c.choice.clone()),
            _ => MemePick::Unasked,
        },
        user_language: match response.answers.get(USER_LANGUAGE) {
            Some(Answer::Choice(c)) => UserLanguage::ALL
                .into_iter()
                .find(|l| l.as_str() == c.choice),
            _ => None,
        },
    })
}

/// Read a chat with Jev.
#[allow(clippy::too_many_arguments)]
pub async fn read(
    jev: &dyn Evaluator,
    conversation: &[Turn],
    reply: &str,
    region: &str,
    slang: &[SlangTerm],
    memes: &[CatalogMeme],
    window: Window,
    openjev: bool,
) -> Result<Reading> {
    let request = reading_request(conversation, reply, region, slang, memes, window, openjev);
    let response = jev.evaluate(&request).await.map_err(Error::Reading)?;
    parse_reading(&response)
}
