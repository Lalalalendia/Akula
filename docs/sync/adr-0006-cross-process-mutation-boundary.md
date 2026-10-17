# ADR-SYNC-006 — External-process mutation boundary and first safe local-apply slice

**Status:** proposed, test-backed boundary. **Date:** 2026-10-17. **Scope:** canonical [SYNC PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11), specifically stage 04g / native `VaultPort`. This ADR does not grant write authority.

## Observed boundary

- A process-wide `Core::file_mutations` lock protects cooperating Akula Core instances **in one process**, not other Akula processes, scripts, editors, filesystem sync clients, or malicious actors.
- The registered `PinnedReadOnlyRoot`, `DocumentHub` admission and durable preimage SQLite WAL/FULL capture can detect stale file IDs, path replacement, dirty editors and mismatched content. `revalidate_bound_preimage` (PR #90) is an additional **read-only** check, not a commit token.
- Pathname-based `write_file_atomic_impl`, `rename`, `trash` and history retention must not be reused as an authoritative *remote-initiated* mutation simply because an earlier check passed.
- A POSIX `rename` after checking an inode ID is **not** atomic compare-and-swap against that inode. Windows name-based replacements have analogous identity races. A mutex that other processes ignore cannot close this gap.
- This program prefers refusing a destructive change or saving both versions over optimistic last-write-wins. The existing immutable revision DAG preserves known revisions but does not make external filesystem mutation atomic.

## Verified tests and limitations

The cross-process suite in `core/src/sync/host_admission.rs` launches **the same Rust test binary as a separate OS process**, not a thread. The parent holds the Core File Gate while the child independently:

1. Renames the protected original and creates a new file with **identical bytes but a different file ID**, then requires host revalidation to refuse the stored preimage.
2. On Unix, renames an intermediate directory and inserts a symlink to an external directory, then requires host revalidation to refuse and preserve both original/outside bytes.

A marker file proves the child fixture ran. The tests demonstrate the process mutex is not an interprocess lock and that the existing read-only checks reject the *persisting* races exercised by these fixtures. They do **not** prove that a swap immediately after revalidation will be detected.

## Implementation sequence / acceptance gates

### 04g-M1 — Additive-only local creation behind a private, disabled feature

**Goal:** safely materialize a new file without overwriting any pre-existing document. This is narrower than full `VaultPort` and does not authorize remote deletes/replacements.

1. Obtain a trusted Core workspace root, bind its live registry ID, and acquire a closed-document reservation / File Gate interlock in the documented order.
2. Pin each existing parent-directory component using OS handles / directory-descriptor-relative APIs. Reject symlink, junction/reparse, hardlink alias, case/Unicode/Windows device ambiguity. Remote object names never become absolute filesystem paths.
3. Stage a bounded verified immutable blob, with durable intent and operation ID outside the vault. Use a platform-native **no-replace** final creation in the pinned parent; fsync file and parent or the platform-equivalent durability sequence. Do not expose partially written plaintext as the final visible filename.
4. If the leaf exists, **refuse without changing it**, even if its bytes or name are equivalent to the expected state. A colliding remote version becomes a separate conflict candidate, not overwrite.
5. On any unsupported filesystem, feature, permission, OS error, ambiguous result or failed preservation/durability check, **fail closed** and preserve all confirmed data. A crash may leave a clearly scoped staging artifact that is recovered or quarantined, but must not produce a false confirmed baseline.
6. Verify Windows/macOS/Linux using ordinary API tests and a real second process changing leaf/parent during selected failpoints. Include restart after staging, after exclusive create, before/after fsync, and after lost acknowledgements.

**Exit evidence:** an exact-head three-OS CI artifact showing create-if-absent with zero unintended overwrites, no escape from vault, no phantom baseline and deterministic recovery. Keep the executor inaccessible to end users and remote scheduling until that evidence exists.

### 04g-M2 — Existing-file replacement, rename and trash

Do not infer permissions from M1. Replacement requires a platform-specific operation whose exact preconditions and external-writer assumptions are explicit; where name-and-file-ID atomic compare-and-swap cannot be guaranteed, **use a non-destructive alternate file/conflict** rather than silently clobber an external change. A protective copy is necessary, not sufficient, for identity-correct atomic mutation. Remote delete must remain soft-delete only after a proven tombstone and safe retention receipt. Test each action independently.

### 04g-M3 — Two genuine clients, then S3

After M1/M2 admission passes: independent Akula processes, shared MinIO, clean and conflicting Markdown + binary files, offline changes, network timeouts, incomplete listings, process kill/restart, and preserved history. Enable S3 only once the entire local-apply path is verified. No OAuth providers or UI during M1.

## Hard non-goals of this PR

No production `VaultPort` implementation, no OS file-writing API, no `PreservationReceipt`, no remote baseline change, no cloud provider and no Sync settings UI. Tests here are regression evidence for a dangerous boundary, not evidence of a completed sync engine.


## SYNC-04g-M1 experimental implementation checkpoint (PR #101)

**Status:** Draft / cross-platform CI required. This is a private method, not an activated sync executor.

- Implementation files: core/sync/local_create.rs, core/sync/local_create_tests.rs, core/sync/pinned_read.rs, core/sync/pinned_windows.rs, core/sync/host_admission.rs.
- Gate: require a Core-registered workspace UUID matching the STAGED journal intent, immutable content in SqliteDagStore reverified by BLAKE3, no confirmed baseline, DocumentHub closed-document reservation, then Core process-wide File Gate. All paths are SyncRelativePath values.
- Stage bytes as an operation-named, exclusively created temporary file in the OS-handle-pinned parent. Sync the stage; only then publish with Linux renameat2 RENAME_NOREPLACE, macOS renameatx_np RENAME_EXCL, or Windows NtSetInformationFile(FileRenameInformation) from the opened stage HANDLE with ReplaceIfExists=FALSE and RootDirectory pinned.
- Reopen and verify published OS file ID and all bytes, recheck parent ID and workspace. On any ambiguity leave the journal at STAGED and never acknowledge a baseline.
- A process kill before no-replace publication leaves a recognizable temporary stage; on retry creation refuses that stage rather than deleting or silently reusing it. Recovery currently requires an explicit future reconciler. If the process dies after successful publish but before acknowledgment, the final file remains and the STAGED journal cannot drive another overwrite.
- Testing must prove: ordinary and Unicode creation, destination/case/NFC collisions, symlink/junction ancestors, foreign process wins no-replace race, and real child process exit after staging while journal stays unconfirmed.

### Unresolved safety/release restrictions

1. Windows directory metadata durability is not asserted merely from flushing a regular stage file. No confirmed baseline is set until a separate Windows crash/recovery audit proves this.
2. A hostile process with write access to the same staging directory can race the stage name; post-publication identity/hash verification detects many swaps but is not a transaction against hostile same-user writes. Do not use this as an authorization for replacement/rename/delete.
3. Path alias/case-fold preflight scans cannot eliminate active rename races; only kernel no-replace can guarantee a pre-existing destination is not overwritten.
4. Orphan stages are intentionally retained on ambiguous errors. A separate, provably non-destructive recovery/quarantine policy and stage exclusion from ordinary sync scans are mandatory before any production activation.
5. No normal Core files/HistoryService/Search events are emitted by this prototype, and it does not call VaultPort. S3/MinIO, Yandex, Google, Sync settings and automatic apply remain disabled.


## SYNC-04g-M1b durable publication ledger (PR #111)

**Status:** Draft until exact-head CI. Inert Core-only hardening after M1, not production recovery.

- The existing SQLite Sync journal migrates schema v1 to v2 atomically (WAL / synchronous=FULL). The new `sync_local_publication_attempts` table preserves operations and baselines. Unique operation UUID and `(vault_id, portable collision key)` constraints block a repeat claim.
- The conditional INSERT is durable **before the first filesystem mutation**. Identical or portable-name-alias retries fail, even if the stage/final filename is missing after a restart. Success and uncertain failures retain the attempt.
- `local_publication_review_queue` exposes a read-only, tamper-checked inspection list. It is **not** physical quarantine, cleanup, permission to replay, or a confirmed baseline. Presence means an OS side effect *might* have happened.
- `.akula-sync-stage-*` is a reserved internal namespace, rejected from portable/wire Sync paths. The local scan **fails closed** when an orphan stage is present instead of returning a partial preview; hidden stages remain outside the Markdown search index.
- Regression gates: SQLite schema migration/restart, invalid intent, duplicate/portable case/NFC name claims, corruption, two independent connections, real process exit, foreign-writer competition, orphan preflight refusal.

### Remaining release blockers

1. Read-only, handle-pinned inspection of both stage and final file identities with an explicit user/host resolution policy; no blind cleanup, claim clearance or auto retry.
2. Platform-specific Windows directory rename durability under power loss (a flushed NT stage alone cannot prove parent name persistence).
3. Hostile same-user stage-handle replacement, external-process parent switches, full conflict-preserving write protocol and History/Search notifications.
4. Two-client filesystem/provider fault matrices before S3/MinIO release. No user-facing Sync activation in M1b.

**Risk decision:** deliberately leave unresolved claims permanently blocked until an independently accepted recovery design is implemented.


## SYNC-04g-M1d-a — one-shot recovery audit and native-baseline refusal

**Status:** in implementation via PR #125. No production `VaultPort` mutation or user-facing sync is enabled.

1. Journal v3 adds one immutable `sync_local_recovery_evidence` row per M1b publication attempt, with a BLAKE3 checksum over serialized M1c descriptor-verified historical evidence. The row includes the registered vault, portable relative name, expected content hash, parent file ID, stage/final file IDs and lengths, and mismatch classification. It is tied to the existing SQLite WAL/FULL journal, not the vault.
2. A Core-registered host can record **one** historical observation after read-only M1c inspection. Later observations cannot overwrite it; changed bytes or changed OS file identity must be treated as a new incident requiring separate policy, not proof of the old case. No stage is removed; the SQLite one-shot claim remains locked.
3. **Mandatory negative gate:** `StageDownload` cannot advance through the generic journal's Applied, Verified or Committed APIs. Even a pre-existing Verified phase has no valid native File Gate durability receipt, and `recovery_queue` refuses to recommend RetryCommit for local downloads. Upload and remote manifest actions retain their own existing transitions.
4. Real child-process exit cases cover both (a) after stage fsync but before publish and (b) after successful atomic no-replace publish but before any baseline acknowledgment. They are useful power-*process* interruption tests, **not** a Windows power-loss/disk-cache persistence proof.

### SYNC-04g-M1d-b release criteria — outstanding

- Define an operator-only resolution flow that does not infer provenance from matching BLAKE3 bytes. Any physical quarantine must hold OS file handles/identities and publish a no-replace, durable copy **outside** the synced vault. Never unlink the original stage merely to make a scan pass.
- Verify exact parent-directory rename persistence over real Windows storage crash/power loss or conservatively refuse to claim durable publication on unverified filesystems. A clean process exit, NtSetInformationFile success or Flushing a stage file are insufficient.
- Validate genuine interprocess adversarial races after inspection, recover orphan stage/final conflicts without overwriting, preserve immutable revision history, and emit host History/Search events only after proven admission.
- Keep the existing one-shot publication and audit records until a separately reviewed, durable, operator-authorized recovery protocol can retire them. No automatic claim clearance or retry.
