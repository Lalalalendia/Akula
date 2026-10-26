# Akula v0.3.15 — signed Windows release and Win7 SP1 acceptance

Status: **release candidate in PR**, not a signed or published release until
protected GitHub Actions completes. Public latest at the time of preparation:
`v0.3.14`. Version is `0.3.15`, not `3.15`.

## Source and release admission

- This release updates the canonical four Windows-version mirrors together:
  `aquilum-app/native/Cargo.toml`, `core/Cargo.toml`, `Cargo.lock`
  (both workspace packages), and `native/packaging/akula-windows.json`.
  Linux's independent unpublished packaging version is intentionally not bumped.
- Exact merged `main` must pass required Application checks and release isolation.
  Prior Win7 target full compile+NSIS smoke evidence is
  [#40343345098](https://github.com/Lalalendia/Akula/actions/runs/40343345098)
  (Windows Server 2022; NOT native Windows 7 evidence).
- The signed workflow first builds a single Windows 7/10/11-compatible full
  Akula with opt-in S3 and pinned Shiva, checks static PE imports and lack of
  external UCRT/VC runtime dependencies, creates an ordinary NSIS installer,
  tests installed GUI/PDF, signs the tested installer and release manifest under
  the unchanged Akula owner key, then tests genuine in-place upgrade from the
  previous published Akula with preservation of notes, AppData and Aquilum.
  Independent Linux verification checks both Minisign signatures and tampering.
  No automatic publish is permitted by an ordinary PR/merge.

## Publishing the requested v0.3.15 for real-world testing

Only after the version-bump PR is **merged** into protected `main`:

1. Open [Akula Windows signed candidate (review only)](https://github.com/Lalalendia/Akula/actions/workflows/akula-windows-signed-candidate.yml) in GitHub Actions.
2. Select **Run workflow**, branch `main`, `release_version=0.3.15`,
   and **`publish_beta=true`** for the user-requested public release.
3. Complete the required `akula-release` protected-environment approval.
   The `windows-build`, `independent-verification` and
   `publish-windows-beta` jobs must all end SUCCESS.
4. Verify the GitHub Release `v0.3.15` contains exactly:
   `Akula_0.3.15_x64-setup.exe`, its `.sig`, `latest.json` and
   `latest.json.sig` with published SHA-256. Never publish an unsigned LAB ZIP.
5. This workflow retains `akula-windows-win7-sp1-qa-NOT-RELEASE` as a
   separate **seven-day** QA Actions artifact, containing the **same signed
   installer** plus independent NSIS PE JSON, Akula/Shiva PE JSON,
   `PROVENANCE.txt` and `scripts/akula-win7-sp1-acceptance.ps1`.

## Real Windows 7 SP1 x64 installation

Use a **disposable snapshotted Windows 7 SP1 x64 VM** without any actual Akula
profile or production vault. Extract the QA artifact from the successful
signed-release workflow run. Resolve the paths if extraction adds a directory.

Run built-in Windows PowerShell (compatible script is PowerShell 2.0):

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\akula-win7-sp1-acceptance.ps1" -Installer ".\dist\Akula_0.3.15_x64-setup.exe" -InstallerAudit ".\dist\win7-installer-pe-audit\full-audit.json" -Provenance ".\win7-lab-output\PROVENANCE.txt" -DisposableVm -UninstallAfterTest
```

The script refuses the wrong OS/real user profile, verifies installer and
installed EXE SHA-256 against exact pipeline reports, exercises Shiva PDF,
cycles Notepad cover/uncover **20 times** with PNG captures, and checks
real editor save + cold reopen with manual visual confirmation. Also inspect
Settings, DPI 100% and 125%, Aero/Basic mode, keyboard, uninstall and startup.
Retain screenshots/evidence and report all failures in the Win7 task.
Perform installed Windows 10 and Windows 11 regression with the same installer.

**Do not declare Windows 7 supported merely because this is released.**
This Win7-target build is experimental until actual native Windows 7 SP1
acceptance. Windows 7 is no longer security-supported. The workflow publishes
as latest once explicitly approved, so users on other Windows versions may
receive it; ensure the risk is understood. NSIS has no automatic crash rollback:
reinstall the previous signed `v0.3.14` if recovery is required.
