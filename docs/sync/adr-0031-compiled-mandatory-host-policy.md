# ADR-SYNC-031 — Compile-time mandatory host admission for installed sync binaries

**Status:** opt-in packaging guard / P0-B3, **not** a finished installer, malicious-writer boundary or public S3 release authorization. Depends on #258.

## Problem

The original #251 two-process falsifier proved that a second Akula Core opened with a distinct appdata could write/create under A's enrolled root, because normal `Core::open` did not receive the shared host custody source. PR #258 proved guarded ordinary write/create only with the explicit internal `open_with_installed_host(...)` constructor. A second Core built and launched without that constructor still bypasses the ledger.

## Constructor override guard

For builds compiled with `sync-installed-host`, the explicit
`Core::open_with_installed_host` constructor must also use **only** the
exact compile-time installer ledger path. A caller-selected absolute path,
including a valid freshly provisioned empty SQLite database in private
appdata, results in persistent HOLD. This is covered by
`explicit_constructor_cannot_select_a_private_empty_host_database`.
Fail-closed refusal of a differently spelled but equivalent path is
intentional until OS-verified pinned identity is available; pathname
equivalence is not authorization.

## Narrow policy change

- Add separate Cargo feature `aquilum-core/sync-installed-host` (implies only the AWS-free `host-custody` feature) and forward `aquilum-native/sync-installed-host` to it. **Ordinary local builds retain their existing behavior; this feature is not the default** and is not automatically coupled to the existing `installed` packaging feature.
- In this feature, every `Core::open` obtains its shared host DB path solely from compile-time `AKULA_INSTALLED_HOST_CUSTODY_DB` (an absolute path injected by the trusted build/installer pipeline). Appdata and mutable **runtime** environment values never choose this path.
- Absence, empty/relative path, invalid schema or inaccessible host DB **do not revert to legacy local writes**. Core remains constructible for read-only inspection, but `installed_host_required` stays true. Ordinary `write/create` require the shared SQLite `BEGIN IMMEDIATE` epoch from #258; other File Gate routes, direct HistoryService, purge and guarded editor writes are HOLD until integrated through a sealed host permit.
- The existing S3 appdata/OS-lock writer intentionally cannot bypass mandatory host mode using its older, independent permit. No implicit S3 overwrite/delete or identity-bypass exception is introduced.
- Existing legacy #251 negative test runs only in builds **without** this new feature. Its underlying bypass remains open in default local/unconfigured `sync-s3` builds, and it must not be reported fixed for all process variants.
- A dedicated **real child OS-process** test runs with an empty build-time host path and a forged runtime `AKULA_INSTALLED_HOST_CUSTODY_DB`. It proves that two distinct appdata processes refuse write/create/binary and preserve primary bytes even when runtime environment is manipulated. A separate real child OS-process test compiles a uniquely named disposable `akula-ci-installed-host.sqlite3` path into the Core, provisions an enrolled root in that exact shared host database, and then opens A and B via the **ordinary** `Core::open` under different appdata. Both must refuse writes, with B spoofing a different runtime path. This proves the compiled source actually selects one database across processes rather than merely disabling every write. Unenrolled positive create/write+history remains covered by #258's explicitly bound test.

## CI and compatibility

This branch is rebased onto merged #263 (AWS-free local host custody). Both `Core` and native feature flags must be checked with `cargo tree --features sync-installed-host` to prove no AWS config/S3/STS SDK is transitively enabled. The `Local host custody without AWS` matrix explicitly runs the two-process negative and positive installed-source tests on Linux, macOS and Windows. Existing S3 transport checks remain a separate compatibility matrix; passing S3 CI alone does not prove this reduced dependency build.

- Test `cargo test --locked -p aquilum-core --features sync-installed-host installed_host_tests` on Linux (MinIO job), Windows and macOS (custody jobs). The first scenario uses an empty compile-time DB path to prove HOLD; a second targeted Linux/Windows/macOS scenario compiles an absolute disposable path under `runner.temp` and verifies **positive shared host discovery plus enrolled-root refusal** using the default Core opener. Standard Application tests still compile/run default and `sync-engine` local builds.
- The default native binary now enables only the local `host-custody` feature (PR #263), without S3 or AWS SDK. A separately selected `aquilum-native --features sync-installed-host` binary also compiles the same mandatory Core policy for GUI/MCP, but **must not be distributed** before its installer provisions and attests an agreed host ledger.

## Unclosed P0 release gates

1. Installer must select **one stable host path** for every build/profile/OS process and create a trusted host namespace with fixed owner/ACL. Compile-time absolute path alone does **not** attest permissions, physical inode/file identity, or WAL/SHM objects; a hostile same-user process can still alter writable namespaces. Avoid runtime path override and per-appdata fallback.
2. Full actual installed-source acceptance needs packaging plus a **positive two-process** same-host DB test with separate appdata, restart/crash, root/ledger/WAL swaps, and read-only recovery. The empty-DB test proves refusal, not correct eventual installation.
3. All remaining ordinary mutations (rename, move, trash, restore, binary, folder, version rename, retention), native/MCP/standalone adapters and the guarded S3 writer must join a lifetime-verified shared host epoch or stay HOLD. The current mandatory-host feature deliberately blocks them.
4. Compile-time feature is only a cooperative binary policy, not an OS mandatory-access-control mechanism against arbitrary third-party or modified programs. Old binaries can still write unless the OS/privileged broker enforces the namespace.
5. No public remote overwrite/delete, baseline promotion, local automatic replacement, garbage collection, HOLD clearance or cloud-sync activation.

**Decision:** prior to a complete installer and full-process multi-OS proof, ship only the local non-S3 product. A feature-gated mandatory-host build with missing policy must fail closed rather than silently treating a protected root as unenrolled.
