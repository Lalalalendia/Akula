# AKAI-AUTH-01b1a — pinned scoped Markdown read, independent lab

**Status:** isolated Core read candidate, **not a shipping MCP tool or network capability**. This Draft PR is stacked on [#426](https://github.com/Lalalalendia/Akula/pull/426); don't merge until #426 has passed and been merged/rebased. [Agent master plan](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de).

## Problem solved in this slice

The historic MCP `Vault::note` resolves titles/pathnames and later calls `read_text(path)`. The existing symlink/junction preflight is not an OS-handle-relative read and can be raced after the path check. Do not insert a `NotesRead` check into the old tool and call the result secure.

Core already has a pinned directory reader in `sync::pinned_read::PinnedReadOnlyRoot`, implementing Unix `openat(O_NOFOLLOW)` and Windows NT parent-HANDLE-relative child opens. Reuse it. This PR adds `read_bounded_single_link`: the same opened file is checked for a single hard link and size/identity stability. Hardlinks can otherwise redirect authorized reads to content reachable outside the enrolled vault.

## Isolated candidate contract

`mcp::agent_scoped_read::HostPinnedReadRoot` holds both the checked `PinnedProbeRoot` directory identity and `PinnedReadOnlyRoot` handle. Its **only constructor is `#[cfg(test)]`**: there is deliberately no production way to convert an untrusted filesystem path, arbitrary digest, workspace string or `VaultId` into read authority. It must later be created only by an independent, trusted, OS-protected host-custody witness.

`read_checked(credentials, grants, bearer, requested_vault, requested_note)`:

1. Require exact match with the host receipt's `VaultId` before any filesystem lookup.
2. Reauthenticate the high-entropy host-issued agent bearer, and verify current root identity and `NotesRead × WholeVault` grant.
3. Parse a single **exact portable relative `.md` path**, reject absolute paths, dot paths, hidden system entries and path normalization, and **never** resolve by fuzzy title, enumerate other workspaces or permit batch/mixed operations.
4. Open each component relative to the pinned OS handle, no symlink/reparse, no hardlinks, bounded to 2 MiB and validate file state/hash before returning content.
5. Verify physical root name identity, bearer revocation and grant epoch again **after reading bytes**. If anything changed, discard the bytes and return a generic denial.

The response candidate contains only the relative path, Markdown string and revision hash. It does not expose absolute host roots or error-specific file existence. Revocation tests inject a committed grant or credential revoke **between** opened-byte-read and final candidate return.

## Tests (must pass native Windows + Linux exact HEAD)

- Agent A reads nested X/note, B reads nested Y/note; A→Y, B→X and false vault ID deny.
- Absolute, parent, noncanonical, hidden, wrong-extension and foreign paths deny.
- A foreign document hard-linked inside an authorized vault denies.
- Unix direct/ancestor symlink escape denies; Windows tests use existing NT reparse-aware reader.
- Host root replaced/renamed after opened read denies, as does forged physical-root claim.
- Agent credential or grant revoked between bytes-read and return denies; no candidate bytes leave the function.
- No codepath invokes the old unrestricted `tools::call`.

## Still BLOCKING real MCP reading (01b1b)

1. Installer-controlled **independent** signed/permissioned host-custody witness needs to bind the pinned root's physical identity to a stable host-owned VaultId. The existing `sync::HostCustodyRegistry` is feature-gated and itself documents its trust-boundary limitations; do not self-certify by reading the same mutable SQLite store.
2. The `plugins::VaultId` vs `sync::VaultId` mapping must be **explicitly host-enrolled and immutable**; matching UUID values alone prove nothing. Introduce a first-class `AuthorizedToolContext` and one shared policy decision path, not a fake plugin identity or independent unrestricted dispatch.
3. The final network response must hold and revalidate its policy/credential lease **at the point of writing to the connection**. A post-read check alone does not prevent a revoke after the function returns; either serialize the response with revoke or establish an explicit response/fence policy. Never claim atomic revocation from this candidate.
4. In a separate PR, enable only `read_note` for scoped agents after real-custody and response-fence proofs; keep other 29 tools and all metadata/search/write inaccessible. Prove two independent official MCP SDK clients, real enrolled roots and revoke-vs-response on Windows/Linux.
5. Agent-consent management and installer ACLs must be decided and accepted before enabling any installed app setting. Legacy global bearer must not be a fallback.

**No release claim:** CI PASS here means only read-candidate correctness on test-issued roots, **not** installed multiagent authorization, server-side grants, or production OS trust.
