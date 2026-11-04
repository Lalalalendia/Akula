# DESK-15 — externally controlled power-cut witness (manual)

Run this only in an isolated, disposable **Windows 7 SP1 x64, Windows 10 x64, or Windows 11 x64 workstation/VM**. An independently audited Akula release candidate must already be installed at %LOCALAPPDATA%\Akula\akula-app.exe. Do not connect real vaults, Dropbox, S3 or user files. The script never installs software, deletes any file, or controls machine power.

The normal process-kill acceptance is separately provided by `scripts/akula-desk15-real-windows-acceptance.ps1 -LargeDocument`. Run that first. This two-phase witness is additional and does **not** constitute evidence of a real power cut without an independent external power-controller/VM-host record.

## Phase 1 — Prepare (guest workstation)
Use Windows PowerShell 2–5, **not** PowerShell Core. Specify a SHA-256 independently measured from the approved release candidate (do not use a value offered by the test script itself).

```powershell
powershell.exe -NoProfile -Sta -File .\scripts\akula-desk15-powercut-witness.ps1 -Phase Prepare -DisposableMachine -ExePath "$env:LOCALAPPDATA\Akula\akula-app.exe" -ExpectedSha256 "<independently audited 64-character SHA256>"
```

The script builds a private, synthetic 100,000-line UTF-8 note (>16 MiB, <24 MiB), starts the installed Akula and asks you to append one unique marker at EOF via Ctrl+End/Ctrl+V. Verify real GUI contents with Ctrl+A/C. The script waits for the published shadow, verifies its exact on-disk SHA-256, checks private ACL and ensures the original file was never silently saved.

The script emits `DESK15_POWERCUT_PREPARED evidence=...` and leaves the test-owned Akula running. **Before triggering any external power event, copy `checkpoint.txt` and `events.txt` to the independent hypervisor/QA host. Keep the original host archive and independently record its SHA-256; after reboot import an unmodified archived copy of `checkpoint.txt` to a location outside the original guest trial folder for mandatory verification.** If either file or the host copy is missing, the trial is invalid.

The hypervisor/QA operator, independently from the guest, chooses either forced **VM power off** or true physical **power removal** of only this disposable QA target. Do **not** send a graceful Windows shutdown, and do not interrupt a working machine. Capture an external host-controller log with trial ID, UTC timestamp, device ID and event type. After boot, copy that external record onto the guest as a separate text file containing the original trial ID and one exact line `POWER_EVENT=FORCED_VM_OFF` or `POWER_EVENT=PHYSICAL_POWER_LOSS`.

## Phase 2 — Verify (after independent external power event)
Akula must not be running. Run the exact same script revision and installed application digest, using the evidence directory printed in Phase 1:

```powershell
powershell.exe -NoProfile -Sta -File .\scripts\akula-desk15-powercut-witness.ps1 -Phase Verify -DisposableMachine -ExePath "$env:LOCALAPPDATA\Akula\akula-app.exe" -ExpectedSha256 "<same 64-character SHA256>" -EvidenceDir "<previous evidence directory>" -ExternalCutEvidence "<host log imported from independent controller>" -TrustedCheckpointPath "<unchanged checkpoint.txt imported from independent QA host>"
```

Verify first compares the entire guest `checkpoint.txt` byte-for-byte (SHA-256) with the separately archived file supplied using `-TrustedCheckpointPath`. It refuses either archived checkpoint or external event proof placed in the original guest trial directory or its subdirectories, and rejects missing or reparse-point files **and any linked/junction ancestor directories**. Import the host records into an ordinary non-reparse directory (not through Windows shortcuts, junctions or redirected links). Verify rejects same-boot attempts, unexpected changes to the script or installed EXE, missing/changed shadow bytes, broad ACL, mutated source before user consent, missing external log and missing real GUI Restore. After operator-confirmed Restore it checks **exact full-file SHA-256**, shadow retirement and full GUI readback.

**Result semantics**: `DESK15_RESTORE_VERIFIED_EXTERNAL_EVENT_NOT_AUTHENTICATED` proves post-reboot recovery and that an external power-event record *was presented*. It **does not** cryptographically authenticate the host event. A reviewer must independently verify the host logs/timestamps and distinguish `FORCED_VM_OFF` from true `PHYSICAL_POWER_LOSS`. Neither outcome is equivalent to recovery of edits typed *before* the shadow was fsynced/published. Fail closed on missing records; never claim "zero data loss before durability ACK".

Do not promote DESK-13/15 to complete until separate accepted evidence exists for Windows 7 SP1, Windows 10 and Windows 11, and the desired power-loss scenarios.
