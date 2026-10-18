# ADR-SYNC-028 — Shared host custody enrollment, P0-A

**Stage:** internal registry primitive, not yet an activated writer gate. This is a prerequisite for the fail-closed alternate-appdata remediation specified in [Tela P0](https://telawiki.com/spaces/456/pages/7548/akula-sync-p0-custody-appdata) and empirically demonstrated by merged [PR #251](https://github.com/Lalalalendia/Akula/pull/251).

## Confirmed unsafe baseline

A process with appdata A persists an S3 participant and holds its external kernel lock. A second independent process opens the same physical workspace through a separate appdata B. The current ordinary File Gate and direct HistoryService consult only B's private enrollment. B can overwrite A's protected note despite A's HOLD. PR #251 intentionally asserts that unsafe result; until P0-B/P0-C are complete, never treat its green CI as a release permit.

## Installer-owned identity source: implementation P0-A

`sync::host_custody_registry` is compiled only with the opt-in `sync-s3` feature. Unlike a private workspace UUID, an entry keys the actual physical root as `device:file`, and keeps the verified root path, exact remote VaultId, S3 profile, shared external custody scope and physical OS lock identity in a single insert-only WAL/FULL SQLite record. Duplicate exact registration is harmless; a different VaultId/profile/path/physical lock for the same root is refused. A replaced directory that retains the same path but gets a new physical identity is refused.

- **Trusted installation only:** `SharedHostCustodyRegistry::provision_new` requires a pre-existing host-owned directory outside the workspace and all private appdata supplied by the installer. Exclusive `create_new` never truncates or silently repairs an old database. On Unix, its SQLite file is initially owner-readable/writable (0600). The real installer must separately enforce its platform ACLs and bind this host location into every cooperating Core.
- **Runtime must open existing:** `open_existing` never creates a database or bootstraps an empty schema. It rejects missing path, symlink leaf, unsupported schema, SQLite corruption and malformed table. `inspect` checks canonical stored JSON and BLAKE3 against the matching root key AND previous path. Missing record means only *not yet enrolled at the instant observed*, **not permission to write**.
- **Integrity is not authentication:** BLAKE3 and SQLite FULL provide accidental damage detection and durable ordering, not defense from malicious same-UID database replacement. File descriptor identity and trusted installer source remain separate tasks.

## Proof / negative cases

An explicitly installed host database is reopened by a logically independent appdata reader without consulting A's SQLite or local workspace UUID. It sees the same enrollment. Zero/unknown identity and changed root physical IDs are refused. Mismatched vault/profile/lock re-enrollment is refused. Missing database, unexpected schema and tampered digest fail closed rather than becoming a new empty registry. The S3 feature CI runs `host_custody_registry::tests` independent of AWS credentials or MinIO.

## Cross-platform ledger acceptance corrections

The first exact-head CI rejected three assumptions rather than proving writer authorization. The shared ledger validates **every** durable row during `open_existing`, so a tampered checksum must fail at open instead of letting an invalid handle reach `inspect`. A root name must be canonicalized at lookup: macOS may resolve `/var` to `/private/var`, and Windows may normalize `C:\\...` into the `\\\\?\\C:\\...` namespace. This must not hide an existing enrollment after the physical directory is replaced. An alternate pathname in a negative enrollment test must also be an **absolute host-native path** on Windows.

Added explicit regressions for both path spellings and an indexed table missing despite `user_version=1`. An invalid shared DB must remain on disk for investigation; it is not automatically replaced with a fresh empty host registry.

## Critical missing integration — do not mark P0 resolved

This PR deliberately changes **no** `Core::open`, ordinary File Gate, HistoryService, DocumentHub activation, native installer or user sync action. Therefore the existing #251 two-process unsafe bypass is still reproducible, as it MUST remain until an actual positive fix is accepted. There is no global host choice or install credential attestation merely because a DB primitive exists.

Next P0-B acceptance must:

1. Bind **one** installer-trusted registry path into all independent Core processes before exposing registered workspaces or permitting any File Gate/HistoryService mutation. A new appdata may not silently choose its own registry.
2. Serialize **both** enrollment and ordinary file+history publication under a shared kernel OS epoch so a reader cannot observe absent registry before an enrollment that later races its write. Identity checks must use pinned OS objects, not pathname-only metadata.
3. Refuse on missing/corrupt/changed host registry after activation; no blind rebootstrap after lost acknowledgment, no bypass for alternate appdata, direct history, MCP/native, move, trash, restore or post-restart maintenance.
4. Invert the real two-process #251 test: B ordinary write and create MUST fail with no changes to bytes/history while A holds custody. Extend the same proof to rename/trash/restore/direct HistoryService and crash+restart on Windows, macOS and Linux.
5. Never treat this registry's presence as write authorization: guarded writes still require the existing host-owned one-shot intent, pinned OS lock, exact profile/VaultId, no open editor and durable recovery journal.

Separate threats remain: hostile processes with access to the host-owned registry, hardlink/symlink and host directory replacement, Windows ACL/owner authority, cross-user installation, remote writer authentication, CAS lost-ACK, document baselines, GC and remote deletion. Avoid claiming an end-user S3 sync until the full safety chain is proven.
