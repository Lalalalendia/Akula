# RN-4E1b-03c1b1a — external monotonic witness CONTRACT, not an implementation

**Scope:** test-only Windows Rust code stacked on [PR #464](https://github.com/Lalalalendia/Akula/pull/464). The main Akula Core, native installation, P2P File Gate and standalone runtime Broker executable have no new permissions or code paths. All policy-checking code is behind `#[cfg(all(test, windows))]`.

## Why a second trust domain is required

RN-4E1b-03c1b0 proves that a journal can be appended under a Windows kernel exclusive handle and that torn tails cause HOLD. It also explicitly proves that CRC32 cannot detect a valid historical snapshot replacing the entire file while closed. A replay attack can undo previous revocations without violating any in-file sequence, CRC or installer-binding header.

This follow-on adds a **model-only independent witness**: an externally trusted installer binding, a strictly witnessed latest global sequence and the exact complete map of monotonic revocation floors per local ReplicaId. The witness is supplied from unit-test fixture memory only; **no real trusted storage, TPM, signed installer, privileged Broker owner or external counter exists**. The contract refuses if the witness is unavailable or belongs to another installation.

Under a single OS-exclusive already-existing journal handle, the verifier replays ALL journal records then compares its global sequence and entire replica floor map to the external witness. Older journal -> HOLD (rollback); newer journal -> HOLD (journal fsync before external seal); same sequence but different replica floor -> HOLD (divergent policy); corrupted/missing journal -> HOLD; exact match -> `ConsistentDenialSnapshotOnly`, never `Allow`, writer, capability, receipt, `APPLIED` or ACK.

The contract intentionally treats both orders of a potential future two-phase update as **review-needed** until a truly separate trusted authority can reconcile them. It does not claim that two durable objects are atomically committed together or that a privileged adversary cannot forge the test fixture. A future runtime must pin real journal identity and OS ancestry, attest a protected external witness and prevent rollback of **both** objects, or reject all permissions. No consumer may bootstrap witness from a user-supplied journal.

## Verification

Seven Windows-only unit tests: absent witness even for valid journal, matching denial-only state, replay of valid old snapshot against a trusted newer fixture, both sides of the journal/witness commit gap, same sequence with another replica floor, installer binding / missing / torn log, and unknown future/past epoch. Linux builds verify test-code isolation. Full existing journal and SCM research tests still run on Windows. All changes limited to five files.

**Next RN-4E1b-03c1b1:** pick and physically validate a real external witness mechanism (e.g. appropriate OS/TPM monotonic service with crash and migration semantics), plus a signed installer, broker-owned canonical primary file and ALL sidecars. Nothing becomes a P2P permission until actual QUIC channel binding and File Gate guarded no-replace/recovery are proven.

CI PASS, if obtained, proves only the *comparison algorithm*, not that a trusted witness exists. Product sync remains HOLD.