# ADR-SYNC-042 — Generation-bound macOS updater startup receipts

**Status:** P0-C6d part 1 / Draft, CI under review. This is an update
*acknowledgement* protocol, NOT automatic approval to discard old app data.

## Problem

P0-C6c (#344) correctly treats an accepted macOS `open -n` as only a
launch dispatch, not proof that the newly installed app actually started.
It holds the original `Aquilum.previous.app` under a unique
`.akula-install.<UUID>` stage, and refuses another update until
that stage is resolved. This prevents silent data loss, but lacks an
auditable link between the new process and the exact generation just
promoted. Merely seeing the new executable filename is insufficient.

## New two-phase receipt protocol

**Stage 1: pending intent before publication.** While the P0-C6c shared
`~/Applications/.akula-install.lock` BSD lock is held, after safe TAR
extraction and before moving the old app, calculate bounded **BLAKE3**
digests of the extracted `Contents/MacOS/aquilum-app` binary and
`Contents/Info.plist`. Write a unique, immutable
`.akula-updater.pending.v1` record containing exact UUID generation,
both digests and a versioned format header. Use create-new and
`sync_all`; refuse existing, invalid or unreadable receipts.

**Stage 2: first nonempty graphical frame.** Only the native macOS
installed build, after its Core workspace, window and renderer have
initialized, calls `startup_ack::on_first_presented_frame()` when the
first nonempty rendered frame reaches the window output. This schedules
the read-only verification off the GUI thread. The new process:

1. Resolves its actual running native executable and installed
   `~/Applications/Aquilum.app` path, rejecting other locations.
2. Obtains the same cooperative BSD lock as shell install/uninstall.
   If the prior updater still owns it, retries a bounded number of
   times. A different error is HOLD, not a silent bypass.
3. Requires one exact staged generation, a real previous bundle, and
   no unknown staged files. The pending record UUID must match the
   directory generation.
4. Recomputes BLAKE3 over the **currently installed** native executable
   and Info.plist. Confirms both match the pending record, and the
   process executable's canonical path matches the installed binary.
5. Writes a create-new, sync-all
   `.akula-updater.acked.v1` record for that same generation.
   Repeated matching acknowledgements are idempotent; conflicting,
   truncated, aliased or replaced records fail closed.

**No automatic backup deletion.** The old `Aquilum.previous.app`, stage,
both records, and the normal installer HOLD remain in place after ACK.
This deliberate restraint means stage corruption or an application
that closes/crashes after drawing its first frame cannot cause
an irreversible lost-version event merely by satisfying this protocol.

## Acceptance

15 unit tests in the native Rust updater exercise valid and repeated
receipts, no pending update, edited executable and plist, foreign
generation nonce, unexpected staging file, missing previous bundle,
corrupted/missing/conflicting ACK, wrong executable path and
multiple abandoned stages. The existing 20 transaction tests,
including two independent SIGKILL child tests and native BSD-lock
interoperability, remain in the CI matrix.

Required Application Linux test lane compiles this module under test
configuration despite macOS-only production use. A native macOS
`cargo check` and targeted `cargo test updater::startup_ack`
independently validate actual Apple build integration.

## Explicit trust limitations and next gate

- BLAKE3 binds the receipt to the native binary, Info.plist and staged
  UUID. **It does not** prove Developer ID signing or authenticate the
  complete `.app` resource tree, and same-UID attackers able to write
  app/receipt files remain out of scope.
- A nonempty presented frame demonstrates that initialization reached
  the rendering path, not that every user workflow or persistence write
  is healthy. Persistent availability, correct vault data, app signature
  and post-startup health are separate release gates.
- The receipt is created and synced, but full power-loss/fsync-order
  durability is not established by synthetic SIGKILL tests alone.
- Until Akula actually has a pinned Developer ID/bundle identity and a
  reviewed minisign updater key, the release channel remains intentionally
  disabled (`channel_ready() == false`; empty `PUBLIC_KEY`).
- **P0-C6d part 2** must require independently checked publisher identity,
  exact installed generation and sufficiently strong app health/stability
  before safely clearing the protected backup under the same lock.
  Do not add a generic "ACK file exists -> recursive delete" shortcut.
- Also independently bound the download length/memory of updater payload
  before enabling the channel; presently `Updater::download` can grow
  its in-memory Vec for arbitrarily large response bodies.

No remote S3 overwrite/delete, baseline promotion, GC or automatic
custody HOLD release is approved by this change.
