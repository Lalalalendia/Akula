# PC-FG-03 — revision identity / restore contract

**Status:** draft implementation + CI pending. This is a synthetic temporary-Vault regression; no personal or installed application files are accessed.

## Implemented evidence

- Integration test: aquilum-app/core/tests/history_revision_identity_contract.rs, using actual Core::open, workspace registration and ordinary files::gate::write.
- Three Source::Agent saves A -> B -> C must create distinct history IDs with byte-for-byte exact BLAKE3 evidence.
- A stale expected primary hash must reject a restore without modifying current bytes or history count.
- Restoring B using Source::Restore(B.at_ms) must publish a fourth version without rewriting IDs/hashes of A, B or C.
- Repeating the same restore must not create a fifth entry; invalid history ID is rejected.
- Cold Core restart must preserve the earlier IDs, texts and digests.
- This test is intentionally excluded from sync-installed-host builds. It proves the default local File Gate contract only.

## Limits and follow-up

Normal Source::Me typing history can coalesce within the configured idle/cap window: that active version is not immutable. Strict Source::External verified receipts already re-read and BLAKE3-check one new version inside the custody scope. Ordinary version records do not currently persist an independent sealed per-version BLAKE3 digest. Therefore this regression does **not** establish authenticated historical content against hostile same-user tampering, trusted storage custody, physical hard-off durability, or restored GUI undo/redo.

PC-FG-02 is a prerequisite and PR #704 is Ready for review, not merged at this checkpoint. This PC-FG-03 PR must remain Draft until its own exact-head Main PR gate, relevant history profile validation, predecessor review/merge and further tampering / user-facing restore criteria are satisfied.

Notion: https://app.notion.com/p/40332a84beec8182900af091e5cc6814
