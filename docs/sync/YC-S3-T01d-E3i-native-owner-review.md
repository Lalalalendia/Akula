# YC-S3-T01d E3i — Native offline owner-review preview

**Status: research Draft.** Separate feature `yandex-owner-review-ui-preview`; not part of `installed` or default Akula. This is the **first actual settings UI**, not a trusted Vault join.

## User flow implemented

1. Open Settings → Sync in a build **explicitly** compiled with `yandex-owner-review-ui-preview`. Select the intended Yandex bucket in the existing local-only Settings field.
2. Select **Import public proposal JSON**. The OS file picker accepts a candidate JSON file; the host reads at most 4097 bytes, then E3h Core parses a schema-v1 public proposal (max 4096 bytes). Unknown fields, wrong provider, malformed keys/pins/scope, duplicate owners or a bucket different from the manually selected Settings bucket fail closed.
3. The settings page visibly labels the candidate bucket as **untrusted** and shows a newly generated 256-bit receiver challenge. This challenge is ephemeral, valid five minutes, and bound to the local in-memory candidate.
4. Send the challenge to the real owner over an independently authenticated channel; obtain the **entire 64-character lowercase comparison fingerprint** via that independent channel, not from the imported proposal or Yandex S3. The text input lives only in the open `SettingsDialog`, never in `AppConfig`, clipboard, JSON settings or logs.
5. Select **Compare code locally**. E3i consumes the one-time attempt before any comparison. A success returns **Fingerprint matched OFFLINE only. Vault remains UNTRUSTED.** Invalid input/mismatch/expiry fails and consumes the challenge. Changing Settings section or closing Settings discards the entire review.
6. This screen has **no** cloud network call, credentials access, ETag proof, trusted SQLite root+policy enrollment, File Gate or Sync writer. **It intentionally does not call E3e `ReceiverJoinSession::enroll_after_independent_owner_review_once`.** Production admission requires a *new, scope-bound E3e challenge* after verifying the real provider account binding.

## Security boundaries

- The JSON is untrusted, including its public keys, digests and VaultId. Anyone controlling both the proposal and the purported confirmation channel can produce a matching **unkeyed BLAKE3** digest. Independent authentication of the owner/channel is an actual prerequisite, not magically supplied by a hash comparison.
- UI strings explicitly say not trusted; a local match is not owner signature verification. Full DSSE/Ed25519 validation, live pinned read-only Yandex V2 GET, root/publisher SQLite atomic enrollment and anti-rollback witness are separate release gates.
- If the host crashes, reopens Settings, changes Settings section, or closes the dialog, the in-memory code and challenge are discarded.
- No user notes/files are modified, and a preview build **must not** be confused with the installed release.

## Acceptance tests

Core opt-in:
- `preview_attempt_matches_once_without_creating_an_enrollment_receipt`
- `preview_attempt_rejects_bad_import_and_oversize_without_any_network_or_disk`
- Existing E3h codec and forged owner tests remain present and exercised in the parent feature.

Native opt-in:
- `ephemeral_state_starts_without_any_stored_owner_code`
- `owner_review_preview_has_no_config_setter_or_remote_authority_action`

Dedicated Ubuntu 24.04 / Windows 2022 CI first lists all four exact test names (prevent zero-test false greens), runs the Core and Native test filters, and separately checks the real `installed` Native build without the opt-in feature. Do not mark merged/accepted before both OS workflows pass.

## Next mandatory gates

1. Implement an **owner-side authenticated invitation/QR generation** flow and recipient-side provenance check. The owner must independently sign the exact candidate values and receiver challenge; the recipient must not trust an S3-delivered keyset.
2. Bind a user-approved account/principal to the actual `S3RemoteAnchorScope`, mint a new E3e challenge and repeat the independently confirmed fingerprint before any real Yandex GET or E3c SQLite enrollment.
3. Verify Windows CredUI mask + native UIA Cancel and `storage.viewer` live Yandex read-only session in a separate disposable bucket.
4. Add an independent monotonic anti-rollback witness. Without it, complete SQLite+S3 rollback/freeze remains possible.

[Source RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1) · [E3h #519](https://github.com/Lalalalendia/Akula/pull/519) · [E3g #507](https://github.com/Lalalalendia/Akula/pull/507).
