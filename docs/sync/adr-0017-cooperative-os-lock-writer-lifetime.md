# ADR-SYNC-017 — Cooperative OS lock lifetime spanning File Gate and HistoryService

Status: SYNC-04g P0 experiment / **test-only real writer exercise, NOT enabled in ordinary Akula writers** (2026-10-18).

## Provenance and problem

PR #184 persists an existing external lock's physical identity and VaultId in two independent Core SQLite registries, but observation releases the OS lock before any normal file mutation. The ordinary File Gate `write` route releases its process-global mutex before `record` invokes HistoryService, whose process-global mutex is distinct. Therefore acquiring and releasing the OS lock only around the primary Markdown write would leave a cross-process race between the file and its saved versions.

## Implementation

- `with_cooperative_scope_lock` acquires the existing pinned root-relative OS lock after current registry, physical ID and owner/mode or Windows owner/DACL checks. The caller's closure executes *while the original OS HANDLE remains locked*. Registry/root, held filename, alias ancestors and owner/ACL are checked again afterward, still under lock. A busy lock fails rather than waits indefinitely. The prior read-only `inspect_external_scope` now delegates to the same pre/post checks with a trivial closure.
- `HeldProbeLock::recheck_held_name` is compiled outside `cfg(test)` as a negative forensic check. An attacker with path namespace authority can still rename the file immediately after checking or during the closure, and a post-write denial is not rollback or crash recovery.
- Private `SqliteCustodyIdentityStore::with_cooperative_writer_lifetime` requires the *persisted* local workspace ID, expected VaultId, root volume/file ID and custody scope path to match. While the OS lock is held it compares the pinned custody scope and lock identities to the saved SQLite receipt **before entering the closure**. It checks the same unchanged receipt again before releasing the lock, and then rechecks registration. A missing/corrupt/changed receipt never starts the operation.
- The ordinary `Core::file_mutation_guard_for_write` and HistoryService remain unchanged. This wrapper is not connected to general public/desktop write entrypoints; callers must explicitly use it. This slice implements a real **lifetime experiment**, not authorization for arbitrary sync writes.

## True two-process native test

The native `oslock_cooperative_` tests launch the compiled test binary as a second process (not a thread). Both Core processes have distinct app-data SQLite workspace/custody stores and the same OS-identity-bound external lock and declared VaultId. The child acquires the lock and waits before touching user files. The parent attempts a real File Gate write, receives a denial and proves the original file is unchanged. After release of the test barrier, the child runs the real ordinary `files::gate::write`, which writes the Markdown note and records a named version through HistoryService. It signals only after the history has become visible, **but remains alive holding the kernel OS lock**. The second writer is still refused. Forced child termination releases the kernel lock; the other Core then completes a second actual File Gate write and history operation under the identical lock. The lock contents remain unchanged.

A separate test retains the old OS lock object, substitutes another file at the same name, and verifies the persisted OS ID check refuses to execute any closure. Existing tests for corrupt receipt, registry root replacement, incompatible VaultId, symlinks, ACEs, crash recovery and multi-process lock creation remain required.

## Lock order and safety boundary

Cooperating order: (1) one externally scoped OS lock, (2) File Gate process-global mutex during primary disk work, released, then (3) HistoryService process-global files mutex and state mutex during version persistence. The test holds step (1) across the entire File Gate call and until step (3) completes. Never acquire File Gate again while holding a HistoryService mutex; never emit synchronous callbacks that re-enter the same OS lock; do not use a blocking lock acquisition that could deadlock with existing process-global locks.

## Still blocking production activation

- Lock file resides in an externally supplied, same-user-writable directory. Current owner/DACL checks do **not** prevent malicious same-user/administrator namespace replacement, inode reuse, pre/post validation races or tampered SQLite receipts. There is no durable broker-owned custody root. The test demonstrates safety only when *both* independent Cores cooperate on the same pre-existing lock object.
- In-process calls not wrapped in the new helper do not participate. Other entrypoints — editor saves, direct HistoryService commands, rename/trash/restore, retention, file purges, sync staging/publication and plugins — must be inventoried and brought under one non-recursive guard with a proven global lock order before claiming production-wide coverage.
- The existing File Gate returns normal history errors after primary bytes may already be written. Cross-vault transfers remain blocked; bilateral PREPARED/PUBLISHED/VERIFIED, checkpoint reconciliation and crash-safe recovery are not implemented by this experiment.
- No automatic sync overwrite/delete, managed cross-vault move, confirmed-baseline commit or HOLD clearance permitted.

Next P0: agree an OS-backed common custody-root installation and security principal for the cooperating process threat model; then integrate the held guard into every production mutation path, run two-process write/history/fault/power-loss matrix and check normal editor save latency.

Related: ADR-SYNC-013, ADR-SYNC-014, ADR-SYNC-015, ADR-SYNC-016; https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11