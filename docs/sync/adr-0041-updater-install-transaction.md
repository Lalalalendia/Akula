# ADR-SYNC-041 — Shared macOS update lock and recoverable app promotion

**Status:** P0-C6c Draft, integration acceptance pending. Built-in Akula update
channel is still DISABLED: `channel_ready() == false`, `PUBLIC_KEY == ""`.
This is NOT an authorization to publish a signed Akula release or mutate S3.

## Previous failure modes

Inherited `updater/apply.rs` constructed staging and backup names using the
process PID, recursively deleted an existing staging directory without
knowing its contents, and replaced an installed `.app` via two renames
without consulting the shell installer's advisory lock. It deleted the
old `.app` immediately after the second rename. A crash between renames
could leave no installed application, and a simultaneous shell install or
uninstall could corrupt both generations.

## P0-C6c implementation

The macOS updater now delegates its on-disk authority to the new
`updater/macos_transaction.rs` module. It refuses locations other than
the **exact existing user-local `~/Applications/Aquilum.app`** used by
`scripts/install.sh` and `scripts/uninstall.sh`. A translocated bundle,
symlinked destination or arbitrary `/Applications` installation is
unsupported until explicitly designed and packaged.

1. Use a stable `~/Applications/.akula-install.lock` file and macOS
   `flock(2)` exclusive nonblocking advisory locking. The shell installer
   uses macOS BSD `lockf(1)` inherited-FD mode, which uses the same
   BSD `flock(2)` mechanism. Keep the opened FD alive through recovery,
   decompression, publication, launch dispatch and rollback; let the OS
   release the lock if the updater crashes.
2. Under the lock, scan all `.akula-install.*` staging directories.
   If more than one exists, refuse. If exactly one contains the previous
   bundle and the published destination is missing, restore the previous
   bundle BEFORE attempting another update, and stop. Unexpected staging
   content is retained for human inspection. If both generations exist
   or a stage is ambiguous, **HOLD without deleting either**.
3. Atomically create a new, random UUID-based stage under the same physical
   `Applications` directory; never reuse or recursively overwrite a
   PID-named stage. The P0-C6b strict streamed TAR parser only writes
   inside this empty stage; a failed extraction may remove ONLY the
   known, unpublished staged candidate, never an old installed backup.
4. Before publication, verify the expected native app bundle files exist.
   Rename old destination -> stage/`Aquilum.previous.app`, then candidate
   -> destination. If the second rename fails, attempt to restore the old
   bundle; preserve evidence if restoration also fails.
5. Dispatch the new app via macOS `open -n`. If macOS refuses launch,
   rename the new generation to `Aquilum.failed.app` and restore the
   previous version. If launch is accepted, **preserve the old backup**
   until a separately authenticated *new-process startup acknowledgement*
   exists. A successful `open` dispatch alone is not proof of successful
   application startup, and the next update correctly sees unresolved
   staging and refuses instead of deleting the old generation.

## Tests and contract boundaries

Direct standalone Rust regression tests on Linux and macOS cover
lock contention, symlinked destinations/lock file, independent stage
creation, recovery after old->backup, refusal after new promotion,
concurrent ambiguous stages, rollback on refused OS dispatch,
unexpected staged entries, and preservation of the previous app. On
macOS one native test uses `/usr/bin/lockf -k -t 0` to independently
prove that the Rust-held file lock conflicts with the actual shell
utility's BSD lock semantics.

A targeted native macOS `cargo check` is required to prove the real
`apply.rs` integration, not merely the standalone test harness.

## Remaining barriers

This stage deliberately leaves the new app's backup **pending** after
an accepted OS launch. A distinct P0-C6d startup acknowledgement must
verify which code-signed new generation actually initialized, remove
the preserved backup only under the same shared lock, and implement
crash/restart idempotence. Until that point the updater is intentionally
not user-ready and will HOLD further updates. True power-loss durability
still requires packaged disk/OS crash acceptance. Rust `apply()` is
not an independent signature checker; the signed-update pipeline and
minisign public key must be separately provisioned and validated.

Advisory locks do not prevent arbitrary same-user processes from
mutating files directly. Protecting against hostile peers requires a
distinct OS-owned install authority, not just this cooperative lock.

No S3 overwrite/delete, baseline advancement, GC, automatic HOLD
clearance, or installed-host protected build activation is permitted.
