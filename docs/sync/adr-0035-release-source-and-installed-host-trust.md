# ADR-SYNC-035 — Immutable release source and staged installed-host custody

**Status:** Release-source enforcement proposed in this PR; OS-owned custody deployment
is **not** implemented or approved. No remote write/delete/GC or silent HOLD clearance.

## Release issue: a moving `main` is not a release revision

Before this change the manually dispatched release workflow checked out the selected
`ref` independently in the Windows, macOS, Linux and manifest jobs. A fast-moving
`main` could therefore produce validly signed packages from different commits
under a single version and update manifest, despite each individual runner passing
its own smoke test. In particular, a newly introduced custody feature could be
present in some binaries but absent in others.

The release workflow now first checks out the selected ref in one **source** job,
resolves its full Git commit SHA and passes this SHA as a required dependency
to every platform and manifest job. Every downstream checkout and
`git rev-parse HEAD` must agree with that source. Builds do not re-resolve
a moving branch. The required Application PR gate checks this topology.

The existing standard packages still compile:
`cargo build --release -p aquilum-native --features installed`.
A separate contract test refuses accidental `sync-installed-host`, `sync-s3`
or `--all-features` in this published local-only release workflow.

The unsigned `custody-release.json` artifact records the selected source SHA,
`custody_mode=local-only`, the audited Cargo feature set, and that neither
an installer ledger nor S3 mutator is provisioned. This is **workflow provenance
metadata**, not cryptographic proof of installed file content or OS attestation.
Existing package signatures and smoke tests remain required independently.

## Distribution identity is a separate release blocker

The fork's current package names and UI still inherit Aquilum branding.
This PR updates the **shell installer** and generated release manifest to
`Lalalalendia/Akula`, and refuses to generate a release manifest when its
Akula signing public key is missing. The in-app updater already targets this
fork but stays disabled until a dedicated Akula key/channel is provisioned.
Old upstream v0.2.6 packages in explicit upgrade smoke tests remain historical
fixtures, not the new release channel. The fork's GitHub Releases API returned
no published release at this checkpoint. A separately signed and manually
smoke-tested first Akula release plus working upgrade/uninstall path are still
required; this PR does not create or publish one.

## Three different properties — do not mix them

1. **Local-only installer recipe (NOT a published Akula release)** —
   `installed`; no mandatory host registry, no S3 writes. The fork's GitHub
   Releases API returned no published releases at this checkpoint.
   This PR switches the inherited shell install and generated download URLs
   to `Lalalalendia/Akula`, so no upstream binary is mislabeled as Akula.
   Until the first signed Akula release exists, install fails without
   modifying the already installed application or the vault.
   Normal local editor behavior must remain available without S3.
2. **Cooperative installed-host prototype** — opt-in
   `installed,sync-installed-host`; `Core::open` uses one compiler-selected
   absolute SQLite pathname without consulting appdata or runtime environment.
   Real two-process native CAS/history positive+negative tests run on three OSs.
   Registry/WAL/SHM identity checks detect substitutions at measured points.
   This is NOT an installed product, trusted cold start, or defense against
   arbitrary same-user filesystem rewrites.
3. **Production protected S3 editor** — **HOLD** until trustworthy host
   provisioning/authority, protected writer and recovery are separately
   accepted. A path compiled into a binary is an identifier, not proof
   of who created the object currently at that path.

## Why setting an ACL is not sufficient on its own

A SQLite ledger writable by the desktop user's principal can also be edited
or replaced by another process running as that user. File identity and BLAKE3
can detect some alias/corruption cases, but BLAKE3 in a writable ledger is
not an authentication code. Making the ledger root/SYSTEM-owned would stop
the desktop client from taking `BEGIN IMMEDIATE` directly, so a strongly
OS-trusted registry needs an independently authorized service/broker to
mediate transactions. A broker alone also cannot prevent a hostile same-user
process from editing a user-writable Markdown file directly. Product claims
must distinguish **cooperating Akula process safety** from a guarantee
against hostile processes with the same OS permissions.

The next installer design gate must explicitly pick and document its threat
model. For cooperative-only rollout, a per-user OS-resolved location may be
adequate *after* real package testing but must not be labeled hostile-process
safe. If hostile-process isolation is a product requirement, the proposal
must additionally isolate target file-writing authority, not just the ledger.

## Required independent installation acceptance

- **Trust source:** all installed app processes (including `--data-dir`
  variants, MCP, updater and restarted instances) obtain the *same* OS-derived
  host identity. The installer provisions once; interrupted upgrades must
  never create a fresh empty enrollment roster. No runtime env or appdata
  substitution, and no silent fallback to legacy Core.
- **Filesystem identity:** verify the installer authority at cold open, pinned
  parent/ancestor and main DB/WAL/SHM identities, symlink/junction/hardlink
  attacks, immutable/mutable ACL ownership, and crash/restart. A valid empty
  database at the same path must **not** reset existing enrollment.
- **Mac, Windows, Linux:** specify the exact privilege boundary and installation
  mode separately (user-local vs elevated OS install, update and uninstall).
  Windows ProgramData can be relocated, and user-local macOS DMG/AppImage
  installs must not pretend to have installed a root-owned host service.
- **Positive/negative packaged E2E:** signed installer -> installed app ->
  two independent processes with different appdata -> legitimate CAS and
  two persisted history revisions for an unenrolled vault, HOLD on a genuinely
  enrolled vault, stale-CAS refusal and unchanged original bytes.
- **Mutation coverage:** all native/MCP File Gate, history, trash, restore,
  rename and startup cleanup routes must either hold the same authority
  across primary+history or fail closed; no independent per-appdata fallback.
- **Crash:** simulate power-loss timing around PREPARED, primary publication,
  history persistence and acknowledgement; ambiguous results return
  NeedsReview and never blind retry or auto-delete. Preserve old versions.
- **Regression:** the entire published local-only build must continue to
  edit normally with neither S3 nor an OS host service installed.

These acceptance requirements are **not** satisfied by this ADR, by
`custody-release.json`, or by #264/#278/#282/#283/#284 alone.

## Deployment decision

Continue shipping only the ordinary local `installed` build. Introduce a
separate, non-published protected-host packaging lane only when a concrete
installer/OS-identity proposal and the full positive+negative acceptance
have been specified. Do not flip the release workflow to
`sync-installed-host` merely because the Rust Core tests are green.

Related: #264 mandatory compiled source, #278 native positive, #282
SQLite/WAL/SHM swap rechecks, #283 S3 editor host enrollment, #284 File Gate
mutator matrix, and #251 alternate-appdata falsifier.
