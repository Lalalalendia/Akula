# AKCB-01: Akula cross-boundary authority, disclosure and durable-apply contract (v0)

Status: **research / bounded model; NO production activation**. Date: 2026-10-21.
Canonical Akula product: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b). See also [SYNC-CRASH-MASTER-01](https://app.notion.com/p/3ff32a84beec8127a9aaf14101ef45f1), [AKULA-AGENT-MASTER-01](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de), [AK-SEC-IFC-10](https://app.notion.com/p/3fc32a84beec810f86c4e901af9a2302), [Staya master plan](https://app.notion.com/p/3fe32a84beec81e5acf7c081fe5e0bfd).

## 1. Scope and honesty boundary

This document **composes existing contracts**; it does not add a second Sync Core, File Gate, key authority, network transport or product feature. A read-only bounded checker is supplied as `akula_cross_boundary_model.py` next to this file (run with `python3 ...`). Its successful execution means **its own abstract transition system satisfies its own assertions**, not that installed Akula obeys them. GitHub's Rust and real-OS tests are separate evidence.

The narrow model contains **one document, one received write, two actors (remote receiver + concurrent editor), one grant epoch transition, one buffered read, one crash and no durable recovery promotion**. It enumerates all reachable states to a fixed point (no depth truncation). It does not model multiple documents, byzantine peers, disk-controller lies, physical power loss, symlink races, complete QUIC framing, variable-size network writes or distributed quorum formation. It must not be used as an ALLOW/ACK gate.

The following terms are **distinct**:

| Term | Witness required | NOT sufficient |
| --- | --- | --- |
| TransportReceived | authenticated channel and bounded receipt | IP address, QUIC connection, peer display name |
| BytesVerified | object ID/length and content/Bao or BLAKE3 verification against an authorized expected root | a successful `recv`, ETag, uploader claim |
| Staged | private bounded temporary copy associated with operation and pinned source | verified but uncommitted transfer |
| WriteAuthorized | trusted subject + protected root identity + action/scope + fresh grant/policy epoch + expected document revision | bare VaultId, path, UUID, local SQLite row, SSH/TLS login |
| FileGateCommitted | one canonical writer, pinned path identity, and verified publication evidence | `rename` or buffer contents alone |
| HistoryDurable | matching history/preimage record survives specified crash model | callback returned, SQLite row merely inserted in another process |
| JournalDurable | durable operation record bound to content, path, policy and history | in-memory flag, unkeyed hash of untrusted metadata |
| Applied | all preceding conditions, explicitly **local** apply proof | transport/download/staging complete |
| APPLIED_ACK | Applied plus authenticated response and checked disclosure policy at emission | `VERIFIED_ITEMS`, `Staged`, local consistency `HOLD` |
| HOLD | safe refusal/preservation on uncertainty | a later license to retry unconditionally |

## 2. Protected identity and authority

Host-trusted authority `A` is a tuple, **not** a caller-supplied path:

```
A = (host_issued_principal, enrolled_vault_root_identity, object_identity,
     capability, allowed_sinks, policy_digest, grant_epoch, expiry,
     trusted_issuer_epoch, optional_user_approval)
```

A `VaultId`/`DocumentId` is a typed **identifier**, not proof of authorization or physical continuity. When applicable, host root custody must bind the physical opened handle and immutable registration receipt. Resolve aliases through a trusted resolver; do not let a remote `SourceRef`, imported Markdown or MCP request mint authority. A valid QUIC peer identity proves a channel peer, not the right to overwrite or disclose a document.

Define an operation-specific **authority lease** from a real host broker. It cannot survive grant revision, policy change or root identity change without fresh verification. Revocation semantics: **no NEW output after an invalid epoch**. It cannot erase bytes previously observed by a remote peer. If a response has already partially crossed the socket boundary, classify it as `PartialDisclosureHold`, not as zero disclosure; no automatic replay from a stale lease.

A user-visible consent checkbox cannot replace a host-issued grant. For native plugins loaded with process rights, broker checks without OS confinement cannot guarantee isolation from direct filesystem/network syscalls.

## 3. Executable safety obligations

**S1 — authority:** Any external `Disclose` or `FileGateCommit` action requires an active host-validated lease for the exact subject, root, object, operation, and epoch. `PolicyEpoch` is checked at the **last controllable boundary** before bytes are emitted or mutation is committed. A policy revoke concurrent with a blocked write must result in deterministic order or safe `HOLD`.

**S2 — no unauthorized disclosure:** `DISCLOSED => disclosure_authorized_at_emission`. `read`/search/index is *not* itself permission to send to an MCP client, plugin, peer, clipboard or model API. Cache hits, count queries, titles and derived vectors are outputs too. Partial network writes with unknown remote receipt cannot claim no disclosure.

**S3 — verified receive:** `FileGateCommitted => VerifiedBytes && DurableClaim && FreshAuthority && PinnedSourceAndDestination`. A received chunk is not a live document. A BLAKE3 match authenticates bytes relative to a **trusted expected digest**, not their author or legitimacy.

**S4 — single publisher:** All user-visible file mutations pass through the existing File Gate/DocumentHub reservation and current OS custody; no standalone S3/P2P agent `rename`/`write` bypass. Host instance and note identity are bound to the held permit throughout the operation.

**S5 — ordered durable apply:** `Applied => FileGateCommitted && HistoryDurable && JournalDurable && ProtectedOriginalRetained`. Every receipt states its crash model (process, VM hard-off, storage cache lie, physical hardware); an FS+two-SQLite transaction cannot be presumed atomic. On an unknown result, do not upgrade the confirmed baseline.

**S6 — truthful remote acknowledgement:** `APPLIED_ACK => Applied && VerifiedBytes && AuthorizedAcknowledgement`. A remote ACK for `Staged`/`LocallyConsistentHold`/`NeedsReview` is false. A `mark_applied` that has not survived an independently justified recovery regime cannot be asserted merely by the checker.

**S7 — no blind replay:** Once a durable claim is consumed or outcome is ambiguous, the same `op_id` can be **read-only probed** for existing proof; no second File Gate invocation based solely on timeout/reconnect. A novel user intent needs a new authorization and operation identity. Unknown outcome -> `HOLD` unless an accepted recovery procedure can prove promotion safe.

**S8 — preserve preimage:** Before uncertain replacement, retain bound original and necessary history. A rollback must be a separately authorized, state-checked operation; cleanup/GC must respect active holds and causal descendants. Protecting two paths on the **same physical SSD** is not disaster recovery.

**S9 — no silent merge:** Concurrent edit with changed expected revision is a conflict, not last-writer-wins. Unknown/missing remote listings are never authoritative tombstones. Merge families must be chosen by object class (immutable, manual merge, domain resolver, CRDT, STRICT); policy, trust roots, revocations and signed/finalized objects cannot be automatically text-merged.

**S10 — provenance/derived policy:** A derived answer inherits the joint confidentiality restrictions of every input. `valid_signature(author)` does not entail `fact_true`. Any accepted/final state needs an authorized provenance decision, not model confidence. Index/search/vector/graph are rebuildable **projections**, never the source of truth.

**S11 — protected progress:** `HOLD` must include reason, frozen evidence reference, policy/authority epoch and recovery conditions. An actor can exit HOLD only through a versioned, explicitly authorized recovery path with fresh witness and demonstrable safety; indefinite HOLD is an acceptable safety outcome when authority is unavailable. No general unconditional liveness guarantee during partition.

## 4. One document: reference state machine

```
RECEIVED (transport receipt only)
  -> VERIFIED_BYTES (trusted digest and bounds)
  -> CLAIMED (durable one-shot op_id, fresh authority, expectedRevision)
  -> FILEGATE_COMMITTED (pinned identity, protected original retained)
  -> HISTORY_DURABLE
  -> JOURNAL_DURABLE
  -> LOCALLY_APPLIED (confirmed baseline may advance only here)
  -> APPLIED_ACK (authenticated emission under current policy)

At ANY ambiguous boundary: UNKNOWN/HOLD --[read-only cold inspection]-->
  (a) still HOLD, (b) DENY, or (c) a *separately specified* promotion
      supported by independently accepted durable proof.
```

This is a specification **ordering**, not proof that the current services can atomically execute it. The `FILEGATE_COMMITTED` label in the abstract checker stands for a trusted publication event; the production gate has additional OS path-lock, host-epoch and preimage obligations. In the reference model, a crash even after `LOCALLY_APPLIED` but before `APPLIED_ACK` yields HOLD; we deliberately do not infer a replay-eligible ACK from SQLite on restart.

## 5. Boundary-by-boundary authority matrix

| Boundary | Producer guarantee | Consumer recheck | Unsafe shortcut / fail state |
| --- | --- | --- | --- |
| OS Host Broker → MCP | host-minted principal, registered physical vault, durable epoch | exact vault/resource/scope, epoch at dispatch **and response** | common bearer alone → deny |
| MCP/Plugin → File Gate | canonical intent, fresh expected revision, explicit capability | note pinned identity and held-custody permit | path-only API bypass → deny |
| Staya/Roha → Akula Sync | bounded authenticated transfer, manifest and independent bytes check | trusted object ID/root, authorization and source identity | verified download ≠ APPLIED |
| RemoteStore → Sync Planner | versioned stable listing or verified manifest | completeness, tombstone provenance, baseline | 403/timeout/partial listing ≠ deletion |
| File Gate → History → Sync journal | preimage, source/identity and durability witnesses | linked operation receipt and cold-open validation | multi-store half-commit → HOLD |
| Source Registry → Search/Vector → AI | SourceRef with version, scope and freshness; derived lineage | output scope at aggregation and emission | counts/snippets/embeddings leaking forbidden input → deny |
| Policy/Revocation → Cache/Events/Peer | monotonic trusted epoch, current rights | sink-side recheck and cache invalidation | cached grant/old epoch → refuse new disclosure |
| Installer/Release → Host Trust | independent publisher/root identity and antirollback | OS-protected issuer and binary/hash chain | repo commit/unsigned manifest alone ≠ installed authority |

**Non-goals in this v0:** all-provider distributed transactions; permissions revocation of data already published; proving strong noninterference including all timing channels; arbitrary native plugin sandboxing; production recovery activation; automatic remote baseline promotion.

## 6. Object classes and merge boundaries

| Object | Class | Safe disconnected action |
| --- | --- | --- |
| Immutable chunk/manifest | Immutable; verify content and parent | copy verified bytes; never merge bytes |
| User draft/note | Branch-on-conflict unless a versioned domain resolver is proven | edit own branch and preserve competing heads |
| Task/checkbox | stable operation identity + compare-and-swap target | delay conflict or preserve intent; never trust `path:line` after edit |
| Canvas/media/time segment | format-aware operations with durable element/time identity | copy immutable original; edits remain separate revisions |
| Signed or published artifact | Immutable/append correction | new revision, keep signed predecessor |
| Membership/grant/revoke/key epoch | STRICT authority | fail closed for new writes and disclosures |
| Search index/embeddings/cache | Derived projection | rebuild from currently authorized source revisions |
| AI assertion/knowledge graph edge | Provenance-bound claim | append evidence/contradiction; no CRDT merge of truth |

These classifications **extend**, not replace, the separate [Consistency Classes](https://app.notion.com/p/3ec32a84beec815a91a0f059c73b6a18) and [Merge Resolver Registry](https://app.notion.com/p/3ec32a84beec81f7a836f83dc50be558) research. That earlier work concerns LesHa and is not automatically an implemented Akula wire format.

## 7. Checked small model and adversarial scenarios

Run:

```sh
python3 docs/research/akula_cross_boundary_model.py
```

Only the Python standard library is required. The checker performs exhaustive reachability over its bounded finite state space (not a sampling run); prints the actual state/edge counts, witness traces and negative controls. It fails if the bound is exceeded. The documented baseline execution explored **304 reachable states / 518 transitions**, with 16 different action labels, a maximum shortest trace depth of 12, and rejected **7 intentionally malformed states**. This does **not** establish a Rust / OS correctness claim.

Required witnesses:

1. Normal receipt → verify → claim → File Gate → history → journal → applied → ACK.
2. External editor wins between claim and publication → `hold_stale_or_revoked`, no overwrite.
3. Grant revoked between buffering and sending → drop buffer; no disclosure.
4. Grant revoked after claim but before publication → HOLD; no file mutation.
5. Crash just after File Gate publication, before history → HOLD and original preserved.
6. Crash after local applied but before remote ACK → HOLD, not a blind resend.

Mutation (negative-control) oracle rejects: premature ACK, publication without claim, false publication fence, advancing baseline before apply, unauthorized disclosure, blind retry, deleting protected original during HOLD.

### Missing falsifiers (not proved by the model)

- Same-UID malicious process and malicious extension bypassing SDK.
- ABA identity/path change, reparse point/junction and symlink swap during a real OS write.
- Concurrent policy revoke while an HTTP response is already partially flushed; packet-level proof of `PartialDisclosureHold`.
- FileGate + HistoryService + SQLite independent power-loss ordering, actual `fsync`, Linux ext4 and Windows NTFS hard-off / physical power loss.
- Two writers / two devices with different policies and a split-brain upstream source of grant epochs.
- Real Iroh/QUIC disconnect/reconnect and a committed ACK with delayed/lost delivery.
- Index poisoning/aggregate leaks under restricted multi-vault search; cached embeddings after revoke.
- `OutOfSpace`, interrupted garbage collection, key rotation and mixed-version protocol upgrades.

## 8. Source-of-truth to test mapping and release gates

| Obligation | Existing code / research anchor | Next executable falsifier | Gate |
| --- | --- | --- | --- |
| Revocation to disclosure | `aquilum-app/core/src/mcp/agent_scoped_read.rs`, `agent_vault_grants.rs` | installed host-issued principal + real response fence; revoke during blocked write | HOLD; present files self-describe as **INERT** |
| File publication and crash | `core/src/files/gate/write.rs`, `docs/sync/p0g3d-native-receipt-plan.md` | OS-child kill/power-cut across each durable boundary; cold oracle must be independent | Existing Sync P0g HOLD until true durable receipt |
| Staya → File Gate | Staya `VERIFIED_ITEMS` and G2 FsStore/IPC | a real 2-process verified transfer then **separately guarded** Akula apply, no premature ACK | HOLD for product integration |
| Multi-provider source truth | `core/src/sync/provider.rs`, `s3_v2_trust_anchor.rs` | partial listing / rollback / stale response / authority rebind / provider mutation | unverified provider -> read-only/HOLD |
| Plugin/AI information flow | `AK-SEC-IFC-10`; Akula query/event theory | A/B hidden-note experiment across search, counts, snippets, citations, caches, sink output | unsafe plugin execution disabled |
| Cross-object versions | `DocumentHub`, `Sync Planner`, planned `SourceRef` registry | rename, duplicate, content-equal different ID, deleted/recreated object | ambiguous identity -> no automatic merge |
| Quorum/recovery | Staya owner DENY epoch / external CAS labs | stale writer + partition + recovered authority; independent monotonic witness | lab claims are not product authority |

Do **not** merge or activate a write API simply because this specification or checker passes. CI green for a model is not release green for installed Akula.

## 9. Next bounded steps (one narrow PR per executable gate)

- AKCB-02: Add a **production-independent trace bridge**: record abstract events from current Rust test fixtures without enabling any new write path; compare event traces with AKCB-01 invariants.
- AKCB-03: Extend model to two concurrent documents / two independent grants and mixed actions, then verify revocation and source identity across search/MCP/AI output.
- AKCB-04: Connect a **trusted** host-issued disclosure lease to real MCP HTTP and test socket flush / revoke races; do not claim this capability while current code is inert.
- AKCB-05: Combine Staya transport-only receipt with existing cold File Gate/History test under real OS crash, prove `APPLIED_ACK` separation; product integration remains HOLD until successful end-to-end gates.
- AKCB-06: Define operator-guided promotion out of HOLD only after independent release/host authority and durable witness; verify safety **and conditional progress** under documented fairness assumptions.

No calendar promises. No changes to Core, active MCP, installer, agent grants, native service, remote writer or release settings in AKCB-01.
