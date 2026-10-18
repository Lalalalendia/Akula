# ADR-SYNC-036 — Cooperative macOS install lock and interrupted-generation handling

**Status:** staged in P0-C3b PR; **not** a signed release or installer custody
authority. Scope is the inherited **user-local macOS shell install/uninstall**
pair and temporary files under the user's `~/Applications`.

## Previously solved by #297

The normal update path copies the DMG bundle to a fresh
`~/Applications/.akula-install.XXXXXXXX/Aquilum.app`, checks the expected
native executable and code-signature structure, and saves the currently
installed application to `Aquilum.previous.app` in that staging directory
before renaming the new bundle into place. An ordinary failed copy or rename
invokes an EXIT-trap rollback. This is not an atomic *multi-step* update and
does not recover from SIGKILL or power loss.

## Remaining windows addressed here

1. **Multiple cooperating installers/uninstallers:** both scripts keep an
   open fd 9 on a persistent `~/Applications/.akula-install.lock` file and
   acquire macOS `lockf -s -t 0 9`. Do not unlink the file between runs:
   doing so can create distinct inodes with independent advisory locks.
   Contention refuses the operation before the network or filesystem
   mutation. The kernel releases the advisory lock after a process dies.
2. **Interrupted after moving old app to backup, before publication:**
   if exactly one prior `Aquilum.previous.app` is visible in a staging
   directory and `~/Applications/Aquilum.app` is absent, a new installer
   invocation restores that previous app **while holding the shared lock**,
   cleans up that recovered stage and exits without fetching/reinstalling.
3. **Interrupted after publishing new app, before cleanup:**
   when a destination is present alongside a staged previous version, code
   cannot determine which generation the user considers accepted. It
   **refuses to overwrite or delete either**, reports the exact staging
   directory, and requires manual review. Uninstall also refuses if
   interrupted-stage evidence exists.
4. **Incomplete stage without backup / suspicious stage alias:** fail
   closed without deleting an unverified file. Manually inspect the
   interrupted stage before retrying. Cleanup of a recovered backup happens
   only after the app has been restored.

## Acceptance and limitations

- Linux CI executes the *real* Bash install/uninstall scripts against
  temporary isolated `HOME` directories, synthetic DMG/curl/ditto/mv
  mocks, and an fd-lock adapter backed by `flock`. It checks concurrent
  install, concurrent uninstall, SIGKILL after backup, SIGKILL after
  promotion, ordinary promotion failure, intact Markdown vaults, and
  successful recovery without implicit network retry.
- A macOS GitHub Actions lane repeats the fixture on real `/usr/bin/lockf`
  rather than Linux's adapter. This establishes the intended BSD fd-lock
  behavior with real isolated processes; it does **not** validate the
  authenticity of a production DMG.
- `lockf` is advisory; unrelated programs with the user's filesystem
  privileges can still modify or delete the app. Neither `codesign --verify`
  with the current ad-hoc packaging identity nor HTTPS alone authenticates
  a dedicated Akula publisher. A signed Akula channel and independent key
  provisioning remain release blockers.
- MacOS `mv` operations on a single volume are name-atomic individually,
  but the two renames are **not a single atomic transaction**. No
  power-loss durability guarantee exists without explicit OS sync and
  actual packaged restart tests. A power loss during a rename or metadata
  flush is not covered by a synthetic SIGKILL test.
- The in-app updater is currently disabled for the fork and **does not
  share this shell lock**. Before enabling it, updater install/uninstall
  must use an equivalent single authority; do not claim cross-updater
  serialization yet.
- A hostile same-UID process or alternate installation path is out of
  scope. This is cooperative shell-installer safety only, not the
  OS-authorized custody broker needed for protected S3 writes.

## Release decision

Keep local-only published-package recipe (`--features installed`) and
continue withholding `sync-installed-host` from release. No current
change enables remote S3 overwrite/delete, automatic baseline promotion,
garbage collection, or unverified HOLD clearance.

Related: ADR-SYNC-035, #287 release immutable source and signed origin gate,
#297 staging and ordinary rollback, and the canonical Tela P0-C3 plan.
