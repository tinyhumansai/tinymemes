# TinyMemes

Makes OpenHuman agent replies more fun, but only when the chat can take it.

```
conversation + reply
      │
      ▼
 ┌──────────┐   one Jev call: chat_intent, reply_intent (Choice),
 │  Read    │   frankness (Score), playful + serious (Noul),
 │          │   slang_best (Choice) + slang_enough (Noul) over indexed slang
 └────┬─────┘
      ▼
 ┌──────────┐   pure policy → 0–10 rating → Off / Light / Spicy / Unhinged
 │  Rate    │   serious ≥ 0.35 → 0; bad news capped at Light
 └────┬─────┘
      ▼ (not Off)
 ┌──────────┐   plan meme searches for the reply's intent → fetch from
 │  Remix   │   Imgflip / GIPHY / Tenor → rewrite in slang with [[meme:N]]
 └──────────┘   markers → resolve to ![title](url)
```

## Region: India by default

`Region::india()` is the default pack:

- **Voice:** Hinglish in Latin script, or Devanagari if the user writes in it.
  Technical terms stay untranslated.
- **Meme catalog:** 13 Imgflip templates (Hera Pheri, Sacred Games, Panchayat,
  Taarak Mehta, Moye Moye…). Each was checked by hand and each carries its
  *meaning*, so the model places it where the joke lands. Catalog hits rank
  ahead of Imgflip's global top 100. GIPHY gets `lang=hi`, and Tenor gets
  `locale=en_IN` and `country=IN`.
- **Blocklist:** English profanity and Hindi gaali. It keeps gaali out of the
  learned slang index and the learned meme catalog; rewrites are not rejected for it
  (the rewrite prompt asks for no gaali).

`Region::global()` is the plain English alternative. Packs are plain data, so a
host can extend them.

## Slang that grows: `SlangIndex`, searched when Jev says so

The slang offered to the rewrite comes from an in-memory index, not a fixed list.

1. The index is **seeded** from the region's curated terms.
2. **Jev decides when to search.** The same Jev call that reads the chat also
   sees the index's top 40 terms. It answers two questions about the reply
   being remixed:
   - `slang_best` (Choice): the term most specific to this reply's topic and
     moment, or `none_fit`. Generic fillers like "bhai" don't count.
   - `slang_enough` (Noul): does the index already have slang suited to this reply?

   A search runs only when Jev picks `none_fit` or `slang_enough` < 0.5. Its
   best pick is passed to the rewrite as a hint. As the index fills, Jev says
   "enough" more often, so searches taper off. Jev has no tool calling (it
   answers only Choice, Score and Noul questions), so this is how it makes the call.
3. **The search is about the reply itself** ("Hinglish slang that would fit
   naturally in a casual reply to this message: …"). The web model writes the
   actual searches. The same query isn't repeated within 7 days, and there's
   a cap of 50 searches per region per day.
4. **Vet** each candidate before it enters the index:
   - its source must be a page the web search actually cited
   - the source domain must not look adult or spammy
   - it must be a word or short phrase, not a sentence or variant ("Achha (questioning)")
   - at most 3 terms per source page per search
   - it must clear the blocklist
   - **one batched Jev call** must give it p ≥ 0.6 of being genuine, inoffensive
     regional slang with that meaning
5. **Rank** by intent fit, corroboration (found by several searches) and usage
   (used in replies). The top 18 that fit the tier are offered.
6. **Persist** with `to_json` / `from_json`. OpenHuman can store the snapshot in
   its own memory.

Research runs inline with a 25 s budget, and a failure or timeout just uses the
current index. Hosts can turn inline research off with `.learn_inline(None)`,
check `Reading::wants_more_slang`, and research in the background instead.
`engine.learn_slang(intent)` warms an intent from generic templates.
`Outcome::learned` reports what each step added, corroborated and rejected.
`OpenRouterWebResearcher` uses OpenRouter's web plugin and costs about $0.01 per
search.

| Tier | Rating | Rewrite | Memes |
| --- | --- | --- | --- |
| Off | 0–2 | none | 0 |
| Light | 3–4 | a few casual phrases | 0 |
| Spicy | 5–7 | slangy internet voice | 0 |
| Unhinged | 8–10 | full group-chat | 1 (none if one was sent in the last 3 replies) |

## Guarantees

- **Fails open.** `MemeEngine::process` never errors. If Jev, the model, or every
  meme source fails, the original reply comes back with `Outcome::skipped` set.
- **Code and links survive.** A rewrite that changes a fenced block, an inline
  code span, or a URL, or more than doubles the reply, is thrown away. The original
  wording is kept and the memes are still added.
- **Serious chats stay serious.** The `serious` Noul gates everything, however
  frank or jokey the user's tone is.

## Use

Every backend is a trait (`Evaluator` for Jev, `ChatModel`, `MemeSource`,
`SlangResearcher`, `WebSearch`), so a host plugs in its own stack:

```rust
let engine = MemeEngine::builder(jev, chat_model)
    .source(Arc::new(Imgflip::new(http.clone())))
    .slang_index(index)
    .researcher(Arc::new(SearchResearcher::new(host_search, chat_model.clone())))
    .build();
let out = engine.process(&conversation, &reply).await;   // never fails; falls back to `reply`
```

### No Jev? The LLM answers Jev's questions

`LlmEvaluator` wraps any `ChatModel` and answers the same Choice, Score and
Noul questions as JSON. It is validated like a Jev response: unknown options
are dropped and out-of-range levels are clamped. So the whole engine (reading,
slang fit, slang verification) works with no Jev route at all.

### Configuration: host first, `TINYMEMES_*` env overrides

| variable | meaning |
| --- | --- |
| `TINYMEMES_OPENROUTER_KEY` | OpenRouter for the rewrite model and web research |
| `TINYMEMES_MODEL` | rewrite model (default `deepseek/deepseek-v4-flash`, reasoning disabled) |
| `TINYMEMES_JEV` | `auto` (default), `typesafe`, `openrouter`, `openjev`, or `llm` |
| `TINYMEMES_TYPESAFE_KEY` / `TYPESAFE_API_KEY` | TypeSafe System One |
| `TINYMEMES_OPENJEV_KEY` / `OPENJEV_API_KEY` | OpenJEV |

Inside OpenHuman, anything unset falls through to OpenHuman's own settings.
Standalone, `MemeEngine::from_env()` builds the whole engine from these
variables (it also accepts `OPENROUTER_API_KEY`):

```sh
OPENROUTER_API_KEY=… cargo run --example remix
TINYMEMES_JEV=llm OPENROUTER_API_KEY=… cargo run --example remix -- chat.json
```

Optional extra meme sources: `GIPHY_API_KEY`, `TENOR_API_KEY`. Imgflip needs no key.

## Vendoring into OpenHuman

OpenHuman vendors this repo at `vendor/tinymemes`. Its host adapter
(`crates/openhuman-core/src/tinymemes/`) supplies:

- **Inference:** OpenHuman's configured provider for the `summarization` role
  (managed backend or BYOK), with reasoning switched off for tinymemes' calls
  only (the rest of OpenHuman keeps its configured reasoning).
- **Jev:** always the OpenHuman-managed Jev (TinyHumans backend). When it is
  unavailable (signed out, offline session), OpenHuman's LLM answers Jev's
  questions instead. Another Jev route only when `TINYMEMES_JEV` sets one.
- **Web search:** OpenHuman's search stack, through `SearchResearcher`.

It is gated by `OPENHUMAN_TINYMEMES=off|on|ab|ab:NN`. The Jev client is unified
with OpenHuman's vendored copy by one line in the root patch table:

```toml
[patch."https://github.com/tinyhumansai/tinyinference"]
tinyinference-decisions = { path = "vendor/tinyagents/vendor/tinyinference/crates/tinyinference-decisions" }
```
