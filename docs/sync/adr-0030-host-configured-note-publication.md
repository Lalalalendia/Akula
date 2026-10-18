# ADR-SYNC-030 — Explicit host-configured File Gate note publication admission (P0-B2)

**Status:** opt-in cooperative prototype; NOT complete cross-appdata authority or a release gate. Depends on merged #252 and #255.

## Reproduced defect

The [#251 two-process falsifier](https://github.com/Lalalalendia/Akula/pull/251) demonstrated that a second Core using a separate `appdata` can overwrite/create in a workspace enrolled by A even while A holds its external OS lock. P0-A adds an installer-owned shared SQLite registry; P0-B1 adds `with_unenrolled_epoch`, an actual cross-process BEGIN IMMEDIATE transaction held through a caller's mutation callback. Neither primitive was yet used by the normal File Gate.

## Narrow, real integration

- When explicitly constructed via `Core::open_with_shared_host_custody(appdata, events, registry)` under the `sync-s3` feature, the Core retains an **immutable host registry path** before starting DocumentHub/watchers. The caller must be trusted native host/installer code; no workspace UI preference may supply independent registries.
- Ordinary `File Gate::write` and `create` call `Core::with_shared_host_note_epoch` **before the process-wide publication lease**, and retain the host SQLite IMMEDIATE epoch across primary file mutation AND HistoryService. Callbacks, notifications, and search updates run after the epoch is released. The guarded writer route still uses its separate verified OS lock/one-use durable-intent permit.
- For configured Core, missing/incompatible/damaged shared ledger, unresolved or overlapping registered roots, unregistered target, symlink/reparse file leaf, changed physical root, or an enrolled root causes HOLD rather than unlocked write. Root volume/file identity is observed through `PinnedProbeRoot` and rechecked before positive acknowledgement.
- The shared `with_unenrolled_epoch` checks enrolled **ancestor and descendant** root paths under the same SQLite writer lock. A second appdata cannot select a nested workspace to evade an enrolled parent's path record.
- Existing `Core::open` and builds without `sync-s3` keep their prior behavior. This is intentional backwards compatibility and also an explicit **release-blocking bypass** until the installer makes a single trusted host source mandatory for all cooperating processes.

## Verifiable acceptance

Explicit `sync-s3` CI lanes run on Linux, Windows and macOS:

1. A registers protected custody in both its appdata and shared host ledger, holds a real external kernel lock; a **real child OS process B** with different appdata and explicit shared host configuration attempts ordinary File Gate write and create. Both return refusal; primary original remains unchanged and no new note appears.
2. Configured B with a missing shared host database refuses write/create without constructing a replacement.
3. An installed but unenrolled shared ledger allows ordinary note creation and modification, with persisted HistoryService.
4. A separately registered nested workspace cannot execute callbacks while an ancestor host enrollment exists.

No test here asserts the old unconfigured #251 falsifier is fixed. It must remain an explicit negative test until P0-B3/C makes the installer-host identity source mandatory.

## Known gaps (release P0)

- Native host installer is not yet wired to the explicit constructor. A new process launched with plain `Core::open` (or a different configured host ledger path) still bypasses the new epoch. Physical ledger directory authority, signed/broker identity, ACL and malicious same-UID namespace swaps are not proven.
- Only ordinary `write/create` are protected by this epoch. Binary copy, mkdir, rename, move, trash, restore, retention, direct HistoryService APIs, MCP/native adapters and raw filesystem callers still need end-to-end protection. Old primary-success/history-failure semantics are unchanged; no automatic rollback is claimed.
- Root resolution and postflight are cooperative checks, not a root-relative fully pinned capability to prevent hostile TOCTOU. Concurrent mutation/recovery across hosts and network filesystems is unsupported.
- A separate P0 must connect real protected S3 editor enrollment to the same host ledger, preserving vault/profile/physical lock binding and OS epoch ordering, before cross-appdata tests can be inverted in production.
- No remote S3 overwrite/delete, automatic local replacement, baseline promotion, garbage collection or HOLD clearance is authorized.
