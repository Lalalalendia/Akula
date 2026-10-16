# Akula sync implementation roadmap

**Status:** proposed plan, NOT implemented. **2026-10-16.** The feature must be built into Akula, NOT distributed through a plugin store.

**Full architecture + diagrams (Tela):** [Sync architecture](https://telawiki.com/spaces/456/pages/7463), [immutable per-document revision RFC](https://telawiki.com/spaces/456/pages/7465/git-rfc), [11-PR implementation plan](https://telawiki.com/spaces/456/pages/7469/11-pr). [Notion research](https://app.notion.com/p/3fb32a84beec81c1b561db8ad0b19df6). [Source review](obsidian-plugin-research-and-architecture.md).

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

## Development PR sequence

Each PR should be independently testable. The following PR numbers are **planned ordinal labels, not already-created GitHub PRs**.

| Step | Deliverable | Acceptance gate | Needs |
| --- | --- | --- | --- |
| **PR 01** | Versioned protocol v1, VaultID/DocID/DeviceID, normalized paths, `SyncConfig` (non-secret) | serde schema roundtrip; reject unsupported version, symlink/traversal/case/Unicode/path collisions | RFC |
| **PR 02** | SQLite WAL journal, confirmed baseline, pending queue, crash recovery, device identity | restart midway through pending op twice; no deletion/duplication and old local bytes unchanged | 01 |
| **PR 03** | Immutable content blobs, per-file parent DAG, multiple heads, rename and tombstone | two offline revisions survive upload in either order; merge revision points to both parents | 01–02 |
| **PR 04** | Deterministic planner + executor + in-memory FakeRemoteStore, Core File Gate integration | two-client tests: concurrent edit, dirty buffer, delete-vs-edit, dropped response, duplicate retry | 02–03 |
| **PR 05** | Real S3 adapter, MinIO CI, pagination and streamed binaries | MinIO two-device roundtrip and disconnect/restart, no silent last-write-wins | 04 |
| **PR 06** | Optional encryption envelope + Keychain + password/recovery UX primitives | encrypted blobs **and** metadata; no plaintext credentials; bad passphrase/tamper/nonce tests | 03–05 |
| **PR 07** | Built-in Settings → Sync, working S3 onboarding, first-sync preview, status/conflicts | UI user journey completes without CLI, survives restart, supports pause/retry | 05–06 |
| **PR 08** | Yandex Disk OAuth and paginated REST provider | real sandbox two-client E2E + 401/429/5xx + encrypted mode | 07 |
| **PR 09** | Google Drive v3 + desktop OAuth PKCE/drive.file + remote stable IDs/changes feed | real sandbox two-client E2E + renames, cursor replay, token refresh, encrypted mode | 07 |
| **PR 10** | Optional readable unencrypted Markdown mirror and cloud-web edit importer | observed third-party changes imported as separate revision; provider limitations clearly surfaced | 08–09 |
| **PR 11** | Production readiness: cross-platform, stress, crash, docs, migration/rollback | Windows/macOS/Linux smoke + 2-device/3-provider pass + no data-loss defects | 01–10 |

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

**A: foundation (PR 01–04).** Two offline clients safely reconcile branches using only in-memory storage.

**B: S3 product slice (PR 05–07).** MinIO/S3 works through actual built-in UI, with optional encryption. Internal beta only.

**C: three-provider parity (PR 08–09).** Yandex Disk + Google Drive adapters implement the same RemoteStore behavior without provider-specific forks in the planner.

**D: external edits and release gate (PR 10–11).** Unencrypted optional cloud compatibility, data integrity and UI acceptance across platforms.

**First coding task:** PR 01 (Rust sync models + wire format + unit tests). This roadmap and the architectural research are design artifacts, not proof of implemented syncing.