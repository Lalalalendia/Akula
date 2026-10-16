# Akula sync implementation roadmap

**Status (2026-10-17):** staged implementation. **Merged:** stage 00 ([PR #22](https://github.com/Lalalalendia/Akula/pull/22)), stage 01a ([PR #29](https://github.com/Lalalalendia/Akula/pull/29)), stage 02a ([PR #30](https://github.com/Lalalalendia/Akula/pull/30)). **Draft / CI:** stage 01b ([PR #31](https://github.com/Lalalalendia/Akula/pull/31)). No working cloud sync, no native Sync UI. Feature must remain built into Akula, NOT a plugin marketplace.

**Full architecture + diagrams (Tela):** [Sync architecture](https://telawiki.com/spaces/456/pages/7463), [immutable per-document revision RFC](https://telawiki.com/spaces/456/pages/7465/git-rfc), [canonical PR 00–11 plan](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11) and [historical product/crypto plan](https://telawiki.com/spaces/456/pages/7469/11-pr). [Notion research](https://app.notion.com/p/3fb32a84beec81c1b561db8ad0b19df6). [Source review](obsidian-plugin-research-and-architecture.md).

## Product contract

Akula Settings → **Synchronization** → **Yandex Disk / Google Drive / S3** → Connect → optional checkbox **Encrypt cloud data** → confirm first-sync preview → Start.

User sees only status, progress, retry/pause and meaningful conflict resolution. No git commands, branch management, own backend or plugin installation. Automatic local changes and periodic remote polling with immediate check on startup/network resume. Offline editing must always work.

- Native app has one active cloud provider per vault in v1. Multi-cloud fanout, server-hosted Akula accounts, mobile client and live CRDT editing are out of scope.
- Local .md files remain plaintext and accessible outside Akula; cloud copy is encrypted only when the checkbox is enabled.
- Enabling encryption for a new sync root prompts for a passphrase + a **one-time recovery key**. Derive/wrap a random vault key with Argon2id, use a maintained AEAD implementation and versioned encrypted envelopes for content and path metadata; OS credential store for cached keys/tokens. The provider can still see object counts, sizes and access timing. Forgotten password **and** recovery key cannot be reset.
- Changing encryption mode on an existing sync profile MUST run a separate previewed and reversible remote namespace migration (old object history/trash may remain with the provider). No instantaneous destructive mode flip.
- S3 naturally requires more setup (bucket/region/endpoint/credential source); put all but essential fields under Advanced.
- First sync is non-destructive: show counts and conflicts; never equate a 403/429/5xx/partial page list with remote deletion.
- No direct cloud-browser editing while encrypted. Unencrypted cloud-file editing requires the separately tested plain-Markdown mirror/import bridge (PR 10); immutable Akula revision objects are canonical.

## Existing verified integration locations

| Responsibility | Current location | Rule |
| --- | --- | --- |
| Data model, sync engine | `aquilum-app/core/src/lib.rs` → new `sync/` module | No dependency on Masonry UI |
| File events | `aquilum-app/core/src/files/watcher/mod.rs` | Watcher is a hint; full reconciliation is still needed |
| Safe writes/history/indexing | `aquilum-app/core/src/files/gate/write.rs` | Guard local revision and open editor buffer; do not bypass Core |
| Existing local version history | `aquilum-app/core/src/history/service.rs`, `history/store.rs` | NOT a distributed immutable DAG; keep existing UI functionality |
| UI events | `aquilum-app/core/src/app_core.rs` | Expose sync status without blocking event sink |
| App settings | `aquilum-app/core/src/settings/models.rs`, `settings/manager.rs` | Non-secret connection/profile identifiers only |
| Native Settings page | `aquilum-app/native/src/ui/settings/schema/section.rs`, `data.rs` | Add built-in Sync section and localized text |
| Core tests | `.github/workflows/app-checks.yml` | Existing `cargo test --workspace --locked` |
| Native GUI smoke | `.github/workflows/native-check.yml` | Existing Linux/macOS/Windows gated run |

Recommended layout: `aquilum-app/core/src/sync/{mod,model,identity,journal,revision,planner,executor,local,crypto,provider}.rs` and `providers/{s3,yandex,google}.rs`.

## Canonical development sequence — stages PR 00–11

The **canonical numbered backlog** is [Tela: Akula sync PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11). Previous "11 PR (01–11)" and "00–09" are historical ordinals, **not separate implementation backlogs**. Stage labels are not GitHub pull request numbers.

**Actual implementation:** **Stage 00 merged** — [PR #22](https://github.com/Lalalalendia/Akula/pull/22), `6bc25fb796734afc2402842a94ebe25328db1260`. **Stage 01a merged** — fail-closed conflict copies [PR #29](https://github.com/Lalalalendia/Akula/pull/29), `ab5bf72f3b837ded9597b2374e7d2a60b2b89ea9`. **Stage 02a merged** — in-memory DAG and conservative reconciliation planner [PR #30](https://github.com/Lalalalendia/Akula/pull/30), `96a15727628cf76df8c3d796e4ed69d05f690b1c`. **Stage 01b draft** — SQLite append-before-apply [PR #31](https://github.com/Lalalalendia/Akula/pull/31): prior E0597 compiler error fixed; new app/perf tests pending. These are safe kernel slices only: **no working cloud sync**; stages 01 and 02 remain incomplete until journal metadata failure/replay and adversarial DAG acceptance gates. Native app must not expose an unverified Sync UI.

| Stage | Deliverable | Acceptance gate | Prerequisite |
| --- | --- | --- | --- |
| **00** | Protocol IDs/version/path policy; non-secret config; `VaultPort` + `RemoteStore` + capabilities; deterministic fake; disabled-by-default `aquilum-core::sync` | schema/version rejection, Unicode/traversal/collisions, CAS/page/drop-response tests, Rust core/native checks green | ADR |
| **01** | Fail-closed `SessionIo::preserve/append/remember`, protective copy, Core File Gate regression | preservation failure always blocks overwrite/delete; dirty buffers safe | 00 |
| **02** | Pure SnapshotDiff planner, object states/SafetyPolicy, **minimal immutable per-document DAG** with parent revisions/heads and tombstones | divergent r1/r2 remain accessible; merge retains parents; delete-vs-edit, missing root, refilter, clock skew | 00 |
| **03** | SQLite WAL/FULL operation journal, idempotency keys, baseline/receipts, migrations, recovery | kill/restart/lost reply at each state transition; no double destructive op | 01–02 |
| **04** | Guarded native Core bridge, watcher/rescan, safe binary staging and executor with FakeRemoteStore | 2 independent fake clients, 50+ fault/conflict cases, buffer guard, symlink races, safe copy | 01–03 |
| **05** | S3/MinIO provider, paging >1000, streaming, conditional mutations where supported | real 2-client MinIO E2E, create/edit/delete/rename/binary, failure readback | 04 |
| **06** | Optional AEAD encrypted cloud blobs+paths, Argon2id key wrapping, recovery + Keychain | bad passphrase, tamper/truncation/nonce reuse, cross-device unlock, never log secrets | 02–05 |
| **07** | Built-in Settings → Sync, S3 onboarding/preview, progress/retry/pause, encrypted checkbox | S3 UI beta without CLI; restart, encrypted/plain modes, no public plugin loader | 05–06 |
| **08** | Yandex Disk OAuth/REST adapter, full pagination, rate-limit resilience | real 2-client OAuth/401/403/404/429/5xx and encrypted-mode E2E | 04, 06–07 |
| **09** | Google Drive API v3 + OAuth PKCE, stable ID/parent mapping and changes cursor | real 2-client rename/permissions/token-refresh/duplicate-name/encryption tests | 04, 06–07 |
| **10** | Plaintext Markdown mirror **only if not encrypted** + import of observed external/cloud edits into DAG | cloud/web edits safely imported; never claim unobserved intermediate revisions | 08–09 |
| **11** | Cross-platform release gate, stress 10k+, large binaries, corruption/crash/recovery/rollback | Windows/macOS/Linux, two clients x all providers, no silent data loss | 00–10 |

**Architecture decision (ADR 0001):** keep the implementation built-in as `aquilum-core::sync`, using explicit host/provider traits; no separate `aquilum-sync` crate or plugin marketplace is required for v1. Revisit extraction only if actual coupling requires it. PR #22 documents the decision in `docs/sync/adr-0001-built-in-sync-contract.md`.

**Safety rule:** an immutable DAG is deliberately **per logical document**, not a full Git implementation. Existing local `HistoryService` remains a distinct facility. Unknown/inaccessible/excluded/placeholder remote entries never imply deletion. For provider APIs lacking atomic CAS, reject unsafe mutation and make weaker guarantees explicit. Remote GC stays disabled until retention/reachability/ack safety is proven.

**Parallelism:** stages 01/02 can proceed separately after 00; Yandex/Google adapters may be developed in parallel after `RemoteStore` and S3 semantics are stable, but they cannot be declared releasable without real end-to-end tests.

### Implementation and CI rules

- Use append-only immutable revisions/blobs. Never rely on a single globally overwritten remote HEAD.
- Persist pending before remote upload; write the confirmed baseline only after verified remote + local application. Lost HTTP responses must not create duplicate revisions.
- Never replace a file with unsaved editor changes, a stale local fingerprint, failed protective copy or an ambiguous remote version.
- Keep cloud sync logs/credentials/SQLite/search cache/.aquilum history out of the synced vault by default; encrypt remote metadata when encryption is enabled.
- Retain tombstones/revision history; **do not enable automatic remote garbage collection** until stale-client acknowledgements and retention safety have been proven.
- MinIO test service in a separate integration workflow; provider secrets only in protected GitHub Actions workflows. Do not test destructive scenarios on real user accounts.
- No provider is release-ready based on mocks alone. Exercise create/edit/delete/rename/binary/Unicode/case conflicts, offline concurrent edits, encrypted pairing, network failures and process kills across **two independent clients**.
- `main` should not expose a half-working Sync tab. Merge disabled internal code in small slices; enable the user feature only once a provider's end-to-end path is safe.
- Add UI telemetry/diagnostics locally without writing note contents, passwords, file names in encrypted mode or token material to diagnostic logs.

## Milestones

**A: Protocol and safety (00–04).** Inert contracts, confirmed protective copies, immutable per-document DAG, crash recovery and two-client fault tests.

**B: S3 beta with optional crypto (05–07).** MinIO/S3 provider and native UI; separate encrypted and plaintext acceptance.

**C: Yandex/Google parity (08–09).** Provider adapters share the same planner and honor specific weaker guarantees.

**D: Cloud editor compatibility and production (10–11).** External plaintext edits, cross-platform release gate.

**Next coding task:** validate [stage 01b PR #31](https://github.com/Lalalalendia/Akula/pull/31) including typing latency, implement stage 01c failure-propagating `remember`, complete stage 02b storage/replay and only then start stage 03 SQLite WAL sync journal. No feature toggle should expose partial sync.