# AKULA-DEPLOY-D2 — scoped local read API (staging only)

**Status:** experimental integration, NOT a release. Based on [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334). Builds on [D1 #409](https://github.com/Lalalalendia/Akula/pull/409) and [D1.5 #415](https://github.com/Lalalalendia/Akula/pull/415), both independent Draft PRs at the time of implementation. Do not merge this branch until both dependencies pass their own exact-head CI and security review.

## Proof boundary

- Feature-gated Linux-only `aquilum-core/node-lab` also enables the distinct `node-catalog-lab` module. Ordinary Core and installed Akula DO NOT enable them by default.
- Existing D1 standalone process still uses the same hardcoded `127.0.0.1` binding, fixed 3-second total header deadline, one request per socket, maximum 32 requests and 3-second socket I/O timeout.
- Optional **operator-supplied** `--vault-id UUID`, repeated `--catalog-note NAME` and `--reader-note NAME` enable multi-document mode. The catalog is admitted using a **duplicate of the D1 host's already-pinned root directory fd** (no second pathname lookup after the host's inode preflight), with at most 16 top-level Markdown notes of at most 64 KiB each. The process refuses unknown, duplicated or noncatalogued reader names **before calling Core::open**.
- A single test Bearer token is associated with exactly one static scope `reader_ids`. `GET /v1/documents` returns only granted filenames and stable VaultId-scoped IDs; `GET /v1/documents/<64hex-id>` reads permitted original UTF-8 content with raw-byte BLAKE3 revision. `?rev=<64hex>` requests conditional revision verification; stale => 409.
- **Authentication precedes ALL metadata/permission checks and file access.** Unknown and known-but-ungranted IDs produce the same 404. Scope is checked before the pinned filesystem read. Invalid paths never enter a file resolver. Only `GET` is available. No writer, upload, editor mutation, MCP router, S3, network relay or public listener is added.
- The previous single-note `GET /v1/note` remains available only when multi-document mode is disabled (to preserve existing D1 tests); in scoped multi-document mode that route is explicitly **404**, preventing bypass of the narrower reader permission set.

## Lab-only invocation (not a deployment guide)

Create isolated, disposable vault and state directories, with e.g. `public.md`, `hidden.md`. Start a developer build with a **fresh random** token and an explicit test vault UUID. Never use production/user data or publish this endpoint through a reverse proxy:

```sh
cd aquilum-app
cargo build --locked -p aquilum-core --features node-lab --bin akula-node-lab
export AKULA_NODE_D1_TOKEN="$(openssl rand -hex 32)"
cargo run --locked -p aquilum-core --features node-lab --bin akula-node-lab -- \
  --vault-root /tmp/akula-d2/vault --appdata /tmp/akula-d2/state \
  --note public.md --vault-id 00000000-0000-0000-0000-000000000001 \
  --catalog-note public.md --catalog-note hidden.md \
  --reader-note public.md --port 0 --max-requests 12
```

The process prints `AKULA_NODE_D1_BOUND=127.0.0.1:PORT`; `GET /v1/documents` requires Authorization: Bearer and lists ONLY public.md. Knowing the derived ID for hidden.md does not grant access. The token is not a device identity and the CLI VaultId is **operator-asserted but not independently host-registered**, so this is neither a multiuser service nor real File Gate custody.

## Integration tests and regression gate

The new `core/tests/headless_node_d2.rs` opens actual TCP connections to a separate OS process. It checks:

1. Without token or with incorrect token => 401, before list/metadata.
2. A two-document catalog lists only the single granted document, never hidden file/ID/content. Known ungranted ID => 404; unknown ID => 404.
3. Allowed ID => exact original Markdown bytes including CRLF and raw BLAKE3; wrong expected revision => 409, matching => 200.
4. Non-GET => 405; traversal-looking URL => 404; legacy single-note route => 404 while scoped mode is active.
5. Same filename under another typed VaultId gets a different ID and previous ID is not accepted.
6. A reader filename not in operator-admitted catalog refuses startup **before writable Core appdata initialization**.
7. Replaced leaf inode after admission returns 409 and does not leak new content.
8. Vault bytes remain unchanged on normal run; the existing D1 test suite continues checking absolute HTTP deadlines, pin-root behavior, forced process death and RSS.

The dedicated workflow `core-node-d2-scoped-read.yml` executes three test suites: D1 real-process security (9 tests), D1.5 catalog including an externally pinned root FD race regression (6 tests), D2 end-to-end (4 tests), plus default-feature Core check. A green profile is necessary but **not sufficient** for shipping or main merge.

## Remaining security blockers

- Production VaultId/host authority is not established by a CLI-provided UUID; do not infer registration or trust from object identifiers or directory names.
- There is only ONE test token and ONE static scope. No persistent device identity, per-device token, expiry, revoke, certificate pinning, TLS, approval UI or per-document ACL lifecycle.
- A hostile same-user writer can race a read of the same inode; metadata before/after are not a cryptographically immutable snapshot. No guarantees for hostile root, kernel or hardlinks.
- No cancellation of the blocking accept() loop, bounded total lifetime, shutdown on idle, load testing, external reachability, Android, ARM64, OCI, RouterOS, S3 or P2P sync.
- No remote write exists; any future write must be separately admitted via Host/File Gate with durable CAS and trusted identity.

**Admission gate:** keep Draft/unmerged until D1 #409 extended CI + App gate, D1.5 #415 exact-head CI + App gate, AND D2 combined CI pass. Security review must verify single-note bypass denial, no metadata leakage, root lifetime, revision edge cases and ownership boundaries.
