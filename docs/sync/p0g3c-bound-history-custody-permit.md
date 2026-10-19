# P0g-3c — bind the custody history permit to the exact Core and note

**Status:** narrow fail-closed hardening of existing S3/custody internal bridge. It is **not** a native receipt, S3 apply coordinator, remote baseline advancement, OS durability claim, or host installer authority.

## Prior gap

P0g-3a File Gate accepted a non-Clone but unbound `HeldCustodyHistoryPermit(())` that was minted inside the held external OS lock. P0g-3b added a strict `HistoryService` witness API but the parameter `_permit` was ignored. Future integration could therefore accidentally pass a permit created for one Core or note into another. A rejection only in HistoryService would be too late: the primary file might already have been replaced.

## Narrow change

- Only the existing guarded writer can create the sealed permit, within the checked, single external OS-custody callback and retained closed-editor reservation.
- Borrow the **exact registered `Core` and checked target `Path`** in the non-Clone permit. Do not use a reusable UUID or process-global boolean as authorization.
- At the start of the **existing Core File Gate** checked writer, before prewrite and `write_file_atomic_impl`, reject mismatched Core pointers or note paths. This is pre-primary and does not emit success.
- In both normal and strict HistoryService custody APIs, independently require the permit's Core to own **that exact `HistoryService` instance** (pointer identity) and the identical note path, before any history mutation.
- Preserve existing successful guarded note writes, OS-lock ordering, and history notification **after** OS unlock. Do not change ordinary local editing or public HistoryService bool semantics.

## Negative acceptance

Tests forge a permit only within this white-box test module (not via a production constructor) and attempt wrong-Core and wrong-note File Gate writes. Both must reject before the prewrite callback and leave primary bytes unchanged. A permit from Core A must also be denied by Core B HistoryService, in both legacy and strict APIs, and by Core A for a different note, before any version is created. Existing positive OS custody/HistoryService tests remain required on Windows and Linux.

## Explicit remaining HOLD

The permit is a lexical cooperative-process capability: it is **not** a proof against an uncooperative same-UID writer, a portable atomic inode compare-and-swap, or a crash-proof operation log. P0g-3b's version proof remains point-in-time. A later P0g-3d must combine independently retained protected original, File Gate primary/postflight identity, HistoryService version and SQLite V7 in a native power-loss-safe receipt, with cold-start recovery. Until then `StageDownload::mark_applied`, remote ACK and `ConfirmedBaseline` promotion remain disallowed.

Platform acceptance: Windows and Linux. macOS frozen per [MAC-HOLD-01](../platforms/MAC-HOLD-01.md).