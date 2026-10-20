# AKULA-DEPLOY-D1: read-only personal Node loopback lab

**Status:** experimental proof, not a server product or a release. Based on [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334). D0 proof: [PR #405](https://github.com/Lalalalendia/Akula/pull/405) and [CI](https://github.com/Lalalalendia/Akula/actions/runs/39320538026) (green, still separate Draft).

## Scope

A deliberately tiny Linux-only server process, opt-in via `node-lab` feature on aquilum-core. It is **not built** with the default feature set nor installed as desktop Akula. The code uses Core for process lifecycle and private SQLite/settings state and opens its only served Markdown note using a pinned Linux directory file descriptor. No Native GUI, Masonry, WebView, S3, Staya, MCP tool dispatch, full-text index creation, File Gate write path, plugin runtime or sync activation.

**Threat boundary / constraints:**
- Hardcoded IPv4 loopback (`127.0.0.1`), not configurable to LAN/WAN; one request per TCP connection; max 32 requests before automatic shutdown, with 3-second I/O timeouts **and an absolute 3-second HTTP-header receipt deadline** (a trickling peer cannot reset the deadline).
- Explicit 64-character hexadecimal token supplied by operator in environment variable; all endpoints require one Bearer authorization header, with constant-time verification. No CORS, logs of content/token or unauthenticated health metadata.
- Exactly one server-configured, top-level `.md` filename; incoming URL has **no path parameter** and cannot choose a file. No listing/search on untrusted input. `GET /health` and `GET /v1/note` only, all mutations denied.
- Operator supplies existing `--vault-root` and disjoint writable `--appdata`; reject overlapping paths before Core startup. Root opened once through `open(O_DIRECTORY|O_NOFOLLOW)`; contents read through `openat(pinned_fd, allowed_filename, O_NOFOLLOW)`, not a second pathname lookup.
- Limit header to 8 KiB and source to 64 KiB; reject request bodies/pipelining, symlinks, nonregular content, oversized files and changing file metadata. The result includes BLAKE3 of the raw returned bytes; bytes are read only from a trusted allowlisted file.
- No security claim against a fully hostile same-user process modifying the contents of the already-open regular file, a compromised host/root, malicious hardlinks inside the trusted vault or shared-key theft. A two-stat check detects many ordinary concurrent changes but is **not** an immutable filesystem snapshot. These are D2+ threat-model questions.
- This API is not accessible to browsers on other devices; there is no authenticated WAN/mobile TLS entrypoint yet.

## Developer-only example (disposable directory, no secrets in CLI)

```sh
cd aquilum-app
cargo build --locked -p aquilum-core --features node-lab --bin akula-node-lab
# Create /tmp/akula-d1/vault and /tmp/akula-d1/state yourself.
# Put one small allowlisted allowed.md inside the vault.
export AKULA_NODE_D1_TOKEN="$(openssl rand -hex 32)"
cargo run --locked -p aquilum-core --features node-lab --bin akula-node-lab -- \
  --vault-root /tmp/akula-d1/vault \
  --appdata /tmp/akula-d1/state \
  --note allowed.md \
  --port 0 \
  --max-requests 8
```

The binary prints `AKULA_NODE_D1_BOUND=127.0.0.1:PORT`. To read:

```sh
curl --fail-with-body -H "Authorization: Bearer $AKULA_NODE_D1_TOKEN" \
  http://127.0.0.1:PORT/v1/note
```

Replace `PORT` with the emitted port. This is a disposable developer demonstration, not an operating guide for a shared or persistent server. The prototype ends after its configured request count, and it MUST NOT be put behind a reverse proxy.

## Test acceptance

`core/tests/headless_node_d1.rs` starts **real subprocesses**, speaks actual TCP, checks:
1. No authorization and incorrect authorization => 401.
2. Correct token => health and exactly one permitted note, raw bytes and BLAKE3 match; source file tree unchanged after shutdown.
3. Parent traversal and encoded traversal in URL => 404, POST => 405, Content-Length => 400.
4. Allowed-name symlink to an external private file => 403.
5. 64-KiB+1 content => 413 (bounded read).
6. Symlink root, operator-supplied traversal filename and overlapping vault/appdata => fail before listening/writing state.
7. Pinned directory survives replacement of the original vault pathname after startup: a later request still reads the originally opened root, not the substituted path.
8. Force SIGKILL after one successful request, restart a new process with the same private appdata, verify source vault bytes and read permission remain correct.
9. Drip an incomplete HTTP header over multiple socket writes and require response 408 within the absolute request budget (not per-chunk read timeout).
10. Read the maximum allowed 64-KiB fixture 16 times, capture process VmRSS from Linux /proc and assert a developer-lab ceiling of 512 MiB, without claiming an ARM64/RouterOS resource profile.
11. Dedicated Linux job and ordinary `--no-default-features` Core check; desktop installed build unaffected.

Run:
```sh
cd aquilum-app
cargo test --locked -p aquilum-core --features node-lab --test headless_node_d1 -- --nocapture
```

## Gates still open before a usable personal Hub

- **D1 lab green CI alone is not production assurance.** The dedicated tests now exercise a 3-second total HTTP-header deadline, root rename, SIGKILL/restart and a basic RSS ceiling; **independent security review and physical resource measurements remain open** for credentials lifecycle, hostile local writes, disk races, sustained traffic and CPU. No static operator token in distributed packages.
- Next step: a reviewed **public read contract** shared by Core and separately packaged Node; per-object ACL, stable identity, revisions, multiple files with safe root-relative lookup, event and listing limits, authenticated encrypted network transport or trusted VPN. A single test token cannot identify or revoke different clients.
- Separate cross-process File Gate/Sync authorization before any write endpoint. Current MCP tool registry must not be re-used as the user-facing Node API.
- Linux/ARM64, multiarch OCI, Android, TV and RouterOS have **not** been tested here. Do not expose any of this to WAN or put it on a production router.

**Decision:** keep experimental Node as its own opt-in binary behind a feature; do not create a monolithic UI+Server executable or second independent File Gate.
