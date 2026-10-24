//! The remix operation: timeout, delivery, background research, drain, service.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tinymemes::{Role, Turn};
use tinymemes_bus::{
    DeliverRequest, REMIX_INTERFACE, RemixRefusal, RemixRequest, RemixResponse, RemixResult,
    SkipReason, WireRole, remix_methods,
};
use tokio::sync::Notify;

use crate::docs::Documents;
use crate::engine::{self, Deps};
use crate::host::{BusHost, SharedHost};

/// How long background research may run after the reply has gone out.
const RESEARCH_BUDGET: Duration = Duration::from_secs(120);

fn turns(request: &RemixRequest, remixed_ids: &std::collections::VecDeque<String>) -> Vec<Turn> {
    request
        .conversation
        .iter()
        .map(|t| {
            let role = match t.role {
                WireRole::User => Role::User,
                WireRole::Assistant => Role::Assistant,
            };
            let remembered = t.id.as_ref().is_some_and(|id| remixed_ids.contains(id));
            Turn {
                role,
                text: t.text.clone(),
                remixed: role == Role::Assistant && (t.remixed || remembered),
            }
        })
        .collect()
}

fn unchanged(reply: &str, reason: SkipReason) -> RemixResponse {
    RemixResponse {
        reply: reply.to_owned(),
        remixed: false,
        skipped: Some(reason),
        score: None,
        wants_more_memes: false,
    }
}

/// Run one remix against `host`. Never errors: every failure returns the
/// original reply with a content-free reason. Dropping the returned future
/// cancels any in-flight host callback. Research that the reply earned runs
/// after this returns, under `drain`, and ends with the host's `End` callback.
pub(crate) async fn run(
    host: SharedHost,
    drain: Arc<Drain>,
    request: RemixRequest,
) -> RemixResponse {
    run_with(host, drain, request, Arc::new(Deps::production())).await
}

/// [`run`] over explicit [`Deps`].
pub(crate) async fn run_with(
    host: SharedHost,
    drain: Arc<Drain>,
    request: RemixRequest,
    deps: Arc<Deps>,
) -> RemixResponse {
    if !request.enabled {
        return unchanged(&request.reply, SkipReason::Disabled);
    }
    let budget = Duration::from_millis(request.timeout_ms);
    let work = remix(host.clone(), &drain, &request, &deps);
    let (response, jobs) = match tokio::time::timeout(budget, work).await {
        Ok(done) => done,
        Err(_) => (unchanged(&request.reply, SkipReason::Timeout), Vec::new()),
    };
    if request.deliver {
        let delivery = DeliverRequest {
            correlation: String::new(),
            reply: response.reply.clone(),
            remixed: response.remixed,
        };
        // Delivery is the host's; its failure cannot change the reply.
        let _ = tokio::time::timeout(budget, host.deliver(delivery)).await;
    }
    // Research starts only now, after the reply has gone out.
    let background = Background::start(jobs);
    if background.is_empty() {
        // Nothing runs on: tell the host it can forget this request.
        let _ = tokio::time::timeout(budget, host.end()).await;
    } else {
        background.finish_with_end(host, drain);
    }
    response
}

/// Research the reply earned, held back until it has been delivered.
type Job = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Background research for one request, running after delivery. Dropping it
/// before [`Background::finish_with_end`] (the call was cancelled) aborts the
/// tasks, so nothing keeps calling back for a request nobody waits on.
#[derive(Default)]
struct Background {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Background {
    fn start(jobs: Vec<Job>) -> Self {
        Self {
            tasks: jobs.into_iter().map(tokio::spawn).collect(),
        }
    }
    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
    /// Hand the tasks to a coordinator that sends `End` once they are all done.
    fn finish_with_end(mut self, host: SharedHost, drain: Arc<Drain>) {
        let tasks = std::mem::take(&mut self.tasks);
        let Some(permit) = drain.admit() else {
            for task in &tasks {
                task.abort();
            }
            return;
        };
        tokio::spawn(async move {
            let _permit = permit;
            for task in tasks {
                let _ = task.await;
            }
            let _ = tokio::time::timeout(RESEARCH_BUDGET, host.end()).await;
        });
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn remix(
    host: SharedHost,
    drain: &Arc<Drain>,
    request: &RemixRequest,
    deps: &Deps,
) -> (RemixResponse, Vec<Job>) {
    let mut jobs: Vec<Job> = Vec::new();
    let docs = Arc::new(Documents::fetch(&host).await);
    let ids = docs.remixed_ids();
    let built = engine::build(
        &host,
        request,
        deps,
        tinymemes::IndexStore::load_slang(docs.as_ref()),
        tinymemes::IndexStore::load_memes(docs.as_ref()),
    );
    let engine = Arc::new(built.engine);
    let conversation = turns(request, &ids);
    let outcome = engine.process(&conversation, &request.reply).await;
    // The same criterion the host applies before it swaps the text: a rewrite
    // that differs only in surrounding whitespace is the original reply, so it
    // must neither be reported as remixed nor remembered as such.
    let remixed = outcome.remix.is_some() && outcome.reply.trim() != request.reply.trim();

    // Slang research, off the critical path, only for a reply that was
    // actually remixed and only when Jev judged the index short of slang.
    if let (Some(reading), true) = (&outcome.reading, remixed) {
        let policy = built.slang.policy();
        if reading.wants_more_slang(policy.search_below)
            && let Some(permit) = drain.admit()
        {
            let (engine, host, docs, slang) = (
                engine.clone(),
                host.clone(),
                docs.clone(),
                built.slang.clone(),
            );
            let (intent, reply, correlation) = (
                reading.reply_intent,
                request.reply.clone(),
                request.correlation.clone(),
            );
            jobs.push(Box::pin(async move {
                let _permit = permit;
                let research =
                    tokio::time::timeout(RESEARCH_BUDGET, engine.learn_for_reply(intent, &reply));
                match research.await {
                    Ok(Ok(Some(r))) => tracing::info!(
                        correlation = %correlation, added = r.added, corroborated = r.corroborated,
                        rejected = r.rejected, failed_jev = r.failed_verification,
                        "[tinymemes] slang research"
                    ),
                    Ok(Ok(None)) => {
                        tracing::debug!("[tinymemes] slang research skipped (repeat or budget)")
                    }
                    Ok(Err(e)) => tracing::warn!(error = %e, "[tinymemes] slang research failed"),
                    Err(_) => tracing::warn!("[tinymemes] slang research timed out"),
                }
                let _ = tinymemes::IndexStore::save_slang(docs.as_ref(), &slang);
                docs.flush(&host).await;
            }));
        }
    }

    // Meme research, off the critical path: a meme was allowed but nothing in
    // the catalog fit. New GIFs are vetted before Jev can pick them.
    if outcome.wants_more_memes()
        && let (Some(reading), Some(permit)) = (&outcome.reading, drain.admit())
    {
        let (engine, host, docs, memes) = (
            engine.clone(),
            host.clone(),
            docs.clone(),
            built.memes.clone(),
        );
        let intent = reading.reply_intent;
        let correlation = request.correlation.clone();
        // Only a short, generic meme concept derived from this is searched.
        let moment = request
            .conversation
            .iter()
            .rev()
            .find(|t| t.role == WireRole::User)
            .map(|t| t.text.clone())
            .unwrap_or_default();
        jobs.push(Box::pin(async move {
            let _permit = permit;
            let research =
                tokio::time::timeout(RESEARCH_BUDGET, engine.learn_memes(intent, &moment));
            match research.await {
                // The query is derived from the user's message, so only
                // counters are logged.
                Ok(Ok(Some(r))) => tracing::info!(
                    correlation = %correlation, found = r.found, added = r.added,
                    rejected_rating = r.rejected_rating, rejected_topic = r.rejected_topic,
                    duplicates = r.duplicates, failed_jev = r.failed_verification,
                    "[tinymemes] meme research"
                ),
                Ok(Ok(None)) => {
                    tracing::debug!("[tinymemes] meme research skipped (repeat, budget, or busy)")
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "[tinymemes] meme research failed"),
                Err(_) => tracing::warn!("[tinymemes] meme research timed out"),
            }
            let _ = tinymemes::IndexStore::save_memes(docs.as_ref(), &memes);
            docs.flush(&host).await;
        }));
    }

    if remixed || outcome.learned.is_some() {
        // Best effort: a lost save only loses usage counts.
        let _ = tinymemes::IndexStore::save_slang(docs.as_ref(), &built.slang);
        let _ = tinymemes::IndexStore::save_memes(docs.as_ref(), &built.memes);
        docs.flush(&host).await;
    }
    if remixed && let Some(id) = request.reply_id.clone() {
        Documents::mark_remixed(&host, id).await;
    }
    let skipped = match (&outcome.skipped, remixed) {
        (_, true) => None,
        (Some(_), _) if outcome.rating.is_none() => Some(SkipReason::Failed),
        _ => Some(SkipReason::BelowThreshold),
    };
    let response = RemixResponse {
        wants_more_memes: outcome.wants_more_memes(),
        score: outcome.rating.map(|r| r.score),
        reply: if remixed {
            outcome.reply
        } else {
            request.reply.clone()
        },
        remixed,
        skipped,
    };
    (response, jobs)
}

/// Counts in-flight remixes so a close can wait for them and refuse new work.
#[derive(Default)]
pub(crate) struct Drain {
    closing: AtomicBool,
    active: AtomicUsize,
    idle: Notify,
}

/// Released on drop, including when the call future is cancelled.
pub(crate) struct Permit(Arc<Drain>);
impl Drop for Permit {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

impl Drain {
    /// Admit one request, or refuse it when closing.
    pub(crate) fn admit(self: &Arc<Self>) -> Option<Permit> {
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closing.load(Ordering::SeqCst) {
            drop(Permit(self.clone()));
            return None;
        }
        Some(Permit(self.clone()))
    }
    /// Refuse new work without waiting.
    pub(crate) fn begin_close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }
    /// Refuse new work and wait until every admitted request has finished.
    /// The module ABI has no async close hook, so the service only calls
    /// [`Self::begin_close`] when it is dropped; this is the draining half.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn close(&self) {
        self.begin_close();
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) struct RemixService {
    pub(crate) connection: tinybus::Connection,
    pub(crate) drain: Arc<Drain>,
}

impl Drop for RemixService {
    fn drop(&mut self) {
        self.drain.begin_close();
    }
}

fn encode(result: RemixResult) -> tinybus::Result<serde_json::Value> {
    serde_json::to_value(result).map_err(|_| tinybus::Error::failed("invalid remix response"))
}

#[async_trait]
impl tinybus::Interface for RemixService {
    fn name(&self) -> tinybus::InterfaceName {
        REMIX_INTERFACE.try_into().unwrap()
    }
    fn members(&self) -> Vec<tinybus::MemberName> {
        tinymemes_bus::REMIX_METHODS
            .iter()
            .map(|m| (*m).try_into().unwrap())
            .collect()
    }
    async fn call(
        &self,
        _: &tinybus::MemberName,
        _: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        encode(Err(RemixRefusal::Unauthenticated))
    }
    async fn call_with_context(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
        context: &tinybus::CallContext,
    ) -> tinybus::Result<serde_json::Value> {
        if member.as_str() != remix_methods::REMIX {
            return Err(tinybus::Error::failed("unknown remix operation"));
        }
        let (request,): (RemixRequest,) = serde_json::from_value(args)
            .map_err(|_| tinybus::Error::failed("invalid remix request"))?;
        let Some(caller) = context.authenticated_sender().cloned() else {
            return encode(Err(RemixRefusal::Unauthenticated));
        };
        let Some(_permit) = self.drain.admit() else {
            return encode(Err(RemixRefusal::Closing));
        };
        let proxy = self.connection.proxy(
            caller.as_str(),
            tinymemes_bus::HOST_PATH,
            tinymemes_bus::HOST_INTERFACE,
        )?;
        let host: SharedHost = Arc::new(BusHost::new(proxy, request.correlation.clone()));
        encode(Ok(run(host, self.drain.clone(), request).await))
    }
}

#[cfg(test)]
#[path = "remix_tests.rs"]
mod tests;
