# VIDEO-18b-03 — real protected NSIS installation laboratory

Status: **NON-RELEASE / TEST-ONLY / SECURITY HOLD**.

This stage goes beyond the VIDEO-18b-02 manually provisioned protected folder.
It compiles an actual NSIS setup using pinned NSIS 3.11.0 and installs five
**inert text markers named like EXEs**, not real Akula executables, in a
GUID-specific Program Files directory. The isolated test installer requests
administrator privileges and never registers the production Akula identity,
uses product update keys, downloads sidecars or publishes binaries.

The lab checks the installed file SHA-256 values against the staging originals.
A temporary real Windows user without administrator privilege must read all
five mock files, fail **11 actual mutations** (overwrite five, delete, rename,
inject an EXE, rename the installation directory, create a child directory,
overwrite the NSIS uninstaller) and successfully write a separate synthetic
local media file. The elevated NSIS uninstaller must remove its own test
directory without touching the media; cleanup deletes the disposable user and
all temporary paths.

The CI is PR-only on GitHub-hosted Windows and checks out the exact SHA.
A Node contract suite rejects insecure privilege level, per-user install
directories, missing source triggers and accidental release functionality.

**Unproven / release blockers:** publisher-signed real five-binary packaging,
migration from existing per-user LocalAppData installations, Authenticode/NSIS
signature admission, anti-TOCTOU and reparse checks for all updater parents,
atomic five-binary upgrades with journal/first-frame ACK/rollback and
verification on real supported Windows devices and media sources.

This is not a shipping installer. The VIDEO-18 release-HOLD and stock
two-EXE production NSIS must remain unchanged. No public release or deploy.
