# AK-P2P-B2-WAN-ACCEPTANCE — two physical Windows PCs

Status: **HOLD — no physical two-WAN evidence yet**. This is a QA procedure,
not a release authorization or proof that the signed installed client works.

## Scope and prerequisites

Use one explicitly selected **synthetic** nonempty file no larger than 8 MiB.
Use two independently installed QA-only Windows builds at the same **exact commit**,
on two *physical* PCs with independently verifiable **different WAN networks**.
Avoid real documents, secrets and sensitive filenames. Document the QA build SHA
and the Windows version on both devices. Neither B3 CI nor a two-process
single-runner test counts as this test.

Each endpoint must have a stable OS-backed Iroh identity after cold start.
The human operators must pin each other's full public EndpointId through the
Akula consent UI, and verify the non-revoked contact before the test.
Keep full EndpointIds, public IPs, UDP endpoints, credentials and private paths
out of exported evidence. Public truncated IDs (up to 16 hex characters) are
sufficient in the evidence packet. **No auto-trust or background listener.**

## Execute B2

1. Launch the **QA debug** Native client on both computers. To enable the
   optional structured proof line in an operator-visible test console, set
   `AKULA_P2P_B2_QA_EVIDENCE=1` in each process environment **before** launch.
   It is OFF by default because the full content BLAKE3 can reveal information.
   If the GUI hides standard output, B2 remains HOLD until the test operator
   captures the authorized QA diagnostics; do not invent a log.
2. On PC A, select exactly one test file and the OS-pinned PC B contact, then
   explicitly choose **Send via Internet**. The sender must report
   `INTERNET_DISCOVERY_READY=true` only after n0 is online.
3. On PC B, independently select the pinned PC A contact and choose
   **Receive via Internet**. No user-entered LAN IP or port. Keep both
   processes alive through sender ACK and receiver Inbox verification.
4. Capture `AKULA_B2_QA_EVIDENCE` from A and B. Compare the **full 64-hex
   BLAKE3 and byte count**. A must show `sender_verified_peer_ack`, B must show
   `receiver_local_inbox_verified` with an Inbox receipt UUID. A receive
   timeout is **FAIL**, not delivered.
5. Review each `selected_quic_path_snapshot`: `direct`, `relay` or
   `unknown`. This is only the route selected at the instant of logging,
   not a full path history; a relay connection can migrate to direct.
   If route is `unknown`, keep the route-proof requirement **HOLD**.
   Independently record that the endpoints were on different WANs, without
   publishing public addresses.
6. Cold-stop and cold-start PC B. Confirm the same receipt remains in
   permanent Received Inbox and matches BLAKE3, without silently importing
   into Vault. This is evidence toward B3, **not B3 acceptance**: authenticated
   sender filename/type, real Save to Vault and confirmed delete still need
   their own installed-client tests.

## Optional offline A/B log correlation (not WAN proof)

With the user's explicit permission, capture each endpoint's opt-in
`AKULA_B2_QA_EVIDENCE` console output into **separate local UTF-8 text files**,
and check them without uploading them or sending data to any service:

```bash
python3 interop/akula-staya/qa/b2_evidence_check.py --sender sender.log --receiver receiver.log
```

The stdlib-only checker rejects missing/duplicate records, incorrect sender or
receiver phase, unexpected fields, a missing/noncanonical receiver receipt,
invalid/nonmatching byte counts or full BLAKE3, and false claims of signed or
durable delivery. Status `B2_CORRELATION=MATCHED PHYSICAL_WAN=UNVERIFIED`
(exit 0) means **only** that the two untrusted diagnostic lines correlate.
An `unknown` path returns `HOLD` (exit 2); invalid records return
`REJECTED` (exit 1). The checker prints no log content or digest.
Its isolated CI fixture tests are run with `--self-test`.

The logs are not cryptographically signed, and matching diagnostic lines cannot
prove the source file's actual digest, two physical Windows endpoints, independent
WAN networks, live TLS pin validity, full QUIC path history, or survival of a
cold restart. These require the separate operator observations above. **Never
promote B2 to PASS on an offline MATCHED result alone.**

## Evidence and decision

For each test store a redacted record:
`gate | exact SHA A/B | Windows versions | independent WAN witness |
short endpoint IDs | source byte count | BLAKE3 A/B | Inbox receipt |
selected QUIC route snapshot A/B | cold restart observation |
PASS/FAIL/HOLD | operator and UTC timestamp`.

**B2 PASS** only if the two real PCs exchanged the actual file, the byte
count/BLAKE3 agree, identity pins held, independent WAN is evidenced, and
neither side reported a false success. A route snapshot is one observation,
not proof that *all* bytes used that path. If any required evidence is
missing, status remains **HOLD**. B1, B3, B4 and B5 are independently
accepted and are not inferred from B2.

Do not enable P2P in the signed production installer, publish an installer,
merge Draft PR #905, or change the default P2P-off feature from this test.
