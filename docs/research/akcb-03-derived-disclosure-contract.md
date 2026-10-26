# AKCB-03 — two-source derived disclosure & epoch isolation (test-only v0)

Date: 2026-10-21. **DRAFT / HOLD**. Stacked after AKCB-01 ([#517](https://github.com/Lalalalendia/Akula/pull/517)) and AKCB-02 ([#529](https://github.com/Lalalalendia/Akula/pull/529)). No production capability, no HTTP/stdio MCP route, no S3/QUIC ACK, no indexing of real user sources.

## Distinguishing issue

An actor may read document X but not Y. A derived COUNT, TOP, title, snippet, embedding or AI summary influenced by X and Y is **not safe** merely because the final output quotes no forbidden text. Derived output retains the confidentiality restrictions of every input. Removing Y from rendered rows **after** computing global aggregates is insufficient.

The repository already has inert authentication, grants, pinned read and disclosure-epoch labs. AKCB-03 reuses the actual Rust AgentCredentialStore and AgentVaultGrantStore in a separate cfg(test) module, not a new production broker or search engine.

## Executable scope

- Agent A has NotesRead on X only; agent B has independently issued NotesRead on both X and Y. X and Y have distinct synthetic Origin { id, root_registration } records.
- A test-only pre-filter drops Y **before** aggregate COUNT and title construction. Modifying hidden Y does not alter A's output bytes. An explicit combined X+Y derived candidate for A is denied.
- A candidate records a current grant lease for every trusted origin. All leases and agent credentials are rechecked under cooperative SQLite locks held through full emission and flush. Missing/swapped source leases deny.
- Revoking B's Y grant after staging blocks the mixed result but leaves B's X-only and A's X-only result available. Regrant increases the epoch, so stale cached data remain blocked. A newly prepared mixed candidate is accepted.
- A concurrent revocation cannot return a durable ACK while the test-only response is being emitted with the locks. After four simulated emitted bytes then a BrokenPipe, result is PartialDisclosureHold, never a claim of zero disclosure.
- Revoking B's identity credential blocks already staged B output without removing A's grant.

## Evidence

- Native producer: [akcb_03_composite_lab.rs](../../aquilum-app/core/src/mcp/akcb_03_composite_lab.rs). Imported strictly with cfg(test) in [mcp/mod.rs](../../aquilum-app/core/src/mcp/mod.rs).
- Independent trace oracle: [akcb_03_derived_trace_check.py](./akcb_03_derived_trace_check.py). Exactly **20 scenarios** with expected outputs, source list, count and zero bytes for denied responses. Four corrupted traces plus duplicate JSON object keys must be rejected; five negative controls overall.
- CI: [AKCB-03 Composite Disclosure Lab](../../.github/workflows/akcb-03-derived-disclosure.yml). Rust must emit JSON on the CI runner itself; a synthetic oracle-only PASS is never accepted as native evidence.

Reproduce:

~~~bash
cd aquilum-app
AKCB_03_TRACE_OUT=/tmp/akcb-03-trace.json cargo test -p aquilum-core --locked akcb_03_native_grants_and_derived_disclosure_trace -- --nocapture
cd ..
python3 docs/research/akcb_03_derived_trace_check.py /tmp/akcb-03-trace.json --self-test
~~~

## Coverage and limitations

AKCB-01 S1/S2/S10 are modeled only for this **synthetic test-only two-source dataflow**. We exercise actual SQLite grant/credential functions, not real search indexing or actual MCP request delivery. Physical root registration is a synthetic fixture, not OS/installer verified; source completeness is trusted in this lab; OS custody and uncooperative same-UID processes are outside scope. Existing 01d tests are a cooperative epoch-ordering technique, not an end-to-end HTTP response fence or a guarantee of client receipt.

None of this proves confidentiality of production Dataview/Tantivy/vector/AI output, timing side-channels, agent plugins, a live HTTP connection, or cross-device authorization. All production routes remain unchanged and their previous HOLD gates remain.

## Next gate

Verify AKCB-01 and AKCB-02 on their exact CI heads, then run AKCB-03. AKCB-04 must prove source-complete lineage through a real query engine and a trusted host-root/identity issuer plus emission fence. No second Sync Core, File Gate or standalone permission database is permitted.
