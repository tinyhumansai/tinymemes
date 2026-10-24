# Storage schema v2

`tinymemes-bus` exposes `tinymemes_bus::storage::collections()` as the module's
pure collection declaration. A host binds storage to the caller and grants the
module access to those collections through TinyBus. The module connects to
that bound storage during setup and runs its own migration before registering
the storage-backed services; the host does not compose a migration registry.

| v1 | v2 | Scope class | Change |
|---|---|---|---|
| `tinymemes_documents` (`slang`, `memes`, `remixed`; body `{"json": "<text>"}`) | `memes.state` (same ids; owner `memes`, version 2) | Tenant | the `json` text is parsed into the body |
| `remix_markers` (`remix-state`) | `memes.remix_markers` (`remix-state`) | Tenant | the document body is copied unchanged |

The module reads and writes one JSON text per `IndexKind` (`LoadIndex` /
`SaveIndex`) through the caller's bound document port. It stores those texts in
`memes.state`, one document per kind, as parsed text (v1 kept the text as a
string in `{"json": "<text>"}` in `tinymemes_documents`). The same mapping is
used by the reader and migration:

- `state_body(kind, text)`: an object is used as the body; the remixed ring (an
  array of ids) becomes `{"ids": [...]}` because a body is an object. Text
  with an unexpected shape is kept verbatim as `{"unparsed": "<text>"}` so
  migration does not destroy it. The module treats that marker as a fresh
  start, as before.
- `state_text(kind, body)`: the text returned by `LoadIndex` for a v2 body.
- `state_id(kind)`: `slang`, `memes`, or `remixed`.

Before upgrading, the host must stop the previous module instance so it cannot
write either legacy collection while the migration is running. On open, the
module declares the collections, acquires its owner migration lease, and
advances separate progress markers while applying `state_v2`
(`tinymemes_documents` to `memes.state`, version 1 to 2) and `markers_v1`
(`remix_markers` to `memes.remix_markers`). The state step preserves each
record's id, body meaning, and absolute expiry. The marker step preserves its
id, body, schema, and absolute expiry. Each destination is a new row, so the
host assigns its creation metadata. A source remains until the target has been
verified, making an interrupted copy safe to retry. Progress documents are
owner-scoped in `tsd.migrations`; the lease is stored in `tsd.leases`.

The current remix marker document uses the caller's private
`memes.remix_markers` collection declared by the bus contract. It keeps the
legacy snake_case, object-only body and requires no body transformation.
