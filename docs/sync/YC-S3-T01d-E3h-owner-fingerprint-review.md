# YC-S3-T01d E3h — independently matched owner fingerprint (Core laboratory)

**Status:** Draft, research-only opt-in `signed-vault-manual-owner-review-lab`. No default Akula native behavior, GUI approval dialog, production permissions, note changes or S3 write.

## Why this exists

E3g supplies a real, bounded, Yandex-specific read-only S3→SQLite path, but the caller still supplies `IndependentlyApprovedJoin`. Validating a signature against attacker-chosen public keys does not prove those keys belong to the expected owner. A credential-confirmation screen or S3 listing cannot establish the owner.

This step defines a **domain-separated 256-bit fingerprint** that an owner-controlled device can calculate and deliver through a separately authenticated channel. The receiving user must enter or scan that external value. Akula must **never prefill it from the candidate Vault, the signed invitation or the S3 endpoint**.

## Fingerprint v1

`blake3::Hasher::new_derive_key("akula.yc-s3.independent-owner-join-approval.v1")`, followed by length-prefixed, unambiguous public fields in this order:

1. Fixed Yandex provider, endpoint `https://storage.yandexcloud.net`, region `ru-central1`, path-style marker.
2. Exact bucket, VaultId and namespace current-head object key.
3. Full 64-character lowercase BLAKE3 genesis-root fingerprint and publisher-policy fingerprint.
4. **Receiver-minted**, fresh nonzero 32-byte challenge, lower-hex.
5. Unique independently approved Ed25519 owner public keys, sorted lexically, including their distinct-key quorum and cardinality.

The hash is a comparison code, not a MAC, signer identity, proof of freshness, secret token or authorization by itself. Two independent channels that both came from the malicious S3 provider are *not* independent approval.

## E3h network-to-trust gate

`ReceiverJoinSession::enroll_after_independent_owner_review_once(..., entered_out_of_band_hex)`:

- Spends the existing one-time challenge **before** comparing the 64-character external value, running a network request or touching SQLite.
- Rejects malformed, uppercase, truncated or incorrect independent entries. All errors consume the session; a new request requires a new owner challenge.
- Only after a full match delegates to the **same** E3g DSSE/Ed25519 owner check, Yandex-bound bounded pinned V2 GET, and E3c atomic local root+publisher policy insertion.
- In the E3h branch, the older laboratory `enroll_from_yandex_read_once` is restricted to `pub(crate)` for in-crate regression tests. External code cannot call that raw network entrypoint. The lower-level E3c laboratory enrollment API is still public behind its separate research feature, so shipping must exclude it and expose **only the reviewed host-controlled admission surface**.
- No text is saved to settings JSON, logs, S3 or user Markdown.

## Tests and acceptance

- Fingerprint binds VaultId, bucket, keyspace, full root/policy pins, owner keys and quorum, challenge; changing any one changes the output.
- Permuting the exact authorized owner key set does not change its fingerprint.
- Duplicate/all-zero keys cannot form an approval packet; malformed entries never match.
- A wrong manually entered fingerprint burns the live receiver session before any S3 request; an exact subsequent attempt fails `AlreadyConsumed`.
- An exact matching fingerprint **still cannot bypass forged DSSE invitation checks** or register the local journal.
- CI must prove that four named tests exist before executing both the E3h suite and previous E3g/E3a regressions on Ubuntu 24.04 and Windows 2022, with default Core and installed native excluded.

## Release holds

This is **not a shipping trust UI**. The app has no independently authenticated owner-channel or manual entry screen yet, and an API caller can still lie about the source of its supplied digest. A production implementation must never display the digest calculated from a candidate as the value to type back into the same application. It must compare with the value seen *on a separate authenticated owner-controlled device/channel*.

Also unresolved: real user-authorized private Yandex `storage.viewer` test, Windows CredUI masked field + Cancel, provider principal provenance, protection from a malicious complete SQLite rollback and S3 freeze/split view. Nothing enables File Gate, object PUT/CAS/DELETE, user note changes or automatic Sync.

Source [Notion YC-S3-T01d RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1). Dependency [E3g #507](https://github.com/Lalalalendia/Akula/pull/507).


## Offline proposal import for the Native review wizard

A new bounded `OwnerReviewProposal::parse_untrusted_json` accepts **at most 4096 bytes** of proposal metadata; the schema is strict `deny_unknown_fields` and version 1. It contains only public candidate information:

```json
{
  "schema": 1,
  "provider": "yandex-object-storage",
  "endpoint": "https://storage.yandexcloud.net",
  "region": "ru-central1",
  "force_path_style": true,
  "bucket": "example-private-bucket",
  "prefix": "akula/vault",
  "vault_id": "00000000-0000-4000-8000-000000000001",
  "genesis_root_blake3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "genesis_policy_blake3": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "owner_keys_hex": ["1111111111111111111111111111111111111111111111111111111111111111"],
  "owner_quorum": 1
}
```

Those values illustrate the wire format, **not an authentic owner certificate**; a valid JSON file must not ever be called trusted. The parser rejects oversized or malformed packets, unknown schema/fields, incorrect Yandex endpoint, invalid namespace, malformed/noncanonical pins, duplicate public keys, zero keys, and quorum outside the permitted keyset. Its typed `approval_for_challenge` uses an E3e receiver-issued challenge and can be fed to E3h's `verify_independently_entered_owner_fingerprint`. The proposal itself MUST NOT supply the "independently entered" fingerprint used for authorization; user must obtain that value from the authenticated owner, separate from S3 or the proposal file.

This is the first import boundary for the future Native settings wizard. It provides **neither a visible file picker nor an active enrollment UI yet**, and no signed invitation, root contents or key custody. The native controller must import the candidate, create the scope-specific challenge, wait for the owner-controlled separate 256-bit code, then verify DSSE signatures and the real pinned V2 read before E3c admission. Both Linux and Windows test the codec and its rejection cases under the existing E3h CI.
