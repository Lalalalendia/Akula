# VIDEO-18b-02 — standard-user tamper-negative Windows lab

Status: **TEST-ONLY / NON-RELEASE / NOT YET A PROTECTED AKULA INSTALLER**.

## What the experiment demonstrates

A disposable, administrator-provisioned fixture under a GUID-named subdirectory
of **Program Files** contains exactly five *mock* .exe files. The test turns
off inherited ACLs, grants Local Administrators and SYSTEM full access and
grants Built-in Users only read/execute. It changes the owner to the
Administrators group.

A randomly named, disposable Windows local account without administrator
privileges gets a genuine Windows token via \`LogonUserW\`. Using
\`WindowsIdentity.RunImpersonated\` and **actual file operations**, the test
requires that this account can read all five files but cannot overwrite
any of them, delete or rename the yt-dlp helper, inject an extra helper,
replace the protected directory, or create a new junction-staging directory
inside it. The test independently rehashes the five original files.

A separate *synthetic* media folder, outside the protected executable root,
permits the standard account to write. The ordinary application's local
video library and the user's real profile are never touched.

This job must run on **GitHub-hosted Windows**, on a PR's exact HEAD, with
an elevated fixture-provisioning account, not on developer machines. It
deletes the disposable user and both GUID-named fixture folders in a
\`finally\` block. No generated password is written to logs, no installer
is built or published, and no actual helper is executed.

## Not yet proven

- The current actual Akula NSIS installs under user-writable LocalAppData,
  **not** this protected Program Files fixture.
- The lab cannot claim automatic transition from that per-user installer
  to an elevated, signed product identity; uninstallation and migration
  need separate negative testing.
- Only a standard non-administrator token is tested. Malware running with
  administrative rights can bypass these controls; a standard user's
  profile and data can still be altered by another process with that user's
  access.
- This first test does **not** prove TOCTOU immunity during process
  execution, secure update staging, symlink/reparse parent substitution,
  all five binary signatures, generation-journal recovery, or
  rollback across power loss. Those remain VIDEO-18b blockers.
- The five fake binaries do not establish signed PE authenticity, licensing
  clearance, or real YouTube extraction compatibility.

## Links to the normative Windows APIs

- [Microsoft icacls — inheritance and grant syntax](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/icacls)
- [Microsoft WindowsIdentity.RunImpersonated](https://learn.microsoft.com/en-us/dotnet/api/system.security.principal.windowsidentity.runimpersonated)

Acceptance requires both the negative source contract and the actual
Windows standard-user test to be green on the **same exact commit**. Keep
VIDEO-18 release-HOLD enabled. Only a separately reviewed NSIS
implementation and protected release admission can ship video.
