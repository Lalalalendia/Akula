# ADR-SYNC-007 — Local forensic archive sealing contract (M1d-f1)

Status: **PROPOSED CRYPTO PRIMITIVE ONLY**. This ADR documents an inert, independently testable building block. It does **not** imply archived recovery data is currently encrypted.

## Why this exists

SYNC-04g-M1d-b currently retains immutable stage and conflicting-final copies as **plaintext BLOBs** in host-owned SQLite with WAL/FULL. M1d-c and M1d-e review only verified fingerprints and record non-destructive decisions, but they cannot erase the confidentiality risk of these indefinitely retained copies. Locally owned Markdown notes, document attachments, backup snapshots, SQLite pages and WAL can reveal the user's content if app-data is accessed by another actor.

The full SYNC PR-06 encrypted *cloud* profile is a separate planned capability. Local forensic archive confidentiality is an additional requirement: whether the user chooses plaintext or encrypted cloud Sync must not inadvertently expose their local recovery archives.

## Proposed inert format

The new `aquilum_core::sync::sealed_archive` contract implements a versioned **AES-256-GCM** authenticated encryption envelope using the pre-existing locked `ring 0.17.14` crate:

| Field | Bytes | Meaning |
| --- | ---: | --- |
| `AKARV1\0\0` | 8 | fixed format marker; unknown formats rejected |
| nonce | 12 | OS CSPRNG generated fresh per encryption |
| body | 0–33,554,432 | encrypted plaintext (can be empty) |
| tag | 16 | AES-GCM authentication tag |

The **associated authenticated data** (AAD) is domain-separated and canonically encoded as: `akula:sync:recovery:archive:aes256gcm:v1\0` + 16-byte vault UUID + 16-byte operation UUID + 64-bit LE path length + exact original UTF-8 historical relative path + 1-byte archive slot. Slot is `Staged=1` or `ExistingFinal=2`. This blocks cross-vault, cross-operation, cross-path and stage/final copy swaps with the same key. AAD does not appear in ciphertext: it MUST be derived from a trusted registered Core workspace and immutable verified operation intent, not remote-supplied labels. Rollback of a whole old encrypted row remains outside this primitive's protection.

The contract requires an externally supplied **high-entropy 256-bit key** through `RecoveryArchiveKey`. The key is zeroized on Drop through `zeroize`. It has no Debug or public key export API, no password derivation, and no key storage. It is forbidden to derive an encryption key from an ordinary user password without a separately reviewed KDF policy. Per-vault OS keychain/TPM-backed persistence, recovery keys, loss-of-key UX and device migration are unresolved dependencies. Random 96-bit nonces avoid deterministic ciphertext; design requires key lifecycle and per-key message count limits before activation.

## Explicitly NOT activated

- Neither `SqliteSyncJournal::record_local_recovery_copy_once`, its historical verification reads, nor the UI review path is changed by this PR. **New real recovery archives still follow legacy plaintext behavior** until a separate storage-schema and key-custody gate. This PR MUST NOT be described as closing the confidentiality blocker.
- No automatic plaintext-to-ciphertext migration is allowed: SQLite UPDATE does not securely wipe old pages, WAL files, backups, snapshot copies or SSD wear-leveling areas. A future migration must preserve all originals, fail closed when keys are unavailable, prove durability and confidentiality, and require a separate privacy-safe retention policy.
- Ciphertexts do not prove who created a same-hash filesystem object, prevent same-user host malware from reading documents or prove actual Windows power-loss directory durability. No restore, physical quarantine, deletion, retry, cloud baseline commit or production Sync activation is authorized.

## Test and acceptance evidence required

Run `m1df1_*` on Windows, macOS and Linux, as part of full exact-head CI. Scenarios: empty/UTF-8/binary round trips; independent random nonces; incorrect key, nonce, ciphertext, tag, magic/version and truncation; swapped vault, operation, historical path or stage/final role; reject payload above 32 MiB. No plaintext fallback, exceptions, key material or content logs.

### Follow-up PRs (each independently tested)

1. Host key management: OS-backed key custody, per-vault separation, bootstrap/recovery and safe failure when locked or absent.
2. Encrypted SQLite format: versioned sealed-copy rows, strict copy-once checks, authenticated readback and review fingerprint compatibility. Never duplicate a plaintext copy during the new path.
3. Legacy migration and rollback: secure consent flow, persistent privacy classification, durable separate snapshot and explicit warning that historical SQLite/WAL/backups may retain plaintext. Do not silently purge the only recoverable user bytes.
4. Local app-data ACLs/OS handle identity, confidential diagnostic export policy, guarded retention and operator-authorized purge. These gates must precede a user-facing activation.

Canonical project reference: [SYNC PR 00–11](https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11).
