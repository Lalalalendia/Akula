# Akula / RN-4E1b-03c0: broker-side policy kernel before trusted OS enrollment

**Status:** pure non-authorizing, unit-test-only model. Follow-up to [Draft #450](https://github.com/Lalalalendia/Akula/pull/450) which has a real cancellable SCM-owned Windows pipe, without any grant operations. The installed Akula is not connected to this research.

## Why this specific step exists

Having a genuine LocalSystem service and a process-bound named pipe proves **where a local call arrived**, but not which remote QUIC peer established an authenticated session. Simply transporting an EndpointId or DeviceId through an IPC frame is **untrusted data**, even if all identifiers match an enrolled entry.

This step adds a standalone std-only Rust model in `aquilum-app/host-broker-lab/src/broker_policy_kernel.rs`, registered under `#[cfg(test)]` only. It implements a typed **local-replica-scoped** enrollment/revocation state machine with no positive authorization result, no app integration, no filesystem and no live networking.

## Explicit typed scope and negative rules

- `VaultId` may identify the **same remote vault** on two separate legal local `ReplicaId` values.
- Each local replica binds a pinned physical root (volume + native object identity), `VaultId`, `DeviceId`, approved remote public endpoint key, issuer public key, exact BLAKE3 object digest/length and expiration.
- A second ReplicaId cannot reuse the same physical root. A registered ReplicaId cannot silently change its VaultId or physical root. Claimed root, peer and object substitutions fail.
- Monotonic policy epochs: first provision 1; later replace/revoke require exact compare-and-swap and checked increment, never wrap. Revocation is durable only **within the memory process**, not a persistent security claim.
- Expiry uses the supplied trusted clock; a peer/client cannot establish trusted time via a manifest.
- Critically, the only successful-looking diagnostic is `ConsistentOnlyNoAuthenticatedConnection`: it is **not Allow/Permit**, cannot be converted into a file writer, and means precisely that syntactic fields matched while no trusted QUIC channel binding was presented.
- The internal `InstalledAuthority` witness has **no production constructor**; tests can build it only under `#[cfg(test)]`. On a cold restart, the entire ledger is empty and all former grants fail shut.

Seven test cases cover cold restart, two legal local replicas, cross-replica substitution, root reuse/identity rebinding, revoke/CAS/reissue, expiry/overflow and invalid scope.

## What this CANNOT establish and what comes next

The tests do **not** provision an installed service-owned policy store, WAL/SHM, evidence-bearing physical root handle, durable revoke version, signed installer identity, or authentic `Connection::remote_id()`. An actual Broker-owned durable ledger MUST fail closed on corruption, backup rollback, staleness or missing OS custody, with a monotonic epoch that survives process restart. Until an actual authenticated QUIC connection is owned by a trusted component (or explicitly channel-bound to it), the client-supplied endpoint key never authorizes a request. There are zero File Gate writes, user-vault mutations, receipts, APPLIED or ACK.

Next **RN-4E1b-03c1**: establish installer/Broker-owned protected policy storage and independent real Windows DACL+service-SID positive fixture, then transactional durable enroll/revoke with cold restart and two-process adversarial CAS checks; use the existing Akula RN4C/RN4D/host-registry experiments for parity, not a second competing user-owned authority. Next **RN-4E1b-03c2**: QUIC connection ownership and channel-binding proof, not a client-supplied remote ID. Only then may Core-owned File Gate commit be evaluated, subject to Staya G2-06.

**Release policy:** even a green CI accepts only a denial-state algorithm and scope regression tests. Do not merge into native, enable user sync, or interpret diagnostic matches as grants.
