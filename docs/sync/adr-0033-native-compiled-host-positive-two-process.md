# ADR-SYNC-033 — Positive native installed-host admission, two physical processes (P0-B4b)

**Status:** test-only proof, stacked on [P0-B4a / PR #274](https://github.com/Lalalalendia/Akula/pull/274); not approval for an installed release, S3 overwrite/delete or any automatic sync.

## Gap

The negative `Workspace::open` test in P0-B4a proves that an installed-host binary **without** a compiled shared host ledger refuses writes under alternate `--data-dir`. That is necessary but not sufficient: a policy that rejects every write could pass. The next falsifier must demonstrate that an explicitly configured, shared **existing** installer ledger safely admits normal local publication to an *unenrolled* workspace from two separate native processes.

## Exact acceptance

The dedicated [native host positive CI](../../.github/workflows/sync-native-positive-host.yml) compiles `aquilum-native --no-default-features --features installed,sync-installed-host` on Linux, macOS and Windows, **without AWS SDKs**. A disposable Python-only fixture creates a schema-1 SQLite host database in `runner.temp` with WAL/FULL before either Core opens. Its unique filename `akula-ci-native-positive-host.sqlite3` is injected into Rust's **build-time** `option_env!` during compilation, not selected via an appdata-local setting or runtime environment.

The real native `Workspace::open(Some(appdata), Some(vault), wake)`:

1. Process A creates `Shared.md`, verifies full bytes and BLAKE3, and verifies the initial persisted history version.
2. Process B is a **different OS process**, with distinct appdata, same physical vault, the same compiled host ledger and an intentionally invalid *runtime* `AKULA_INSTALLED_HOST_CUSTODY_DB`. It must update the note under the actual raw-file CAS and persist a distinct second history version. Parent verifies the child test actually ran.
3. Process A attempts a stale expected-hash save; it must fail with `Conflict`, leave B's bytes unchanged and not append a history version.
4. Cold reopen of appdata A must read both versions without requiring the first Core's in-memory history. The installer host database must still be schema-1, undamaged and empty (the test performs **no** enrollment).

The full test runs in a temporary root; **no real user data** is opened. The local installer fixture is not an authorized production trust root. This PR does not modify Core admission behavior, File Gate publication, durability settings, or the ordinary release workflow.

## Unclosed P0

- Installer-provisioned OS-owner/ACL and pinned physical identity of the SQLite **database, WAL, SHM, parent directory and lock scope**; same-user malicious process and replacement races are not solved by a compile-time path.
- Native *enrolled* two-process refusal using the same verified installed ledger, including real lock-lifetime and blocked S3 writer interactions. The Core-level proof in #264 does not substitute for full installed-app E2E.
- Durable PREPARED/PUBLISHED/VERIFIED receipt, crash/restart, ambiguous acknowledgement, recovery/HOLD, link swaps and Windows/macOS/Linux installed artifacts.
- Release workflow currently uses `--features installed`, **not** `sync-installed-host`. Do not imply it passes this safety policy, and do not switch it to an unprovisioned host mode that would disable local editing.

**Gate:** exact-head positive-native matrix and mandatory application/security CI must pass before any merge. Even passing it does not clear product Sync Beta.
