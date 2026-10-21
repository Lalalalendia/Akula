# AKAI-HOST-INSTALL-01 — independent Windows per-machine Broker MSI

**Research-only installer candidate. Not a signed shipped product.**

## Problem

Akula's existing Windows beta installer is per-user NSIS, places the desktop binary under \`%LOCALAPPDATA%\Akula\`, and uses HKCU registration. It MUST NOT install a LocalSystem service or elevate a user-writable desktop binary.

Windows service installation should use the transactional native Windows Installer service tables instead of a custom "sc create" action or an install switch in the app. WiX v5 produces an **independent** per-machine MSI with the Broker binary exclusively under Program Files.

## Implemented

- A WiX v5 x64 MSI defines \`AkulaHostBrokerResearch\` as a **manual demand-only** LocalSystem service, with \`--service-custody-pipe\` and a dedicated unrestricted Service SID plus built-in Windows Installer service removal. The MSI does NOT start it or create C:\ProgramData\AkulaHostBrokerResearch or any vault registration. There is no grant/enroll/write operation in this binary.
- The installer only includes the standalone \`host-broker-lab\` executable already merged in #485 and #489. The per-user NSIS packaging and installed Akula GUI are unchanged.
- A real hosted Windows runner builds the MSI with pinned WiX 5.0.1, installs it with msiexec and checks the exact SCM ImagePath, Manual+Stopped+LocalSystem and Service SID state, Program Files binary equality and ordinary-user-writable ACLs; it then uninstalls and verifies removal.
- A separate signature gate requires \`Get-AuthenticodeSignature\` to return \`Valid\` for both PE and MSI, with exact same independently supplied trusted leaf certificate thumbprint. On GitHub CI the binaries are intentionally unsigned and **MUST fail the gate**. Neither the unsigned MSI nor service executable is uploaded to a release.

## Acceptance versus release

This PR is accepted only if the **exact HEAD** passes actual Windows MSI creation and install/uninstall, no writable user paths, no authority marker bootstrap, no unexpected service start, unsigned candidate denial, and mandatory Akula Main PR checks if present. The signing key, issuer certificate and attested release pipeline are **not available**; therefore **production release remains HOLD** despite green CI.

A caller-supplied thumbprint is a diagnostic preflight parameter, **not a production trust anchor**. Before product activation it MUST be pinned in a protected release policy enforced independently of caller input. The signed service executable must be verified as installed after transactional copy, across upgrades and on service start. Windows Authenticode validity alone does not establish host root custody or prevent administrator-level attackers.

This MSI never provisions the protected ProgramData marker because policy enrollment is not designed. A truly trusted \`AuthorizedToolContext\` and live MCP \`read_note\` remain unavailable. A separate enrolled vault/root+local-replica+plugin VaultId policy and monotonic revocation witness are future hard gates.

## Sources

- [Microsoft Windows Service installation via WiX](https://learn.microsoft.com/en-us/dotnet/core/extensions/windows-service-with-installer?tabs=wix)
- [Microsoft Windows ServiceInstall MSI table](https://learn.microsoft.com/en-us/windows/win32/msi/serviceinstall-table)
- [Microsoft Authenticode cmdlet](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/get-authenticodesignature)
- [WiX v5 CLI](https://velopack.github.io/wix5-docs/docs/tools/wixexe/)

[Notion master](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de) · [Host base PR #485](https://github.com/Lalalalendia/Akula/pull/485) · [Joined SCM PR #489](https://github.com/Lalalalendia/Akula/pull/489)
