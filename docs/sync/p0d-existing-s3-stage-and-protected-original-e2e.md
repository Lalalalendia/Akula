# SYNC-APPLY-P0d — Joined real MinIO existing-revision stage + original protection

**Status:** opt-in, read-only workspace preparatory bridge. No native file
replacement, S3 apply, baseline advancement, or remote write authority.

## Why this change

#316 confirmed that an actual S3 V1 two-revision fast-forward could be
materialized and recorded in private WAL/FULL state as `StageDownload /
Staged`. #300 separately captured a pinned local original with the real
registered Core and OS custody, but its fixture supplied synthetic staged
bytes rather than a remote transfer. Neither test demonstrated the same
operation moving across BOTH established gates.

## Implementation

`stage_and_protect_verified_existing_s3_read_only` composes the two
existing implementations without adding a duplicate parser, new local
writer or journal advancement:

1. Verify registered S3 profile, complete immutable DAG, existing committed
   baseline/path, stable remote HEAD, content length, immutable BLAKE3 digest
   and bounded upload via the actual localhost MinIO transport; store only
   private `Staged` plus verified cached bytes.
2. Independently recheck the same journal operation and approved profile,
   hold the already bound external OS custody lock, and persist the original
   local bytes/physical file identity into host appdata SQLite WAL+FULL.
3. Return only stage and protected-original diagnostics. No result from this
   function is a File Gate permit, history receipt, a per-document baseline
   commit, or safe authority to retry after a crash.

The second step can fail **after** stage has committed: retain the verified
cache and uncommitted `Staged` journal as inspection evidence. Never infer
an applied file, auto-retry a publication, or mark a remote revision
confirmed. A remote HEAD may change again between staging and later use:
the future writer must revalidate ancestry/HEAD, exact local preimage,
editor state, root/custody IDs and installed-host admission.

## Integration acceptance

The previously existing two-revision `live_minio_existing_s3_fast_forward_stages_without_local_mutation`
test is extended instead of adding an overlapping test. It uses actual
SDK/MinIO writes and readback with approved binding, then:

- simulates a concurrent local edit after staging; protected original
  capture must fail and leave no preserved receipt;
- verifies journal stays `Staged`, baseline remains unchanged, and the
  intervening local bytes are untouched;
- restores the confirmed local bytes, stages idempotently again, and saves
  the original under a real external OS lock;
- reopens the WAL/FULL protected-original store, checks exact bytes,
  source path/hash and journal identity; confirms the local Markdown and
  baseline did not change;
- confirms the generic `mark_applied` continues to deny StageDownload
  with `LocalDownloadNeedsNativeReceipt`.

MinIO test is invoked by the existing workflow's isolated
`live_minio_ -- --ignored --test-threads=1` command; no duplicate bucket
initializer or second workflow.

## Next blocked gate: *actual* existing-file apply

A separate PR must implement once-only durable update intent, File Gate
CAS under shared host custody, non-symlink descriptor-relative atomic
replacement, HistoryService transaction evidence, parent/target fsync,
crash and lost-ACK HOLD/NeedsReview, and a native publication receipt.
Only after independent readback and cross-process restart acceptance may
the per-document journal/baseline advance. Windows/macOS/Linux installer
flavors and malicious same-user protection remain separate products.

No public `--features installed` release switches in this PR.
