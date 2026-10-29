# S3-E2E-01 — cloud VaultId → Native incoming-file HOLD

**Scope:** one verifiable cross-layer seam after merged [PR #560](https://github.com/Lalalalendia/Akula/pull/560), not a release of S3 Sync. Source [S3-E2E-01 Notion matrix](https://app.notion.com/p/40032a84beec8173a23ddb868408b4a6). This change addresses why a received file remains invisible in Native's recovery screen.

## Root cause

- A shared remote `VaultId` cannot equal the machine-local `workspace_id` on two independently installed clients.
- The existing default `native_recovery_overview()` selects local journal rows only when `claim.vault_id == VaultId::from_uuid(local_workspace_id)`. That is correct for existing local-only claims, but it silently selects **zero** S3 rows from an otherwise successful registered cloud receive.
- A real S3 file could therefore be present and `Staged` with an immutable local publication claim, while the existing Native overview reports `NoLocalPublicationClaims`.
- Replacing the remote VaultId with the local workspace UUID would corrupt the distributed identity. The appropriate solution is a **separate read-only S3-bound observation path**.

## Implemented

`native_s3_bound_publication_overview(core, appdata, registered_root)` is compiled only under `aquilum-core/sync-s3`. It uses the existing **host-owned, registered read-only** `s3-profiles.sqlite` binding and the canonical `sync-operations.sqlite3` journal:

1. Reopen the Core-selected registered workspace, enforce canonical app-data outside the user vault, reject symlink/hardlink databases and unsupported schemas.
2. Load the persistent remote VaultId/profile identity through the existing `SqliteRegisteredS3Bindings::open_existing_read_only` and `load_for_registered` checks. No S3 or AWS credential request occurs.
3. Examine a bounded, corruption-checked journal of one-shot local publication attempts. Only display claims matching both the bound `VaultId` **and exact `profile_id`**. Mixed or corrupted claims under the same remote VaultId result in `InspectionUnavailable`, not a "clean" status.
4. Recheck registered root identity before returning a maximum of five historical paths and exact total count. These are **historical names**, not proof that a file still contains the received bytes.
5. Leave the existing local-ID `native_recovery_overview` unchanged. No false alias between local workspace identity and cloud identity is introduced.

The opt-in Native build `s3-e2e01-review-preview` calls that new Core read-only function in Settings → Sync and shows the remote incoming file as **HOLD / manual investigation** with no action buttons. EN/RU text explicitly denies application, S3 ACK, retry, overwrite, deletion and `ConfirmedBaseline`. No default or `installed` feature activates this preview or AWS SDK.

## Cross-process proof and safety

The merged #560 MinIO test now writes the receiver's SQLite operations to the **same canonical host journal filename** used by Native instead of a disposable `ops.sqlite`.

- The child is a separate OS process with an independent Core/app-data SQLite database and pinned S3-bound registration. It receives a 1 KiB Markdown file via verified S3, then shows a **single** S3-bound review entry. The old local UUID overview remains empty, proving the ID mismatch is not being hidden by accidental aliasing.
- After the child's exit, a cold Core opened by the parent still shows that exact historical operation path and ID, while the note bytes/BLAKE3 remain correct.
- `JournalPhase::Staged`, lack of `ConfirmedBaseline`, rejection of `mark_applied`, no duplicate local creation and a single review queue entry remain mandatory.
- A separate Core unit test uses a mismatched local workspace registration and a forged profile ID to prove **no cross-workspace path disclosure or positive "clean" review**.
- A Native schema test proves a pending remote file is shown only as a status with no apply buttons, and corruption becomes HOLD/blocked rather than a success state.

Linux/Windows opt-in CI [`s3-e2e01-native-review.yml`](../../.github/workflows/s3-e2e01-native-review.yml) must verify exact test names before exercising Core and Native. Existing MinIO CI runs the real two-process fixture. These are different acceptance gates.

## Still blocked (do NOT overstate)

- No installed Native initiated download, S3 trust join, user selection of a remote note, worker, writable sync executor, historical native durability receipt or remote ACK.
- MinIO is not a real Yandex Object Storage test. Real bucket-scoped `storage.viewer`, independently authenticated owner trust and installed UI flow remain separate gates.
- User-modified notes, deletion, bulk retry, implicit overwrite, baseline promotion and automatic Sync remain forbidden.
- `S3-E2E-01` stays **Laboratory partially**, NOT `Сквозной PASS Akula`. Next milestone: Native user-initiated read-only verified preview/choice, then separately reviewed guarded apply/receipt protocol.
