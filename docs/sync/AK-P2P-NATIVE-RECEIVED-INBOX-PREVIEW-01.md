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
