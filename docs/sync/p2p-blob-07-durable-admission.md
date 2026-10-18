# P2P-BLOB-07 — durable staging reservation journal (Linux research only)

**Status: experimental, stacked on [P2P-BLOB-06 #314](https://github.com/Lalalalendia/Akula/pull/314) and P2P-BLOB-05 #301.** No Akula workspace files, File Gate, native desktop, S3 V1/V2, or Roha/noq forks have been changed. Do not merge into production.

## Why P2P-BLOB-06 is insufficient

The existing StageBudget atomically reserves declared bytes and slots in one process. Reservations disappear on SIGKILL, while previously accepted FsStore data/outboards can remain on disk. Reopening a fresh empty StageBudget without reconciliation can let new peer traffic oversubscribe the device.

The P2P-BLOB-05 SIGKILL test safely re-fetched an entire 8,388,621-byte file after losing previous partial-range state, but this is not a disk capacity guarantee.

## Implemented: fail-closed single-writer Linux reservation journal

The new independent module is staya/roha-transport/src/durable_admission.rs. It is not a replacement for an OS quota.

- Requires caller-created private **0700 metadata directory** separate from FsStore and workspace. Rejects symlink/unsafe metadata, unexpected files, linked owner locks and unsupported schema.
- Holds an advisory OS exclusive lock using Rust 1.91 standard-library File::try_lock for its entire lifetime, so a second cooperating process cannot start an independent ledger. [Rust API](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock) is stable since 1.89.
- Writes bounded (maximum 256 records) binary snapshots with magic, version, generation, exact local limits, content BLAKE3, EndpointId, expected byte lengths and per-record verified state. A trailing BLAKE3 checksum detects corruption; invalid or inconsistent snapshots, duplicate keys or changed policy fail closed.
- Persists **before** any network GET or staging writer. Every retained reservation remains accounted after SIGKILL/restart regardless of whether its corresponding content is partly received, complete or not yet verified.
- Commit: create-new temporary file, exact write, sync_all file, atomic rename to snapshot and sync_all parent directory. Leftover uncommitted next-file, uncertain commit or failed sync prevents further admission.
- Verification compares exact expected length and full BLAKE3 and then commits verified state. This cannot authorize File Gate publication or free a reservation.
- Release first requires an externally owned callback to delete and sync **all** backing staged content, then persists the ledger reduction. Callback error leaves reservation intact. Trusting callback is a separate custody obligation, not an intrinsic hard disk quota.
- Restart never silently discards abandoned reservations. Damaged metadata requires explicit operator/custody reconciliation, not a fresh empty ledger.

## Validation and reproducibility

New direct Rust tests: clean reopen retains verified and incomplete reservations; byte ceilings persist after restart; simultaneous owner lock is denied; corruption, orphan pending snapshot, incompatible limits, unexpected files, oversize and duplicate objects fail closed; failed cleanup callback retains reservation. A separate OS process durably reserves quota, is killed by Linux SIGKILL without clean shutdown, then the next process sees the same reserved byte count and refuses oversubscription.

A direct Roha/noq QUIC test reserves in the journal **before** networking, uses patched iroh-blobs 0.103.1 to GET a known hash, verifies length+BLAKE3, records verified state, reopens ledger, denies a competing reservation, and removes its disposable physical file before releasing the ledger. Only private GitHub runner/temporary staging; no user data.

All earlier GET/GET-MANY/OBSERVE/PUSH refusal, SIGKILL content recovery, fork pinning, formatting and docs must still pass on the latest exact head. This lab adds no new crate dependency.

Reproduction from a checked-out pinned source tree:

    cargo test --locked --manifest-path staya/roha-transport/Cargo.toml -- --nocapture

## Non-goals / remaining production gates

**This remains a bookkeeping reservation, NOT a hard physical quota.** It can undercount Bao outboard proofs, DB metadata, write amplification and untracked files, and cannot stop a non-cooperating local process from writing. No claim of power-loss/torn-disk durability is made; only Linux process SIGKILL and clean flush barriers are exercised. The convenient in-memory GET fixture is not a production streaming-memory limit.

The independently authorized VaultId, RevisionId, DeviceId to EndpointId grant, and known hash+length are prerequisites, NOT implemented here. No File Gate, S3, protected original files, Windows/Android support or peer enrollment are changed.

Next gates: physical OS-enforced bounded staging volume including all FsStore overhead; trusted startup reconciliation of ledger plus physical staging; actual bounded streamed bytes and cancellation; explicit signed peer grants; two physical devices on direct LAN and own relay with controlled latency and loss. Keep all experiments in draft stacked PRs.
