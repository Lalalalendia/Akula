# RELEASE-WINDOWS-APPLY-HOLD-01 — refuse unsafe automatic Windows install

**Status: DRAFT / FAIL-CLOSED / NO RELEASE.**

This PR isolates an existing unsafe compatibility mechanism. The inherited
Windows `updater::apply(&[u8], before_exit)` wrote a downloaded installer to
a PID-named directory in the ordinary OS temp location and attempted to
launch the installer with `/S /R /UPDATE /ARGS`. It had no durable
prior-generation backup, first-start GUI acknowledgement, or crash rollback.
The disabled `channel_ready()` gate prevented normal users from invoking it,
but an accidental release-channel toggle would have exposed that path.

## What is now enforced

- Windows `apply()` **always returns HOLD**, with an actionable error.
  It does not write an executable, launch a process, end the current app,
  invoke `before_exit`, or modify Akula/Aquilum data.
- A Windows-only Rust test invokes this function with synthetic unsigned bytes,
  confirms `HOLD` and verifies that the pre-exit closure is not called.
- An independent Windows beta Node policy test checks the source boundary
  against regression to `ShellExecuteW`, temp installer writes or process
  exit. The guard accepts both the currently disabled empty publisher key
  and a separately reviewed pinned public key; **key provisioning must not
  enable the release channel**.
- The GitHub release workflow and updater publication channel remain disabled.

## Avoid duplicate signing mechanisms

The complete signed-candidate, version, platform, manifest and SHA-256 checks
are being implemented separately in
[PR #522 — Windows real NSIS upgrade and signed candidate](https://github.com/Lalalalendia/Akula/pull/522).
The independently owner-provided Minisign public key and protected workflow
are handled in
[PR #527 — owner key pinning](https://github.com/Lalalalendia/Akula/pull/527).

Earlier experimental Windows admission work in this branch duplicated #522.
That duplicate implementation and its CI fixture **were reverted** before
acceptance. This PR must remain limited to forbidding unsafe install.

## Still missing before user-facing Windows auto-update

1. The **actual** verified Akula publisher manifest and download must be
   joined with the exact candidate bytes in the updater (no raw `Vec<u8>`
   without a typed proof at the installation boundary).
2. OS-protected staging with a single authoritative lock, package rehash,
   Akula-only identity and verified destination, durable journal and
   protected previous version.
3. Generation-bound first-GUI acknowledgement, reboot/crash detection,
   **automatic** rollback and tamper-resistant persisted version floor.
4. Complete Windows 10/11 installed and physical acceptance, separate
   publisher approval and only then enabling a signed release channel.

The real two-version NSIS laboratory in #522 proves upgrade compatibility
and **manual** restoration only. It is not equivalent to crash-safe
in-application update, and this change does not make it so.

The ordinary update flow must fail safely until the missing guarantees are
implemented and independently verified.
