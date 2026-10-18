# P0 internal-sync mutation HOLD in mandatory installed-host Core

**Status:** testable fail-closed correction. Not a cloud writer, a receiver
integration, or installed release authority.

## The bypass

The public File Gate matrix (#284) already blocks copy/rename/trash/etc.
without a held shared host SQLite enrollment epoch. But
`RegisteredReadAdmission::create_staged_with_bound_vault_and_hook`, used
by opt-in additive local create and the new S3 additive receiver (#296),
takes `Core.file_mutations` directly. That process-local mutex **does not
check** the installed-host shared SQLite ledger or preserve the host epoch.
Likewise `archive_local_recovery_copy_for_vault` can write recovered bytes
under the workspace through a process-local mutex.

The separate cooperative custody OS lock in S3 receiver experiments does
not automatically replace the installer-host admission contract across
different appdata and after a cold registry swap. Even though existing
source code limits these routes to additive/no-replace operations, they
must not create an alternative host-policy writer.

## Minimal fix

At the beginning of each of these two native mutation routes, while no
primary filesystem byte has been touched:

- If the associated Core requires the installed-host epoch, refuse with
  `PermissionDenied`. This applies to `sync-installed-host`, witnessed-host
  opt-in flavor and explicitly installed Core, not to ordinary unhosted local
  experiments. The guard is only compiled when `host-custody` is present.
- Do not silently acquire a different private appdata lock, rebootstrap
  a ledger, or treat a held `Core.file_mutations` as shared authority.
- Read-only pinned diagnostics, preimage capture and recovery inspection
  remain available. No S3 overwrite/delete/GC/baseline permission changes.

The existing local additive proof continues in unhosted mode, preserving
its current experimental value. It is **not** automatically elevated into
installed-host S3 beta.

## Acceptance

A real registered workspace and fully staged journal/blob fixture is
opened through `Core::open_with_installed_host` WITHOUT a provisioned host
ledger. `create_staged_if_absent` must return PermissionDenied before
creating the destination. The staged intent and baseline remain unchanged;
the code must not create the missing host SQLite automatically.
The analogous archive helper must refuse before creating any recovery copy.

The local host-custody CI matrix runs this negative test on Linux, macOS,
Windows, alongside Application and relevant S3 regressions.

## Next gate

Allow these operations in an installed-host flavor only after they hold a
verified shared host epoch across the actual descriptor-relative filesystem
mutation **and all durable history/journal evidence**, with safe uncertain
commit handling. This is separate from #299's cold-start witness source.
Until then, additive S3 receiver remains an opt-in experiment, not an
installed protected publisher.
