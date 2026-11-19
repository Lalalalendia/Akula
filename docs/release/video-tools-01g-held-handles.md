# VIDEO-TOOLS-01g — held Windows file handles and concurrent mutation proof

**Status: NON-RELEASE / no executable launch.** Continues merged VIDEO-TOOLS-01d, 01e and 01f. Stock Akula 0.3.21 remains the two-EXE application; the independent optional Video Tools publisher signing key is still unapproved. VIDEO-18 RELEASE HOLD / `VIDEO_FIVE_CHANNEL_READY=false` remains unchanged.

## The TOCTOU gap

Previous native `verify_signed` authenticated a Minisign manifest and PE SHA-256 by path, but each `File` was closed after verification. A future path-based launcher could therefore observe a different file after an earlier successful check. Checking the same pathname twice is not a sufficient defense.

## What this change implements

- The native Windows module `tools_package_pin_windows.rs` opens the package **root directory** and the fixed list of components with `FILE_SHARE_READ` only — never `FILE_SHARE_WRITE` or `FILE_SHARE_DELETE`. Files and ancestors reject reparse points. The root and all component handles stay owned by a single guard.
- `tools_package_pin_admission.rs` is a read-only diagnostic bridge: it first requires a fixed OS-known Program Files package identity and protected ACLs, then opens all six declared package files (three PEs, manifest, Minisign signature and uninstaller). While the handles are held, it calls the existing authenticated Minisign verification and reads **the same open handles** to compare manifest bytes, signature bytes and all three PE SHA-256 digests. It returns only an opaque guarded diagnostic value; never a path to execute.
- The current production trust gate is unchanged: `APPROVED_VIDEO_TOOLS_KEY` is empty and `shipping_admission_enabled()` is false. Thus no signed package is authorized by this PR. The uninstaller is held and checked as a regular protected file, but its own digest is not currently present in the signed manifest.
- The real GitHub-hosted Windows 2025 NSIS laboratory compiles the **actual native Rust handle source** both as unit tests and as a test-only holder. It matches the three installed executables to staged original bytes, then holds the four installed files and their directory while the elevated parent requests Windows write/delete rights.
- Every denied request must produce **ERROR_SHARING_VIOLATION = 32**, not merely `ACCESS_DENIED=5`: four write attempts on files, five delete attempts on files/root. A positive control proves a write handle can be opened **after** the guard closes. Hashes and the protected ACL policy are rechecked before ordinary NSIS uninstall.
- The PR-only CI also exercises source mutation contracts, keeps read-only GitHub token and performs no signing, staging upload, runtime fetch or release.

## Acceptance

The Windows job must emit both `VIDEO_TOOLS_01G_NATIVE_PIN_PASS` and `VIDEO_TOOLS_01G_PIN_RACE_PASS` after testing real files. Source-contract tests and Application CI must be green before merger.

## Important limits

Held handles prevent ordinary concurrent write/delete openings during their lifetime; they **do not** by themselves solve the full path-based `CreateProcessW` hand-off after releasing those handles. An already-running process, a highly privileged kernel attacker, hard-link abuse beyond the tested scenarios, power loss and recovery remain separate threat analyses. Do not claim execution integrity or enable downloads from this PR.

**Next gate:** design and prove the verified-handle-to-process boundary with publisher-controlled signed releases and actual Windows 10/11 end-to-end tests. No launch is added here.
