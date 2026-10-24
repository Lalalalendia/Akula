# B1-E2E-07 — one installed Akula → signed VPS Node → second installed Akula

**Status: implementation branch, NOT ACCEPTED.** Canonical [Windows/VPS acceptance matrix](https://app.notion.com/p/40032a84beec81d0a25dd178456910bb) and [Node V1 service boundary](NODE-V1-SERVICE-GATE.md).

The Node-V1 Core/service integration was merged to main as `cfb2b328` (PR #697). That merge included real QUIC A→Node→B, B1d0 root-signed file/receipt on disposable Linux, systemd restart tests and Windows SSH *read-only* verification. It did **not** install Node on a real VPS or enable the installed Windows **Install Node** button.

## Required single product acceptance

A clean installed Windows Akula obtains independently pinned signed SSH KEX and Linux facts from the user's own supported Debian/Ubuntu x86-64 VPS. It verifies exact release-signed Node bytes with an independent release trust anchor; shows the fixed list of privileged changes, pinned host, release digest, service identity, UDP/43871 implications; receives **separate one-use owner approval**; verifies release/owner signatures again inside a fixed privileged Linux authority; commits the exact ELF and root-owned CAS to /opt via the existing B1d0 code; provisions immutable policy v3 and non-root service identity; starts systemd and proves actual health after reboot. An authorized second installed Windows Akula retrieves the approved object with source A offline, via File Gate create-new without overwriting existing Vault data. Wrong host, signature, owner, revoked permission, stale nonce, changed plan, interrupted commit, reboot, corrupted bytes and repeated requests must HOLD or DENY rather than install twice.

No human has authorized a real VPS target in this task. Do not infer VPS addresses, private credentials, signing keys or deployment consent from Notion or GitHub notes.

## Integrated work package / source-to-user mapping

| Gate | Evidence required |
| --- | --- |
| B1-E2E-00 | One signed Linux release and independently pinned public signer, one stable Windows client/release manifest, immutable package digest |
| B1-E2E-01 | Actual OS Windows signed SSH KEX (already PASS); reverify immediately before any remote operation |
| B1-E2E-02 | Immutable plan, verified package bytes, distinct explicit user click, owner-signed limited approval and immutable digest/nonce |
| B1-E2E-03 | Existing B1d0 verified-FD + one-use detached owner signature + root-owned durable witness CAS + fixed /opt |
| B1-E2E-04 | Provision non-root static service UID, one-shot identity enrollment, root policy v3 then systemd health |
| B1-E2E-05 | Reboot and crash/reconnect into real health or explicit Needs Review without silent regenerated keys |
| B1-E2E-06 | Negative-path and failpoint tests at each root authorization and fsync boundary |
| B1-E2E-07 | Installed Windows A → real owner-authorized VPS → installed Windows B, source offline, no overwrite |

### Concrete implementation recorded on the working branch

The opt-in `aquilum-core/server-install-review` feature introduces `server_release_lab::review`:

- Reuses the **existing** B1a signature verifier and hashes *the actual bounded package stream*. No new signing format or signing key.
- Validates the existing B0 digest-bound plan and that its public version, package digest and signer ID match the **independently verified signed manifest**.
- Binds a 10-minute, randomly generated single-use UI review nonce to exactly one InstallId, immutable plan digest, host SHA-256 and package SHA-256.
- Refuses changed host, user, fingerprint, package, echoed plan, replay, clock rollback, expired review, forged release signature, modified/oversized package or unsupported B0 facts. Rejections consume the local review attempt.
- Returns **InertUserConsent only**. This is not B1b1b `SignedOwnerApproval`, not a CAS commit, not signed package provisioning, not SSH install authority and not systemd activation. An app calling `expected_echo` without a genuine user click cannot obtain owner privileges.

The ordinary installed Windows Akula is intentionally **unchanged** by this module; `server-install-review` is an opt-in Core feature. Do **not** enable the existing Install Node button by adding this feature alone. An untrusted SSH server can never supply the pinned B1a signer or owner approval.

### Remaining implementation (not accepted)

- Production release trust anchor/floor and published signed Node artifact; no test signing key may serve as production identity.
- Actual Windows immutable plan/consent UI tied to verified SSH and signed package.
- Fixed pinned SSH install transport and root-only helper backed by B1d0, separately trusted owner signature and durable CAS.
- Policy v3/identity enrollment/systemd service activation, reboot and recovery transaction.
- Exportable **public-only** Node pairing config and two installed Windows/File Gate recipient acceptance.

Do not create per-module public release previews or treat laboratory markers as user acceptance. Continue on the same integrated feature branch until the full slice is wired, then run the exact-head CI and actual owner-authorized VPS acceptance. Nothing in this document grants access to a server.
