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

### Root-owned, nonroot-readable OS policy — isolated acceptance candidate

The new lab-only `open_existing_root_provisioned` code path reads
**only** a fixed `/etc/akula-node/wow-mail` directory. It cannot
accept a path from an untrusted caller, network frame, GUI control or CLI.
The existing Node's `/etc/akula-node` parent must remain root-owned
and not writable by the service.

- `/etc/akula-node/wow-mail`: `root:akula-node`, mode `0750`.
- `owner-mail-policy.bin`: `root:akula-node`, mode `0640`.
- `owner-mail-epoch-floor.bin`: `root:akula-node`, mode `0640`.
- Both files contain **public owner-signed policy data** and a minimum
  epoch/digest witness, **not any private signing/encryption key**.
- Every ancestor is opened with file-descriptor-relative
  `O_DIRECTORY|O_NOFOLLOW`, must be root-owned and not group/world
  writable. Both files use `openat(O_NOFOLLOW)`, require uid 0, the
  running service's exact effective GID, single hardlink, fixed mode,
  bounded size, signature, exact digest/epoch, Node root policy and
  independent SQLite transaction guard. The nonroot service does not
  have write permission to these files or their directory.
- The new path is still behind `node-wow-mailbox-lab` **and** the
  release-profile `compile_error!`. The legacy same-UID test-only
  `open_existing` is unchanged.

A disposable GitHub-hosted Linux CI test provisions these **public**
files under `/etc` using `sudo install`, verifies nonroot read and
write denial, performs signed local LIST/FETCH, rejects a group-writable
directory and a symlink attack, then deletes only files and directories
it created. The test refuses a pre-existing Akula Node installation.
It does **not** deploy a service or touch a user-owned VPS.

**Still not production anti-rollback:** uid 0 can roll back both
manifest and floor; the same-epoch digest can be reset by a privileged
root actor. This validates an unprivileged-service boundary, not TPM
attestation or remote owner approval. Production root-helper CAS,
durable ordered rotation, service restart rules, root revocation vs
SQLite COMMIT synchronization, and installer provenance remain HOLD.

### Detached first-mail publication consent (inert CAS proposal)

The owner-signed mailbox **manifest** is NOT an instruction to install
anything. The separately domain-separated Ed25519 action approval in
`approval.rs` authorizes a **specific proposed operation** for a
specific node and mail manifest, but is still insufficient by itself
to touch root-owned files.

The canonical signed approval binds `NodePublic`, Iroh EndpointId,
`BLAKE3(signed-manifest)`, first-only mail policy epoch 1, expected
independent CAS witness revision, nonzero approval nonce, nonzero
privileged-helper challenge, and a bounded issued/expiry window
(maximum 15 minutes). Bad or missing signatures, wrong endpoint or
policy digest, expired tokens, zero nonce, or stale expected witness
revision fail closed. Signed policy tokens are not interchangeable with
signed installation approvals; the domains differ.

The opt-in `first_mail_publish_ci` now insists on this second signature
before it stages *any* pending root file. The CI lab tests forge the
approval with an unrelated owner and confirm that **no root file is
created**; the usual crash-point and nonroot read scenarios continue
with the separately signed correct approval.

**Explicit non-shipping limitation:** expected witness revision 0 is
only a CI first-install/genesis assumption, not externally witnessed
monotonic state. `InertMailCasProposal` returns
`requires_independent_cas_commit=true`, but this disposable CI
first-install writer still has **no production B1d0 CAS witness
consumption** or verified operator UI confirmation. It must not be
repackaged into the installed root helper until that additional
authority is checked at the same commit boundary.

### Root-owned two-file publish fence (receipt-last, recovery HOLD)

As a prerequisite to a trusted root-helper transaction, the nonroot reader
now requires a **third immutable root-owned file**:

- `owner-mail-commit.bin` (root:service-group, `0640`, single link).
  Exact fixed record: `WOWMLC01 || u64 epoch || BLAKE3(signed_policy) ||
  BLAKE3(epoch_floor) || BLAKE3(previous fields)`. It is a
  **completion receipt, not an authorization grant or signature**.
- A missing, corrupt, replaced or symlinked receipt refuses the mail role.
  Its inode is pinned after first opening and rechecked on each request.
- Any remaining `owner-mail-policy.pending`,
  `owner-mail-epoch-floor.pending` or `owner-mail-commit.pending`
  forces HOLD, including before first activation. No automatic repair.
- This ensures that merely publishing the manifest and its epoch floor
  after a partial crash is insufficient to admit the mailbox. A future
  root helper must create and durably read back all the fixed files,
  publish the receipt LAST, and fsync directory metadata before return.
- The standalone same-UID test fixtures do **not** obtain production
  authority by this addition; the three-file check applies only to
  the fixed root-owned OS path.

The current OS acceptance test deliberately installs the first two
root-owned files WITHOUT a receipt and verifies HOLD. It then installs
the receipt and proves authorized LIST/FETCH; later it tests pending
files, missing receipts, byte-identical replacement (old instance
HOLD), group-write and symlink refusal.

**Not finished:** writing and publishing all three files under a
trusted root-exclusive CAS transaction, binding the external B1d0
owner approval/witness nonce to a particular mail manifest, privileged
crash recovery, host reboot and root-vs-SQLite revocation ordering.
The complete B1d0 `/opt` file installer/witness is a related
mechanism, not a reusable authorization to deploy mailbox policies.
No user VPS or installed service is changed.

### Transaction-bound SQLite mail-role fence

The default-off Node bridge additionally binds `MailRoleGate` from the
already verified and independent operator signature: exact
`MailboxPolicy`, signed manifest BLAKE3, and mail policy epoch.
This requires an already provisioned matching
`custodian_mail_role_guard` row in the **same WOW SQLite database**
(no silent creation on open).

WOW's `load_policy(..., role_gate)` now checks this row and the exact
stored mailbox quota/recipient/issuer values while holding the SAME
`BEGIN IMMEDIATE` transaction as reservation, W3+W12 COMMIT,
recipient LIST/FETCH and encrypted W4 register/deposit/fetch. A
separate operator connection that sets `revoked=1` serializes
against those writes/reads. Binding is single-use per Journal and
cannot automatically adopt a new epoch. Explicit operator-only
provisioning enforces strictly increasing epochs after revocation.

The historical standalone WOW lab defaults to `role_gate=None` for
compatibility. The Akula Node lab **always** binds a verified gate
before serving any request; an absent/stale/mismatched SQLite witness
is HOLD. The local operator-provisioning method must never be exposed
over the network. This feature is pinned to
[`66ec8ec3`](https://github.com/calex794/wow/commit/66ec8ec37a6f6f622971ee87733f25ef08c3a463)
in WOW #95 and has independent exact-head CI gates.

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
production custodian signing-key enrollment. The original DB/policy comparison remains a preflight, but a new SQLite
role gate additionally checks the pinned digest, policy epoch and revocation
**inside the same local WOW transaction**. This still does NOT make
external root-owned policy-file changes atomic with a SQLite COMMIT:
the operator-signed root manifest and monotonic witness must be protected
by a production root authority with a defined revocation/commit boundary.
The lab must not be exposed over public QUIC or activated through the
installed signed Node until those are independently implemented.

Also required: independent operator approval and release signing,
versioned root-owned mail-role policy, revocation/currentness at
transaction boundaries, quota/TTL enforcement, installed systemd
lifecycle, safe Windows setup, two independent physical devices and
owner VPS restart/power-loss E2E with recipient-signed W4. CI runner
alone does not satisfy these acceptance gates.
