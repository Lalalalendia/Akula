# ADR-SYNC-037 — Compiled installer witness in ordinary Core (P0-B6b-1)

**Status:** opt-in Rust/native integration with disposable OS-process test fixtures.
**NOT** a published installed-host product, OS-owned installation, or S3 write
authority. The standard local-only Akula release is unchanged.

## Problem

B6a/#293 added the physical ID verifier, but the ordinary native
`Workspace::open -> Core::open` and S3 activation still called the unanchored
host SQLite opener. After every Core exits, replacing a valid enrolled SQLite
with a new valid empty database would look like an unregistered workspace.
A pathname selected at compile time is not a trusted history of its contents.

## Minimal opt-in mechanism

- `aquilum-core/sync-installed-host-witness` and
  `aquilum-native/sync-installed-host-witness` imply `sync-installed-host`.
  Neither feature enables AWS or is present in `release.yml`.
- The selected binary has two **compile-time** settings:
  `AKULA_INSTALLED_HOST_CUSTODY_DB` and
  `AKULA_INSTALLED_HOST_WITNESS_PATH`. Runtime overrides, GUI `--data-dir`,
  private appdata and per-workspace settings cannot select a new source.
- The outside-SQLite witness is a strict, bounded, versioned JSON record
  (`version=1`, non-nil installation UUID, exact ledger path, expected OS
  physical parent + main SQLite IDs). The producer is the independently
  trusted installer, never a remote server or the candidate DB on cold open.
- The receipt **must be in a different non-nested parent directory** from
  SQLite. The reader rejects a missing or changed parent, symlink/reparse
  and hardlink aliases, missing/oversized/malformed/unknown fields, changed
  file identity during read, version/installation/path mismatches.
- The Core's one `open_installed_host_registry` method is used by BOTH
  `with_unenrolled_host_epoch` and S3 enrolled-host registration. It loads
  the witness and calls the B6a SQLite expected-identity verifier before
  ordinary note/history admission or global enrollment. A failed check is
  an explicit HOLD. No implicit witness creation, repair, re-bootstrap,
  fallback to `open_existing` or independent appdata source in this flavor.
- Legacy explicit `host-custody` and installed-host flavors continue to work
  exactly as before for current CI/non-protected local usage. That compatibility
  means **these older variants are NOT witnessed production custody**.

## Evidence and edge cases

New `app_core_witness_tests.rs` is exercised on independent Linux, macOS and
Windows CI runners, using two separately compiled absolute paths in distinct
run-scoped directories. The TEST-ONLY provisioner creates the receipt by
observing the original fixture DB; this is intentionally not a safe
production issuer.

Acceptance cases: positive existing witnessed ledger (unenrolled note/history),
independent second OS process with another appdata and an ignored forged
runtime witness path, S3-enrolled protected-root refusal, closing the open
DB then replacing it with a **real valid empty SQLite database**, fresh
cold-open refusal and unchanged user bytes, absence of receipt => HOLD, then
restoring the original DB and receipt => legitimate write permitted again.
The original enrolled DB survives in a separate named file for inspection
throughout substitution.

All established local/AWS-free host, native positive, S3 custody and
application gates must also remain green at the exact PR HEAD.

## Explicit non-claims / P0-B6b-2

This witness file is only independently **located**, not independently
**trusted**, until real platform packaging supplies OS-verified installation
identity and correct ownership/ACLs over the receipt, SQLite main DB and
WAL/SHM namespace, all relevant ancestors and any helper/broker. Another
process with the same user permissions could rewrite the receipt along
with SQLite. OS inode/file IDs can be reused and do not authenticate their
content. No hostile same-user security guarantee is justified by this ADR.

A robust real install must define and test per-platform issuance and
upgrade/uninstall preservation, rollback and in-flight transaction recovery.
The install UUID currently validates only non-nil shape; no system registry,
OS signature or broker attests its value.

Even after an OS-owned receipt is deployed, there is no blanket permission
for rename/copy/trash/restore, S3 overwrite/delete, GC, remote baseline
promotion, automatic reconciliation, or recovery of ambiguous primary+history
commits. All those require separate write/admission and product E2E acceptance.

**Do not enable this flavor in** `.github/workflows/release.yml`. Keep the
published `--features installed` local mode independent from Sync Beta.

Related: #282, #283, #284, #293, ADR-SYNC-035 (release),
ADR-SYNC-036 (cold physical witness).
