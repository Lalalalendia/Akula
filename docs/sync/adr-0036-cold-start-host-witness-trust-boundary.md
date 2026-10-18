# ADR-SYNC-036 — Cold-start host ledger identity witness (P0-B6a)

**Status:** primitive and negative acceptance only; **NOT** installed-release
authority, installer provisioning, Sync Beta, or protection against hostile
same-user writers.

## Why #282/#284 are not enough

PR #282 pins the open SQLite main object and its immediate parent, rejecting
main/WAL/SHM pathname substitution during an already-live registry epoch.
PR #284 proves a missing mandatory installed ledger keeps structural File Gate
operations on HOLD without breaking positive unenrolled note create/write.

After all processes quit, however, an attacker (or a broken installer) can
rename the **real** parent or SQLite main file, then replace the original
pathname with a genuine schema-1 **empty** SQLite registry. An unanchored
`SharedHostCustodyRegistry::open_existing(path)` cannot tell that its old
enrollment was discarded: it sees an authentic SQLite schema and no entries.

This remains true even with `--features sync-installed-host`, because the
compiled absolute path is not a previously authenticated physical identity.
A BLAKE3 checksum inside the replacement DB is not authentication.

## Narrow B6a contract

- `HostLedgerObjectIds` is a four-number snapshot: OS device/file identity
  for the immediate parent directory and the SQLite **main file**. The live
  object is held as before; no extra descriptors are opened/closed inside a
  Unix SQLite transaction (closing them could break POSIX `fcntl` locks).
- `HostLedgerIdentity::require_expected` compares a pinned live object with
  a pre-existing expected snapshot and checks names before and after.
  Mismatch means `InvalidHostLocation`, not empty enrollment.
- The separate `SharedHostCustodyRegistry::open_existing_with_expected`
  invokes the comparison **before** opening SQLite and after validating
  schema and durable rows. It never provisions, repairs, writes an anchor,
  or authorizes a File Gate mutation.
- The existing unanchored `open_existing` remains unchanged so current
  local mode, CI fixtures and release are not inadvertently switched.
  **No production Core or release workflow consumes the witness yet.**

**Fundamental trust requirement:** a caller may use these IDs for a
security decision ONLY when the expected values come from independently
protected, installer-issued state. Re-reading IDs from the proposed DB/path
during cold startup, a sidecar in the same writable directory, a runtime
environment variable or alternate appdata would make this test circular.
The tests pass expected IDs via environment **only as a disposable harness**.
That environment is never a production authority.

OS physical IDs can be recycled; a hostile same-user process that rewrites
the original inode, changes an independently mutable witness, or forges
another filesystem namespace may defeat the comparison. Neither a copied
manifest nor `chmod 0600` against the same user addresses this.

## Cross-OS negative evidence required

`host_cold_identity_tests.rs` shares the existing host-registry fixture.
The local host-custody CI runs it on Linux, macOS and Windows.

1. Provision/enroll original ledger, capture the original identity as a
   separately held fixture claim, drop every SQLite connection.
2. In a genuine second OS process, open with that expected identity, observe
   the enrolled record, and verify positive behavior.
3. Replace the original path with a *different, valid and empty* SQLite DB.
   The second OS process must return `InvalidHostLocation`; demonstrate that
   the current **unanchored** opener still sees `None` for the old enrolled
   workspace. Restore the original DB and reopen with the unchanged witness.
4. Rename the original parent away; create another **real** directory with
   a valid empty host SQLite at the compiled name. Expected-identity open must
   refuse despite no symlink/junction. Preserved original enrollment remains
   available for forensic review.
5. Existing native positive two-process CAS/history acceptance, S3 transport,
   Rust/policy and frames must remain green on the exact PR head.

Tests run in temporary directories, never real user vaults. The test's
explicit external witness is **not** a shipped installer trust root.

## Installer/OS decisions still required (P0-B6b)

| Distribution | Safe claim available now | Hostile same-user protection prerequisite |
| --- | --- | --- |
| Windows per-user NSIS | Cooperative checks and fail-closed path policy | Privileged/isolated host broker and OS-owned identity/ACL for DB, WAL, SHM and all ancestors; per-user writable SQLite is insufficient |
| macOS copied .app / DMG | Cooperative checks only | Approved privileged helper or equivalent independently protected trust root; ordinary same-user Keychain/app support paths alone do not defend against same-user replacement |
| Linux deb/rpm | Cooperative checks; can install root-owned files | Dedicated privileged broker/IPC retaining writable SQLite namespace itself; root-owned DB whose WAL directory is writable to the user does not solve the threat |
| Linux AppImage | Local-only/cooperative install, no privileged provisioning implied | A separate explicit broker installer; an unprivileged AppImage cannot silently claim OS-owned custody |

A trusted cold-start claim also needs installer-instance identity, rollback
policy, physical ancestor and SQLite sidecar lifecycle validation, not just
`(device, file)`. Upgrades must preserve enrollment; uninstall must not
silently erase it. On corrupt/missing/changed anchor, present **HOLD +
recovery instructions**, retain original user bytes, and never automatically
adopt a valid-looking replacement database.

## Release decision

Do **not** change `.github/workflows/release.yml` from
`--features installed` to `sync-installed-host` in B6a. The current
packaged local Desktop mode must keep working independently of unfinished S3
trust policy. Before a guarded installed release, build and test the
installer-owned anchor/broker, native two-process enrolled S3 refusal,
crash/ambiguous-ACK durable receipt, update/uninstall and Windows/macOS/Linux
end-to-end packaging. No remote overwrite, delete, auto-apply, GC or
baseline promotion is authorized by this work.

Related: #282, #283, #284; ADR-SYNC-031/034.
