# TinyMemes

Makes OpenHuman agent replies more fun, but only when the chat can take it.

```
conversation + reply
      │
      ▼
 ┌──────────┐   one Jev call: chat_intent, reply_intent (Choice),
 │  Read    │   frankness (Score), playful + serious (Noul)
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
- **Blocklist:** English profanity and Hindi gaali. A rewrite that uses one is
  discarded and the original wording is kept.

`Region::global()` is the plain English alternative. Packs are plain data, so a
host can extend them.

## Slang that grows: `SlangIndex`

The slang offered to the rewrite comes from an in-memory index, not a fixed list.

1. The index is **seeded** from the region's curated terms.
2. **Research, step by step.** While an intent has fewer than 6 web terms, each
   remixed reply with that intent runs the *next* unused query template (e.g.
   "Hinglish slang people use when celebrating a win"). Templates are re-run
   after 7 days.
3. **Vet** each candidate before it enters the index:
   - its source must be a page the web search actually cited
   - the source domain must not look adult or spammy
   - it must be a word or short phrase, not a sentence or variant ("Achha (questioning)")
   - at most 3 terms per source page per search
   - it must clear the blocklist
   - **one batched Jev call** must give it p ≥ 0.6 of being genuine, inoffensive
     regional slang with that meaning
4. **Rank** by intent fit, corroboration (found by several searches) and usage
   (used in replies). The top 18 that fit the tier are offered.
5. **Persist** with `to_json` / `from_json`. OpenHuman can store the snapshot in
   its own memory.

Research runs inline with a 25 s budget, and a failure or timeout just uses the
current index. Hosts can turn inline research off with `.learn_inline(None)` and
call `engine.learn_slang(intent)` in the background instead. `Outcome::learned`
reports what each step added, corroborated and rejected. `OpenRouterWebResearcher`
uses OpenRouter's web plugin and costs about $0.01 per search.

| Tier | Rating | Rewrite | Memes |
| --- | --- | --- | --- |
| Off | 0–2 | none | 0 |
| Light | 3–4 | a few casual phrases | 0 |
| Spicy | 5–7 | slangy internet voice | 1 |
| Unhinged | 8–10 | full group-chat | 2 |

## Guarantees

- **Fails open.** `MemeEngine::process` never errors. If Jev, the model, or every
  meme source fails, the original reply comes back with `Outcome::skipped` set.
- **Code and links survive.** A rewrite that changes a fenced block, an inline
  code span, or a URL, or more than doubles the reply, is thrown away. The original
  wording is kept and the memes are still added.
- **Serious chats stay serious.** The `serious` Noul gates everything, however
  frank or jokey the user's tone is.

## Use

```rust
let engine = MemeEngine::builder(Arc::new(jev_client), Arc::new(chat_model))
    .source(Arc::new(Imgflip::new(http.clone())))
    .build();
let out = engine.process(&conversation, &reply).await;
send(out.reply);
```

`Evaluator`, `ChatModel`, and `MemeSource` are traits, so OpenHuman can plug in
its own Jev client, provider, and catalogs. `tinyinference_decisions::Client`
implements `Evaluator` directly, and `OpenAiCompatible` covers OpenRouter.

Live run (one Jev call plus up to two model calls):

```sh
OPENROUTER_API_KEY=… cargo run --example remix
OPENROUTER_API_KEY=… cargo run --example remix -- chat.json
```

Optional: `TINYMEMES_MODEL` (default `google/gemini-2.5-flash`), `GIPHY_API_KEY`,
`TENOR_API_KEY`. Imgflip needs no key.

## Vendoring into OpenHuman

Add `vendor/tinymemes` as a submodule, depend on it by path, and point its Jev
dependency at the copy OpenHuman already vendors by adding one line to the
existing patch table:

```toml
[patch."https://github.com/tinyhumansai/tinyinference"]
tinyinference-decisions = { path = "vendor/tinyagents/vendor/tinyinference/crates/tinyinference-decisions" }
```
