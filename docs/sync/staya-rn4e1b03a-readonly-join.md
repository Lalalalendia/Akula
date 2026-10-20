# RN-4E1b-03a — read-only replica + peer grant correlation, security falsifier

**Experiment only; not product admission.** Based on `main@2174fa228406487a3fe92f9273bdfcdd36517803`, retaining the accepted standalone ledger logic from [#421](https://github.com/Lalalalendia/Akula/pull/421) (`cfa0e983`) and host replica-read logic from [#427](https://github.com/Lalalalendia/Akula/pull/427) (`debcf08a`). Both are imported into one isolated **default-OFF** feature-gated test environment; the installed Akula executable, File Gate, S3 Sync and `Cargo.lock` remain untouched. No historic stacked branch is wholesale merged.

## Safety question

A shared `VaultId` identifies the same remote vault across **multiple legitimate local replicas**. A grant is scoped to a specific selected local Core/root, `DeviceId`, authenticated `EndpointId`, signed object metadata, issuer and epoch. Merely checking that a host record and an unrelated grant record both name the same `VaultId` is insufficient: two separate appdata SQLite ledgers can each contain a valid grant for the same remote vault.

The new test-only `RegisteredPullGrantLedger::recheck_selected_replica_for_lab` checks **Core object pointer, registered workspace ID, registered physical root identity/path and bound VaultId** against the selected local Core/root. The sibling `peer_replica_join_lab_tests` then correlates that observation with the existing read-only host `inspect_exact_replica_scope` (physical root + shared vault + local profile + synthetic external-lock identity) and the full persisted grant snapshot. No new public permit, QUIC listener, native feature or user-vault writer is added.

## Test matrix — 4 explicit falsifiers

1. Two separate, registered roots share the **same VaultId, DeviceId and EndpointId**; both observations are legal, but their separate SQLite grant ledgers **cannot be used against the other Core/root**. Their host profile/lock scopes cannot be mixed.
2. Wrong DeviceId, changed nonce, mismatching remote endpoint, expiry and revocation all fail before any file mutation.
3. An independently reopened Core, host ledger and P2P grant ledger retain exactly the same per-local-replica separation.
4. **Deliberately positive security counterexample:** an unprivileged user can provision a SECOND syntactically valid host SQLite file and insert the same local registration; read-only observations still match. Therefore JSON+BLAKE3 and independently opened SQLite handles **do not prove installed OS Broker/ACL ownership**, and cannot constitute a production write-admission. This is a deliberately highlighted *known blocker*, not a passing product security gate.

## Evidence required and next barrier

Run at exact head on Linux and Windows: 4 new unit tests, existing host-registry (including independent process) test suite and RN-4C 9 / RN-4D 3 / RN-4E0 4, `Cargo.lock` unchanged and rustfmt. The test-only observation returns only `()`; even a positive result is not atomically fenced across two SQLite DBs and does not grant an authorization capability or a receipt.

**RN-4E1b-03b P0:** establish a genuinely installed-host/Broker-owned identity with OS owner/DACL and immutable local P2P replica enrollment (separate from S3 profile) and real authenticated `Connection::remote_id()`; then design a typed, bounded Core-owned admission lifetime with a defined lock graph across host and grant databases, File Gate and HistoryService. No ordinary `with_unenrolled_epoch` for an enrolled vault. Do not hold SQLite write locks over network/Bao transfers. For user file publication require separate STY-G2-06 durable byte recovery and a pre-apply intent + no-replace readback + crash ambiguity policy. `APPLIED` and ACK remain forbidden.

**Keep Draft/HOLD**; this is a pure research regression test, never a new product Sync or S3 permission.
