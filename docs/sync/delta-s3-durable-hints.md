# SYNC-DELTA-01i — durable, profile-bound metadata hints across app restart

**Stage:** opt-in, read-only metadata cache; no scheduler or local File Gate writes. Implemented after the [10k-note S3 A/B](https://github.com/Lalalalendia/Akula/actions/runs/39075239066) showed the importance of avoiding old-object re-downloads. The benchmark excluded expensive cold cache materialization. This stage addresses that **specific cold-start gap**, without widening trust.

## Data and provenance

Two **separate private app-data SQLite files**:
- Existing `S3RemoteAnchorJournal` stores a *verified remote observation* (profile/bucket/account assertion/endpoint/region/path-style/prefix/VaultId, root ID/hash/ETag, canonical root bytes). It is not a per-document ConfirmedBaseline.
- New `S3VerifiedHintsJournal` stores a **metadata-only previously fully-pinned hint cache**, bound to the current observation's same profile fingerprint, vault ID and exact `manifest_id:blake3` snapshot ID. A header stores revision count, total raw JSON byte count and an unkeyed domain-separated BLAKE3 receipt; child rows store immutable revision ID, canonical `S3RevisionRefV1` JSON and exact checked revision JSON bytes. No user Markdown body, S3 access keys, password or blob bytes stored.

Do not place either SQLite file inside the vault; application must supply a protected private app-data path. Revision JSON can still contain user note relative paths, hashes and structural history, so its contents are metadata with privacy implications.

## Write path (never implicit)

`S3VerifiedHintsJournal::persist_after_full_pin(scope, anchor_journal, &fully_verified_view)`:

1. Load and revalidate the profile-bound old remote observation anchor and its canonical manifest.
2. Require the input `S3VerifiedRevisionMetadataStore` to have passed full DAG verification in the canonical materialization pipeline. Demand same exact vault ID and `manifest ID:BLAKE3`, and exactly the number of revision references in the anchor. A manually assembled arbitrary view cannot use public constructors.
3. Enforce at most **32 MiB raw JSON cache bytes**, **32,768** revision records, **64 KiB** per record, **512 bytes** per canonical ref; precompute a domain-separated BLAKE3 receipt over context+ordered row bytes.
4. `BEGIN IMMEDIATE` (separate SQLite), `PRAGMA synchronous=FULL`, foreign-key cascade, single transaction replaces header plus all child rows atomically. A crash leaves either old or new cache, never a partly published record. If a profile ID/vault ID is reused with a different context, **fail closed**, never silently replace.
5. Verify the remote anchor has not advanced after committing. If another process advanced it independently, the saved old cache remains inert and future loads reject its mismatched snapshot.

This method does **not** advance the remote observation anchor or write to S3; all network verification happened earlier during the full pin.

## Recovery path

`AwsS3ProbeClient::inspect_selective_from_persisted_hints(scope, remote_anchor, hints_journal, limits)`:

1. Compare the **actual configured S3 SDK client** (bucket, region, endpoint, path style, exact vault root) to the supplied scope *before any network I/O*.
2. Load the S3 remote anchor from the separate journal. If missing/corrupt/foreign => explicit Hold.
3. Load the metadata hint header. If absent => explicit MissingCache (no silent empty vault). Require exact snapshot binding and bounded record count/bytes.
4. Preflight SQLite COUNT, SUM(json bytes), MAX per-row JSON/ref lengths in SQL **before materializing BLOBs**, preventing unbounded allocations from a corrupt/maliciously damaged file.
5. Reconstruct `S3VerifiedRevisionMetadataStore` **privately** by parsing exact canonical reference JSON, checking each revision UUID, BLAKE3 over revision JSON, revision semantics, duplicate IDs, total bounds, receipt and **full DAG closure**, including tombstones and both sibling offline heads.
6. The reconstructed cache never escapes as a public RemoteStore; feed it only to already merged `inspect_selective_s3_revision_changes`, which walks mutable S3 head with ETag/race checks and verifies any *new* objects. The final return is again an **advisory `S3SelectiveRevisionPreview` only**.

## Security and limitations

- Old blobs are **not re-downloaded on cold app restart**, meaning old object availability has **not** been proved current. A rogue S3 operator can delete them, so this is never a fresh full pin, native file-write receipt, conflict-resolution decision, auto-delete permission or GC authority.
- BLAKE3 is **not a MAC** and SQLite is not rollback protected against a malicious local actor replacing both databases. Independent writer authentication and trusted anti-rollback remain future prerequisites if this state is ever to influence File Gate.
- The calling app still must decide when a fully verified cache should be persisted and when cold recovery should be attempted; this PR does not activate a background sync scheduler/UI or Google Drive/Yandex adapters.
- Only immediate 1-generation selective advancement is supported. Missing more generations still returns `VerifiedFullHistoryRequired`, to avoid missing an intermediate sibling or tombstone.
- `S3VerifiedRevisionMetadataStore` is created in memory on recovery, so peak RAM remains bounded by 32MiB raw JSON plus structures. DB on disk also contains row/index overhead; it is not encrypted at rest by this stage.
- Verified metadata rows are only *read-only hints*. A complete S3 availability pin and File Gate remain required before any protected user file mutation.

## Acceptance

- Unit tests: empty genesis cache idempotent storage/reload, cache bound to valid remote observation only, wrong bucket/account assertion => refusal, checksum tamper, malicious oversized BLOB, missing/empty cache, strict budget and older cached snapshot following remote-anchor generation advance => refusal.
- Real **disposable localhost MinIO** extension to `live_minio_selective_one_edit_reuses_unchanged_revision_and_blob`: full genesis pin, save profile/vault-bound SQLite anchor + hint cache, reopen databases, discard in-memory cache, publish one changed revision and content blob, call **new disk-backed selective read** and verify only new immutable objects were accessed and exact old snapshot preserved; no user file write.
- Run `--features sync-s3 verified_cache_disk::tests`, existing `live_minio_` and all Rust/OS safety gates on exact final PR commit before merging.
- Cold-launch **performance figures** need a new measured variant; do not reuse the 10k warm-memory cache comparison as evidence for cold-start speed until an actual cold benchmark completes.

## Follow-on roadmap

1. Bench 10k app restart: previous fully pinned cache in new SQLite, drop memory, reload and measure SQLite CPU/RAM and actual subsequent HEAD/GET/latency with 0/7/1000 edits vs fresh network pin.
2. Validate disk cache confidentiality and profile/account identity handling before turning on native storage adapters.
3. Prove immutable multi-generation retention for extended offline history; longer offline chain remains a Hold until this is done.
4. Full production File Gate and S3 provider staging must remain independent security gates.

References: [Notion SYNC-DELTA-01](https://app.notion.com/p/3fc32a84beec81a2aca9c5b384fcb360), [Tela](https://telawiki.com/spaces/456/pages/7501/sync-delta-01-shard-reuse), [PR #175](https://github.com/Lalalalendia/Akula/pull/175), [PR #181](https://github.com/Lalalalendia/Akula/pull/181).
