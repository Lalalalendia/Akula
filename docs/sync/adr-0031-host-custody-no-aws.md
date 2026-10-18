# ADR-SYNC-031 — compile the host custody gate without AWS

**Stage: P0-B3a (build-time prerequisite, not a release security gate).**

The real cross-appdata OS-process evidence from #251 and the host-bound
`File Gate write/create + HistoryService` adapter in merged #258 must be
available in **normal native** builds. Previously all shared-host code was
behind `sync-s3`, while `aquilum-native` depended on bare `aquilum-core`:
the normal desktop therefore compiled none of these checks.

## Narrow implementation

- Introduce `aquilum-core/host-custody` with **zero additional dependencies**.
  It compiles the existing installer-provisioned shared SQLite registry,
  host-bound Core constructor, lexically held `HostUnenrolledEpochPermit`,
  process-wide ordinary note publication lease, note File Gate write/create
  adapter, and direct HistoryService denial.
- Keep `sync-s3 = [sync-engine, host-custody, AWS SDK dependencies…]`, so
  previously compiled S3 paths and full AWS tests stay unchanged.
- Make `aquilum-native` depend on `aquilum-core/host-custody` rather than
  enable `sync-s3`. Merely having local OS custody enabled does **not**
  enable cloud sync, network work, an S3 writer, signed identity or a UI toggle.
- Expose `Core::open_with_installed_host(appdata, host_db, sink)` to the native
  host crate. It is **not an attestation**: the host/installer still must
  supply and pin one trustworthy path independent of `--data-dir`. Do not
  bootstrap a fresh registry on open, infer absence as permission or read a
  claimed location out of private appdata.
- Keep all guarded S3 editor, transport and AWS-related code feature-gated
  behind `sync-s3`; only local custody functions move.

## Explicit security limitations

**This does not fix the #251 normal `Core::open` alternate-appdata bypass.**
The native `Workspace::open` still calls the legacy constructor. Activating
host mode everywhere must be a follow-up reviewed change with an
installer/OS-attested source and no legacy writable fallback. A missing or
corrupt installed source must HOLD before *all* mutation APIs, not silently
fall back to an unprotected Core. Until then no product S3 auto-sync, local
overwrite/delete, confirmed baseline, GC or guarded release claim.

## Acceptance

1. `cargo test --locked -p aquilum-core --no-default-features --features host-custody host_core_epoch_tests -- --nocapture` on Windows, macOS and Linux, including actual A/B processes and independent appdata.
2. `cargo test --locked -p aquilum-core --no-default-features --features host-custody host_custody_registry::tests` on each OS.
3. Native build and tests with the local feature, no required AWS environment or SDK.
4. Existing `sync-s3` tests remain green because `sync-s3` now depends on
   `host-custody`. The default `aquilum-core` feature set remains empty.

**Next P0-B3b:** select a single installer-provisioned host binding at the
actual native startup, require it for every Core/MCP instance, and rerun the
original #251 test against **normal** `Workspace::open` across distinct appdata.
