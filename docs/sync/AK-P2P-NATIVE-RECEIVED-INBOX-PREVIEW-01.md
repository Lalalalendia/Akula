# AK-P2P-NATIVE-RECEIVED-INBOX-PREVIEW-01

Stacked on [#672](https://github.com/Lalalalendia/Akula/pull/672) and [#642](https://github.com/Lalalalendia/Akula/pull/642).
**Opt-in UI preview, default OFF. No installed Akula feature yet.**

- The existing Native **File transfers** modal displays a separate **Received Inbox** section.
- Source is the same local per-user path as the one-file receiver, never fake test fixtures or remote metadata.
- One worker reads the existing SQLite ledger in read-only/query-only mode and cold rehashes at most **24 newest entries**; opening an empty Inbox creates no folders or database.
- The view shows bounded receipt ID, byte count, sender fingerprint prefix and precise status:
  Verified locally, Pending confirmation, Needs review.
- It never calls File Gate, opens any user content, starts a network receiver or issues ACK.
- The source schema currently lacks trusted original filename and creation timestamp. Do NOT invent either; the displayed ID is only a technical placeholder. Sender-provided names must remain display-only if later added.
- Windows root ACL, reparse/ancestor races, physical dual-PC acceptance, sender signing, user **Open/Save** actions and signed installed build remain unproven blockers.

Validation: exact-head default-off app build, opt-in Native compile, real seeded ledger projection and corruption regression, then actual installed Windows GUI pointer/UIA acceptance. Native tests are *not* a substitute for installed acceptance.

## Explicit Save Copy action (still opt-in)

A verified row exposes **Save copy**, which opens the system Save dialog in a
background worker. The action obtains a fresh, read-only SQLite claim and
independently reopens and hashes the same pinned source file descriptor,
then calls Core `save_verified_stage_as_new`. This rehashes the source while
writing a fresh private sibling candidate and publishes with atomic no-clobber
hard-link. Selecting an existing destination fails without replacement.
Pending and Needs-review rows have no button; a stale click still rechecks
the bytes before publication. No network, Vault, sender ACK or original
filename is inferred. The suggested `received-<UUID>.bin` name is an
explicit temporary placeholder until authenticated filename metadata exists.

**Not yet delivered:** internal Open action for arbitrary untrusted file
types, original extension/filename, physical installed-app testing, Windows
ancestor reparse/ACL custody. Do not turn the flag on in a public installer.
