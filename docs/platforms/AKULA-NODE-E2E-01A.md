# AKULA-NODE-E2E-01a — operator-local verified replica via existing Sync DAG

**Status:** opt-in Linux laboratory; not NODE-E2E-01 product PASS. Canonical specification: [AKULA-NODE-ARCH-01](https://app.notion.com/p/40132a84beec8118b1a2fffc90fb808c). Separate Bootstrap acceptance: [AKULA-SERVER-E2E-01](https://app.notion.com/p/40032a84beec81d0a25dd178456910bb).

## Exact scope

The implementation is `aquilum-app/core/src/node_replica_lab.rs`, compiled only for Linux when `node-replica-lab` is explicitly enabled. The existing D1.5 `ReadCatalog` supplies **one locally operator-allowlisted UTF-8 Markdown document** (at most 64 KiB). The existing Sync Core `SqliteDagStore` is the **only replica blob/revision writer**. No duplicate CAS engine, new network protocol, own File Gate writer, Rust web server, cloud SDK, daemon, SSH client or UI switch is introduced.

The test operator must provide: real local source directory, *separate* private Node state directory (0700, owned by UID), typed `VaultId`, `DocumentId`, `RevisionId`, top-level Markdown filename and independently expected lowercase BLAKE3. A known UUID is not a grant. The root is read via Linux pinned FD, `O_NOFOLLOW` catalog admission; the state directory is checked disjoint from source. SQLite file name is derived from a validated VaultId, created 0600 and checked for symlink/hardlink/wrong ownership before access.

### Durable handoff

1. Admit exact top-level Markdown source through `ReadCatalog` and enforce raw-byte BLAKE3, prior to opening the Node state DB.
2. Reuse `SqliteDagStore::store_blob` (SQLite WAL, synchronous=FULL) to append an immutable content-addressed blob.
3. Insert one immutable parentless `RevisionRecord` with the precise DocumentId, filename and blob hash via `SqliteDagStore::ingest`. Pending/unexpected revision refuses publication.
4. Reconstruct the DAG, verify the exact revision scope, fetch `verified_revision_content`, rehash the bytes and compare to the admitted fixture. The result is an **operator-local observation** only, NOT owner-signed durable receipt or permission to delete the original.
5. A second real OS process reopens the independent Node database, obtains and checks bytes **even after the source filename is removed**. No privileged helper or network authorization is exercised.

A crash between steps 2 and 3 may leave an unreferenced blob in the existing SQLite store. It must NOT create an active DAG revision or be exposed through the narrow `verify_local_replica` API. There is no automatic cleanup/deletion or implicit retry.

## Exact-head tests and falsifiers

| Test | Must prove |
| --- | --- |
| `two_real_processes_read_durable_replica_after_original_disappears` | Stage, process exit, original offline, new process validates exact BLAKE3 bytes and 0600 DB |
| `denied_digest_wrong_vault_document_revision_and_unsafe_sources` | Wrong digest before DB creation, unknown VaultId, mismatched DocumentId/RevisionId, symlink/path traversal all denied |
| `abrupt_exit_before_revision_keeps_blob_unservable_until_explicit_retry` | Actual process exit=73 after blob WAL commit, no active revision, explicit repeat stages successfully |
| `revision_collision_and_corrupted_sqlite_blob_never_serve_wrong_bytes` | Conflicting revision cannot replace existing; direct SQLite corruption rejected after reopen |
| `private_state_must_not_be_inside_the_source_vault` | Never initialize Node sqlite inside local Vault |

Run with:

```bash
cd aquilum-app
cargo test --locked -p aquilum-core --features node-replica-lab --test headless_node_replica -- --nocapture
cargo check --locked -p aquilum-core --no-default-features
```

Existing D1/D1.5/D2 feature tests continue in the same profile workflow. Read-only default Desktop/Node build receives no new writer or enabled feature.

## What this intentionally DOES NOT prove

- This is a private **local test fixture**, not a network-admitted replica or real Node service, and no other DeviceId can yet access it.
- An input `VaultId` is a typed label, not independently enrolled owner authority. The state path is protected only against obvious local configuration errors and unprivileged different UID access; malicious same-UID races and SQLite file modification are outside the lab claim.
- A BLAKE3 digest detects accidental/tampered bytes but does not authenticate an owner. A local readback after `FULL` SQLite commit is not independent hardware-power-loss evidence, not encrypted at rest, and not a signed owner receipt.
- This is a **single parentless revision**. A signed grant, source-host File Gate capture, full revision ancestry/merges, network Wire/API, revocation and file-level ACL are absent. Do not expose `verified_blob` or this lab module as a public Node route.
- No user Vault files are written; stage and read never perform delete, replace, rename, or baseline advancement. The only writable SQLite belongs to an explicitly separate lab state directory.

## Next gates (no duplicate implementation)

**NODE-E2E-01b:** map actual signed Owner/Device/Node grants and fixed VaultId-scoped object permissions into existing Sync Core identity. Reuse one verified/staged object protocol, not raw public `PUT`, then re-evaluate what physically durable **receipt** means. Keep Node data-plane disabled until accepted.

**NODE-E2E-02:** independently authenticated second process/device reads the permitted replica after first client is offline; unauthorized client and revoked grant reveal neither existence nor bytes.

**Release gate:** real signed akula-node, sealed production custody, B1 owner-approved install, health after reboot, actual VPS acceptance. QEMU/Linux lab PASS is only a necessary precursor.

**Source:** [Sync DAG store](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/dag_store.rs), [Node read catalog](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/node_catalog_lab.rs), [D2 original test](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/tests/headless_node_d2.rs).
