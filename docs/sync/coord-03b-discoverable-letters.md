# SYNC-COORD-03b — anchored, discoverable V2 delta-letter hints

**Stage:** production-compiled, read-only discovery over the existing V1 S3 root. Not a publisher, signed identity, E2EE layer, File Gate apply or baseline acknowledgement. Depends on 03a pure encoder (PR #243).

## Gap closed

02c/02e fast packed delta readers accepted an arbitrary caller-supplied ordered array of opaque V2 letter BLAKE3 hashes. A separate S3 client knew the existing saved V1 root, but could not discover which letters belonged to each newly observed immutable V1 manifest generation. A fast benchmark accepting handwritten digest lists was not an end-to-end synchronization feed.

03b introduces one OPTIONAL bounded, canonical, write-once **advisory** hint object per new immutable V1 manifest at the deterministic key:

`<vault-prefix>/v1/vaults/<vault-id>/coord/letters/<manifest-id>.json`.

The key can only be derived from a V1 manifest whose ID and BLAKE3 are independently checked against the registered S3 profile's durable, private SQLite anchor and the current V1 head's exact immutable ancestor chain. The hint encodes schema=2, exact vault, generation, manifest UUID/digest and a **BLAKE3 of the immutable V2 letter**. It cannot replace `refs/current.json`, alter the V1 manifest, invent a revision, or grant a S3 or local writer.

The new `AwsS3ProbeClient::inspect_v2_offline_selective_packs_discovered`:

1. Checks scope/profile/vault and the saved independently full-pinned previous snapshot and metadata cache before reading locators.
2. Walks the bounded (at most 8 generations) verified V1 ancestor chain from the exact saved anchor, in old-to-new order.
3. Reads exactly one deterministic locator for each canonical V1 manifest (bounded to 2 KiB with SDK HEAD/If-Match GET/HEAD), rejects an unknown schema, noncanonical serialization, wrong manifest ID/BLAKE3/generation, foreign vault and malformed digest.
4. Supplies the resulting hashes to the existing 02e reader, which independently **reverifies all linked V1 manifests and their full unchanged+changed reference indices**, checks all V2 letters/packs and DAG membership, then checks the latest V1 root and ETag once more.
5. Returns only the existing advisory preview, always `requires_full_apply_gate=true`. Missing/unpublished/corrupt locator, same-generation rewrite, rollback or an altered remote root is a **HOLD**, never `no changes`, deletion or a new trust anchor.

Unchanged V1 head (zero edits) has no V2 letter; call existing zero-change V1 observation separately. No bogus generation is created to reuse this fast path. A client without a locator can separately invoke the **conservative full V1 pin**; this API never silently falls back to weaker trust.

## Real MinIO proof

The existing ignored two-client 02c/02e localhost MinIO test now writes each locator **test-only** after uploading its already verified immutable V2 letter and pack. It intentionally probes the interval between a successful gen2 V1 root CAS and locator publication: discovery MUST fail. Once gen2+gen3 locators exist, autonomous discovery must match the older manually supplied-letter 02e preview byte-for-byte, including concurrent content sibling and tombstone. An adversarial gen4 dropping the tombstone is rejected again by both paths.

The deterministic keyspace branch is strictly validated under the existing vault namespace; S3 LIST is never used as a source of truth. Pure Rust negative tests cover cross-vault/cross-manifest spoofing, unsupported schema, unknown fields, noncanonical bytes, missing predecessor/generation 1, malformed BLAKE3 and disallowed key suffixes. Existing S3/MinIO CI explicitly runs them.

## What this deliberately does not solve

An immutable locator is **not** authenticated remote writer identity, a cryptographic signature, content E2EE, capability negotiation, or an atomic transaction coupled to the V1 root CAS. A crash may leave a verified newer V1 root without its locator; the fast path will hold and the full V1 fallback must remain possible. A malicious writer with access to the bucket can cause denial-of-service by publishing a conflicting bad locator; anchored V1 membership and pack checks prevent it from authorizing a false successful delta.

The next actual **publishing** milestone must use an independently authorized registered Core source, protected OS custody, persistent cloud writer identity and crash-durable CAS intent, and publish the immutable pack + letter + locator before guarded V1 root CAS. No current code here creates that source authority or enables remote writes.
