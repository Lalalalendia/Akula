# ADR-SYNC-012 — Exclusive, self-checking OS lock bootstrap: controlled two-process evidence

Status: **04g-3b-7c-b5, test-only precursor** (2026-10-18). This is NOT production bootstrap, a trusted VaultId or permission to mutate files.

## Why

PR #145 proved ordinary pairwise OS advisory locks and crash release; PR #150 pinned existing lock objects to OS descriptor/HANDLE identity; PR #153 bound lock inspection to a local Core's registered SQLite workspace. None created the lock safely. Before any process can rely on a shared OS mutex object, we need evidence that concurrent first-time initialization cannot silently produce two different named objects or erase an incomplete state. SQLite workspace UUIDs are **local to each Core** and must not be treated as a cross-client vault identity.

## Implementation: only disposable test vaults

- `PinnedProbeRoot::bootstrap_exclusive_for_test()` is compiled **only under `#[cfg(test)]`**. It never executes during normal Akula startup, File Gate writes, HistoryService or sync. The private inspection method from PR #153 remains strictly read-only.
- Unix: create one existing-root-relative leaf through `openat` with `O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC` and mode `0600`; on collision only reopen the existing named object. Windows: open the known root with child-add authority and verify its OS identity matches the pinned read-only root; create with root-relative `NtCreateFile(FILE_CREATE)` and refuse replacement/truncation.
- The new v1 test fixture record is `AKULA_FILEGATE_LOCK_V1` + 16-byte random shared generation UUID + BLAKE3 digest of the preceding bytes, with an **exact, bounded size**. A child finding an existing incomplete, unknown-version or checksum-invalid record must fail closed, preserving bytes, not repair it or create another pathname.
- The winner syncs the file contents via `File::sync_all()` before observing the recorded generation. On Unix, a parent-directory `fsync` is attempted; if unsupported, `parent_sync_completed=false` rather than asserting directory durability. Windows does **not** claim parent-directory durability, even if file flush succeeds. Filesystems may lie about sync/durability.
- After creation/collision, a fresh open relative to the original pinned root confirms `regular file`, `nlink=1`, stable physical file ID and the exact checked record. Root pathname identity is rechecked. The initializer never deletes or silently reconstructs a lock object.

## Exact native tests

- Newly created lock preserves OS file ID, generation UUID and byte content across a fresh root handle (a simulated Core restart). A repeated call opens the same object and reports `created=false`.
- A crash-shaped truncated or checksum-corrupted existing record refuses without overwriting. Hardlink aliases, Unix symlink/foreign-special leaf and pinned-root replacement refuse, with external bytes unchanged.
- **Two independent Rust test subprocesses** wait on a shared start barrier and race their exclusive creation against the same root. Exactly one may report `created`; the other may report `existing` on the same OS ID + UUID, or may explicitly refuse if it observes the winner's incomplete record. Refusal is safe, re-creation is not. A third fresh reader must confirm the winner's record and OS identity.
- The existing `oslock_` CI step selects all new `oslock_bootstrap_` tests on Linux/macOS/Windows, alongside previous two-process locking tests. Full Rust + Sync Engine, Docs, S3/MinIO and protected-original lanes must complete on the exact final head before merge.

## Security limits and release gate

1. A checksum is **not a signature or MAC**. Any process able to edit the vault can forge or swap a valid-looking record. An exclusive O_EXCL/FILE_CREATE initial creation is not permanent custody, nor does a file handle prevent the name from being replaced later. The test object is deliberately named `.akula-filegate-oslock-probe` and is never installed as a trusted production lock.
2. During the creation-to-flush window, a second process can see an incomplete record. This version returns an error and may require manual retry in tests; it does not grant write authority. Real initialization must be carefully sequenced and must never use an incomplete record to authorize writes.
3. The shared random generation is an **untrusted forensic identifier**, not an authenticated, stable vault ID. Attestation/ownership of the lock to a registered root and durable restart identity requires a separate contract.
4. Directory-entry durability on Windows, on network shares, and across power loss is **not proved**. A filesystem reporting successful flush is not proof of hardware persistence. No automatic cleanup or repair exists.
5. The normal File Gate and HistoryService still use process-local mutex/HOLD logic. Managed cross-vault moves, local sync overwrite/delete, automatic HOLD clearance, confirmed baseline commits and any mutation on the strength of the probe remain prohibited.

**Next P0:** separate durable root/lock custody anchored to authenticated vault identity, tests of a malicious rename/unlink while two processes hold different old/new file IDs, permission/ACL policy, and cross-process Core + HistoryService admission under a proven lock-order contract. Only afterward can bilateral PREPARED → PUBLISHED → VERIFIED transactions be considered.

See [ADR-SYNC-010](adr-0010-pinned-lock-identity-probe.md), [ADR-SYNC-011](adr-0011-registered-lock-forensics.md), [canonical Tela roadmap](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).