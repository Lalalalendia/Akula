# P0g-3d-01: byte-level custody, publication and crash-window audit

**Status:** source-level audit completed; **not** a durability proof or permission to enable StageDownload apply.
**Source baseline:** Akula main `2c1d0e954affd96ccc12caffa570e5c9f278487e` (accepted #385).
**Target:** Windows + Linux. macOS remains Frozen/P2.

## 1. Actual pipeline and evidence boundaries

This path comprises *separate* durable domains, not a single ACID transaction:

```text
Verified S3 staged bytes + confirmed baseline
  -> registered Core / closed-editor admission / held cooperative OS custody
  -> private protected preimage (SQLite DB A; WAL + synchronous FULL)
  -> V7 one-shot existing-replace claim (SQLite DB B; WAL + synchronous FULL)
  -> File Gate mutex + exact Core/path permit + prewrite source identity
  -> single AtomicWriteFile primary publication (workspace filesystem)
  -> HistoryService version write (workspace .aquilum/history)
  -> optional postflight pinned byte/identity inspection
  -> [NO durable receipt / NO remote ACK / NO ConfirmedBaseline promotion]
```

Existing helpers are **preparatory or internal**; they are not a wired production S3 executor. This is a source-level order of *available* operations, not a claim that one production code path currently runs every step.

| Boundary | Inspected source | What the current code actually proves | What remains unproved |
|---|---|---|---|
| Protected original | [protection_store.rs L79–214](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/protection_store.rs#L79-L214) and [s3_preimage_capture.rs L131–215](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/s3_preimage_capture.rs#L131-L215) | Up to 32 MiB of BLAKE3-checked original bytes, operation/path + root/file identity, stored and re-read in a private SQLite BLOB, with WAL/FULL | Atomicity with another DB, primary inode, or HistoryService; physical device/power-loss guarantees |
| One-shot attempt | [journal.rs L319–325, L528–548](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/journal.rs#L319-L325) and [existing_replace_attempt.rs L48–163](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/journal/existing_replace_attempt.rs#L48-L163) | Schema V7, `BEGIN IMMEDIATE` + conditional `INSERT OR IGNORE`, unique prior-baseline/path claim, operation/intended bytes and original identity; refuses replay after an ambiguous commit | Primary publication, HistoryService version, native receipt, persisted state transitions past claim |
| Admission / lock | [s3_reserved_replace.rs L51–176](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/s3_reserved_replace.rs#L51-L176) and [guarded_note_write.rs L185–265](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/guarded_note_write.rs#L185-L265) | Core-specific closed-editor reservation, physical original check, single cooperative custody lifetime and postflight hooks; post-preparation failures become `NeedsReview` | Exclusion of non-cooperating same-UID writers or a kernel compare-and-swap on the destination inode |
| File Gate publisher | [gate/write.rs L146–194](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/files/gate/write.rs#L146-L194), [document.rs L118–177](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/files/document.rs#L118-L177) | Exact Core/path permit checked before mutation; one existing File Gate publisher; raw-byte BLAKE3 expected-hash CAS check before `AtomicWriteFile::commit()` | POSIX/Win32 namespace races between name-based check and rename; synchronizing receipt to published inode |
| History write | [service.rs L407–451](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/history/service.rs#L407-L451), [store.rs L238–279](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/history/store.rs#L238-L279) | Legacy held-custody method saves with owner check; strict method can return exact `version_id/content_hash/content_length` for bounded external `CreateNew` | **File Gate calls legacy bool**, not strict witness; version directory's *new ancestor* creation durability and post-crash version identity |
| Local readback | [s3_reserved_replace.rs L87–114](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/s3_reserved_replace.rs#L87-L114) | Two pinned postflight reads and stable expected bytes/file identity inside held lifetime | A stable read is not an fsync/rename receipt and does not survive as an authority after process restart |
| Remote/apply gate | [journal.rs L1236–1315](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/core/src/sync/journal.rs#L1236-L1315) | `StageDownload` is explicitly refused by `mark_applied`, `mark_verified`, and `commit` | No legal path to `ConfirmedBaseline` without a *new*, separately reviewed native receipt integration |

## 2. Exact `atomic-write-file` 0.3.0 semantics

Akula's [Cargo.lock package entry](https://github.com/Lalalalendia/Akula/blob/2c1d0e954affd96ccc12caffa570e5c9f278487e/aquilum-app/Cargo.lock#L508-L516) pins `atomic-write-file = 0.3.0`. Inspected source of that **exact tag**, not a newer release:

- [`v0.3.0/src/lib.rs` L607–618](https://github.com/andreacorbellini/rust-atomic-write-file/blob/v0.3.0/src/lib.rs#L607-L618): `commit()` calls `sync_all()` on the temporary file, then `rename_file()`. Thus it would be **incorrect** to claim File Gate never syncs the staged bytes.
- [`v0.3.0/src/imp/unix/mod.rs` L183–190](https://github.com/andreacorbellini/rust-atomic-write-file/blob/v0.3.0/src/imp/unix/mod.rs#L183-L190): Unix `renameat` is followed by `fsync(parent dir)`; a failure of the later fsync can occur **after** namespace mutation. Linux anonymous-temp mode also contains a [post-link `fdatasync`](https://github.com/andreacorbellini/rust-atomic-write-file/blob/v0.3.0/src/imp/unix/linux.rs#L38-L65). This is a strong existing primitive, not an end-to-end S3 receipt.
- [`v0.3.0/src/imp/generic.rs`](https://github.com/andreacorbellini/rust-atomic-write-file/blob/v0.3.0/src/imp/generic.rs): non-Unix (including Windows) uses temporary `File::create_new` and `std::fs::rename` after `sync_all()`, with **no explicit parent-directory sync in this implementation**. Do not infer equivalent Windows namespace durability.
- [Rust `File::sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all) attempts to flush file content/metadata, not arbitrary related directory entries.

**Conclusion:** Linux File Gate already has file and parent-directory sync *inside the library*. Do not add a second publisher just to duplicate it. On Windows the missing durable directory/namespace contract is a distinct design blocker; investigate a native backend or retain a conservative HOLD.

## 3. SQLite boundaries

- Both `SqliteProtectionStore` and `SqliteSyncJournal` independently configure `PRAGMA journal_mode=WAL; synchronous=FULL`; the V7 claim and protected original may reside in **different database files**.
- [SQLite documentation](https://www.sqlite.org/pragma.html#pragma_synchronous) describes WAL/FULL transaction durability subject to the operating system, filesystem and actual `xSync` behavior. It does **not** make writes across two databases plus history files and a renamed workspace file one atomic transaction.
- [SQLite WAL documentation](https://www.sqlite.org/wal.html#the_wal_file) requires the live WAL state to remain associated with its database; never treat only `.sqlite3` as a sufficient backup while the WAL is active.
- The protected-original database is host-selected app-data outside the vault; a *new* DB file/parent directory creation and movement across volumes must not be silently treated as durable without explicit platform proof.
- V7 evidence includes attempt ID, vault/document, normalized portable path collision key, old baseline revision+operation, preimage and target BLAKE3, pinned root and original identities, original length, intent digest. It does **not** include the actual newly published inode, HistoryService version ID or a durable phase.

## 4. Byte/identity and authorization model

- **Hash is not physical identity:** identical preimage/new bytes can appear at a *different inode/file ID*. Keep path, size, BLAKE3 and platform physical identity as distinct fields.
- **In-memory Core pointer is not persistable authority:** `HeldCustodyHistoryPermit` intentionally uses address identity within its lock lifetime. A restarted verifier must rely on separately authenticated durable registered vault/workspace mapping, not serialize an address.
- **History witness is insufficient alone:** the strict API requires external-source `CreateNew` and performs a byte-for-byte version readback under its lock. Its current `VerifiedHistoryVersion` lacks root/path, native file ID, parent directory sync evidence, V7 attempt binding and cold-start authority.
- **Possible source of silent proof loss:** `File Gate::write_checked_history_with_prewrite` calls legacy `try_saved_with_custody()` and gets only `bool`. This is acceptable for ordinary editing but must **not** be reused to mint an S3 receipt. Implement an explicitly opted-in strict branch, with errors after primary mutation mapped to `NeedsReview`.
- **Potential new history directory:** `store::write_version` does `create_dir_all(dir)` before atomically publishing the version. Although the version's directory is fsynced on Linux by `AtomicWriteFile`, newly created *ancestor entries* may require additional directory sync or a fail-closed first-use policy. Verify, do not assert durability from `create_dir_all`.

## 5. Crash-window acceptance table

| Inject crash/failure at | What might exist | Restart behavior (mandatory) |
|---|---|---|
| Before protected preimage SQLite commit | Only original primary | No apply; require full new admission, not a receipt |
| After protected preimage; before V7 attempt | Private original, no irreversible claim | Verify both stores; no automatic overwrite |
| During/after V7 commit; before File Gate | One-shot claim may exist, primary unchanged | One-shot remains consumed or ambiguous; HOLD, never replay |
| After prewrite identity check; before atomic commit | Old primary and maybe staged temp | HOLD on ambiguous attempt; never assume target absence proves retry permission |
| During/after commit, before history | Old **or** new visible file; on Linux sync failure can follow rename | Cold-read destination and protected original; NeedsReview on uncertainty |
| After primary, during history version write/metadata | New primary, zero/one/multiple history files | NeedsReview; do not silently manufacture an immutable version |
| After strict witness, before durable receipt | New primary and point-in-time history observation | No ACK; cold verifier must revalidate all evidence |
| After receipt commit, before cold verifier | Receipt and artifacts possibly on separate persistence domains | No automatic promotion; reconstruct and verify full chain |
| During recovery with changed path/inode, missing WAL, hash mismatch or unsupported fsync | Evidence inconsistent | `NeedsReview`; protect original; no destructive cleanup |

Process termination (SIGKILL / TerminateProcess) proves a narrower property than sudden hardware power loss. Distinguish simulated process crashes from VM hard-reset and actual persistence/fault testing in reports.

## 6. Narrow next changes — no S3 activation

1. **3d-02:** keep `SqliteProtectionStore` as the independent original; implement a *read-only* cold-open verifier for exact V7 claim + bound BLOB + registered workspace identity, without writer capability. Verify original retention and detect missing/moved WAL and schema mismatches.
2. **3d-03:** decide receipt schema in an explicit additive journal migration (do not retrofit V7 into an apply grant); bind claim, verified current destination physical ID/hash, strict HistoryService ID/hash/length and durable phase. Enforce transition ordering and post-crash idempotency.
3. **Strict history bridge:** opt in to the strict `CreateNew` path **without** changing ordinary editor history semantics. Any failure after the primary publisher entered is `NeedsReview`.
4. **Linux:** test a single existing publisher and directory-fsync result, including simulated failure *after* rename. Audit newly created history ancestor directories; avoid duplicate primary swaps.
5. **Windows:** prove/implement an appropriate native file+namespace durability strategy separately. Existing `native_windows_replace_probe.rs` and Linux `native_existing_replace_probe.rs` are **test-only falsifiers**, not production authorities.
6. **Recovery matrix:** two processes/two Core instances; every boundary above; preserved original, physical identity and byte-level diff; separate power-cut claims from clean process restart.

**Go/no-go:** even if all currently existing Windows/Linux CI passes, P0g-3d is not accepted until cold-restart and durability evidence exist. StageDownload/remote ACK/baseline promotion remains HOLD.
