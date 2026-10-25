# NODE-V1-WOW-MAIL-POLICY-01 — independently signed mail-role boundary

Status: **LAB ONLY, default-off, non-shipping; deployment HOLD**.

Sources: [WOW CUSTODIAN-02](https://app.notion.com/p/40132a84beec8139a590c29e7a4eaa8c),
[Akula NODE-ARCH-01](https://app.notion.com/p/40132a84beec8118b1a2fffc90fb808c),
[WOW PR #95](https://github.com/calex794/wow/pull/95).

## Security decision

Node-V1's existing root-owned policy version 3 authorizes FILE replica
Put/Get. Its grant digests and roster are **not** mail authorization.
The owner/operator key independently pinned by the existing Node-V1
RootTrust can verify a **separate** owner-signed, domain-separated WOW
mail-role manifest.

No mail role is enabled merely because a Node Iroh endpoint answers
connections or a custodian has a valid Ed25519 key.

## Non-shipping lab contract

`interop/akula-staya/src/node_wow_mailbox_lab/policy.rs`
uses an explicit binary Postcard-encoded manifest and detached
Ed25519 signature over:

`Akula/NodeV1/WOW/OwnerMailRole/lab-v1\0 || canonical_claims`.

The signed claims bind:
* version/monotonic policy epoch;
* exact Node signing key, Node transport EndpointId and the
  independently pinned operator public key;
* mail issuer, sender, recipient and custodian (four distinct IDs,
  nonzero epochs, exact 32-byte verification keys);
* one opaque mailbox tag and scope, STORE/LIST/FETCH/W4-return
  role permissions (no unknown rights), bounded quota and maximum
  W3 size, start and expiration time.

For these tests, the Node operator key is read through a
`#[cfg(all(feature="node-v1-transport-tests", debug_assertions))]`
method on the **already verified** Node RootTrust. The helper
returns only the operator public key. It never returns file-scoped
Put/Get grants or creates mail permissions.

The signed lab fixture is kept in a SAME-UID, 0700 directory as
`owner-mail-policy.bin` (0600, single-link regular file). It is opened
NOFOLLOW with inode verification, length bounded to 4096 bytes,
re-encoded to reject alternate/trailing encodings, signed domain checked,
and re-read before **every** local WOW operation. A changed policy file
—even a legitimate new epoch—causes HOLD until explicit review/reopen.
Clock rollback / expiry also cause HOLD.

The WOW TrustRegistry is derived ONLY from this successfully verified
signed policy, not from caller-supplied public keys. The exact
single-mailbox, issuer epoch, recipient, custodian, scope and byte/offer
quota must additionally match the durable SQLite mailbox-policy row
(`custodian_mailbox_policy`). A second mailbox or broader SQLite quota
is refused.

The role **still only accepts loopback TCP** via the WOW signed
protocol; Akula's installed Node Iroh service, ALPN, fixed CLI modes,
root-owned installer, app File Gate and shipping Cargo workspace remain
untouched.

## Checks

Opt-in CI:
`cargo test --manifest-path interop/akula-staya/Cargo.toml
--features node-wow-mailbox-lab --test node_wow_mailbox_lab`.

Negative tests: forgery by another operator, altered signed Node key
or endpoint, unknown capability bits, quota inflation, owner policy
replacement after acceptance, expired policy, unsafe file modes,
symlink, hardlink, original DB changed/extended without a signed grant,
wrong custodian signer, changed Node root policy and public bind attempt.
Ensure the prior offline W3 LIST/FETCH and independent Node state
restart test still passes.

## Explicit gaps / DO NOT SHIP

This lab does **not** load a truly root-owned production mail policy:
test fixtures are SAME-UID and the time is supplied by the caller.
It lacks a root-protected monotonic epoch/rollback floor and
production custodian signing-key enrollment. The DB/policy comparison
is a **preflight**, NOT an atomic authorization inside the same SQLite
commit. It must not be exposed over public QUIC or activated through
the installed signed Node until those are independently implemented.

Also required: independent operator approval and release signing,
versioned root-owned mail-role policy, revocation/currentness at
transaction boundaries, quota/TTL enforcement, installed systemd
lifecycle, safe Windows setup, two independent physical devices and
owner VPS restart/power-loss E2E with recipient-signed W4. CI runner
alone does not satisfy these acceptance gates.
