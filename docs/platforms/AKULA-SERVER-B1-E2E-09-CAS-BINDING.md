# B1-E2E-09 — signed owner intent to existing B1d0 CAS types (HOLD)

## Problem after B1-E2E-08b

The root helper's authenticated AF_UNIX preview and canonical B0v3 + owner-frame intent were implemented, and B1d0 already had a protected, durable file/CAS writer. Their typed data boundary was still not connected: an IPC frame is not itself an OS-owned StageReceipt, ExternalApprovalScope, ProposedFloor or WitnessCheckpoint. Constructing those values directly from a peer would be a privilege escalation.

## Implemented, non-shipping read-only bridge

- Only after verifying the running root helper, Linux peer UID, independently registered owner signature, canonical B0 v3 plan, signed Node package FD and protected witness genesis, the internal intent reader now derives a strictly matched CheckedB1d0Inputs value.
- Its exact B1d0 data structures are built from the verified signature claim, independently pinned B1a FD receipt and protected witness, **not** from arbitrary JSON or a new install protocol.
- It checks InstallId, approved plan digest, pinned SSH host digest, bootstrap-helper digest, Node package SHA and byte length, B1a manifest fingerprint, proposed release floor, generation, signed approval nonce, previous witness floor and revision.
- First-install input must have genuine externally enrolled witness genesis, an incremented release floor and a nonzero proposed tip; idempotent replay is false. No claim can select a different witness/host or force a second installation.
- A mismatched value refuses before any CAS call. Existing owner-frame signature verification and root-owned release FD opening are unchanged; this adapter does not accept unsigned claims as authority by itself.
- The existing B1d0 CAS writer remains responsible for independently reopening root trust/FD/witness at COMMIT time and consuming the nonce durably under its destination lock.

## Test and acceptance

The dedicated B1-E2E-09 CI tests positive typed correspondence and negative mutations to owner, plan digest, host, Node SHA/size, manifest, nonce, floor and witness. It also reruns strict bounded IPC framing tests and checks that the actual compiled helper continues to refuse the --commit CLI argument with exit 2. Tests use synthetic data; no production signer exists inside CI.

**This is not an installer or CAS dispatch.** No root IPC opcode, no service socket/unit, no write to /opt, no process execution, no VPS interaction and no Windows Install Node authority are introduced by B1-E2E-09.

## Next true milestone

The separately signed production helper and owner/release key enrollment must be independently approved outside CI. Only then should an audited root-only dispatch couple these verified typed inputs to the already proven B1d0 CAS, generate its own challenge, and prove durable one-time receipt, systemd isolation and reboot against a real VPS. The Windows UI stays HOLD until this verification succeeds.
