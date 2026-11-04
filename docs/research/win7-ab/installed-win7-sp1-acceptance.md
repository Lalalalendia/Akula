# PC-DESK-08: real Windows 7 SP1 acceptance of ordinary Akula NSIS

Win7 LAB builds complete Akula and Shiva PDF for the Win7 Rust target,
audits their PE imports, builds a normal NSIS installer and exercises its
installed GUI and PDF on Windows Server 2022. This does NOT prove Win7 runtime.

## VM and package

- Use a new disposable Windows 7 SP1 x64 (build 7601) VM snapshot.
- No existing Akula installation, real vault or account credentials.
- Obtain one successful LAB Win7 Akula and Shiva PDF build artifact ZIP.
  Do NOT use the separately published v0.3.14 installer: same version can
  name a Windows-10-only signed public build.
- Extract the artifact. Locate four files from the SAME successful CI run:
  Akula_0.3.14_x64-setup.exe; win7-installer-pe-audit/full-audit.json;
  PROVENANCE.txt; scripts/akula-win7-sp1-acceptance.ps1.
  The ZIP extraction may add a parent folder. Use actual local paths.

## Command in built-in Windows PowerShell (no pwsh required)

In the extracted artifact directory, run on the disposable Win7 VM:

    powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\akula-win7-sp1-acceptance.ps1" -Installer ".\dist\Akula_0.3.14_x64-setup.exe" -InstallerAudit ".\dist\win7-installer-pe-audit\full-audit.json" -Provenance ".\win7-lab-output\PROVENANCE.txt" -DisposableVm -UninstallAfterTest

The command must fail if the actual file paths differ or proof files
are missing. Supply accurate paths from that same archive, never dummy hashes.

## What the test actually verifies

1. Win7 SP1 x64 OS and explicit disposable-VM admission. Refuses existing
   Akula installation or profile, does not touch a personal vault.
2. SHA-256 of the actual NSIS against independent CI installer PE audit.
3. Normal NSIS install and SHA-256 of installed Akula + Shiva against provenance.
4. PDF-to-text and text-to-PDF roundtrip on actual Windows 7.
5. 20 Notepad cover/uncover cycles with before/after screenshots.
6. A human types a unique marker into Akula's real editor and saves it.
   Its on-disk persistence and cold reopen are then verified.
7. A human checks all 20 exposed screenshots, then explicitly accepts or
   rejects the recovered UI. Optional uninstall preserves the external note.

Evidence is written into Documents/Akula-Win7-QA-<timestamp> in the VM:
evidence.txt, screenshots, test fixtures. Keep this alongside exact GitHub
run URL. A script PASS does not bypass a human visual review.

## Still required outside this script

Manual DPI 100% and 125%, Aero and Basic themes, minimize/restore,
Settings, keyboard/mouse input; corresponding Windows 10 and Windows 11
installed tests; verify required VC/UCRT/TLS components; signed installer,
update and rollback. The signed candidate must build the identical audited
Win7-compatible binaries using the protected owner-signing workflow.
Publishing requires a new version strictly greater than v0.3.14 and explicit
approval after physical QA. Windows 7 itself is no longer security-supported.
