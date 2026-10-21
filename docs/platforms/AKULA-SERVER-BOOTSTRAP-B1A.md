# AKULA-SERVER-BOOTSTRAP B1a — offline signed native release gate

**Status: experimental library, not an installer.** Source: [AKULA-SERVER-BOOTSTRAP-02 / B1](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34), related [AKULA-DEPLOY-01](https://app.notion.com/p/3fe32a84beec817cb9b7e7fa5a552334). B0a is merged; B0b real OpenSSH is separately awaiting general CI and B0c is separately proving an independent disposable Ubuntu VM. B1a is **independent of all SSH operations**.

## Boundary

Only the opt-in `aquilum-core/server-bootstrap-b1a-release-lab` feature compiles this portable module. Default Akula, File Gate, Sync, installed Windows, Node HTTP and real SSH are unchanged.

This code is a **candidate admission verifier**, not installation authority:

- A `TrustedReleaseSigner` is supplied from an **independent, trusted release-key source**; it is not obtained from an SSH server, the package archive or a download endpoint. The only signing key present in this PR is a fixed, clearly labelled, reproducible **test key inside a regression test**. This repo does not publish or approve any real Akula Node signing key.
- `SignedRelease` authenticates a typed Linux x86_64 manifest using Ring **Ed25519** and a deterministic versioned binary signing domain. Manifest fields bind package platform, numeric major/minor/patch, strictly increasing release sequence, exact original byte size, exact SHA-256, validity window and release signer ID. No ambiguous JSON canonicalization, user-controlled shell input, paths, package URLs, `latest`, or tag resolution.
- Fail closed on signature tampering, signer substitution, wrong public key, zero digest/size/sequence, unsupported/bad manifest fields, premature/expired signature or a metadata validity window exceeding 90 days.
- `ReleaseFloor` is an explicitly **separately persisted** monotonic release authority supplied by the caller, with sequence, semantic version and previous signed manifest fingerprint. A lower sequence or version refuses, and an **equal sequence with different signed content** refuses. An exact retry of the same authenticated manifest is allowed for idempotence.
- `VerifiedRelease::verify_package_stream` streams original package bytes through Ring SHA-256 with a hard 512 MiB maximum and exact length. A truncated, appended or bit-flipped artifact is refused before any installation. The output `VerifiedPackageReceipt` is an inert result: it has no ability to execute the package, open an SSH session, mutate a directory or create a service.
- The domain-separated BLAKE3 fingerprint identifies the authenticated manifest **and Ed25519 signature**. It is not a signature by itself, not a client-device identity, not a File Gate authorization, not a state snapshot and not proof of an installed binary.

## Test cases

`aquilum-app/core/tests/server_bootstrap_b1a.rs`, eight tests:

1. A valid ephemeral-test-signed manifest plus byte-exact test payload returns an inert receipt.
2. Altered version, release sequence, SHA-256, validity or signature after signing is denied.
3. Wrong signer ID or wrong pinned release public key fails.
4. Lower sequence, lower semantic version at higher sequence, or a same-sequence **different authenticated payload** is denied. An exact same-epoch retry and a monotonic new release pass.
5. Not-yet-valid and expired releases denied, as well as overly long validity windows.
6. Zero sequence, empty/huge size, missing digest, path-like signer IDs, invalid expiry denied.
7. Same-size hash corruption, truncated payload or appended bytes denied.
8. Failure during streaming returns an explicit IO refusal, not a receipt.

Workflow: `.github/workflows/core-server-bootstrap-b1a-release.yml`:

```sh
cd aquilum-app
cargo test --locked -p aquilum-core --features server-bootstrap-b1a-release-lab --test server_bootstrap_b1a -- --nocapture
cargo check --locked -p aquilum-core --no-default-features
```

## Open security gates before real B1

- **A real release key has not been provisioned.** Define an independently trusted key ceremony with public-key distribution, signer rotation and revocation, emergency security release handling and release metadata freshness. Longer-term TUF-like role separation / threshold signatures require a separate design.
- **Monotonic floor is not durable yet.** The pure verifier only enforces a trusted caller-supplied floor; a client losing, restoring or rolling back that state can still accept an old package. Before shipping, journal the approved sequence/version/fingerprint in OS-protected durable state with atomic write/fsync and recovery proofs.
- **Clock trust is external.** Caller supplies `now_unix`; a manipulated or rolled-back system clock can defeat freshness checks. Test secure timestamps / trusted update metadata before deployment.
- **No path/inode custody.** A hostile concurrent writer could replace a file between successful streaming verification and later installation. B1b must use pinned OS-owned file descriptors, symlink refusal, immutable transaction staging, write-fenced temporary directory, explicit plan-approval CAS and post-install readback verification. A receipt by itself is not permission to execute bytes.
- **No protected installer command or systemd changes.** B1b must separately implement minimal-capability service, a dedicated unprivileged OS account, exact pinned artifact install, run/stop/restart on a disposable VM, idempotent InstallId, power-loss/SSH-drop recovery, single service and rollback that cannot delete user data.
- **SQLite/metadata downgrade hazard:** rollback of the Node binary does not imply rollback of an incompatible persistent database. Require tested snapshot/restore boundaries, migration policy and retention before offering one-click rollback.
- **No network download, automatic upgrades, user vault imports, NodeId/DeviceId pairing, Docker/OCI/ARM64 or RouterOS** is enabled by this code.

Acceptance for B1a is exact-head isolated tests plus standard Application checks. Acceptance for real B1 is a separate signed package and **a verified installation on a disposable Debian/Ubuntu VM**. B0c VM proving SSH cannot replace actual B1 installation tests.

Upstream design references: [The Update Framework](https://theupdateframework.io/docs/metadata/), [OpenSSH host-key checking](https://man.openbsd.org/ssh_config), [systemd.exec sandboxing](https://www.freedesktop.org/software/systemd/man/systemd.exec.html).
