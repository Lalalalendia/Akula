# AKULA-DEPLOY-D1.5 — typed multi-document read catalog (prototype)

Parent: [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334).
Predecessor: [D0 PR #405](https://github.com/Lalalalendia/Akula/pull/405)
and [D1 loopback Node PR #409](https://github.com/Lalalalendia/Akula/pull/409).

**Status:** independent, developer-only *library contract* for a future personal Node.
It does not expose a socket, extend MCP, change File Gate, ship server packages,
or modify the already-running D1 CI. Optional feature: **node-catalog-lab**,
Linux only. Not compiled by installed Akula.

## Stable ID, not client-chosen path

A trusted local operator constructs ReadCatalog::open(vault_root, registered_vault_id, allowed_names). The argument is the existing strongly typed Core sync::VaultId; the runtime MUST independently validate that it identifies the exact protected vault. A local workspace ID/path is not an acceptable substitute.

- Root is opened **once** as a Linux O_DIRECTORY | O_NOFOLLOW dirfd;
  later reads use openat(dirfd, stored_filename, O_NOFOLLOW), not the root
  pathname. No absolute paths, dotdot, nested paths, symlink entries,
  control characters, backslashes, directories or non-Markdown names.
- Allowlist has 1 to 16 explicit top-level .md filenames. Every entry must
  exist, be regular and be at most 64 KiB at admission. The entire catalog
  fails on a missing, duplicate, oversized or otherwise invalid entry.
- Every admitted file has a stable 64-hex ID based on domain-separated
  BLAKE3 of the typed VaultId and operator-admitted filename. The ID is independent of admission
  order and stable across restarts for the same registered vault. Identical filenames in two different vaults receive different IDs. It is **not** a secret credential,
  authorization, object capability or immutable inode identity.
- A later request can resolve **only** an ID in the immutable catalog, never
  an arbitrary filesystem path. Device/inode identity is checked at read.
  Replacement or symlink swaps are denied until a trusted operator re-admits.
- Reads are limited to 64 KiB and check before/after device, inode, size,
  mtime and ctime. Return a raw-byte BLAKE3 revision, optionally require an
  expected revision. UTF-8 must be valid; original CRLF is preserved.
- Listing exposes only operator-admitted entries. Authentication and per-device
  scopes are **outside** this module and MUST precede its use by any host.

## Falsifiable test matrix

Test file: aquilum-app/core/tests/node_catalog_lab.rs.

1. Three Markdown files retain stable IDs regardless of allowlist order,
   and the same filename receives a distinct ID with a distinct typed VaultId.
   An unlisted fourth file and unknown ID cannot be accessed.
2. Raw BLAKE3 over the exact source bytes, including CRLF. A stale revision
   refuses to return the changed document.
3. Bad names, traversals, duplicates, excessive entries, missing files,
   symlink root, symlink leaf, and directory disguised as Markdown are denied.
4. Rename the initially trusted root and replace its pathname with a new
   directory: the catalog remains bound to the original pinned root. A
   replaced registered file inode is not silently reattached to its old ID.
5. Same-inode edits get a new revision; symlink swap is denied. Verify exact
   64 KiB boundary, oversized content and invalid UTF-8 refusal.

Workflow: .github/workflows/core-node-d15-catalog.yml

Commands:

    cd aquilum-app
    cargo test --locked -p aquilum-core --features node-catalog-lab --test node_catalog_lab
    cargo check --locked -p aquilum-core --no-default-features

## Nonclaims and the next gate

- **NO** public API, HTTP listener, multiuser access control, pairing, device
  authority, expiration, revocation or token handling in this change. IDs
  cannot substitute for user-specific ACL checks.
- Malicious same-inode mutation racing both metadata checks is **not**
  cryptographically prevented. Same-user hostile processes, hardlinks and
  compromised host permissions require their own threat model.
- No recursive paths, symlinked notes, media, searching, pagination, event
  streams, remote writes, remote synchronization, Android or RouterOS tests.
- This catalog is immutable within one process. Adding/removing an object
  or changing vault authority requires a separate, explicit handshake.
- **Do not wire this into D1 yet.** Await exact-head 9-test D1 CI and
  independent review of its transport/authority; then connect the catalog
  to a separately reviewed, authenticated and loopback-only Node endpoint.
- Do not infer product readiness or a changed Desktop release gate.
