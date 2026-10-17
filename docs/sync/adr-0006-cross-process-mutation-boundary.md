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
