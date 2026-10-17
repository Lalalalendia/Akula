# ADR-SYNC-002 — Pinned read-only Unix preflight (Stage 04g-1)

Status: **experimental, not a write authority** (2026-10-17).

## Intent

Create a small proof that a local file can be opened for **bounded reading** through directory descriptors pinned to a host-selected workspace root. Do not turn the existing CoreVaultScanner's path-only observations into permission for upload, overwrite, rename, delete, or baseline commit.


## Stage 04g-2 — Windows read-only NT handle proof

- Windows now opens the initial trusted workspace directory with Win32 OpenOptions and FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, checks that the opened handle names a directory and not a reparse point, then retains this handle.
- Relative components use NtCreateFile via a narrowly scoped ntdll FFI. Every call sets OBJECT_ATTRIBUTES.RootDirectory to the already-open parent handle and supplies exactly one validated UTF-16 component.
- NtCreateFile uses FILE_OPEN (existing only), read-only access, FILE_OPEN_REPARSE_POINT and OBJ_DONT_REPARSE; intermediate components require FILE_DIRECTORY_FILE and the final component FILE_NON_DIRECTORY_FILE. Opened reparse objects are explicitly rejected through FILE_ATTRIBUTE_REPARSE_POINT.
- This implementation requests no writable access and provides no mutation, durable snapshot, remote upload permission, or File Gate write authorization.
- The host must still bind the starting root to the registered workspace and check directory identity/permissions, including NTFS volume boundaries, before this code can be used as an authority. This code remains crate-private.
- The Windows CI exercises nested Unicode files, strict bounds, refusal to traverse an NTFS junction, and an attempted rename/replace of the pinned root path. If a test fails on Windows it is a blocking finding, not a reason to disable the security assertion.

**Security caution:** This first stage is not a complete Windows sandbox. Windows device namespaces, hard links, mount points, ancestor reparse points at the initially provided root pathname, concurrent mutations of an already-open file and filesystem-provider-specific semantics remain outside this read-only probe. No path-based fallback or destructive application is authorized.

## Scope

- On Unix, the root is opened as an FD with directory and no-follow flags. Each child directory is opened with O_DIRECTORY | O_NOFOLLOW, and the leaf uses O_NOFOLLOW. The root FD remains live even when its pathname is renamed or replaced.
- Read is capped at 32 MiB. Before reading, require a regular file. After reading, compare metadata, size and the captured bytes. Return a BLAKE3 content hash only for the bytes actually captured.
- On Windows, the 04g-2 prototype opens children through an NT directory handle relative to its pinned parent. There is no path-based fallback. No Windows write authority is provided.
- The API is crate-internal; it does not implement VaultPort or hook up a sync worker.

## Limits and risks

1. Workspace registration and UI editor-buffer admission remain outside this prototype. A caller must never select a root from an untrusted remote manifest.
2. This does **not** freeze external writes or provide an atomic full-tree snapshot; same-size concurrent changes can evade metadata comparisons. A directory may be renamed while it is open. No destructive operation may rely on this result.
3. The O_NOFOLLOW requirement rejects symlinks at each path component on Unix, but the initial ambient path to a registered root can contain parent symlinks. Registration/root identity and mount-boundary policies need a separate OS authority design.
4. An open file or directory may be moved after opening. Do not infer current directory membership from a pinned descriptor, and do not treat hard links or changed mount topology as safely resolved.
5. Cross-platform security requires a Windows descriptor/handle implementation, macOS race testing, bound-buffer coordination, durable protection receipt and an atomic mutation executor. Until those gates pass, no S3 or Sync UI activation.

## Test evidence required

- Unix: root pathname replacement does not redirect an already pinned reader, intermediate/leaf symbolic links are rejected, nonregular files and oversized reads are rejected.
- Windows: NTFS junction traversal and root reparse points fail closed, Unicode nested reads succeed, and replacing a root pathname does not redirect an already-pinned handle.
- GitHub Actions: Linux, macOS and Windows targeted tests; Main PR gate with workspace + sync-engine tests.

## Follow-up

Stage 04g-2 ties trusted root registration and DocumentHub + unsent host editor buffers to the OS authority, with platform-specific handle identity verification. Stage 04g-3 adds durable protective copies, safe atomic write and readback, then journal commit. No direct write path is introduced by this PR.

Canonical roadmap: https://telawiki.com/spaces/456/pages/7479/akula-pr-00-11
