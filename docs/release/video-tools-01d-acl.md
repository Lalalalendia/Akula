# VIDEO-TOOLS-01d — fail-closed installed ACL laboratory (non-release)

This step extends the disposable Windows 10+ three-EXE NSIS laboratory in #1043 / #1047. It does **not** integrate Video Tools into normal Akula or authorize their execution, installation for users, publisher signing, or distribution. VIDEO-18 RELEASE HOLD and `VIDEO_FIVE_CHANNEL_READY=false` remain unchanged.

## Scope and threat model

The earlier #1043 check inspected only the installation root and only two write-like permissions. A separate EXE might still be modified via an inherited or explicit ACE, an unsafe owner, a null DACL, attribute changes, ACL changes, deletion, or ownership takeover.

The new read-only `Assert-AkulaVideoToolsInstalledAcl` routine:
1. Reuses the #1047 Windows system-known x64 Program Files and exact GUID lab-root constraint, including reparse rejection.
2. Requires exactly the three named video executables and independent `uninstall.exe`, with no extra files or directories.
3. Checks the installed directory **and each of the four individual files** for reparse status, non-null DACL and owner SID.
4. Rejects mutation rights granted to any trustee except LocalSystem, BUILTIN Administrators and Windows TrustedInstaller, including Create/Write/Append, attributes, delete, DeleteChild, ChangePermissions and TakeOwnership; an inherit-only ACE is not effective on the current object.
5. Fails closed on unknown owners or trustees. Even an explicit deny cannot turn an untrusted write-allow ACE into accepted shipping evidence.

On the actual hosted Windows PR runner, the isolated installer is exercised before the lab grants ordinary trustees WriteData and Delete on the root, and WriteAttributes and ChangePermissions on individual executables. Every unsafe grant must be rejected and the previous ACL must be restored before uninstall; the job requires `VIDEO_TOOLS_01D_ACL_MUTATION_PASS`. Static mutation contracts run separately on Ubuntu. The workflow has no deployment, signing or write token.

## What this does not establish

- No trusted publisher key: `APPROVED_VIDEO_TOOLS_KEY` remains empty and shipping admission is disabled.
- No native Akula directory discovery, signed manifest binding, receipt verification, handles pinned across execution, or TOCTOU-safe launch.
- No actual standard-user token denial proof, cleanly published package, Windows 10/11 distribution matrix, or publisher-controlled signing.
- Existing 01a fixture is not the future versioned publisher package, and this ACL guard is not itself activation permission.

**Next gate:** implement the same OS-known-root, owner/DACL and signed-manifest conditions in native Windows discovery; only then user-facing opt-in availability, with a separate execution-boundary review.
