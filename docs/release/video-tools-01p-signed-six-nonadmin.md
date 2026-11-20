# VIDEO-TOOLS-01p — real standard-user read-only access to six-file signed-manifest package

This is a **non-release** Windows hosted PR acceptance. It extends the
previous **four-file** standard-user proof (01f), not a replacement for it.

## Exact real Windows acceptance

In the 01j protected installation, elevated test setup installs exactly **six**
objects in the OS-known `Program Files\Akula Video Tools` root: three real
PE binaries, a `manifest.json`, an ephemeral Minisign
`manifest.minisig`, and the dedicated `uninstall.exe`. Akula's native Rust
witness validates the genuine **signed manifest**, the actual PE hashes,
the protected root and ACL before the following test.

1. Spawn a freshly created local **normal user** with a distinct SID via
   `Start-Process -Credential`, checking inside the child that its token
   is not Administrator. The child runs a hash-pinned, admin-owned PowerShell
   fixture and cannot modify its own test logic.
2. From that actual child token, read every one of the six installed files.
   For each, call Win32 `CreateFileW` requesting write-data, delete,
   change-ACL and take-ownership rights separately, and require actual
   `ERROR_ACCESS_DENIED=5` on all 24 attempts. The legacy seven root/PE
   mutation-right tests remain active.
3. The elevated parent confirms all six SHA-256 digests stayed unchanged,
   rechecks the OS-derived path and has the native witness independently
   re-validate the authentic signature and pinned PE bytes afterward.
4. Account and non-secret test-fixture cleanup are required; no lingering
   lab account is permitted. The existing 01j/01m/01n acceptance and
   uninstall checks still run.
5. Only after these facts does the CI write
   `VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS` as part of the exact-HEAD proof.
   Source-negative tests reject bypassing child token, OS access denial,
   byte checks, account cleanup, native signature verification and the
   release HOLD.

## What this does NOT prove

The package is an **unsigned NSIS** laboratory installer containing a
**signed manifest** authenticated by an **ephemeral** CI key. This does not
constitute owner approval, publisher-signed Windows installer distribution
or even genuine publisher-owned signatures.

This does not exercise Windows UAC prompts, an ordinary user's full
install/uninstall UI, update failure recovery, locked files or Windows 10/11
physical user acceptance. It does not execute any of the optional Video
Tools. Only hosted Windows 2025 genuine local-user filesystem access is
validated.

V1 calls for explicit manual updates and does not require a persistent
anti-rollback floor **after uninstall**. A new floor-storage subsystem is
therefore not introduced without an approved threat-model decision; the
installed-package downgrade and no-overwrite tests remain mandatory.

The ordinary installed Akula download queue remains closed, product
`APPROVED_VIDEO_TOOLS_KEY=""`, and the standard Akula two-EXE
installer/updater remains untouched. **VIDEO-18 RELEASE HOLD** and
`VIDEO_FIVE_CHANNEL_READY=false` remain in force.
