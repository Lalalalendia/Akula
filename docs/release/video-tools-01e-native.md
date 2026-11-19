# VIDEO-TOOLS-01e — native Windows read-only discovery and ACL admission

**Status: NON-RELEASE.** Extends #1045/#1047/#1049. The standard two-EXE Akula installer and VIDEO-18 RELEASE HOLD are unchanged. No tools are launched or published.

## Native boundary

On installed Windows builds, native Rust computes the optional package location exclusively using SHGetKnownFolderPath(FOLDERID_ProgramFilesX64). It rejects pre-Windows-10, non-x64 processes, missing/non-directory ancestors and junction/reparse parents. It does not trust ProgramFiles, PATH, current directory, or caller-selected executable paths.

The only future product root is OS-known Program Files / "Akula Video Tools". The existing GUID disposable NSIS 01a fixture is NOT a discovered production package.

The native ACL gate checks root plus the fixed complete signed payload: manifest.json, manifest.minisig, exactly three video EXEs and uninstall.exe. Every object is regular/non-reparse and has a non-null DACL, owner belonging to LocalSystem, Administrators or TrustedInstaller, and no explicit/inherited mutation grant for any other SID. Unknown grant shapes fail closed. Generic write/all, change ownership and change DACL are rejected.

The probe returns only diagnostic states: Missing, Unsafe, PublisherNotApproved, ReadOnlyAttested. Even ReadOnlyAttested is NOT permission to launch; execution will require handle-bound TOCTOU review. No executable path or handles are returned. APPROVED_VIDEO_TOOLS_KEY remains empty and shipping_admission_enabled() remains false, so ReadOnlyAttested is unreachable in this release.

## CI evidence

- Rust native module is compiled on genuine hosted Windows as an isolated test crate and lab witness, using the same source as the installed application.
- Negative test spoofs ambient ProgramFiles and adds untrusted WriteData at protected root and ChangePermissions on a real installed EXE.
- Every unsafe grant must cause native witness refusal; the exact prior DACL is restored before uninstall. Native source contracts run on Ubuntu and fail if a security check is deleted.
- The job requires VIDEO_TOOLS_01E_NATIVE_WIN32_PASS and remains exact-head, PR-only, no owner keys, no artifact upload, no release token.

## Remaining release blockers

Trusted publisher key custody, actual package signing/receipts, source and PE provenance acceptance, handle-bound TOCTOU-safe execution, real user-mode denial test, Windows 10/11 signed install/uninstall matrix, and the user-facing availability/error flow. Optional package remains manual-install/manual-update. VIDEO_FIVE_CHANNEL_READY remains false.
