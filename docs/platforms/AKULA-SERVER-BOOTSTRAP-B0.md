# AKULA-SERVER-BOOTSTRAP B0 — SSH trust and dry-run contract

**Status: B0a EXPERIMENT, no actual SSH connector or installer.** Source: [AKULA-SERVER-BOOTSTRAP-02 in Notion](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34), also [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334). Runs on top of the normal Akula Core but is **inert and feature-gated** as `aquilum-core/server-bootstrap-b0-lab`.

## What B0a proves

The prototype defines a portable, UI-independent Rust contract for previewing a future *single-user Linux host* before any changes:

1. Accept a **strictly validated** host name / IPv4 / IPv6, SSH port, unprivileged user name and independently pinned SHA-256 digest of the SSH handshake public key.
2. Require a pinned package version, immutable SHA-256 package digest, and locally trusted signer label. **The label is not cryptographic signature validation**; that belongs to B1.
3. Obtain the **SSH handshake public key wire bytes** through a narrow `PreviewTransport` trait; SHA-256 is computed locally with Ring and compared in constant time to the pinned expected key. **If it differs, inspection MUST NOT run**; credentials and arbitrary commands are not part of this API.
4. Only after the exact key match: accept typed read-only observations about Linux OS, architecture, systemd, sudo eligibility, CPU, memory and disk. This B0a profile admits only Debian/Ubuntu x86_64, systemd, sudo eligibility and modest **experimental** min 1 CPU, 512 MiB RAM, 1 GiB free disk. The limits are provisional lab admission values, not production resource requirements.
5. Generate an **inert** deterministic `DryRunPlan`: stable InstallId, bound SSH host/port/user/key digest, pinned package metadata, typed observed facts and an explicit six-step proposal. The domain-separated BLAKE3 plan digest changes if any material input or action changes; the hash is NOT an authorization token, signature, proof of installation, or automatic approval.
6. Honor cancellation before contact, after SSH-key verification and after inspection, without producing a plan or changes.

The six *proposed, never executed* operations are: verify a signed pinned release; create a dedicated OS account; prepare dedicated runtime/state directories; write a restricted systemd unit; bind the future service to loopback only; prove restart and synthetic readback.

## Critical boundary

- There is **NO actual SSH handshake/host-key authentication** in this PR. The transport is a trait and the tests use a fake implementation. The first B0b SSH adapter must cryptographically bind `handshake_public_key` to the key negotiated and proven during SSH key exchange; it may NOT simply ask the unauthenticated server what key it claims to have. No trust-on-first-use or silent changed-key acceptance.
- There is **NO credential field**, remote shell execution, arbitrary command API, sudo call, Docker socket, VM access, key persistence, CI access to production servers, installation, systemd changes, package download, NodeId assignment, device pairing or public HTTP access.
- The `inspect_host` method is assumed to represent a fixed allowlist of read-only checks **only after authentication to the pinned server**. This is a pure contract; B0b must implement and test this invariant with a real hostile/fake SSH server, then disposable Linux VM. No method accepts or executes arbitrary shell strings.
- B0a only rejects insufficient `has_sudo_rights` and records that privilege elevation might be needed for **future** B1; it does not request, use, or cache sudo privileges.
- Neither the host-key SHA-256 array nor signer label are secret credentials. A digest of a plan is NOT a signed/approved plan. B1 needs package signature verification, anti-rollback, exact plan approval bound to an authenticated operator, journal and idempotent InstallId.
- Failures use small typed `B0Error` values; they contain neither host observation contents nor secrets. Client UI can show a warning without leaking credentials.
- Plain `ReadOnlyHostFacts` are **observations**, not a proof of OS integrity or rights. A machine passing B0a is not installed, approved, safe to expose to WAN, or authorized to write user vaults.
- Do **NOT** connect B0a to the feature-gated D2 lab Node's HTTP or install it on a working router. The first supported machine is disposable Debian/Ubuntu x86_64; ARM64, TV, Android and RouterOS are later gates.

## Test matrix

File: `aquilum-app/core/tests/server_bootstrap_b0.rs`

- Stable digest across identical InstallId+input and proposed changes; modified host/package/facts/actions invalidates it.
- Host key mismatch and too-short public-key blob stop **before** collecting any remote observations.
- Dangerous host/user syntax, invalid ports, missing host-key pin and nil InstallId reject **before** transport calls. Numeric IPv6 is accepted as a typed address.
- Malicious/unspecified package version, signer and digest reject before any contact.
- Unsupported Linux, ARM64, missing systemd, no sudo eligibility, low RAM/disk/CPU fail closed.
- Cancellation at all three gates; failure of host-key access or read-only facts cannot result in a plan.
- Different InstallId, port or package digest produces a different approval digest.

Local test:

```sh
cd aquilum-app
cargo test --locked -p aquilum-core --features server-bootstrap-b0-lab --test server_bootstrap_b0 -- --nocapture
cargo check --locked -p aquilum-core --no-default-features
```

Dedicated PR workflow: `.github/workflows/core-server-bootstrap-b0.yml`.

## Next gates, not implemented here

**B0b: actual pinned SSH handshake & fixed read-only probe.** Implement through a reviewed SSH library (or narrow OpenSSH process adapter with strict key pinning, argument separation, dedicated known-hosts identity, noninteractive fixed commands), a fake SSH server that changes the negotiated key, and a disposable Linux VM. Verify 0 transmitted credentials before trust; failed auth/sudo, cancellation and lost connection return a refusal, not permission to retry unapproved operations. A controlled `sudo -n` probe might be allowed *after host key authentication* but cannot mutate or prompt for privileged passwords.

**B1: separate explicit installation transaction**, pinned signed package + measured systemd service + durable journal and rollback, test empty vault only; no mobile/remote file writer. The B0a `DryRunPlan` itself must never be an executor.

**B2+: actual device identity, permission scopes, revoke and protected transport** before personal Android / Internet / Roha deployments.

This experiment must not block Desktop Beta, File Gate, or safe two-client Sync acceptance.
