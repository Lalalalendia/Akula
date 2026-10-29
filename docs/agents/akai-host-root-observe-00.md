# AKAI-HOST-ROOT-OBSERVE-00 — physical root identity from Windows HANDLEs

**Research-only root observation, not enrollment and not MCP authorization.**

## Why

The current Broker policy kernel has a structural \`PinnedRoot\` (volume and file identifier), but it exists only under \`cfg(test)\` and can be populated from arbitrary bytes. Without an OS observation, it is not a real physical local vault binding. The standalone Broker also has a pinned protected ProgramData marker, but that marker is not a registered vault.

## Implementation

- \`windows_root_observation.rs\` opens a strictly drive-absolute **local fixed disk** directory path; UNC, drive-relative, ADS, dot-dot and special Windows path prefixes are denied. It opens **every ancestor** through real Win32 \`CreateFileW\` with \`FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS\` and no \`FILE_SHARE_DELETE\`; any non-directory or reparse component is denied. All ancestor HANDLEs remain open for that observation, but Windows can STILL rename directories while they are open. A read-only `still_names_this_root` diagnostic reopens the path and compares FILE_ID_INFO; a missing or substituted path is a mismatch/refusal. This is only a snapshot, not a secure future read.
- The final opened HANDLE supplies \`FILE_ID_INFO\` (64-bit volume serial and full 128-bit file ID), **not** a path hash or caller-supplied UUID. Missing/zero/unsupported identities are refused. Distinct directories on a fixed volume should differ; repeated opens of the same directory should match.
- Native Windows tests exercise repeated/sibling identity, refusal of file/relative/UNC/traversal claims, a real junction, and a real rename attempt: if Windows permits the rename, the original path no longer names the pinned object, and the moved path retains the original identity. The existing Foundation workflow covers Rustfmt, Rust tests, Windows SCM, Linux compile and no production activation.
- This is an **internal**, unused Broker module: no installer issuance, no agent grant, no JSON-RPC argument, no transport reply field. Existing 32-byte IPC remains deny-only.

## Known gaps

This observed (volume,file ID) pair identifies a **live** filesystem object on one Windows computer but is not an immutable or cryptographic identity. Filesystems can reuse identifiers over time; volume cloning, recovery, running as Administrator, storage remount, in-place rollback, hardlink/alias or an ancestor renamed **while HANDLEs remain open** can invalidate path-to-object mappings. Rechecking a path cannot eliminate a subsequent TOCTOU window and must never issue authority. A registered physical root must be bound to a local replica identifier and canonical \`plugins::VaultId\` by an **independent protected, signed installed issuer**; those components are absent.

Further root-validation gates require fixed ancestry ownership/DACL; FileIdInfo alone does **not** authorize reads and is not evidence of a trusted issuer. The next implementable slice is an OS-protected enrollment ledger with binary provenance, monotonic policy epoch, cold-open behavior and service-authenticated caller. The existing per-user NSIS and MCP \`read_note\` remain unchanged and default-deny.

Authoritative Windows docs: [FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info), [GetFileInformationByHandleEx](https://learn.microsoft.com/en-us/windows/desktop/api/winbase/nf-winbase-getfileinformationbyhandleex).

[AI-agent master plan](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de)
