# Persistent remix markers: historical dependency draft

The dependency narrative below records PR6 and its actual original test graph.
PR6, TSD27, and TinyBus60 are now merged. The catalog follow-up removes the
historical replacement sections, pins canonical TinyBus40814, and uses the
explicitly unpublished TSD28 runtime-pin update pending its canonical merge.
Fresh shim-free metadata and marker18 passed; see
[native catalog implementation](native-content-cas-implementation.md).
Merged PR59 callback lifecycle behavior is now in the canonical dependency.
This is not a released module or OpenHuman reset/drain claim.

The current module implements bounded native CAS. The historical staging
sections below record failures and their disposition; they do not describe
the final implementation. Final public-only acceptance is recorded below.

The draft also depends on unmerged TinyBus PR60 at exact
`5256d08c41df5852ac1ccfcb58d4b24282b53cd7`. Both workspace manifests use a
temporary source/name/version-qualified Cargo `[replace]` selecting the same
public GitHub repository at that revision. Cargo rejects same-source `[patch]`;
replacement selectors cannot contain a revision query. Original TinyBus
declarations all remain pinned to canonical `1f255765`. Metadata resolves one
TinyBus identity at 5256 and the module SDK at the same revision; no local paths
or alternate Git URL are needed. Remove replacements and refresh exact pins
after PR60 and TSD27 are canonical before marking this draft ready.


This branch depends on **unmerged** TinyStorageDrivers PR 27 at
`7c5596f37e5d5440fbfa3a049a3ca98571885584`. It is not a released contract.
TinyBus is pinned to canonical `1f2557657db97470cd4efb7e1fc2f374f36fdd22`.
No TinyBus PR 59 lifecycle behavior is consumed or claimed.

The new operation-only marker contract carries one business `reply_id`.
It carries no snapshot, revision precondition, scope, URL, collection or
callback destination. An optional host client accepts only the canonical
opaque `HostCallbackProxy`; default bus features do not enable module loading.
The compiled module retains the original confidential authenticated sender
across its awaited fixed-collection document callback. Neither module config
nor caller strings select native authority.

## Deliberate baseline

The staged module authenticates and decodes the operation, then executes a
fresh native Get through the captured callback. Native refusal is returned as
a sanitized storage kind. Admitted reads deliberately return Unavailable:
there is no persistent CAS implementation yet, and no fake-map substitute.

All newly staged tests are **unexecuted**. No RED or GREEN is claimed.
The excluded proof runner loads the actual SDK-built module with its verified
hash and captures real Memory/SQLite document ports. Cases cover reopen/scope
behavior, ordinary/unbound caller and selector refusal, malformed native state,
stale native grants, and a victim binding A request to same-artifact instance B
on the same broker. The last case must fail callback admission for B rather
than inherit an unrelated proxy test as proof.

## Subsequent production boundary

After actual behavioral RED, the owner module will read and validate native
state, apply its existing owning content reducer, and CAS absent/native-version
writes. Only native Conflict may trigger bounded reread/retry. Content revision
and native document version remain distinct; duplicates require an authorized
fresh Get. Malformed state is refused, never replaced by defaults. A lost
acknowledgment is uncertain and does not imply rollback.

Deterministic competing-read fixtures interpose a two-reader barrier over real native documents, retaining native CAS and state rather than imitating persistence. This work does
not establish research verification, learning/import authority, session drain,
reset behavior, released module availability or host migration.

## First actual baseline

Cancel-aware run 53711 exited 101. The actual SDK module compiled, and all ten
proof cases executed: one unauthorized/selector control passed; eight behavior
cases failed on the deliberately returned Unavailable (two reopen/scope, three
competing-read CAS, one corrupt-state, two stale-grant mutation cases). Native
Get is allowed to read an immutable stale actor snapshot; its mutation must
still be refused once the operation is implemented. Those failures do not
prove the driver fences reads.

The remaining wrong-instance case failed during fixture setup, not at callback
admission: canonical TinyBus reserves the artifact manifest bus name in the
shared broker, so a second same-artifact live instance is rejected as "module
bus name is already owned", even with separate ModuleHost objects. The public
canonical loader exposes no instance-name override. This exact replay proof
remains blocked and unproved; no manifest spoofing or loader relaxation is
introduced here. The test is retained to identify the missing composition
capability. Production CAS implementation is pending the required authority
proof disposition.

Baseline log: `target/gates/remix-marker-red.log`. Earlier statements that all
staged tests were unexecuted describe the pre-run checkpoint only.

## Canonical instance-restriction disposition

The simultaneous instance requirement was replaced by explicit canonical
refusal and sequential replacement controls. Run 64658 exited 101: duplicate
load/no-effects and unauthorized/selector controls passed; the eight intended
Unavailable behavior failures remained. Sequential B setup failed with
"module initialization failed" after A.shutdown, so it was not authentication
proof. Run 49264 also exited 101 with the same disposition after normal
shutdown plus dropping A's ModuleHost while retaining its binding/registration.
The callback refusal assertion has still not executed for a new B instance.

SQLite fixtures now use an explicit `markers.db` file. Canonical
SqliteStorage::open chooses File mode by extension and otherwise chooses
Directory mode with storage.db, so the original directory use was supported,
not itself a setup failure. The cold reopen test scopes and drops the first
module and backend, then opens a fresh SqliteStorage from that same file.
Its cold-reopen assertions remain unexecuted until the marker operation exists.

The current module also requests the stateless ContentState global alias;
ModuleHostInner::drop itself revokes callback routes without explicit peer
detachment. Whether the stateless alias explains the observed replacement
initialization failure is still a hypothesis under investigation. No alias is
manually removed, no manifest is spoofed, and no loader override is added.

## Owning SDK startup finding

The subsequent canonical TinyBus source audit identified an earlier rejection:
`tinybus-module/src/lib.rs` at the native `start_module_mode` subscriber setup
calls `tracing::subscriber::set_global_default` for unlinked modules and returns
TB_CLOSED on failure. Its comment explicitly identifies the global state as
belonging to the mapped cdylib. A second initialization of that same mapped
artifact encounters the prior global subscriber before runtime handshake or
module setup. Linked/static tests bypass this native-only block.

This source-backed startup finding supersedes the alias hypothesis as the
replacement failure explanation. The TinyBus owner is preparing a genuine
compiled reload regression; no correction or canonical availability is claimed
here. TinyMemes setup and CAS stub remain unchanged. The same-artifact
replacement proof must be rerun after the owning correction is available, or
carry an explicit unpublished dependency in a draft; no stateless_alias routing
configuration workaround is introduced.

## Explicit unpublished runtime proof strategy

While the owning native reload correction is not canonical, a bounded local
proof may supply a generated Cargo patch configuration under this checkout's
`target/gates/`. It patches **both** tinybus and tinybus-module from the exact
same clean owning runtime worktree, avoiding duplicate connection types or a
mixed SDK/host contract. `scripts/test-remix-markers.sh --config <path>` passes
that override to root module build, metadata and excluded proof runner alike.
Record the exact runtime HEAD, clean status and patch file with each run.
Normal checked-in manifest pins remain canonical 1f; results with this override
are explicitly unpublished-dependency evidence, never canonical availability.
No PR 59 lifecycle correction is consumed by this bounded override. The runtime
source ancestry must be verified against canonical 1f at handoff. Generated
lockfile changes under local patching must be distinguished from published
canonical dependency metadata before any dependency draft publication.

## Implemented proof and final public graph

The corrected, authenticated 18-case stub baseline (76242) passed three
admission controls and failed fifteen behavior cases with Unavailable. There
were no setup failures. The reviewed CAS implementation ff82038 then passed
all eighteen using the explicitly unpublished local runtime override (61067).

Candidate ed7f60c replaced that development override with the public exact
Git revisions above. The locked actual compiled proof (7879) passed all
eighteen without `--config`, local paths or Git URL aliases. The root/module
and adapter resolve one TinyBus5256 identity; the proof runner has no SDK
dependency. b526207 removed its unnecessary SDK replacement entry.

Final public-graph acceptance 86701 passed default and all-features suites
(87 tests each, no failures/ignored), root and excluded-proof formatting,
strict Clippy, strict public documentation, both Rust1.88 builds, and the
classic compiled content-module example. This retains classic snapshot and
pure reducer behavior. Logs are under `target/gates/public-*.log`.

The executed marker cases include cold SQLite reopen, private-broker scope
isolation, canonical duplicate-name refusal, replacement B refusal through
retained A authority plus fresh B control, exact unauthenticated/plain refusal,
strict stored-envelope refusal, real native CAS races, stale-grant mutation
refusal, u64MAX no-op/exhaustion, eight-attempt conflict bounds, and non-conflict
no-retry. The lost acknowledgment fixture commits an actual native Put then
returns native-port Unavailable; it proves uncertain native-port outcome, not
a lost outer TinyBus acknowledgment. SQLite duplicate-contender parity was
not executed (SQLite distinct contenders were). No Mongo/cloud deployment,
learning/research, import, reset/drain, released module or OpenHuman migration
is established.

## Final coverage and publication checkpoint

At b526207, instrumented workspace87 and actual18 passed (77070), followed
by another actual18 under the owning dependency-coverage invocation (51434).
The real loaded `target/debug/libtinymemes_module.so` was included explicitly
with workspace test binaries and the proof runner in the LLVM export. Its
marker implementation has 47/50 mapped lines (94%) and 58/62 regions (93.55%).
The separately instrumented actual native runner measures the bus client at
19/20 mapped lines (95%), with all five mapped functions executed.

The first show-env capture reused uninstrumented client artifacts; its zero
client counters were not claimed as behavior coverage. The final dependency
report initially filtered out the path dependency: cargo-llvm-cov0.8.7
`--dep-coverage` currently admits crates.io paths only. The direct LLVM export
of that same instrumented runner/profile supplies the client result above.
No test or coverage threshold was disabled. The repository has no separate
coverage CI threshold; the production-only combined mapped summary is
2568/3038 lines (84.53%), across 22 owner files.
This is not an all-files90 claim or complete async branch coverage: async-trait
generated futures provide limited source mappings, while the actual native
CAS/admission/error controls remain independently asserted by the18proofs.

Exact local evidence under `target/gates/`: public-acceptance.log and its
individual gate logs; public-coverage-final-units.log (87);
public-coverage-final-actual.log (18); public-coverage-final-export.json;
public-coverage-objects.json (including actual .so); public-bus-coverage.log
(18); public-bus-native-export.json; public-own-coverage-summary.json.
All commands used the public pinned graph without local Cargo overrides.
The production source has not changed since reviewed ff82038; subsequent
changes are fixture lint compatibility, public graph/CI, and evidence docs.

## Hosted review portability correction

Initial hosted Actions run38240883480 passed all gates, including the18
native markers. CodeRabbit then identified Bash3.2 nounset empty-array
expansion in the shell proof runner. Official bash:3.2 (image digest
0fd7cb8499c63a3c9345e7088a9cd83bb69f6e895e83833859aff838a0312091)
reproduced the unbound-variable refusal before Cargo. A mocked runner
regression preserves all three Cargo command arguments and artifact path:
corrected old-script baseline1PASS/1FAIL, then direct quoted positional
argument forwarding2PASS on Bash3.2 and current Bash. The first assertion
fixture omitted the existing debug path component; it was corrected and
never counted as product evidence. CI runs the two stdlib regressions without
requiring Docker; Bash3.2 verification is explicit local evidence. Rust CAS,
dependency graph and native admission code did not change.
