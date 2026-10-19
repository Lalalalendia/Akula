# RN-4E0 — interprocess revocation fence at the real Akula File Gate

**Status: opt-in research test, no production write admission.** Stacked on [RN-4D #404](https://github.com/Lalalalendia/Akula/pull/404) at exact parent commit `f9710cc22885c878290b883c908a93624d00cdce`. It does not change Core writer code, native product features, dependency graph or deployed Akula.

## What is verified in this step

The prior [RN-4D](../sync/staya-revocation-fence-01.md) tested two SQLite connections but deliberately did not enter the real File Gate. This experiment uses a registered disposable Core workspace, an explicitly provisioned grant database outside that workspace, and **the existing `aquilum_core::files::gate::create`** in a test-only closure under `RegisteredPullGrantLedger::with_fenced_pull_preflight`.

1. **Fence-first order:** parent holds `BEGIN IMMEDIATE`, launches the *actual test executable as a second OS process*, waits for that process's independent SQLite `SQLITE_BUSY` observation, then creates a known Markdown fixture through the real File Gate before releasing the transaction. Only after release can the epoch be revoked.
2. **Revoke-first order:** a new Core in a separate OS process executes `RegisteredPullGrantLedger::revoke`; the original preflight refuses the revoked epoch before `gate::create`, leaving the vault empty.
3. **Fixture-only byte/scope negatives:** a local test helper rejects altered BLAKE3 contents or length *before* the preflight. Wrong claimed remote ID and expiration also deny publication. This **does not verify a real Bao proof or an authenticated QUIC connection**.

The helper is *only in the test crate*. It deliberately accepts a known local constant, not bytes or filenames from the network. The production Core has **no new P2P writer, no new File Gate entrypoint and no write capability returned from a policy check**.

## Why the result is deliberately limited

- `with_fenced_pull_preflight` accepts an arbitrary callback even though documentation calls it read-only: **the Rust type system does not prevent mutation in that callback**. This test exercises that risk under tightly controlled fixtures; do **not** copy the callback pattern into the installed application.
- The file is still created via a root pathname in the legacy `gate::create` path. A cooperating SQLite writer fence is *not* an OS-pinned directory or installer-owned File Gate custody. Hostile same-user filesystem tampering and another uncooperative writer are not blocked.
- The test supplies a static mock EndpointId and a synthetic clock, not actual `Connection::remote_id()`, verified local device enrollment, real issuer key custody or a canonical shared VaultId.
- `gate::create` may write primary file bytes before HistoryService / callback reporting. A success or error cannot be translated directly into a distributed commit. **There is no durable pre-apply intent, no post-crash repair transaction, no signed replica receipt and no `APPLIED`/ACK in these tests.**
- Process-exit recovery is not power-loss durability. On Windows, directory fsync remains an explicit unsolved requirement.
- The test assumes a single known file name in a disposable registered root. A production path must be resolved and pinned using trusted Core File Gate custody; peer-supplied paths cannot be passed through.

## Next gate: RN-4E1 Core-owned publish design

Do not expose an arbitrary `FnOnce` closure as a public write permit. Build a narrow Core-owned operation with no separate disk writer:

1. Core verifies real authenticated QUIC peer, enrolled `DeviceId`, distinct canonical `VaultId`, trusted issuer/nonce, fresh epoch and expiry, object root and length.
2. Retrieve bounded bytes over Roha/noq/Iroh into an outside-vault staging file; verify full Bao proof, final BLAKE3 root, and size while retaining a pinned staged handle. Never hold an SQLite write transaction during the download.
3. Register durable **pre-publication intent** in an independent journal. Acquire real OS root/target custody, a short Core policy fence and the existing File Gate mutation lock in a documented lock order. Re-read grant/epoch inside the fence **immediately before the actual operation**, and keep it held until the operation's committed/ambiguous result is recorded.
4. Publish *no-replace* under pinned directory handles with exact path identity, then independently read back. A side effect that happened before a SQLite commit is **ambiguous**, not success. The recovery state remains `NeedsReview / Partial`; never blindly retry or overwrite user bytes.
5. Only after durable recovery and readback attach signed, host-identified replay-safe receipt/ACK. Validate two processes, crash cuts and Windows/Linux exact-HEAD; later require actual Android-to-PC and independent machines.

**CI gate:** run `rn4c_peer_grant_lab`, `rn4d_revocation_fence` and `rn4e_filegate_boundary` with `--locked` on Linux and Windows. Preserve the test-only scope; do not merge the stacked research chain into `main` or enable sync in installed Akula based only on this experiment.
