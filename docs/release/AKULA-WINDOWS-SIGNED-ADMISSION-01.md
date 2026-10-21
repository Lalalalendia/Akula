# AKULA-WINDOWS-SIGNED-ADMISSION-01 — authenticate Windows update candidates without installation

**Status: DRAFT / TEST-ONLY / updater HOLD.** This is a narrow preparation step for
Akula-to-Akula Windows auto-update, separate from the Linux signed-stage and
the [Windows beta installer smoke](../../scripts/test-windows-beta.ps1).

## Threat boundary

A downloaded NSIS executable or the CI-generated `candidate-manifest.json`
is **not** a trusted update. An attacker may substitute JSON fields or bytes,
offer an Aquilum binary, re-label an ARM64 installer as x64, or replay a
valid but older version. The Windows updater must not grant authority from
network JSON, a GitHub artifact URL, a filename alone, or an unsigned digest.

## This increment

The existing private `SignedCandidate` admission layer now understands
`windows-x86_64` as an independently **verified, inert** candidate:
- Authenticate the **entire raw publisher manifest** with the pinned-key
  Minisign verifier before parsing or trusting any release field.
- Require product `Akula`, supported schema, an exact 40-hex source commit,
  64-hex `Cargo.lock` digest and a strictly newer numeric release version.
- Require the Windows platform entry to name exactly
  `Akula_<version>_x64-setup.exe`. Reject an Aquilum or wrong-architecture
  setup, traversal path or stale package filename.
- Check the exact Windows payload's size and SHA-256 against that authenticated
  entry, with a strict 512-MiB ceiling shared with the network download reader. Continue validating every other signed
  platform entry, so malformed cross-platform metadata fails closed.
- Carry the authenticated bytes, version, source revision and digest inside a
  privately constructible `SignedCandidate`; do not reduce the result to
  an unlabelled `Vec<u8>` at an eventual Windows staging boundary.
- Use the **existing** required Linux Rust CI lane with an ephemeral
  Minisign key generated on the hosted runner. One real detached signature
  binds *both* independent synthetic AppImage and Windows setup payloads;
  altered same-size bytes and a different key must fail.

No actual Windows installer is run by this new admission test. The existing
hosted Windows NSIS GUI tests remain a distinct mandatory acceptance gate.

## Explicitly NOT unlocked

- `release::PUBLIC_KEY` is still empty and
  `release::channel_ready()` remains `false`.
- Production `signed_candidate::admit()` remains limited to a disabled Linux
  x86_64 AppImage path. A caller cannot enable Windows update admission through
  this testing change.
- `Updater::find()`, `Updater::download()` and the `release.yml`
  HOLD workflow are unchanged. The **old Windows `apply()` path now returns
  HOLD unconditionally**, instead of launching the downloaded NSIS installer
  from a process-specific temp directory. This prevents an accidental channel
  toggle from bypassing a signed staging and rollback authority. Any future
  Windows `apply()` implementation must be a separate reviewed transaction
  with a protected prior-generation backup, a first-GUI acknowledgement
  and crash-tested automatic rollback.
- No Akula publisher private key, release, manifest publication, permanent
  trust root, Authenticode certificate or migration from Aquilum is introduced.

## Next independent gates before Windows auto-update

1. An Akula publisher key-ceremony and offline signed-release process,
   a pinned source/lock provenance bundle and a policy-safe monotonic
   version-floor witness.
2. Transactionally stage the exact signed Windows installer with rehash and
   Win32 identity/ACL/symlink protections under a single-instance OS lock;
   never execute an arbitrary URL or silently overwrite a previous installer.
3. Prove Akula v1 → v2 on hosted **installed** Windows, including `Ctrl+A`,
   `Ctrl+V`, `Ctrl+Z` large-document persistence, preserved user data,
   forced termination during upgrade, first-render ACK and actual rollback.
4. Real physical Windows 10/11 acceptance, independent security review,
   explicit release approval and only then enabling a new publisher channel.

Do **not** treat a green schema or cryptographic test as proof that Windows
auto-update works today. Related release lock:
[AKULA-RELEASE-LOCK-01](AKULA-RELEASE-LOCK-01.md).
