# SYNC-05d2j — Explicit V2 trusted-root persistence, fail-closed

**Scope:** private app-data SQLite observation of a previously verified V2
root. This is not root writer authority, a sync baseline, File Gate write
admission, cloud GC, account signature, or a V1→V2 migration. The production
`sync-s3` feature is still opt-in, and no background scheduler is enabled.

## Why

[PR #222](https://github.com/Lalalalendia/Akula/pull/222) established a
full bounded V2 read verifier requiring an *independently trusted exact root
and opaque ETag*. Passing the latest values reported by S3 back to that
reader as "trusted" would amount to TOFU and could silently trust an
attacker-controlled or rolled-back vault. Existing `S3RemoteAnchorJournal`
stores **V1 root schema only** and must remain unchanged.

This PR adds the separate `S3V2TrustedAnchorJournal`, storing an *observation
pin*, not advancing any per-document confirmed baseline.

## Explicit genesis admission

To call `enroll_verified_genesis`, the trusted host must present:

1. A `S3RemoteAnchorScope` built with a separately verified stable
   account/deployment binding, profile ID, bucket, region, endpoint, path
   style, `VaultId`, and exact mutable head key. The scope's fingerprint
   is a non-secret context check, **not** AWS principal authentication.
2. A separately approved out-of-band **BLAKE3 of the exact V2 genesis
   root**. The code rejects digest errors, later generations, or any root
   other than the approved digest. **No first-seen cloud root auto-trust**.
   The initial trusted digest must come from a source independent of
   current, untrusted S3 content (e.g. the locally created original root
   receipt, explicitly reviewed by the host).
3. The exact V2 genesis root and opaque current ETag.
4. A `S3PinnedV2ReadPreview` produced only after a complete
   profile-bound, root-before/after-stable and BLAKE3-verified V2 reader
   run. Its private evidence is sealed to exact scope fingerprint,
   canonical root digest and ETag. Merely forging the visible counts or
   using the receipt for another root/profile fails.

Generation 2 or higher cannot enroll without a trusted genesis. There
is no cross-version migration from V1.

## Storage and restart

The registered Core's `RegisteredReadAdmission` must authorize a
separate **private app-data SQLite path outside the Markdown workspace**.
The existing guarded path validator rejects symlinks and hardlinks
(including Windows reparse points), physical root aliases and paths
inside the user vault. Use a dedicated SQLite database file, separate
from the V1 remote anchor and the normal sync journal. No AWS credentials
or note contents are persisted.

`s3_v2_trusted_roots` has an exclusive profile ID, exact vault ID, scope
fingerprint, root key, canonical root JSON, current ETag, most recent
predecessor root JSON (if any), schema version and domain-separated
BLAKE3 receipt. SQLite uses WAL, FULL synchronous writes and
`BEGIN IMMEDIATE` transaction serialization. Opening and loading
verify record format, checksum, canonical serialization, exact scope,
genesis/predessor chain and schema. Unknown/corrupt rows trigger Hold.

The checksum detects accidental data corruption. As it is not a MAC,
**a local attacker able to replace/rewrite the private SQLite database
could forge it**; this is not a hardware-backed anti-rollback anchor or
remote-writer identity. That additional threat model remains a separate
release gate.

`inspect_saved_root` re-runs the complete S3 V2 verifier against the
stored exact root and ETag. It refuses a missing/changed root, tampered
packet, switched profile, changed account scope or invalid ancestry.
It does not automatically advance a remote checkpoint.

## Monotonic advance

`record_verified_successor` requires a fresh private full-read result
for the exact proposed root/ETag and **exactly generation old+1**,
with the proposed `previous_root_blake3` matching the durable prior
root. No skipped generation, silent fork, same-generation rewrite,
ETag substitution, or cross-vault profile rebinding is accepted.
A successful update stores previous root bytes alongside new ones and
commits atomically, so reopening verifies the immediate V2 ancestor.

This is an **observation anchor only**; a winning S3 CAS/lost ACK
could still be uncertain until an independent cloud read and the
normal durable action journal's separate File Gate admission. The
journal **does not** authorize S3 PUT, delete, GC or user file mutations.

## Tests

Offline tests cover: reject missing genesis or wrong out-of-band pin,
full-read proof mismatches, crash/restart, gen1→gen2→gen3, stale ETag,
skipped generation, fork/rollback, foreign vault/account/bucket
rebinding, corrupted SQL bytes and rejection of private DB/WAL placed
inside a registered user workspace.

The existing [live V2 MinIO test](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/s3_pack_v2_minio_probe.rs)
is extended to create a registered disposable local root + separate
app-data directory, enroll a locally generated V2 genesis using its
out-of-band hash and complete cloud proof, reopen the journal, advance
after the winner's real server-side CAS and lost-ACK reconciliation, and
reopen again to verify the exact winning V2 root. After an intentional
equal-length pack corruption, `inspect_saved_root` must fail.

**Still required before launch:** actual account principal proof,
revocation/key rotation, V1→V2 migration and multi-generation
proof/missed generations, fallback for large blobs, E2EE, bounded 10k
cold packed-read A/B and WAN/AWS S3, per-document File Gate staging,
writer journal admission, remote/orphan GC and two-device release gates.
