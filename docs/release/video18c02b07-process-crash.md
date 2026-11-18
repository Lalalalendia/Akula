# VIDEO-18c-02b-07 — real signed five-PE process-kill recovery (non-release)

**VIDEO-18 RELEASE HOLD: ON.** Only exact-head, ephemeral GitHub-hosted Windows pull-request CI. Neither publisher keys nor a trusted public update channel are admitted. There is no product installer change, production updater change, release artifact, user installation or deployment.

## Previously accepted

PR #1029 proved a genuine `0.3.20 → 0.3.21` native source and PE version increase, two independently tracked Cargo locks, cryptographically authenticated ephemeral manifests for five real Windows executables, protected NSIS current/ and legacy layout, actual rar2 activation, bounded real visible-GUI first-frame ACK, two incorrect-ACK rollbacks and positive confirmation.

It did **not** prove recovery when the OS unexpectedly terminates the updater process with a journal in flight.

## This stage: real external process termination, not an in-process simulation

The same signed `0.3.21` five-PE candidate and genuine `0.3.20` historical installation are used in the already established disposable Program Files NSIS lab. The exact source checkout, protected Program Files root, runner-temp stage, standalone test executable, source lock, ephemeral Minisign manifest and five PE digests are checked by the **separate controller child process**, not inherited as an unchecked parent assertion.

For each of four independently prepared transactions:

1. Spawn a fresh, nonshipping child controller under the hosted Windows runner. The child must acquire the real `InstallLock`, check there is no unresolved journal, and verify its old and signed new generations.
2. Halt at a specific **durable rar2 phase**, still owning the OS lock:
   - `prepared`: journal and signed candidate copied; old current remains installed
   - `retained`: previous five-PE tree retained, current absent
   - `activated`: new five-PE candidate activated without an ACK
   - `ack-preconfirm`: the **real new Akula GUI** presented a visible frame and emitted an authenticated, signed-generation-bound ACK, but `confirm_candidate()` was deliberately not called
3. Child sends one `VIDEO18C02B07_KILL_READY` marker only after checking its exact journal phase. Parent verifies the child is alive and the OS lock is unavailable, then forcibly terminates the child using `Child::kill` (Windows `TerminateProcess`). This deliberately bypasses Rust normal destructors/transaction cleanup.
4. Parent reacquires the actual OS lock, reads the persistent journal, verifies a second transaction is refused as `ActiveTransaction`, and invokes a newly constructed `rar2 UpdateEngine::recover()`.
5. Recovery must report `PreparedTransactionAborted` or `UnconfirmedCandidateRolledBack` (as appropriate). All five old PE SHA-256 digests, legacy flat fallback, and the independent private Markdown note must be byte-identical; the new signed manifest must **not** authenticate the restored old current. No active journal may remain; orphaned transaction staging is cleaned under the lock.

The standard prior positive signed GUI ACK + `confirm_candidate()` and two wrong-ACK rollbacks still run **after** all four crash cases succeed. A regression must not bypass those existing checks. Windows CI requires both `VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS` and `VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS`.

## Explicit remaining limitations

- `TerminateProcess` is **not** a Windows reboot, physical power-cut, storage cache flush or antivirus/locked-PE simulation. Separate reboot/power-loss and Win10/Win11 host acceptance remain outstanding.
- The Minisign test key is ephemeral, and the existing pinned rar2 lab has no reviewed release dependency lock. It is **not** an owner-trusted publisher signature or production-ready update controller.
- A successful authenticated GUI ACK is **necessary but not sufficient** for durable confirmation. This lab specifically proves that killing the controller between ACK and commit restores the full previous generation.
- Shipping `updater/apply.rs`, the ordinary two-EXE Windows installer, `channel_ready()`, user data and publicly published packages remain unchanged.

**VIDEO_FIVE_CHANNEL_READY=false; no release or license HOLD removal.**
