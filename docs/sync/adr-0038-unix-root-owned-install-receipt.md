# ADR-SYNC-038 — B6b-2a: Unix owner preflight for a separately installed receipt

**Status:** experimental, opt-in, AWS-free read-only preflight; not an issuer,
not a privileged broker, not a packaged installation, and **not** S3 authority.
The ordinary `installed` release and B6b-1 `sync-installed-host-witness`
continue unchanged.

## Why a distinct receipt is insufficient

#299 checked the main SQLite physical identity against a receipt whose
location was compiled into Core. It deliberately did **not** verify that the
receipt was controlled by a different OS principal. With all directories
under a user's writable HOME or appdata, that user can replace BOTH SQLite
and receipt after the process exits. Comparing physical IDs then proves
nothing about the previous installation.

## Unix read-only preflight, B6b-2a

The new, **separate** Cargo feature
`sync-installed-host-unix-root-witness` implies
`sync-installed-host-witness`. A Core using it must, before reading the
receipt and again after the read, verify:

1. Receipt path is absolute and already canonical; no symlink aliases,
   `..`, ambiguous parent substitution or redirect into an alternate tree.
2. Every ancestor from immediate parent through `/` is a real, root-owned
   directory with no group/other write permission.
3. The receipt is an actual non-hardlinked, root-owned regular file, with
   no group/other write permission.
4. Existing B6b-1 validation still binds schema 1, non-nil installation UUID,
   exact compiled ledger path, physical parent/SQLite main IDs, bounded exact
   JSON representation and independent non-nested parent namespace.

**Windows always returns HOLD under this feature.** There is no Windows
administrator/SYSTEM SID, protected DACL, SDDL inheritance or IPC broker
validation in this change. An unimplemented platform is not a green result
and cannot fall back to B6b-1 policy within the stricter feature.

This is an **observation** of owner/mode/path, not trusted installation
issuance. The existing fixture generator remains `#[cfg(test)]` and
continues to be rejected by the strict mode. The stage changes no normal
installer, package or updater and creates nothing at Core startup.

## Exact acceptance

A dedicated diagnostic workflow compiles this stricter Core on Linux,
macOS and Windows, without AWS. It checks:

- a valid existing SQLite registry and a structurally correct independent
  witness in a user-owned temporary namespace are rejected, without
  deleting or repairing either one;
- chmod 0444 of a user-owned receipt cannot create privileged authority;
  symlink aliases and writable intermediate directories do not pass;
- on Unix, a disposable file intentionally created by the CI OS-root
  account under an owner-protected `/opt/...` parent is accepted by the
  owner/mode/ancestor *preflight*, including from a second OS process;
- Windows rejects even a supplied path because there is no trusted DACL
  acceptance implementation;
- the legacy cooperative installed-host Core still compiles, and the
  published `release.yml` remains strictly `--features installed`.

The root-owned CI probe is deliberately **not** an Akula JSON receipt. Its
purpose is only to demonstrate real OS metadata verification rather than
trusting `chmod` inside the same user's appdata. There is **no positive
Core host-epoch write** from this probe; that requires later installer E2E.

## Unresolved authority — do not mistake receipt security for DB security

The native Core still uses direct SQLite WAL/FULL writes from the desktop
user. A hostile process with the same user's permissions can change rows
or rollback a ledger without replacing its inode. A root-owned receipt
which records only four physical IDs cannot authenticate SQLite contents.
Even a perfect owner check would not protect Markdown bytes independently
of the desktop user's filesystem permissions.

A real protected Sync Beta needs an explicit, independently privileged
broker (or separately approved equivalent) that controls SQLite main,
WAL/SHM and its directory/ancestor namespace, exposes a tightly checked
transaction/receipt protocol to Core, preserves enrollment on upgrades,
has trusted instance identity and rollback policy, and returns HOLD or
NeedsReview after ambiguous commits. It also needs an installer that can
issue receipt atomically without auto-adopting a preexisting unknown
registry, plus Windows DACL and macOS launchd/privileged-helper choices.

**Do not** enable this feature in `.github/workflows/release.yml`.
Do not use this gate to grant remote overwrite, delete, create, restore,
GC, baseline promotion or automatic Sync Beta publication.
On missing/changed authority, keep user originals untouched.

Related: ADR-SYNC-035/036/037; PR #299, PR #302.
