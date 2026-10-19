# P0g-3b — strict, version-bound HistoryService evidence

**Status:** narrow internal provenance primitive and tests. Not an S3
apply coordinator, not a native OS durability receipt, not an installed-host
writer, and no change to the meaning of `ConfirmedBaseline`.

## Observed gap

The accepted Core File Gate / HistoryService bridge returns
`(FileWriteResult, history_changed: bool)`. But the real
`HistoryService::try_saved_impl` selects several distinct milestone plans:

- `Quiet` or `Skip`: often no new historical bytes, `Ok(false)`;
- `Skip` with a name: writes just a label and returns `Ok(true)`;
- `RevertToPrevious`: deletes a version and returns `Ok(true)`;
- `UpdateLatest`: mutates an existing version and returns `Ok(true)`;
- `CreateNew`: persists a new version (sometimes alongside the preimage)
  and returns `Ok(true)`.

Therefore `history_changed=true` is NOT proof of an immutable history
snapshot belonging to the exact S3 publication. A subsequent `page()`
lookup cannot retroactively attest which version was created in the original
HistoryService mutex lifetime.

## Implemented boundary

- Preserve existing public bool APIs for normal editor, installed-host and
  guarded-custody saves, including their coalescing/skip semantics.
- Internally classify the mutation as `NoChange`, `LabelOnly`,
  `Reverted`, `Updated` or `Created`.
- Add a private **sealed-custody-only** method
  `try_saved_with_custody_verified_version` for a future File Gate receipt
  writer. It can return `VerifiedHistoryVersion` only for a bounded
  **newly-created `Source::External`** snapshot, verified at the time of
  saving. Local `Source::Me` versions may be coalesced in-place later and
  are ineligible for this observation.
- While holding the existing process HistoryService files lock, read the
  exact newly generated version ID, require a regular non-symlink file and
  a single hardlink on Unix, enforce a 32 MiB cap, and verify exact raw
  bytes + BLAKE3 against the claimed saved content.
- **Before any history-store mutation**, reject quiet/no-location,
  label-only, revert and **coalesced UpdateLatest** even when these would
  ordinarily return `Ok(true)`. Rejected strict attempts may not delete,
  rename or overwrite historical versions. Normal editor paths still
  preserve legacy coalescing and revert rules.
- Reject any failed readback/mismatched hash without undoing history bytes.
  A previous primary publication may already be visible; operator review
  is still needed.

This changes no normal File Gate write, no UI/plugin/MCP entrypoint, no
S3 automatic overwrite, no OS-native receipt table and no journal schema.
No caller yet uses this new strict method. It is specifically not an
authority by itself.

**Important distinction: observed != permanently immutable.** After the
process history lock is released, an ordinary revert, history retention, or
an uncooperative same-user filesystem writer can delete or alter even an
externally sourced, newly created history file. The returned ID and digest
are a point-in-time observation, not an immutable archive or a guarantee of
future availability. A production receipt must independently retain protected
bytes/identity and revalidate evidence after restart before any remote ACK.

## CI proof

Run on Ubuntu, macOS and Windows with `sync-s3` enabled:

- Exactly identified newly-created external history version, independently
  re-read with exact bytes, length and BLAKE3.
- Strict local-user source refused before version publication, preserving
  ordinary editor saves; later tampering of a previously verified external
  history version is detected and never treated as permanent evidence.
- Normal `Skip` continues returning false, while strict mode refuses it.
- Label-only history writes cannot impersonate a newly-created snapshot.
- Coalesced latest version cannot be presented as an immutable ID.
- A forged expected hash after a real version write is rejected, leaving
  the ambiguous publication for inspection.
- All existing HistoryService tests remain green, preserving regular
  notes, milestones, retention, failed writes and transaction locking.

Run `Application/Main PR gate` and the dedicated
`Sync history version evidence` workflow on the exact head. Do not merge
a branch based on stale passing results.

## Remaining P0 barriers

The strict history identity must eventually be returned by the **same**
canonical File Gate write under external OS custody and retained closed
editor reservation (#348/#352/#358), not by a second operation. A full receipt
still requires exact S3 DAG revision, V7 immutable attempt, old/new file
identities and hashes, original recovery evidence, platform filesystem
sync outcomes, and a fresh restart verifying appdata/history/user file
together before any baseline promotion.

These tests cannot prove hostile same-user namespace races or machine
power-loss durability. Windows remains HOLD without an approved native
publication/durability strategy.
