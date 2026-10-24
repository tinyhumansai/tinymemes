# Native catalog usage and removal

`ContentUsage1` (`RecordSlangUsage`, `RecordMemeUsage`, `RemoveMeme`) updates the
learned slang and meme catalog with bounded native version CAS. It is served by
`tinymemes-module` next to `ContentState`, `RemixMarkers1` and `Remix`, over the
storage ports the host binds to the module (TinyBus 0.1.6, TSD v2.1). The module
declares and prepares its own `memes.content` collection at startup like its other
collections. Wire types are in `tinymemes-bus` (`content_usage`), the confidential
`ContentUsageClient` is in `tinymemes-client` (feature `host-callback`), and the
implementation is `tinymemes-module/src/content_usage.rs`.

## Contract

Confidential ContentUsage1 provides RecordSlangUsage(region, text),
RecordMemeUsage(media_url), and RemoveMeme(region, title). Inputs contain only
business arguments. They cannot supply a stored snapshot, expected revision,
document selector, collection, destination, backend URL, or research grant.

Each operation requires an authenticated confidential sender and then uses the
storage handle the host bound to the module at startup (`tinystoragedrivers-busproxy`,
TSD v2.1), in the module-owned `memes.content` collection under the fixed
`content-state` key. Authority comes from the host-selected loaded-module
binding and its scoped storage, never the arguments.

The stored format1 envelope is the full slang and meme catalog snapshots,
including every record field and both research histories, with canonical decimal
business_revision. It lives in its own collection, separate from the remix markers: catalog updates never read,
write, copy, or claim ownership of remix markers. The format accepts the owning
serializer's canonical record shape. Decode followed by exact Value roundtrip
comparison rejects unknown nested data and default-normalized missing fields.
It is not a generic classic JSON importer or schema migration.

Existing duplicate catalog entries and their order are legal. Slang usage
increments each matching stored regional entry once using existing token rules.
Meme usage increments only the first exact URL match. Removal deletes every
exact region/title match. Verification scores and source metadata remain data;
no new research admission, deduplication, or validation policy is introduced.

## Implemented operation sequence

1. Decode the member's deny-unknown-fields request and retain actual sender.
2. Get content-state afresh through the module's bound document store. Missing alone
   yields the empty catalog; malformed/unsupported state refuses.
3. Validate format1, canonical decimal u64 revision, exact full record shape,
   and returned native document identity before invoking the owning reducer.
4. Supply that freshly read catalog and internal revision to the existing pure
   reducer. Caller data does not select a snapshot. Reducer overflow is returned
   without Put; unchanged catalog returns without Put or version increment.
5. Serialize the changed canonical catalog and condition Put on the native
   document Version (Absent only for genuinely absent state).
6. Retry only native Put Conflict, with at most eight fresh Get/apply/Put attempts.
   Storage refusal and outer transport errors exit once. Native Version and
   business revision are distinct.

Usage increments are not idempotent. A native-port unavailable acknowledgment
can follow a real committed increment. The module must not retry it; a caller
retry can count again. No rollback, exactly-once delivery, or outer lost-packet
proof is claimed.

## Regression sources

The tests live in `crates/tinymemes-module/tests/support/content_usage.rs` and run
through `compiled_remix.rs`, so they drive the real compiled, SHA-256-admitted
module with the TSD testkit memory backend. The excluded proof workspace includes
the same file. They cover complete metadata/history/duplicate order preservation,
untouched marker documents and sibling scopes, missing no-op, corrupt nested data,
counter and journal exhaustion, two operations racing on real native Version CAS,
an injected conflict that is retried, a non-conflict failure that is not, plain
caller refusal and unknown selectors. Pure wire tests are in
`tinymemes-bus/src/content_usage_tests.rs`.
