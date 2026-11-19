# VIDEO-TOOLS-01f — standard-user Windows access denial (non-release)

This step follows the genuine hosted Windows NSIS, ACL and native Rust tests in #1043, #1047, #1049 and #1050. It does not authorize launching video binaries, publisher signing, installation on end-user machines, or release.

## What the test proves

On a disposable, exact-HEAD GitHub-hosted Windows PR runner, after installing the real three PEs into a GUID-only Program Files laboratory, the test creates a uniquely named local Windows account with a strong randomized password. It logs on through Start-Process -Credential and checks that the resulting process has the account's own SID and is not an Administrator.

The child proves a genuine ability to read one installed EXE, then calls Win32 CreateFileW with seven mutation-capable rights against the actual installed Program Files root and EXE. Each request must be refused with ERROR_ACCESS_DENIED (5): creating a child, deleting a child, writing the EXE, changing attributes, deleting the EXE, changing its ACL, and taking ownership. An arbitrary failed process launch or missing file cannot count as success.

The elevated parent verifies all three PE hashes plus the uninstaller hash and the unchanged ACL policy, then removes the disposable user in finally, checking that removal succeeded. Source-mutation contracts run separately. Both workflows remain PR-only, read-only token, non-release, and require the witness VIDEO_TOOLS_01F_NONADMIN_PASS.

## Explicit limits

The user-token test requests real access rights, without deliberately writing bytes or actually executing Video Tools. It does not eliminate the handle lifetime/TOCTOU issue; a prior digest check is not a guarantee about a later executable. The publisher-owned signing key remains unapproved, shipping admission stays false, and VIDEO-18 RELEASE HOLD remains in effect.

**Next gate:** handle-bound verification of exact signed file bytes and a race test preventing replace/delete while the verified handles are retained. No production launch until execution-boundary integrity is separately proved.
