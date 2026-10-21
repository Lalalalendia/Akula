# AKAI-HOST-CUSTODY-LIVE-RECHECK-01 — deny-only SCM stops on replaced authority pathname

**Independent source hardening. No production installation or positive Broker grants.**

## Evidence behind the change

The existing `windows_store_custody.rs` assumed two pinned Windows HANDLEs made their fixed names immovable. Real Windows tests in [#494](https://github.com/Lalalalendia/Akula/pull/494) falsified this assumption for a directory, despite an open handle without `FILE_SHARE_DELETE`. Microsoft describes `FILE_ID_INFO` and by-HANDLE file identity as ways to compare **open objects**, not as permanent authority for path strings.

## Implemented

- `CustodyWitness::recheck_fixed_names()` reopens the **exact** protected ProgramData directory and authority marker before comparing their fresh by-HANDLE volume+file identity against the two startup-pinned objects. It repeats existing strict owner/DACL/SYSTEM/Service SID checks. Missing, replaced, reparse, or ACL-weakened names force `HOLD`.
- Every connected session of the existing **deny-only** cancellable named-pipe experiment performs the guard before accepting and again after accepting/before reading a byte. On guard failure, it disconnects and stops the real LocalSystem SCM service. No client command is parsed, answered or upgraded to an authorization.
- A real directory rename falsifier tests both OS outcomes: rename may succeed with an open HANDLE, and in that case the old name is absent/new object while the moved name retains the open identity. No assumption about preventing rename is retained.
- The existing real Windows SCM fixture additionally changes the protected marker's DACL **while the service is RUNNING**, wakes the pipe through a real client connection, and requires a spontaneous STOP without invoking `Stop-Service`. It also continues testing missing marker, safe start, recovery, and clean teardown.

## Explicit limitations

- The present by-HANDLE comparison uses existing `BY_HANDLE_FILE_INFORMATION` (legacy 64-bit file index) for a short-lived deny-only diagnostic. Production root identity needs the full `FILE_ID_INFO` 128-bit value proved separately by #494.
- A comparison before a read is **not** an atomic pathname-to-object fence; an object can still be renamed between checks. Any future authorized access must open the pinned object or use a demonstrated handle-relative secure mechanism, not a revalidated pathname.
- This prototype still has NO signed installer, installer-owned enrollment, independent rollback witness, grant issuer, authenticated agent-client permission, positive MCP `read_note`, or File Gate write permission.

## Acceptance

On exact HEAD, standalone Linux and Windows `cargo test/check/fmt --locked` PASS, real disposable Windows LocalSystem SCM+protected marker+pipe fixture PASS including spontaneous STOP on live ACL tampering, Application/Main PR gates PASS. Diff limited to the standalone Broker, its existing Windows fixture, and this ADR.

[Akula AI-agent plan](https://app.notion.com/p/3fe32a84beec818fb8d5f4f81d5d46de) · [#494](https://github.com/Lalalalendia/Akula/pull/494) · [Windows FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
