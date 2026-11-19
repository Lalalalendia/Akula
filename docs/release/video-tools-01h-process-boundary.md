# VIDEO-TOOLS-01h — verified handle to benign Windows process boundary (NON-RELEASE)

**Not a Video Tools activation or shipping approval.** The separate Video Tools release remains blocked: `APPROVED_VIDEO_TOOLS_KEY=""`, `shipping_admission_enabled()=false`, `VIDEO_FIVE_CHANNEL_READY=false`, and VIDEO-18 RELEASE HOLD are unchanged. Stock Akula 0.3.21 still has only `akula-app.exe` and `shiva-pdf-preview.exe`.

## Question answered by the lab

01g demonstrated that Windows prevents opening an installed file for mutation while a read-only native handle with `FILE_SHARE_READ` remains open. It did **not** prove that `CreateProcessW` works under the same sharing contract, or that the guard can remain alive until the launched image starts.

01h adds a separate GitHub-hosted **Windows 2025 / pull-request-only** experiment. It never executes yt-dlp, the video mux or QuickJS. A harmless Rust program is compiled from reviewed source at PR HEAD and copied four times into an isolated, GUID-scoped **Program Files** directory. The four filenames match the existing 01g fixed laboratory identity solely to test the **same** `PinnedPackage` implementation; the bytes are identical harmless fixtures, not the installed Video Tools.

The native Rust parent calls the existing 01g `PinnedPackage::open`, revalidates the OS-known Program Files x64 root and full protected ACL, and compares all four **held file handles** byte-for-byte against the source-built fixture. With those exact handles still open, it invokes Windows `CreateProcessW` with an explicit absolute application name, no inherited handles, and no PATH search. The child reports image startup in a disposable runner directory. The original root and four file handles remain open **until after the child reports startup and exits cleanly**.

## Adversarial acceptance

With an elevated process (not merely an ordinary user), the lab requests write and delete access on the file/directory handles and attempts to rename a pinned executable **both before CreateProcessW and after the benign child reports startup**. All of those attempts must be denied by the kernel with `ERROR_SHARING_VIOLATION=32`, rather than ACL failure. A positive control first proves the elevated user **could** open a write handle before pinning, and again after the guard is dropped; no bytes are actually modified.

The test rechecks all four fixture digests, root and file ACLs, child exit code, and uninstaller-name isolation. A cleanup guard refuses to delete anything outside the exact GUID root or any unrecognized protected files. It does not modify the real stock Akula, actual video executables, normal user files, or any server.

## Limits of the evidence

- The process is a **benign lab program**, not a signed publisher release; the true runtime activation path remains disabled.
- A successful handoff demonstrates ordinary Win32 write/delete/rename resistance while verified handles remain open. It does not guarantee safety against a kernel attacker, preexisting malicious code, compromised OS, undocumented image-loader behavior on all supported Windows versions, or a compromised signing key.
- This does not yet bind a *real publisher-owned Minisign signature* all the way to a production child process. Owner-approved package signing, source/legal provenance, executable compatibility, end-user UI and signed Windows 10/11 acceptance remain separate gates.
- If Windows cannot create the process while the files are pinned, the CI must fail. The test must **not** relax `FILE_SHARE_READ` or release handles just to obtain green results.

**Release HOLD stays on.** No automatic five-EXE updater, no video download activation and no publisher signing from this PR.
