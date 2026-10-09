# TinyMemes

Makes an agent's final reply more fun when the chat can take it. The reply comes
back in the user's own language and register, with regional slang and, sometimes,
a reaction meme. A serious or formal chat is never touched.

Every reply goes through the same three steps. Two learners run afterwards, off
the reply's path:

```
conversation + reply
      │
      ▼
 ┌─────────┐  ONE Jev call: intents, frankness, playfulness, seriousness,
 │  Read   │  user's language, does the reply already match the user,
 │         │  which slang fits, which meme fits (with Jev's probability)
 └────┬────┘
      ▼
 ┌─────────┐  pure policy → 0–10 rating → Off / Light / Spicy / Unhinged
 │  Rate   │  serious → 0 · bad news ≤ Light · meme cooldown · weak meme pick → none
 └────┬────┘
      ▼ (not Off)
 ┌─────────┐  reply already matches the user → keep wording, attach meme (no LLM)
 │  Remix  │  otherwise → ONE rewrite call (language rule, slang menu, meme slot)
 └────┬────┘           → keep code/links/length, repair duplicated lines
      ▼
   reply out ─────────►  background: slang research · meme research + vetting
```

A remixed reply costs one Jev call plus at most one LLM call. In live tests, a
meme-only remix took about 0.6–0.9 s and a full rewrite about 4–5 s. A reply
rated Off costs only the Jev call (about 0.6 s).

## 1. Read: one Jev call

Jev ([`tinyinference-decisions`](https://github.com/tinyhumansai/tinyinference))
reads the conversation as the user saw it. Earlier remixed replies are tagged as
the bot's own style, and memes in them appear as `[meme: title]`. Jev answers
typed questions about the conversation and the reply. It never writes text.

| Question | Type | Used for |
| --- | --- | --- |
| `chat_intent`, `reply_intent` | Choice (celebration, frustration, confusion, banter, grind, bad news, curiosity, gratitude) | rating adjustments, meme moment |
| `frankness` | Score (formal … unfiltered) | rating |
| `playful` | Noul | rating |
| `serious` | Noul (grief, health, layoffs, …) | the safety gate |
| `user_language` | Choice (english, hinglish, devanagari, other) | language rule and slang menu |
| `reply_matches_user` | Noul | meme-only mode |
| `slang_best` / `slang_enough` | Choice / Noul over the slang index | rewrite hint, slang research trigger |
| `meme_best` | Choice over the meme catalog + `none_fit` | which meme, if any; meme research trigger |

Jev's frankness and playfulness questions are about the **user's** tone, so the
bot's own slang in earlier replies doesn't push the rating up.

## 2. Rate

`score = frankness × 6 + playfulness × 4`, then +1 for banter or celebration,
or −0.5 for grind or confusion.

| Tier | Rating | Voice | Memes |
| --- | --- | --- | --- |
| Off | 0–2 | untouched | 0 |
| Light | 3–4 | a few casual words | 0 |
| Spicy | 5–7 | slangy | 0 |
| Unhinged | 8–10 | full group-chat | 1 |

- **Serious gate:** `serious` ≥ 0.35 sets the score to 0, however jokey the user is.
- **Bad news** is capped at Light.
- **Meme cooldown:** no meme if one went out in the last 3 replies.
- **Weak pick:** if Jev's probability for its meme pick (`meme_p`) is under 0.5,
  it counts as `none_fit`. No poorly matched meme goes out.

## 3. Remix

### Meme-only mode

When Jev's `reply_matches_user` is at least 0.6, the agent's reply already
sounds like the user. It isn't rewritten: Jev's meme is attached after the first
paragraph, and no LLM call is made. This is the common case on playful chats
with a model that already mirrors tone.

### Rewrite

One LLM call. The rewrite sees **only**:
- the reply;
- the user's latest message, for language and script;
- the language rule from `user_language`: English stays English, Hinglish stays
  Hinglish, Devanagari stays Devanagari, any other language is mirrored;
- a **slang menu** that follows the language: the region's slang index for
  Hinglish and Devanagari, English internet slang for English, none otherwise.
  Each term comes with its meaning;
- Jev's best-fit term, and the slang used in recent replies (so it varies);
- at most one meme candidate (Jev's pick, with its meaning), placed as `[[meme:1]]`.

It never sees earlier messages, so it can't answer or blend them in.

After the call:
- **Protected content:** a rewrite that changes a fenced block, inline code or a
  URL, or more than doubles the length, is discarded and the original sent.
- **Duplicate repair:** a substantial line the rewrite repeats loses its extra
  copies. The copy mirroring the original line is kept.
- **Meme markers** resolve to `![title](url)`; unknown or extra ones are dropped.
  A meme is never forced in.

Gaali in a rewrite is not grounds for rejection. The prompt asks for none.

## 4. Learning in the background

Neither learner runs on the reply's path. Both persist as JSON snapshots.

### Slang research (`SlangIndex`)

- **Trigger:** Jev's `slang_best` is `none_fit`, or `slang_enough` < 0.5, on a
  remixed reply. Skipped for English and other-language writers.
- **Query:** about the reply itself ("Hinglish slang that would fit naturally in
  a casual reply to this message: …").
- **Vetting:** the source must be a page the search cited; the domain must not be
  adult or spam; the term must be a word or short phrase (not a sentence), at most
  3 per page, and clear the blocklist. Then one batched Jev check (p ≥ 0.6) that
  it's genuine, inoffensive regional slang with that meaning.
- **Ranking:** intent fit, corroboration across searches, and usage in replies.
  The top 18 for the tier are offered.
- **Limits:** the same query isn't repeated within 7 days; 50 searches per region
  per day.

### Meme research and vetting (`MemeIndex`)

- **Trigger:** a meme was allowed (Unhinged, cooldown clear) but Jev found nothing
  in the catalog that fits (`none_fit`, or a weak pick).
- **What to search for:** the **moment the user is in**. The chat model turns the
  user's message into a short, generic meme concept: a known catchphrase or trope
  like "Sharma ji ka beta" or "emotional damage", never names, places, numbers or
  other personal details. The search is `"<concept> Indian meme GIF site:giphy.com"`.
- **Reading GIF pages** (`GiphyPageResearcher`): search engines return pages, not
  media, so each GIPHY GIF page is read for what GIPHY itself publishes there:
  - the media link, switched to the small `200.webp` rendition, from allowed hosts
    only, and checked to be a real image under 1.5 MB;
  - GIPHY's own content rating;
  - the GIF's tags.

  No GIPHY API key is needed.
- **Vetting**, in order:
  1. **Rating:** GIPHY's rating must be G or PG. This is the image-level check:
     GIPHY moderates the media itself.
  2. **Topic filter:** slug, title and tags must not touch politics, religion, sex,
     violence or gaali. In a live test this caught a "Shocked India" GIF whose tags
     were `modi`, `vote chor`, `cjp protest`.
  3. **Meaning:** the chat model names each GIF and says when people post it, tied
     to the moment it was found for.
  4. **Jev**, p ≥ 0.75: a widely recognised meme from the region's internet culture
     with that meaning, and not political, religious, sexual, violent, mocking a
     community, or satire of a real politician, journalist or public figure.
- **Use:** there's no approval step; the filters are the gate. GIFs that pass
  join the catalog Jev picks from, minus memes sent in the
  last 6 replies. Usage is tracked.
- **Limits:** the same query isn't repeated within 3 days; 20 searches per region
  per day; concurrent steps for the same query collapse into one.

## Regions

`Region::india()` is the default. It sets **flavour, not language**:
- **Curated slang:** a Hinglish list with meanings, which seeds the slang index.
- **Meme catalog:** 13 hand-checked Imgflip templates, each with its meaning and
  intents (Hera Pheri, Sacred Games, Panchayat, Taarak Mehta, Moye Moye…).
- **Search phrasing** for slang and meme research.
- **Blocklists:** a gaali list, plus topics a learned GIF must not touch.

`Region::global()` is the plain English pack. Packs are plain data, so a host
can extend them.

## Guarantees

- **Fails open.** `MemeEngine::process` never errors. On any failure the original
  reply comes back with `Outcome::skipped` set.
- **Serious chats stay serious.** The `serious` question gates everything.
- **Code and links survive.** See [Rewrite](#rewrite).
- **No unvetted media.** Only the curated catalog, or learned GIFs that passed
  every check.

## Use

Every backend is a trait, so a host plugs in its own stack: `Evaluator` (Jev),
`ChatModel`, `MemeSource`, `SlangResearcher`, `WebSearch`, `MemeResearcher`.

```rust
let engine = MemeEngine::builder(jev, chat_model)
    .slang_index(slang_index)
    .meme_index(meme_index)
    .researcher(Arc::new(SearchResearcher::new(search.clone(), chat_model.clone())))
    .meme_researcher(Arc::new(GiphyPageResearcher::new(search, http)))
    .learn_inline(None)
    .build();

let out = engine.process(&conversation, &reply).await;   // never fails
if out.wants_more_memes() {
    // in the background
    engine.learn_memes(reading.reply_intent, &user_message).await?;
}
```

`Outcome` carries the reading, the rating, the remix (`mode`, `memes`,
`slang_used`, `duplicates_removed`) and what was learned.

### No Jev? The LLM answers Jev's questions

`LlmEvaluator` wraps any `ChatModel` and answers the same Choice, Score and Noul
questions as JSON, validated like a Jev response. The whole engine (reading, slang
and meme vetting) works with no Jev route.

### Configuration: host first, `TINYMEMES_*` env overrides

| Variable | Meaning |
| --- | --- |
| `TINYMEMES_OPENROUTER_KEY` | OpenRouter for the rewrite model and slang research (reasoning disabled) |
| `TINYMEMES_MODEL` | rewrite model (default `deepseek/deepseek-v4-flash`) |
| `TINYMEMES_JEV` | `auto`, `typesafe`, `openrouter`, `openjev`, or `llm` |
| `TINYMEMES_TYPESAFE_KEY` / `TYPESAFE_API_KEY` | TypeSafe System One |
| `TINYMEMES_OPENJEV_KEY` / `OPENJEV_API_KEY` | OpenJEV |

Inside a host, anything unset falls through to the host. Standalone,
`MemeEngine::from_env()` builds everything from these variables (and accepts
`OPENROUTER_API_KEY`):

```sh
OPENROUTER_API_KEY=… cargo run --example remix -- chat.json
TINYMEMES_JEV=llm OPENROUTER_API_KEY=… cargo run --example remix
```

Policy knobs live in `RatingPolicy` (tiers, serious cutoff, cooldown,
`meme_only_above`, `meme_min_p`), `IndexPolicy` (slang) and `MemeIndexPolicy`
(memes).

## In OpenHuman

OpenHuman vendors this repo at `vendor/tinymemes`, with a host adapter in
`crates/openhuman-core/src/tinymemes/`. It supplies:

- **Inference:** OpenHuman's provider for the `summarization` role (managed or
  BYOK), with reasoning off for tinymemes' calls only.
- **Jev:** the OpenHuman-managed route; OpenHuman's LLM answers its questions when
  that's unavailable.
- **Search:** OpenHuman's web search, for both slang and meme research.

| Variable | Effect |
| --- | --- |
| `OPENHUMAN_TINYMEMES` | `off` (default), `on`, `ab`, or `ab:NN`. Stable per-thread A/B buckets, read every turn |
| `OPENHUMAN_TINYMEMES_DEFAULT` | set during `cargo build` to bake a default into a release |
| `OPENHUMAN_TINYMEMES_TIMEOUT_MS` | remix budget (default 20000); the original is sent on timeout |
| `OPENHUMAN_TINYMEMES_MEME_COOLDOWN` | replies between memes (default 3) |

Treatment threads don't stream answer text, so users only ever see the remixed
reply. Each turn logs one line:

```
[tinymemes] turn arm=treatment score=10 tier=unhinged mode=meme_only memes=1
            meme_pick=Moye_Moye meme_p=0.79 dupes=0 turn_ms=… remix_ms=…
```

State lives under `<workspace>/tinymemes/` (`slang-index.json`,
`meme-index.json`, `remixed.json`). The Jev client is unified with OpenHuman's
vendored copy by one line in its root patch table:

```toml
[patch."https://github.com/tinyhumansai/tinyinference"]
tinyinference-decisions = { path = "vendor/tinyagents/vendor/tinyinference/crates/tinyinference-decisions" }
```

## Layout

| File | What it owns |
| --- | --- |
| `reading.rs` | the Jev request and its parsed `Reading` |
| `rating.rs` | `RatingPolicy`, tiers |
| `agent.rs` | meme-only vs rewrite, prompts, protected-content checks, duplicate repair, meme markers |
| `slang.rs`, `research.rs` | `SlangIndex`, slang researchers |
| `meme_index.rs`, `meme_research.rs` | `MemeIndex` (vetting), `GiphyPageResearcher` |
| `region.rs`, `intent.rs`, `conversation.rs` | region packs, intents, delivered history |
| `llm_eval.rs`, `env.rs`, `model.rs`, `source.rs` | LLM-as-Jev, env config, chat model, meme sources |
