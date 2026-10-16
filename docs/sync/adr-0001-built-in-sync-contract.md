# ADR 0001 — Built-in sync contract (PR 00)

Status: **adopted for the PR 00 branch; implementation not merged**. 2026-10-17.

Canonical specification: [Tela — Akula PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11). This supersedes both the Tela PR 00–09 and the prior 11-PR numbering. The existing [PR #22](https://github.com/Lalalalendia/Akula/pull/22) is a **partial implementation of stage 00**; GitHub PR numbering and milestone labels are unrelated.

## Decision

- Ship sync as a **built-in module in `aquilum-core::sync`** with host/provider traits, not a runtime-loaded plugin or mandatory separate Cargo crate. Splitting the module into an `aquilum-sync` crate is a later refactor only if cyclic dependencies or independent reuse justify it.
- Keep the `sync-engine` feature **disabled by default**. In PR 00 it does not start any worker, expose a settings toggle, register providers, open a socket, alter notes, or run I/O. The public module contains only data models, contracts, validation, a deterministic fake and tests.
- `VaultPort`: only a trusted Core-host implementation may mutate the local vault. Every destructive operation requires a freshly verified local fingerprint **and** a durable `PreservationReceipt` bound to its path/hash/operation. The implementation must reject a dirty editor buffer in the final guarded critical section and preserve Core history/indexing; these are **future PR 01/04 implementation requirements**, not yet working code.
- `RemoteStore`: a single provider-neutral listing/read/head/conditional put/delete interface. A provider that lacks an atomic compare-and-swap **must return an explicit unsupported precondition**, not downgrade to last-write-wins. Capabilities describe provider guarantees. Remote revision/ETag is an opaque validator, never assumed to equal the content hash.
- Listing is only authoritative after all pages complete under the same proven snapshot. An error, lost permission, expired cursor, missing root or incomplete listing is **not evidence for remote deletion**. `FakeRemoteStore` proves the contract for CAS, complete pages, stale cursor and lost HTTP response.
- Local Markdown stays plain text. Optional encryption (stage 06) protects cloud bytes **and names/metadata**, with recovery key. An unencrypted readable cloud mirror and external edits belong to stage 10. At most one active remote provider per vault in v1.

## Safety boundaries deferred beyond PR 00

1. Path symlink validation is currently a **preflight only**. The final guarded write must stop symlink races / path escapes; no current method permits cloud downloads to write local files.
2. Current local `HistoryService` is mutable and not the immutable per-document DAG. DAG, planner and safe tombstones are stage 02, journal is stage 03.
3. `PreservationReceipt` is a **host contract**, not proof that existing implementations preserve bytes. Until the host bridge and failure tests pass, never enable a sync executor.
4. Provider-native CAS is not universal. The planner must differentiate safe immutable-object operations from unsafe mutation on weaker APIs and surface limits.
5. No actual cloud API integration, sign-in, encryption, UI, automatic deletion or garbage collection is enabled.

## PR 00 acceptance

- Typed stable IDs and protocol v1 with unknown critical fields failing closed.
- Portable path identity, case/NFC collision tests, validation for traversal/unsafe names, documented TOCTOU constraint.
- Non-secret profile structure, `VaultPort`, `RemoteStore`, provider capabilities.
- Deterministic fake: guarded create/replace/delete, stable paginated listing, lost response, changed cursor, incomplete listing.
- Default feature stays disabled, existing `aquilum-core` and native tests pass with `cargo test --workspace --locked`. Review and CI are required before merging.

**Next:** PR 01 fail-closed preservation; PR 02 pure reconciliation + minimal immutable revision DAG (can start after PR 00); PR 03 SQLite durable journal after the safety invariants, not before.
