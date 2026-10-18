# ADR-SYNC-034 — Shared host enrollment during verified S3 editor activation

**Stage:** P0-B4c, opt-in installed-host Core and S3 guarded-editor integration. No automatic S3 mutation, no installer rollout, no authorization to clear custody HOLD.

## Previously unsafe gap

`activate_registered_s3_editor` verified an appdata-local S3 profile, VaultId, custody store and observed external OS lock, then called only `DocumentHub::enroll_guarded_participant`. That operation persists the local participant record and sets process-local HOLD, but **does not insert into the shared installed-host ledger**. A second process with a different private `appdata`, using the same compiled host path, would see an *unenrolled root* and could acquire an ordinary shared-host epoch for file creation/overwrite. Tests that manually pre-populate the host registry do not prove this real activation path.

## Change

In the installed-host-required path of S3 guarded editor activation:

1. Verify the registered S3 profile and existing external custody identity using established `RegisteredReadAdmission` and `SqliteCustodyIdentityStore`; compare the root's physical device/file and VaultId with the S3 binding.
2. Arm durable DocumentHub prewrite and acquire the existing ordering: ordinary publication exclusive fence → process File Gate guard → guarded-editor registry guard → HistoryService participant enrollment fence.
3. **Before the appdata-local marker or publishing an active writer**, open the *existing* one installer-selected host ledger and atomically `enroll` an immutable canonical physical-root / VaultId / S3-profile / external OS lock identity. The host SQLite `BEGIN IMMEDIATE` uses the same writer epoch as ordinary host-bound note publication. No creating/replacing the ledger when missing.
4. Persist the appdata-local participant marker, then publish the active editor registry entry. If local enrollment fails **after** the host commit, leave the global enrollment intact (HOLD). There is no automatic rollback to ordinary mutation.
5. An already active identical configuration must re-check host enrollment, not early-return based only on the process's in-memory editor list.

## Tests

A real S3 profile+custody fixture activates an installed-host Core **without any manual call to shared `host.enroll`**. After activation, a separately launched OS process with a different appdata and the same host ledger must refuse note overwrite/create/direct HistoryService, preserving original bytes. Cold readback of the host row checks immutable VaultId/profile, and missing ledger never silently bootstraps. Run on Linux and Windows/macOS `sync-s3` CI.

## Boundaries that remain P0

- This integration applies **only** when `Core::requires_installed_host_epoch` is true. Legacy/default `Core::open` with separately opt-in `sync-s3` guarded editor still has no mandatory global host source and must not be considered safe for S3 protected writes.
- The installed-host guarded editor's actual File Gate writer still rejects until a fully verified host-owned guarded write exception and durable receipt are implemented. The host ledger **records HOLD; it is never itself write permission**.
- Installer has not provisioned/pinned a shared OS-trusted SQLite/WAL/SHM namespace with stable physical identity, owner/ACL, ancestry, and crash recovery. Real directory swap/hostile same-UID filesystem rewriting remains outside this stage.
- Incompatible old processes can still write; #251 product release gate remains open. Standard installer still selects `--features installed` without mandatory host. Other native/MCP/File Gate write mutations and recovery must be verified.
- Never enable remote S3 PUT/DELETE, baseline promotion, GC, automatic sync, or silent HOLD clearance based on this PR.
