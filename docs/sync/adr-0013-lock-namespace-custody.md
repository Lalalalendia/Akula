# ADR-SYNC-013 — A held OS lock is not custody of its filename

Status: `SYNC-04g-3b-7c-b6` — cross-process falsification and negative identity check, test-only (2026-10-18). **No release of File Gate write authority.**

## Tested gap after PR #157

`O_EXCL` / `FILE_CREATE` ensures only one winner for an *individual initial directory entry*. It does **not** keep that name mapped to the same inode / Windows file ID. An adversary with namespace-write access can rename a lock file **while another process holds an exclusive OS lock on its descriptor**, create a new file at the former path and obtain an independent exclusive OS lock on the new object. Both locks can coexist. Copying the original valid magic, UUID and BLAKE3 record does not prevent this, since the checksum is public and the file ID changes.

This is more than stale metadata: two instances that trust only the lock filename could independently commit to one vault.

## Native falsifier

Two real Rust OS processes are involved, never only threads within a Core:

1. The parent bootstraps an existing, correctly formatted test lock in a private disposable vault. A child process opens and **holds** an exclusive lock on this first physical file; it announces readiness and remains alive.
2. While the child still holds that lock, the parent moves the held file to another name, replacing the original pathname either with a newly bootstrapped generation or with a *byte-for-byte clone* of the earlier valid record.
3. The parent demonstrates that the new physical object permits a second exclusive OS lock despite the live child holding the first. The observed success is **a split-lock vulnerability demonstration, not positive authorization**.
4. The child reopens the named object under its pinned root and verifies that the identity no longer matches its *held handle*. Its new test-only `HeldProbeLock::recheck_held_name` refuses, while the parent still holds the replacement lock. Both original and replacement bytes remain unchanged by the test.
5. The parent terminates the child only after observing refusal, distinguishing detection while the first holder is alive from OS cleanup after exit.

Both the fresh-generation and cloned-valid-record scenarios are required on Linux/macOS/Windows by the existing `oslock_` CI filter.

## Important limitations

The new identity recheck runs under `#[cfg(test)]`; it is deliberately **not** wired into ordinary File Gate or HistoryService. Even a positive result cannot guarantee safety once the recheck returns: a hostile directory writer could rename the object *immediately afterward*. No amount of path rechecking or BLAKE3 checksums turns a mutable directory entry into a trusted permanent lock.

The test assumes a filesystem that permits rename of an open object. If some OS filesystems deny the rename, this must be recorded explicitly as a platform/environment restriction; **do not silently skip or claim a cross-platform security proof**.

## Required production custody decision

- **Cooperating-process scenario:** establish one stable lock anchor outside user-editable vault content, keyed by a persistently attested vault identity and opened through a pinned, access-controlled root. On Unix use restrictive owner permissions and a validated persistent physical root ID; on Windows set and validate the actual DACL, open with safe share/rename policies. Never unlink, truncate or recreate a live anchor. Refuse an unknown, damaged or substituted anchor; do not attempt automatic repair.
- **Hostile same-user processes:** user-owned directories and files are insufficient to guarantee exclusive custody against a fully privileged same-user process. Stronger isolation requires a separate OS security principal or broker/service holding authority inaccessible to untrusted writers. Choose and document a realistic attacker model before claiming that Akula defends against this class.
- Persistently reconcile vault identity across multiple independent Core SQLite registries and after restart. The local UI workspace UUID **is not a shared VaultId**; a copied BLAKE3 record is not authenticated identity. Reject untrusted/network filesystem locks unless their identity and lock semantics are tested.
- All cooperating File Gate, HistoryService and sync/native host writers must acquire the **same** OS lock anchor in a proven global order. Acquire-and-recheck is only a **negative** admission guard until bilateral receipts and crash recovery are established.
- Require two-process Native tests for the chosen custodial implementation, permission revocation, same-name swap, root relocation, power-loss/kill, and lock contention. Benchmark hot-path save latency.

**Exit gate:** until the real custody strategy and cross-process writer integration are proven, *no* managed cross-vault move, local sync overwrite/delete, automatic HOLD clearing or accepted-baseline commit may use this probe as authority.

Related: [ADR-SYNC-010](adr-0010-pinned-lock-identity-probe.md), [ADR-SYNC-011](adr-0011-registered-lock-forensics.md), [ADR-SYNC-012](adr-0012-exclusive-lock-bootstrap-probe.md), [canonical Tela sync plan](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
