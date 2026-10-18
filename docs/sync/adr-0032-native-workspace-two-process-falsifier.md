# ADR-SYNC-032 — Native Workspace::open cross-appdata acceptance (P0-B4a)

**Status:** stacked test-only PR atop P0-B3 [#264](https://github.com/Lalalalendia/Akula/pull/264). Not installer activation, not authorization for remote S3 writes.

## Why this test is necessary

The original [#251](https://github.com/Lalalalendia/Akula/pull/251) proved that a *second* OS process can open a physically identical workspace through a different `--data-dir` and bypass the first Core's appdata-local S3 custody. Internal Core-only tests can pass even if the native entrypoint loses the intended build feature or silently chooses a legacy constructor.

This slice exercises the actual `native::workspace::Workspace::open(Some(appdata), Some(vault), wake)`, the function used by the GUI path behind `--data-dir`. It is compiled with the separately selected **AWS-free** `aquilum-native/sync-installed-host` feature.

## Exact negative acceptance

With no build-time installer database, the native binary must construct a read-only/HOLD Core, never create an empty host registry and never treat a runtime environment value as trusted authority. A test opens the same temporary physical vault through appdata A, then through appdata B in a **separate operating-system process** using the same native Rust binary test harness. Under both processes: `gate::write`, `gate::create`, `gate::copy` and direct `HistoryService::try_saved` must refuse. The test verifies the original file and absence of new files, and requires proof that the child test actually executed.

The GitHub Actions host-custody workflow adds Linux, macOS and Windows native-test jobs. Each also verifies the feature's dependency graph contains no AWS SDK packages.

## Boundaries / next barrier

This proves **only** that missing compiler-issued host authority is fail-closed at the native entrypoint. It does not prove installer provisioning, OS owner/ACL and file/WAL/SHM physical identity, a successful multi-process positive *native* write to an unenrolled workspace, or durability after crash/restart. The positive shared-host Core tests from #264 do not substitute for these native installation checks.

The product's existing release pipeline still builds `--features installed` (without `sync-installed-host`) and must **not** be declared P0-safe. Do not enable S3 overwrite/delete, baseline promotion, automatic sync, GC, or general HOLD clearance on this test alone.
