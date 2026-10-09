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
