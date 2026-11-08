# Akula Received Inbox ledger — opt-in Rust laboratory

This standalone Rust crate is **not installed Akula**. It models the local
receiver confirmation boundary common to P2P, S3 and Dropbox. External providers
are *untrusted* even if they report successful uploads, metadata or checksums.

`ExpectedTransfer` must be constructed only after independent Akula trust,
sender-signature / recipient / manifest verification. This crate does NOT do
signature verification, key custody, HPKE or AEAD decryption. Treat all callers
of `reserve` as inside the trusted receiving component.

## Data flow

1. Core verifies a signed, recipient-bound file manifest and obtains its transfer
   ID, sender & recipient identities, size and BLAKE3 (not from S3/Dropbox).
2. `ledger.reserve(expected)` persists an immutable intent before publication.
3. A separate **trusted no-replace file writer**, e.g.
   `p2p_inbox_lab::save_verified_stage_as_new`, verifies the stage and publishes
   exactly one object to `ledger.object_path(id)` in a private OS-owned directory.
4. `ledger.confirm(expected)` independently reopens and rehashes the local file,
   updates SQLite in a transaction, then re-opens and checks it again.
5. `ledger.status(id)` continues checking actual disk bytes on each request;
   SQLite `state=1` alone never produces `VerifiedLocal`.
6. UI may use `VerifiedLocal` to display a file in the Inbox, but sender delivery
   ACK requires additional identity signing and crash/power-loss acceptance. Vault
   import is a **separate explicit user action** through the existing File Gate.

## Run the isolated unit tests

```bash
cargo test --manifest-path modules/drop-inbox-ledger/Cargo.toml
```

Tests cover cold restart, one transfer ID used across different transports,
missing/changed files, a file written before SQLite commit, unattested orphan,
invalid size/IDs, symlink refusal (Unix), and one common ledger for all three
transport types. They do not test Windows reparse points or power-loss durability.

## Known gaps / intentionally absent features

- Not connected to installed Native Akula UI, nor to an actual P2P, S3 or
  Dropbox delivery. No sender ACK or user-facing release claim.
- Does not make the root directory OS-owned or defend against hostile ancestor
  path swap / NTFS reparse races. Those require a pinned root/ACL design and a
  separate Windows-specific admission and filesystem test.
- `synchronous=FULL` and WAL protect SQLite commits only within their
  documented guarantees. Data+directory fsync and NTFS power-loss tests remain
  independent; this is not a durable power-loss delivery receipt.
- No encryption, no remote deletion, no backend secrets and no automatic import.
- Rust compilation/tests must run in a Rust-enabled CI runner; the current
  execution environment may lack cargo. Do NOT mark any acceptance gate PASS
  until actual cargo tests and real Windows usage have been observed.

`ledger.list()` returns every registered item with an on-demand rechecked
`Pending`, `VerifiedLocal`, or `NeedsReview` status. It never authorizes a user
Vault import, and the UI must not treat it as an automatically signed ACK.
