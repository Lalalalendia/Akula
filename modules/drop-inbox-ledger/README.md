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

## Windows disposable delete integration (test only)

Core Windows tests connect actual Ledger request/finish CAS to the
RootDirectory-relative, same-HANDLE disposition probe. Synthetic explicit
confirmation must match UUID, size and BLAKE3, and the opened Ledger file's
volume/file-index/link count must match the DELETE-capable OS handle.
Failure injection covers after-request and after-disposition crash windows,
identical-byte object swapping, consent refusal, duplicate grants and
independent SQLite connections racing to remove a disposable object.
The Windows Core lab pins and records the OS identities of the Inbox root,
objects directory, and SQLite pathname before consuming removal intent;
all three must still match before DELETE-capable open. It also compares
the actual SQLite Windows VFS database HANDLE (using the documented
SQLITE_FCNTL_WIN32_GET_HANDLE) to the pinned SQLite file, before and
after CAS and before the witnessed finish. The original verified file
HANDLE stays live through Windows disposition so the file index cannot be
recycled while the one-shot grant is being consumed. New negative tests
exercise an entire objects-directory swap with byte-identical replacement,
a real exclusive-open Windows sharing denial after SQLite CAS, wrong Inbox
root, and cold reopen after a successful completion. Pre-disposition refusals preserve the bytes and remain one-shot
Requested rather than auto-retrying; a crash after disposition remains
NeedsReview with the original object absent.
This in-process VFS identity binding is not proof against malicious VFS/WAL
replacement, concurrent external writes during commit or NTFS power loss.
Unsupported SQLite VFS controls fail closed. Real Windows physical device
testing, ACL hardening and durable reconciliation remain outstanding.

These tests do not establish power-loss durability or hostile SQLite
replacement protection. The Ledger now requires an opaque
`RemovalDispositionWitness` to finish a deletion; absence of a pathname is
insufficient. Its constructor exists only under internal crate tests or the
explicit `disposable-removal-lab` dev-dependency feature. In the Core
Windows test, the witness is created only after checking the original file
identity against the digest-checked DELETE-capable HANDLE, successful
disposition, and absent name relative to the pinned objects directory.
Isolated Ledger tests deliberately synthesize witnesses to check SQL rules;
such synthetic evidence is NOT independent kernel attestation. A future
production-grade authenticated OS outcome and crash recovery protocol are
still required. GUI Delete, normal signed builds and release remain OFF.

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
