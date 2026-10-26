# STY-TRANSFER-UI-01 — native Akula transfer preparation (local draft only)

**Scope:** First user-visible transfer screen in existing Akula Masonry shell. No new standalone app, QUIC session, delivery permission, File Gate authorization, or remote file writes.

- Upload icon in the left rail opens an accessible native transfer dialog.
- Picker runs on a worker thread. The UI stores only names and metadata for at most 32 regular files. No content is uploaded, copied or hashed. Symbolic links and folders are refused.
- Queue rows truthfully say “waiting for trusted device / 0% sent”. Pause/resume toggles *only local draft intent*; removal discards metadata and never a source file.
- Device list is empty until authenticated discovery and verified entitlements exist. No fabricated peers, fake 100% progress or QUIC ACK == durable receipt.
- Drafts exist in memory only: closing the dialog retains them in this process, restarting Akula discards them. No hidden side effects on notes or File Gate.
- Both Russian and English labels are supported.

## Acceptance

1. Rail shortcut opens the dialog in a native Masonry RenderRoot.
2. File picker completion creates a queue row; pause and remove mutate it.
3. Missing files, folders, symbolic links, duplicates and >32 entries cannot create unbounded drafts.
4. Closing or pausing never reads, mutates, sends or applies file contents.
5. Native Linux/Windows compile, real UI regression and formatting CI must pass on the exact HEAD.

## Required follow-up

Future integration needs verified peer identity and explicit selection, authorized File Gate issue/revocation, an authenticated transfer session with accurate verified progress and receipts, plus safe restart/recovery. Until these gates pass, **all network sending remains HOLD**. Draft metadata cannot silently become authorization.
