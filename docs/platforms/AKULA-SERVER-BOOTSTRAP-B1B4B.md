# AKULA-SERVER-BOOTSTRAP — B1b4b0 root-owned CAS witness signer (CI-only)

Dependency: B1b4a PR #496. This B1b4b0 is a **restricted OS-owned laboratory**, not a remotely independent witness or an installer.

## What changes
- Rust feature `server-bootstrap-b1b4b-os-witness-lab` composes the already-tested B1b1c append-only CAS journal with the root-pinned B1a ELF, real B1b1b owner signature, and B1b4a canonical external witness claim.
- The witness private Ed25519 seed is read only from an independently provisioned root-owned 0700 trust directory: 0600 regular single-link seed, public key and explicit enrollment marker, opened using `O_NOFOLLOW` and verified metadata. The key never comes from the staged package, SSH path or IPC client. Separate root-owned 0700 ledger directory enforces durable CAS persistence and readback. The seed is zeroized after loading.
- A **new** accepted CAS revision must be persisted, fsynced and read back before the protected actor signs it. Exact retransmission cannot obtain a second attestation. Stale revision, expired B1a release, forged owner signature, wrong package receipt, missing explicit enrollment or tampered ledger refuse.
- The canonical signature binds before/after CAS checkpoint, host, signed manifest and package digest, InstallId, approved plan, owner signature digest, owner nonce, challenge and a 30-second witness validity window. Result is a `SignedWitnessClaim` only, never a write/install permission. Each future install must re-check separately under the actual commit transaction lock.
- Disposable CI runs actual UID0 tests using synthetic non-production seed/keys. Nonroot writer isolation relies on Linux mode/ownership; this first slice **does not yet** include a separately running systemd witness service or independent second machine.

## Explicit limitations
A root-owned same-host directory and key are not an independent **monotonic** authority against hostile UID0, full snapshot rollback or stolen host credentials. The local hash chain has no outside witness. The signing actor does not yet isolate from the UID0 helper in its own process, no protected challenge issuance/revocation backend exists, and a local caller can provide expected-before; the actor cross-checks it with the OS-owned ledger but full-disk rollback remains possible. Synthetic Linux tests do not constitute live Windows owner consent, a production installer, or persistent Akula Node.

Next gates: separate OS-owned witness process and authenticated IPC; negative restarts, revocation and one-time challenge issuance; independent remote/TPM-backed monotonic head if hostile root/snapshot rollback is in scope. Then commit-time fresh CAS and fixed signed-Node install with crash recovery. No systemd mutation or Node execution in B1b4b0.
