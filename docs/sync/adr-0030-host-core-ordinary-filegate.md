# ADR-SYNC-030 — Host-bound Core ordinary writer slice (P0-B2a)

**Status:** internal, explicitly selected cooperating-host mode. NOT the default Core launcher and NOT the P0 release gate.

## Purpose

P0-A (#252) installed a shared cross-appdata custody ledger. P0-B1 (#255) made enrollment mutually exclusive with a held ordinary write/history publication through the SAME host SQLite `BEGIN IMMEDIATE`. The missing link was real File Gate participation, not another synthetic transaction test.

This stage adds `Core::open_with_installed_host(appdata, host_db, events)` for a **host-provided** shared database. It is deliberately separate from unrestricted legacy `Core::open`. The native installer/broker is still responsible for selecting and pinning one machine-wide trusted source. A path furnished by user-editable appdata or environment variables is NOT authoritative.

In host mode:

- Note `gate::write` and `gate::create` acquire the local ordinary publication lease, pin the unique UI-registered workspace's OS root identity, reject symlink escapes/aliases, and take `SharedHostCustodyRegistry::with_unenrolled_epoch` against the same SQLite file as enrollment.
- The sealed `HostUnenrolledEpochPermit` exists only within that held transaction. Its Core-bound File Gate mutex exception permits the primary write. Its matching HistoryService exception permits the corresponding history snapshot *under the same host epoch*. Callbacks/search run only after the SQLite transaction is released.
- Refuse when local appdata itself is enrolled, when the host registry is missing/bad/conflicting, when the physical root is replaced, or if a different Core presents the permit. An ambiguous post-operation root/transaction failure is `NeedsReview`, never an invitation to blindly retry.
- While holding the host SQLite writer epoch, also reject a registered **ancestor or descendant** root. A second Core can select a physically different nested workspace with separate appdata; an exact root-ID lookup alone would otherwise miss the enrolled parent. Linux/macOS/Windows host tests cover the real nested File Gate create refusal and a transaction-level callback that must not execute.
- Host mode blocks all other ordinary File Gate routes via `file_mutation_guard_for_write`, and denies direct HistoryService writes via an installed-host-required flag bound at Core construction, before workers start. `guarded_editor` cannot bypass the new ordinary host path in this stage.
- `Core::open` remains unchanged; the separate-appdata bypass from #251 is STILL possible with that unbound entrypoint. This is a working, narrow adapter awaiting mandatory product startup/installer integration, not a shipped fix.

## Verification

Real child OS-process test: A registers the physical vault under a shared host DB and holds its independent OS custody lock while B starts with an entirely separate appdata **but the same host source**. B `write`, `create`, binary create and direct history return denial with original bytes untouched. Independent test: before any enrollment, host-mode `create` then `write` succeed with the full primary+history epoch, while unrelated File Gate and direct history remain denied. Missing host DB never silently bootstraps. Run Linux/MinIO and Windows/macOS matrix.

## Explicit remaining release blockers

1. Native/Core startup must make the same installer-attested source **mandatory for all cooperating Core processes**, including alternate appdata. Keeping legacy `Core::open` writable means #251 has **not** been inverted for the product. Missing/changed host identity must HOLD, not silently fall back.
2. All remaining write routes require equivalent host epoch and history coverage: binary, copy, rename, move, trash, restore, folder tree, purge, direct HistoryService and native/MCP operations. Multi-root actions need deterministic acquisition order and one durable recovery plan.
3. Native OS/installer must attest pinned host DB path, owner/ACL and stable file identity (including WAL/SHM), and block split-brain if namespace replaced. `BEGIN IMMEDIATE` is a cooperating-process mutual exclusion contract on supported local filesystems, not protection against hostile same-user filesystem writers.
4. Full two-process #251 test must assert **false|false** with normal product Core, not only the explicitly bound constructor. Add forced process kill/restart, lost ACK, schema replacement, symlink/junction and inode swap tests. Keep original/HistoryService durability receipt gates.
5. This grants no S3 PUT/DELETE, CAS writer, remote baseline, GC or automatic sync. Remote AWS identity proof (#249) does not substitute for these local guards.

Do not merge or activate without exact-head Rust/S3 and macOS/Windows custody CI green.
