use super::*;
use crate::host::Host;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use tinymemes::decisions::{
    Answer, ChoiceAnswer, EvaluationRequest, EvaluationResponse, NoulAnswer, ScoreAnswer, Usage,
};
use tinymemes::{BoxError, Intent, Meme, MemeSource, Region};
use tinymemes_bus::{
    CompleteRequest, HostFailure, HostResult, IndexKind, RemixOptions, SearchRequest, SearchResult,
    WireTurn,
};

#[derive(Clone, Copy, PartialEq)]
enum Eval {
    Fail,
    Hang,
    NoJev,
    /// Jev answers like a frank, playful chat. `slang_enough` and the meme pick
    /// decide which background research the reply earns.
    Script {
        slang_enough: f64,
        meme: &'static str,
    },
}

struct Mock {
    eval: Eval,
    calls: Mutex<Vec<&'static str>>,
    delivered: Mutex<Vec<DeliverRequest>>,
    docs: Mutex<HashMap<&'static str, String>>,
    states: Mutex<Vec<serde_json::Value>>,
    ended: Notify,
    hang_delivery: AtomicBool,
    rewrite: Mutex<Option<String>>,
    dropped: Arc<AtomicBool>,
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Mock {
    fn new(eval: Eval) -> Arc<Self> {
        Arc::new(Self {
            eval,
            calls: Mutex::new(Vec::new()),
            delivered: Mutex::new(Vec::new()),
            docs: Mutex::new(HashMap::new()),
            states: Mutex::new(Vec::new()),
            ended: Notify::new(),
            hang_delivery: AtomicBool::new(false),
            rewrite: Mutex::new(None),
            dropped: Arc::default(),
        })
    }
    fn note(&self, name: &'static str) {
        self.calls.lock().unwrap().push(name);
    }
    fn count(&self, name: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == name)
            .count()
    }
    fn doc(&self, kind: &'static str) -> Option<String> {
        self.docs.lock().unwrap().get(kind).cloned()
    }
}

fn kind_key(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Slang => "slang",
        IndexKind::Memes => "memes",
        IndexKind::Remixed => "remixed",
    }
}

fn script(request: &EvaluationRequest, slang_enough: f64, meme: &str) -> EvaluationResponse {
    let choice = |c: &str| {
        Answer::Choice(ChoiceAnswer {
            choice: c.to_owned(),
            probabilities: BTreeMap::new(),
            confidence: 0.9,
        })
    };
    let noul = |p: f64| Answer::Noul(NoulAnswer { noul: p });
    let mut answers = BTreeMap::from([
        ("chat_intent".to_owned(), choice("celebration")),
        ("reply_intent".to_owned(), choice("celebration")),
        (
            "frankness".to_owned(),
            Answer::Score(ScoreAnswer {
                score: 3.6,
                legend: BTreeMap::new(),
                probabilities: BTreeMap::new(),
                confidence: 0.8,
            }),
        ),
        ("playful".to_owned(), noul(0.9)),
        ("serious".to_owned(), noul(0.02)),
    ]);
    for id in request.questions.keys() {
        match id.as_str() {
            "slang_enough" => {
                answers.insert(id.clone(), noul(slang_enough));
            }
            "slang_best" => {
                answers.insert(id.clone(), choice("yaar"));
            }
            "meme_best" => {
                answers.insert(id.clone(), choice(meme));
            }
            _ => {}
        }
    }
    EvaluationResponse {
        model: "jev-latest".into(),
        answers,
        usage: Usage::default(),
    }
}

#[async_trait]
impl Host for Mock {
    async fn complete(&self, _: CompleteRequest) -> HostResult<String> {
        self.note("complete");
        match self.eval {
            Eval::Script { .. } if self.rewrite.lock().unwrap().is_some() => {
                Ok(self.rewrite.lock().unwrap().clone().unwrap())
            }
            Eval::Script { .. } => Ok(
                "W release fr 🎉\n[[meme:1]]\nTagged `v1.4.0` — notes at https://example.com/r"
                    .into(),
            ),
            _ => Err(HostFailure::Backend),
        }
    }
    async fn evaluate(&self, request: serde_json::Value) -> HostResult<serde_json::Value> {
        self.note("evaluate");
        if let Some(state) = request.get("state") {
            self.states.lock().unwrap().push(state.clone());
        }
        match self.eval {
            Eval::Fail => Err(HostFailure::Backend),
            Eval::NoJev => Err(HostFailure::Unsupported),
            Eval::Hang => {
                let _guard = DropFlag(self.dropped.clone());
                std::future::pending().await
            }
            Eval::Script { slang_enough, meme } => {
                let parsed: EvaluationRequest = serde_json::from_value(request).unwrap();
                Ok(serde_json::to_value(script(&parsed, slang_enough, meme)).unwrap())
            }
        }
    }
    async fn search(&self, _: SearchRequest) -> HostResult<Vec<SearchResult>> {
        self.note("search");
        Ok(Vec::new())
    }
    async fn load_index(&self, kind: IndexKind) -> HostResult<Option<String>> {
        self.note("load_index");
        Ok(self.docs.lock().unwrap().get(kind_key(kind)).cloned())
    }
    async fn save_index(&self, kind: IndexKind, json: String) -> HostResult<()> {
        self.note("save_index");
        self.docs.lock().unwrap().insert(kind_key(kind), json);
        Ok(())
    }
    async fn deliver(&self, request: DeliverRequest) -> HostResult<()> {
        self.note("deliver");
        if self.hang_delivery.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.delivered.lock().unwrap().push(request);
        Ok(())
    }
    async fn end(&self) -> HostResult<()> {
        self.note("end");
        self.ended.notify_waiters();
        self.ended.notify_one();
        Ok(())
    }
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
        _: Intent,
        _: &Region,
        _: usize,
    ) -> Result<Vec<Meme>, BoxError> {
        Ok(vec![Meme {
            title: format!("{query} meme"),
            url: format!("https://memes.example/{}.gif", query.replace(' ', "-")),
            source: "fixed".into(),
            meaning: None,
        }])
    }
}

fn deps(env: &[(&str, &str)]) -> Arc<Deps> {
    let env: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect();
    Arc::new(Deps {
        env: tinymemes::EnvConfig::from_lookup(|k| env.get(k).cloned()),
        source: Arc::new(FixedSource),
    })
}

async fn run(host: Arc<Mock>, request: RemixRequest) -> RemixResponse {
    run_with(host, Arc::new(Drain::default()), request, deps(&[])).await
}

fn request() -> RemixRequest {
    RemixRequest {
        conversation: vec![WireTurn {
            role: WireRole::User,
            text: "my build finally passed lol".into(),
            remixed: false,
            id: None,
        }],
        reply: "Congrats, the fix was the feature flag.".into(),
        region: Some("global".into()),
        enabled: true,
        timeout_ms: 5_000,
        deliver: false,
        correlation: "c-1".into(),
        reply_id: None,
        options: RemixOptions::default(),
    }
}

fn frank() -> Eval {
    Eval::Script {
        slang_enough: 0.9,
        meme: "celebration",
    }
}

#[tokio::test]
async fn disabled_returns_the_original_without_any_host_call() {
    let host = Mock::new(Eval::Fail);
    let mut req = request();
    req.enabled = false;
    let out = run(host.clone(), req.clone()).await;
    assert_eq!(out.reply, req.reply);
    assert!(!out.remixed);
    assert_eq!(out.skipped, Some(SkipReason::Disabled));
    assert!(host.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failing_evaluator_fails_open_with_the_original_reply() {
    let host = Mock::new(Eval::Fail);
    let req = request();
    let out = run(host.clone(), req.clone()).await;
    assert_eq!(out.reply, req.reply);
    assert!(!out.remixed);
    assert_eq!(out.skipped, Some(SkipReason::Failed));
    assert!(host.calls.lock().unwrap().contains(&"evaluate"));
    assert!(!host.calls.lock().unwrap().contains(&"save_index"));
    assert_eq!(host.count("end"), 1, "the host is told the request is over");
}

#[tokio::test]
async fn a_hung_callback_times_out_and_is_cancelled() {
    let host = Mock::new(Eval::Hang);
    let mut req = request();
    req.timeout_ms = 40;
    let out = run(host.clone(), req.clone()).await;
    assert_eq!(out.reply, req.reply);
    assert_eq!(out.skipped, Some(SkipReason::Timeout));
    assert!(
        host.dropped.load(Ordering::SeqCst),
        "the in-flight host callback must be dropped on timeout"
    );
}

#[tokio::test]
async fn dropping_the_call_cancels_the_in_flight_callback() {
    let host = Mock::new(Eval::Hang);
    let call = tokio::spawn(run(host.clone(), request()));
    while !host.calls.lock().unwrap().contains(&"evaluate") {
        tokio::task::yield_now().await;
    }
    call.abort();
    let _ = call.await;
    assert!(host.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn delivery_carries_the_final_reply_even_when_the_remix_failed() {
    let host = Mock::new(Eval::Fail);
    let mut req = request();
    req.deliver = true;
    let out = run(host.clone(), req.clone()).await;
    let delivered = host.delivered.lock().unwrap().clone();
    assert_eq!(
        delivered,
        vec![DeliverRequest {
            correlation: String::new(),
            reply: out.reply,
            remixed: false
        }]
    );
}

#[tokio::test]
async fn close_refuses_new_work_and_waits_for_in_flight_requests() {
    let drain = Arc::new(Drain::default());
    let permit = drain.admit().expect("open drain admits");
    let closing = tokio::spawn({
        let drain = drain.clone();
        async move { drain.close().await }
    });
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!closing.is_finished(), "close must wait for the permit");
    assert!(drain.admit().is_none(), "a closing drain refuses new work");
    drop(permit);
    tokio::time::timeout(Duration::from_secs(2), closing)
        .await
        .expect("close completes once drained")
        .unwrap();
}

#[tokio::test]
async fn an_idle_drain_closes_immediately_and_a_refused_admit_does_not_leak() {
    let drain = Arc::new(Drain::default());
    drain.close().await;
    assert!(drain.admit().is_none());
    drain.close().await;
}

#[tokio::test]
async fn bus_remix_reaches_the_callers_host_and_fails_open() {
    use tinybus::{Connection, broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
    let bus = MemoryBus::new();
    let broker = Broker::new();
    broker.clone().spawn(bus.clone());
    let _storage = crate::test_support::serve_storage(&bus, "tenant:remix-test")
        .await
        .unwrap();
    let host = ModuleHost::new(broker);
    host.attach_linked_with_config(
        crate::exports::linked_module().unwrap(),
        serde_json::Value::Null,
    )
    .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let client = tinymemes_client::RemixClient::new(&caller).unwrap();
    let mut req = request();
    req.enabled = false;
    let result = client.remix(req.clone()).await.unwrap();
    // Either the bus names the caller and the module answers, or it refuses an
    // unauthenticated peer; it never remixes without a host to call back.
    match result {
        Ok(out) => assert_eq!(out.reply, req.reply),
        Err(refusal) => assert_eq!(refusal, RemixRefusal::Unauthenticated),
    }
}

async fn ended(host: &Mock) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while host.count("end") == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the module reports the request finished");
}

#[tokio::test]
async fn a_host_without_jev_makes_the_chat_model_answer_jevs_questions() {
    let host = Mock::new(Eval::NoJev);
    let out = run(host.clone(), request()).await;
    // The model is the failing kind here, so the original goes out; the point
    // is the route: Jev was asked of the host, then the chat model took over.
    assert_eq!(out.reply, request().reply);
    let calls = host.calls.lock().unwrap().clone();
    let evaluate = calls.iter().position(|c| *c == "evaluate").unwrap();
    let complete = calls.iter().position(|c| *c == "complete").unwrap();
    assert!(evaluate < complete, "{calls:?}");
}

#[tokio::test]
async fn tinymemes_jev_llm_skips_the_hosts_jev_entirely() {
    let host = Mock::new(Eval::Fail);
    let out = run_with(
        host.clone(),
        Arc::new(Drain::default()),
        request(),
        deps(&[("TINYMEMES_JEV", "llm")]),
    )
    .await;
    assert_eq!(out.reply, request().reply);
    assert_eq!(host.count("evaluate"), 0);
    assert!(host.count("complete") >= 1);
}

#[tokio::test]
async fn a_remixed_reply_is_delivered_remembered_and_saved() {
    let host = Mock::new(frank());
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    req.reply_id = Some("run-1:reply".into());
    req.deliver = true;
    let out = run(host.clone(), req.clone()).await;
    assert!(out.remixed, "{out:?}");
    assert_ne!(out.reply, req.reply);
    assert_eq!(host.delivered.lock().unwrap().len(), 1);
    assert_eq!(host.doc("remixed").as_deref(), Some("[\"run-1:reply\"]"));
    assert!(host.doc("slang").is_some() && host.doc("memes").is_some());
    ended(&host).await;
}

#[tokio::test]
async fn an_earlier_remixed_turn_is_told_to_jev_as_the_bots_own_style() {
    let host = Mock::new(frank());
    host.docs
        .lock()
        .unwrap()
        .insert("remixed", "[\"m-9\"]".into());
    let mut req = request();
    req.conversation.insert(
        0,
        WireTurn {
            role: WireRole::Assistant,
            text: "W build fr".into(),
            remixed: false,
            id: Some("m-9".into()),
        },
    );
    req.conversation.insert(
        1,
        WireTurn {
            role: WireRole::Assistant,
            text: "plain".into(),
            remixed: false,
            id: Some("m-1".into()),
        },
    );
    run(host.clone(), req).await;
    let states = host.states.lock().unwrap().clone();
    let turns = states[0]["conversation"].as_array().unwrap().clone();
    assert!(turns[0].get("note").is_some(), "{turns:?}");
    assert!(turns[1].get("note").is_none(), "{turns:?}");
}

#[tokio::test]
async fn slang_research_runs_after_the_reply_and_the_host_is_told_when_done() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.1,
        meme: "celebration",
    });
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    let out = run(host.clone(), req).await;
    assert!(out.remixed, "{out:?}");
    ended(&host).await;
    assert!(
        host.count("search") >= 1,
        "slang research must search: {:?}",
        host.calls.lock().unwrap()
    );
    let calls = host.calls.lock().unwrap().clone();
    assert_eq!(
        calls.last(),
        Some(&"end"),
        "End comes after the research: {calls:?}"
    );
}

#[tokio::test]
async fn meme_research_runs_when_a_meme_was_allowed_but_none_fit() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.9,
        meme: "none_fit",
    });
    let mut req = request();
    // Only a region with a meme catalog asks Jev to pick from one.
    req.region = None;
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    let out = run(host.clone(), req).await;
    assert!(out.wants_more_memes, "{out:?}");
    ended(&host).await;
    assert!(
        host.count("search") >= 1,
        "meme research must search: {:?}",
        host.calls.lock().unwrap()
    );
}

#[tokio::test]
async fn options_can_switch_web_research_off() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.1,
        meme: "none_fit",
    });
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    req.options.research = false;
    run(host.clone(), req).await;
    ended(&host).await;
    assert_eq!(host.count("search"), 0);
}

#[tokio::test]
async fn a_closing_module_starts_no_background_research() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.1,
        meme: "none_fit",
    });
    let drain = Arc::new(Drain::default());
    drain.begin_close();
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    run_with(host.clone(), drain, req, deps(&[])).await;
    ended(&host).await;
    assert_eq!(host.count("search"), 0);
}

#[tokio::test]
async fn research_starts_only_after_the_reply_was_delivered() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.1,
        meme: "celebration",
    });
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    req.deliver = true;
    run(host.clone(), req).await;
    ended(&host).await;
    let calls = host.calls.lock().unwrap().clone();
    let delivered = calls.iter().position(|c| *c == "deliver").unwrap();
    let searched = calls.iter().position(|c| *c == "search").unwrap();
    assert!(delivered < searched, "{calls:?}");
}

#[tokio::test]
async fn a_cancelled_call_leaves_no_research_running() {
    let host = Mock::new(Eval::Script {
        slang_enough: 0.1,
        meme: "celebration",
    });
    host.hang_delivery.store(true, Ordering::SeqCst);
    let mut req = request();
    req.reply = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.".into();
    req.deliver = true;
    let call = tokio::spawn(run(host.clone(), req));
    while host.count("deliver") == 0 {
        tokio::task::yield_now().await;
    }
    call.abort();
    let _ = call.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        host.count("search"),
        0,
        "research must not outlive a cancelled call"
    );
    assert_eq!(host.count("end"), 0);
}

#[tokio::test]
async fn a_rewrite_that_only_adds_whitespace_is_not_a_remix() {
    let host = Mock::new(frank());
    let original = "Done. Tagged `v1.4.0`; release notes are at https://example.com/r.";
    // The engine trims what it returns, so a reply with trailing whitespace
    // comes back different from the request byte for byte, but not in content.
    *host.rewrite.lock().unwrap() = Some(original.into());
    let mut req = request();
    req.reply = format!("{original}\n\n");
    req.reply_id = Some("run-ws:reply".into());
    let req_reply = req.reply.clone();
    let out = run(host.clone(), req).await;
    assert!(!out.remixed, "{out:?}");
    assert_eq!(out.reply, req_reply, "the original comes back untouched");
    assert_eq!(
        host.doc("remixed"),
        None,
        "an unchanged reply is not remembered as remixed"
    );
}
