# PC-FG-03: sealed immutable version IDs (first integrity slice)

**Status:** Draft, exact-head CI pending.

New fixed revisions (Agent, External, Start, Links, Restore, Revert) bind BLAKE3(raw content bytes) into their new history version filename with a ~b3- marker. One existing atomic-write commit publishes both the ID and contents; there is no torn metadata sidecar. Existing timestamps, source/origin labels, retention and rename structure remain readable by the updated parser.

HistoryService version reads check raw bytes against the embedded digest *before* editor CRLF normalization. Changed bytes return no version for restore selection. Sealed IDs refuse in-place writer updates. The active Source::Me coalescing files remain unsealed. Existing filenames are still readable as unverified legacy versions; no migration renames previous IDs.

Evidence: store unit tests for tamper, malformed seal, CRLF, Restore origin, legacy and mutable editor; synthetic Core/File Gate integration test with A→B→C, external byte tamper of B and denial of B read while primary C remains unchanged. Production source no-growth: existing store.rs shrinks, new helper contains the additive implementation.

**Limits:** This is not hostile same-user authentication (the same user can rewrite both bytes and ID), nor a sealed index of all legacy or Source::Me records. A corrupt version can still appear in list metadata; UI verified/unverified/invalid status and broader history-writer HOLD remain separate. Review Windows long-path behavior, history cleanup/relocation and performance before merge. No QEMU hard-off, signed release or Windows installed-GUI acceptance asserted.

Notion: https://app.notion.com/p/40332a84beec8182900af091e5cc6814
