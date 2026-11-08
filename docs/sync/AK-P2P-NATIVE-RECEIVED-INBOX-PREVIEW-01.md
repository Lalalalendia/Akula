# AK-P2P-NATIVE-RECEIVED-INBOX-PREVIEW-01

Stacked on [#672](https://github.com/Lalalalendia/Akula/pull/672) and [#642](https://github.com/Lalalalendia/Akula/pull/642).
**Opt-in UI preview, default OFF. No installed Akula feature yet.**

- The existing Native **File transfers** modal displays a separate **Received Inbox** section.
- Source is the same local per-user path as the one-file receiver, never fake test fixtures or remote metadata.
- One worker reads the existing SQLite ledger in read-only/query-only mode and cold rehashes **24 entries per page**; explicit older/newer controls reach earlier receipts. Opening an empty Inbox creates no folders or database.
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

## Protected destination exclusion

A manual Save Copy must not act as an alternate File Gate write. The opt-in
native controller now gathers the current Akula data directory, current Vault
and all recorded workspace roots (failure to load the workspace list fails
closed), while the backend also excludes the receiver Inbox itself.
Canonicalized chosen parent paths inside these protected trees are refused,
including ordinary symlinked parents leading into a known Vault. A separate
test checks no file is published into either protected root.

**Security limit:** canonical-path prechecks still have an ancestor TOCTOU
window on an actively hostile local filesystem. OS-owned directory handles
and Windows reparse/ACL acceptance remain a release HOLD, not a solved claim.

## Local file-kind hints — opt-in QA only

The Inbox row may display a **tentative local content-type hint** after the
existing receipt has been verified and its pinned file handle independently
rehashes with the immutable expected BLAKE3. Only bounded bytes are inspected:
PDF, PNG, JPEG, GIF and ZIP **header signatures**; complete, safe UTF-8 text
up to 4 KiB; otherwise **format not determined**. This is a usability label,
not a MIME/security validation or proof that the file is safe to open.
The code never dispatches an OS file handler, interprets remote paths, or
turns a guessed extension into a destination path. If the second local
rehash fails, the row becomes Needs Review, never Verified. Native Windows
B3 CI runs unit and seeded-local-file regression tests.

**Original sender filename and timestamp remain unavailable.** They cannot
be recreated from a receipt UUID, hash, or header. Keep the existing
`received-<UUID>.bin` export suggestion until authenticated, bounded
sender metadata is independently specified and verified. Physical WAN
testing was waived for development, not reported as passed.

## Signed Offer V2 display-label projection — QA, not installed

The existing OfferV1 already carries a bounded display-only file label in
its domain-separated manifest commitment. The V2 two-party Ed25519 receipt
covers that commitment, the selected index, both enrolled endpoint IDs,
the offer ID/nonce and the expected local file digest. There is **no need
to invent an extra filename field in the one-file QUIC frame**.

The opt-in Core `signed::display::verified_offer_label_v2` verifies
the entire cold receipt with independent caller-supplied pinned peer IDs
and a rehashed local Inbox observation before returning a typed
`VerifiedOfferLabelV2` display hint. It rejects altered labels, forged
signatures, mismatched file hash/receipt, wrong peers and invalid
selection/time. The type provides only text and the bound receipt UUID;
it exposes no path, export, import, delete or file-open operation.

**Still missing:** storing the accepted signed Offer/receipt under OS custody,
linking its V2 receipt ID to the Native Inbox, performing live receiver UI
consent and authentically issuing the V2 proof in the real QUIC flow. Existing
content-addressed V1 Inbox receipts cannot be relabeled using an unrelated
signed Offer. Until those steps exist, Native displays only UUID, locally
checked type hints and safe `received-<UUID>.bin` export suggestions.
A signed display label is still a claim by the sender, **not independently
verified historical disk filename or a trusted destination path**.

Physical cross-WAN acceptance remains explicitly NOT TESTED (waived only
for development). QA-only / DO NOT MERGE / no signed release.
