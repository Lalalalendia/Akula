# NODE-V1 — local trust and durable one-use authorization boundary

Status: **not a production Node release**. The code is on the single
`feature/akula-node-v1-integration` branch, not enabled by the ordinary
Akula desktop build. No public listener or server-install opcode is added.

## Separation of authorities

| Material | Holder | Meaning |
| --- | --- | --- |
| Signed Windows SSH KEX host pin | Windows Akula | Identity of the SSH installation host; not a Node data grant |
| Release manifest / root CAS receipt | root-only installer | Provenance of installed ELF; not service activation authority |
| Node Ed25519 private key | private non-root Node StateDirectory | Stable Node identity and signed **local-only** health challenge |
| Owner public key, exact Vault, roster/grant digests, epoch | root-owned /etc/akula-node/owner-policy.json | Independently provisioned read authorization pins |
| SQLite issued challenge ledger | non-root private StateDirectory | Atomic issued-first, single-use replay rejection |
| Current Iroh remote_id | actual authenticated QUIC connection | Cryptographic transport identity, not permission by itself |

The root policy is read using a pinned Linux directory handle and NOFOLLOW
child open. The owner signature on the exact read grant and the exact signed
roster are checked against independently root-owned pins. A changed policy
causes a **HOLD** until the process reloads and its session reauthorizes.
A peer cannot nominate its own owner key, epoch or protected Node identity.

The service identity file and SQLite state are checked for exact UID, 0600
files and 0700 StateDirectory, with no symlink/hardlink file admission.
Root policy files must be root-owned and non-writable by group/others.
The service refuses to run as UID 0. A locally trusted same-UID adversary
is not defeated; the installer must isolate this UID.

## Local binary

With explicitly selected Core feature `node-v1`, the separate Linux
`akula-node` binary recognizes exactly two modes:

- `--public-identity`: print only the non-secret Node Ed25519 public key.
  Used by a future privileged installer for independent policy enrollment.
- `--serve-local-health`: require fixed root-owned owner policy, open
  `/run/akula-node/health.sock` under a 0700 RuntimeDirectory and accept
  only a bounded one-byte-version plus 32-byte random challenge. The reply
  is a domain-separated Ed25519 proof from the stable key. A rejected
  request produces HOLD; there is no remote upload, Vault read, TCP, or
  Iroh listener.

**LOCAL_ONLY is not Healthy/Ready.** This unit proves only installed local
process identity when the caller independently verifies the signature and
fresh nonce. The actual signed root helper, service-unit installation,
systemd lifecycle, health-to-Windows bridge and cold-reboot inspection remain
pending and MUST NOT be inferred from this code.

## Durable challenge contract

- An issuer must first call `RootTrust::admit_read` with an actual signed
  roster, owner-signed exact grant and authenticated QUIC peer key.
- Only the resulting sealed Rust admission can create a fresh 256-bit
  challenge in SQLite. Issued rows bind peer, grant digest and root policy.
- Maximum 64 outstanding challenges; each expires after 60 seconds.
- Atomic SQLite DELETE with `BEGIN IMMEDIATE`, `synchronous=FULL` and an
  independently re-read root policy consume the challenge exactly once.
- Missing, duplicate, wrong peer/grant, expired, policy-rotated and
  clock-rollback inputs are refused. Corrupt SQLite never falls back to
  an in-memory permissive fence.
- `interop/akula-staya` feature `node-v1-runtime` implements the existing
  Iroh `OneUseChallengeFence` trait using this durable implementation.
  The original default-off laboratory remains unchanged.

This is a local SQLite durability boundary, **not** a signed end-to-end
receipt after power loss and not an independent revocation oracle if a
hostile root deliberately rolls back both the policy and disk.

## Local authorised replica Put/Get (next integration slice)

The new `OwnerPutGrant` is an exact, owner-signed WRITE claim. It cannot be
substituted with an `OwnerReadGrant`: the signed domains and independent
root-owned digests are distinct. Owner policy schema is now **version 2**;
older schema is intentionally HOLD rather than silently treated as sufficient
write authority. Only one pinned Vault, exact file/revision and a currently
recorded Iroh device can be admitted. Initial bound: a single top-level UTF-8
Markdown file no larger than **16 MiB**. Arbitrary binaries and folder trees
are not enabled.

`NodeHost::issue_put_challenge` first verifies the exact owner-signed grant,
root pins and owner-signed roster. `commit_put` checks the device's Ed25519
proof for the issued nonce. A separate persistent SQLite `put_issued` table
uses FULL sync and one-use transactional consume; `read_issued` (table
`issued`) cannot be reused for Put. Both share a 64 outstanding challenge
quota and rollback clock guard.

After successful proof the new `store::commit` writes a BLAKE3 blob and
immutable `RevisionRecord` via the **existing** `SqliteDagStore`, closes
the DB, reopens independently and verifies the exact document/revision/path,
hash, bytes and length. An orphan blob committed without its DAG revision
cannot be read as an authorised revision. Only after this check and a fresh
root policy read does Node Ed25519-sign a domain-separated `NodePutReceipt`.
The client must independently pin its Node public key and check the receipt.
A receipt confirms the Node's own locally re-opened commit; it is NOT an
independent external durability audit against hostile root or disk rollback.

`issue_read_challenge`/`fulfill_read` independently validate signed
read scope and device proof, consume a READ-only challenge, and read only
matching immutable bytes from the same Node state. These are **local Core
APIs**, not live QUIC endpoints; the network layer is still not connected
to them. No server listener, remote write, installation or File Gate authority
is activated.

## Evidence and release gaps

Code contains negative tests for persist/restart/replay, wrong peer/grant,
unsafe permissions, owner-policy replacement and symlink, owner signature,
current roster and revoke. **None is a verified CI PASS until run on the
exact integration SHA.**

Still blocked: trusted installer enrollment and signed release, actual
systemd RuntimeDirectory/StateDirectory integration, authenticated Iroh
server listener, **remote** Put/Get, independent second-client File Gate
create-new, revoke after reboot and full Windows A → VPS → Windows B E2E.
This source is not compiler/CI-accepted on the integration HEAD. Do not
enable the install button or merge this release slice until all pass.
