# ADR-SYNC-035 — Live host SQLite namespace pin (P0-B5a)

Status: **limited hardening, not product release authority**.

## Gap

The installed Core's `SharedHostCustodyRegistry::open_existing` previously rejected a direct symlink/junction parent and symlink SQLite leaf, then accepted the named SQLite connection. A real host directory could still be moved aside and replaced with another **regular directory holding a valid empty schema-v1 ledger**. A live Core could then reopen or continue consulting a different host location under the same configured pathname, even though its prior enrollment existed elsewhere.

The same applies to renaming the SQLite leaf and replacing it with another regular file. The string path and SQLite user_version are insufficient to distinguish those cases.

## Bounded change

`HostLedgerPin` retains a pinned OS directory handle (`PinnedProbeRoot`) and a read-only opened SQLite file handle for the registry object's lifetime. Record actual volume/file identity from OS handles (Unix `dev/ino`; Windows existing Win32 `GetFileInformationByHandle`, **not** unstable `MetadataExt::file_index`). Do not trust lexical path alone. Before and after shared enrollment/read/ordinary `BEGIN IMMEDIATE` epochs, verify the original directory object is still named by the configured directory path and the named database is the same object as the opened handle. The opening and initial schema scan also check that the path still names the original file.

Any detected change yields **HOLD** (`InvalidHostLocation`). The ordinary publication closure does not run when the parent or leaf has already been substituted before epoch admission. Installer-only `provision_new` also pins its created ledger and never resets existing data. Existing OS lock/SQLite serialization and S3 safety gates remain unchanged.

## Regression

Unix test 1 enrolls the real host, moves aside the **actual directory**, creates a second real directory and valid empty SQLite in its place, then proves an already-open Core cannot execute an ordinary mutation, inspect a false absence or enroll in the substituted ledger. Unix test 2 swaps only the actual SQLite file with an ordinary file while the original handle is live; the Core refuses mutation without touching replacement bytes. Linux/macOS plus Windows compilation and unaffected tests are required on exact PR head.

## Explicit limits

This proves **live object identity checks**, not cold-start identity persistence. A new process that starts **after** an attacker has replaced the entire namespace with a valid empty DB still needs a trusted installer-owned reference independent of the mutable path. These checks narrow but do not eliminate hostile same-UID rename/TOCTOU, do not pin every ancestor, do not protect SQLite `-wal`/`-shm` sidecars throughout each syscall, do not attest owner/ACL and do not provide persistent independent proof of the original host identity. An elevated OS broker or equivalently trustworthy installation/attestation step is the next gate.

**Release remains blocked:** normal packages use `--features installed` without `sync-installed-host`. Local Desktop Beta must not silently lose offline writes. No automatic S3 overwrite/delete, baseline, GC, Sync or HOLD clearance may be enabled on this PR.
