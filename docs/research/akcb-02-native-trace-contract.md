# AKCB-02: native Sync Planner trace bridge (research only)

Status: **draft / not integrated with product**; 2026-10-21. Depends on [AKCB-01](./akula-cross-boundary-contract-v0.md). This is the first deliberately narrow implementation-to-model bridge. It reuses `aquilum_core::sync::plan_snapshot` and does not add a transport, writer, File Gate, release grant, or remote acknowledgement.

## Question and source of evidence

Can the **actual Rust Sync Planner** (not just the Python AKCB-01 model) refuse untrusted/foreign/tampered observations, distinguish a verified absence from a tombstone, preserve divergent siblings, and require explicit initial-transfer preview?

* Native producer: [core/tests/akcb_02_planner_trace.rs](../../aquilum-app/core/tests/akcb_02_planner_trace.rs). It constructs a real RevisionDag, RevisionRecords, and SnapshotDiff, then runs **1152 combinations** through the actual plan_snapshot implementation. The test validates the returned target revision for uploads, downloads, and conflicts.
* Independent verifier: [akcb_02_trace_check.py](./akcb_02_trace_check.py). It compares every actual outcome with a separate symbolic DAG oracle (root -> left/right/tombstone; unrelated root; foreign document; hash mismatch; verified absence; four uncertain observations). All 12 x 12 x 2 x 2 x 2 = **1152** cases must be present exactly once, or the check fails. It requires 11 outcome categories and rejects four synthetic mutations including an invented applied_ack.
* CI: [AKCB-02 Native Planner Trace](../../.github/workflows/akcb-02-planner-trace.yml), path-filtered and cancel-in-progress for updated HEADs. Existing Application checks remain separate.

## Reproduction (Rust stable, Python 3)

```bash
cd aquilum-app
AKCB_02_TRACE_OUT=/tmp/akcb-02-native-trace.json cargo test -p aquilum-core --locked --test akcb_02_planner_trace -- --nocapture
cd ..
python3 docs/research/akcb_02_trace_check.py /tmp/akcb-02-native-trace.json --self-test
```

Without AKCB_02_TRACE_OUT, the Rust integration test exercises the matrix without writing a file. With it, the only additional file is the explicitly supplied test artifact, never a user's vault, File Gate, S3 bucket or QUIC peer. The Python verifier uses only the standard library; the input file size is limited to 1 MB and duplicate JSON fields/cases are refused. **A synthetic oracle-only PASS is not native Rust evidence.**

## Coverage map

| AKCB-01 obligation | AKCB-02 verifies | Explicitly NOT proven |
| --- | --- | --- |
| S3 accepted inputs | known DAG content hash and document identity checks | transport authenticity, Bao, independently trusted expected digest |
| S7 unknown outcomes | pure planner has no replay/mutation action | real durable claims, File Gate, crash/reconnect |
| S9 conflict/absence | 1152 real decisions, missing vs tombstone, conflicts, first-sync preview | OS editor races, persistent UI, multi-node merge |
| S1/S2/S4/S5/S6/S10/S11 | none; no authority granted | **all runtime disclosure, write, ACK and recovery boundaries remain HOLD** |

No APPLIED or APPLIED_ACK action exists in this planner. The next work must connect real File Gate / History cold-oracle traces and MCP response fencing; CI for this slice may only claim planner conformance to the bounded oracle.
