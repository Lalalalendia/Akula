# RN-4E1b-02 — read-only replica-scoped VaultId evidence (corrected)

**Source:** current Akula `main@fc5e7cd2f11df9adefa72b0c736b8dddddbc8c77`. The earlier rejected experimental design forbade the same remote `VaultId` on two distinct roots, but that contradicts the existing Core test `profile_binding::two_independent_local_registrations_bind_same_remote_vault_without_id_alias`.

A `VaultId` identifies the **shared remote vault**, not one local root. Its replicas on separate devices and even two registered local workspaces may legitimately share the same VaultId. The threat is *cross-applying a device's grant from one replica/root/custody scope to another*, not the existence of a second authorized replica.

## This intentionally narrow Core research slice

The already existing `SharedHostCustodyRegistry` stores exactly the host physical root, shared VaultId, S3 profile, external lock identity and custody scope. Add a **read-only, not write-authorizing** `inspect_exact_replica_scope` which verifies an independently selected root physical ID, VaultId, profile ID and lock identity against one durable registration. On any mismatch, deny before handing out even read-only evidence.

Tests prove: (1) two independently registered physical local roots may share the same VaultId, (2) each root, profile and lock resolves to itself, (3) substituting the other replica's root ID, profile or lock is denied, (4) cold reopen preserves two distinct local scopes. Run the full existing host custody / two-process test module on Linux and Windows with `--locked`.

This does NOT prove the caller provided authenticated values, nor installed-host OS privileges or hostile same-user resistance. A read-only host record is not a write permit. The next real task is a typed **host-installed Core -> registered local replica -> P2P DeviceId/EndpointId/epoch** join, with a checked enrolled-root P2P admission distinct from ordinary `with_unenrolled_epoch`. Never infer a shared VaultId from the local workspace UUID; never accept an untrusted remote filename/root.

**No File Gate changes, grants, remote connections, product activation or sync receipts; keep Draft/HOLD and require new exact-head CI.**
