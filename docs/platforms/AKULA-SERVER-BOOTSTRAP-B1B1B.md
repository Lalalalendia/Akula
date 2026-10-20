# AKULA-SERVER-BOOTSTRAP B1b1b — detached operator approval laboratory

**Status: experimental, no deploy/activation authority.** Depends on [B1b1a PR #459](https://github.com/Lalalalendia/Akula/pull/459), B1b0 and B1a. Parent design: [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34).

## Intent

Build a separate, verifiable *signed approval assertion* before any installer has the ability to execute a package or change systemd.

1. The release-signing Ed25519 key (B1a), server SSH host key (B0b), and operator approval Ed25519 key are **different trust roles**. The operator key is independently pinned. It is not learned from the package, SSH server, downloaded metadata, or installed Node.
2. A typed, canonically encoded `SignedApprovalClaim` signs: `InstallId`, B0 plan digest, pinned SSH host key digest, B1a signed-manifest fingerprint, SHA-256 and exact byte size of staged package, previous independently trusted release-floor head, proposed new head, expected external witness revision, one-use nonce, and short validity window (at most 15 minutes).
3. Signature verification is followed by exact comparison against independently supplied `ExternalApprovalScope` plus existing B1b0 staged receipt and B1b1a floor proposal. The expected witness revision and previous head are provided from outside the untrusted package/approval claim.
4. Only an **inert `InertWitnessCasRequest`** is returned. It binds the expected and desired generations but cannot persist a witness, install binaries, execute shell, activate systemd or grant File Gate rights. A separate, independently protected component must enforce a real durable CAS and replay protection before activation.
5. Nine integration tests use only synthetic deterministic TEST keys and test artifact bytes, testing validity, signature tampering, swapped host/plan/InstallId, wrong approver, stale witness revision, expired/future approvals, substituted package/ledger and malformed records. They invoke B1a+B1b0+B1b1a to get a real inert staged receipt and proposal.

## Explicit nonclaims and risk boundaries

- A valid approval signature **is not proof of real human consent**: the Windows/UI review flow, trusted display of exact plan operations, confirmation prompt, explicit per-operation permission and key custody are absent.
- Operator signing keys are test-only; no real credential issuance, hardware-backed key material, revocation, recovery or key rotation has been done.
- The authority/witness remains only a caller-supplied interface; **there is no durable external witness**. A malicious caller could supply a rolled-back floor/revision or use a non-trusted clock. Signature verification cannot compensate for these compromised inputs. The nonce is cryptographically bound, but consumption/uniqueness is not persisted, so replay resistance depends on the future CAS witness.
- No remote SSH installation, production VPS, privileged helper, systemd, NodeId creation, user vault transfer, automated rollback, Android, Docker or installed Windows Akula modification.
- No hostile-root or power-cut guarantee. Real B1b1 acceptance requires a separate protected witness/provisioning process, actual operator-approved typed plan and crash/reboot tests. B1b2 must run real native Node only in an isolated disposable VM.

## Acceptance

Dedicated exact-HEAD Linux CI plus normal Application CI, Rust source/review, rejected wrong-signer/host/plan/state cases. This PR must **not merge** before its B1b1a #459 dependency is accepted and its own files are reconciled with `main`; tests of the stacked branch are not a substitute for final merged-HEAD CI.
