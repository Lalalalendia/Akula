# ADR-SYNC-023 — Direct HistoryService mutations must respect enrolled Core HOLD

Status: opt-in safety boundary, stacked on PR #237 after merged #235. No release authorization.

## Gap

PR #235 persists participants, and `Core::file_mutation_guard_for_write` denies ordinary note, directory and trash mutation after Core restart. PR #237 catches a direct durable metadata path, `File Gate::name_version`. However `Core.history` exposes a `HistoryService` whose own public `name`, `try_saved`, `try_moved`, `try_trashed`, `try_restored` and `maintain_retention` mutate `.aquilum/history` WITHOUT entering Core File Gate. A caller can therefore change/drop retained versions while primary files are held, including on a cold Core.

## This change

- `DocumentHub` publishes one `Arc<AtomicBool>` for this Core's enrolled/corrupt-store HOLD. The flag is initialized conservatively by SQLite v3 participant inspection at Core construction and turns true **before** durable enrollment starts.
- `Core::open` binds that same flag into its `HistoryService` before starting documents, watchers, UI or MCP. The binding is one-time and cannot be replaced with a permissive flag.
- Every public HistoryService mutation named above refuses when the flag is set, including direct calls that entirely skip the File Gate. Read-only `page`, `version` and `disk_before` remain inspectable. This is a conservative full-Core mutation HOLD, not path-scoped S3 authorization.
- To keep the one intentional protected write working, `HistoryService::try_saved_with_custody` accepts a **nonconstructible-outside-sync-module, noncloneable** `HeldCustodyHistoryPermit`. That token is instantiated only inside `save_existing_note_under_custody` after the physical OS-lock callback has entered and before `write_checked_history`. The latter passes the token to `HistoryService` through one verified writer path. Public `try_saved` does not accept or infer that permit.
- Regression: an enrolled Core and a separate cold Core cannot directly rename a version, save new version, relocate/trash/restore history or run retention. The original note and visible history remain unchanged. Existing S3 guarded-note, Windows/macOS OS lock and protected original checks must stay green.

## Remaining limits

A standalone `HistoryService::new` outside Core has no enrolled-context binding, and processes with a different appdata identity or OS writers bypass this Core's policy. Housekeeping invoked through raw low-level modules, OS tools or privileged same-user actors still needs a common, verified physical custody broker. No cross-vault write, S3 overwrite/delete, GC, baseline advancement, automatic recovery, untrusted actor resistance or public sync activation follows from this policy. The token is an internal call-path proof, not cryptographic authority.

Next: assess all standalone HistoryService constructions, File Gate mutators and source-of-truth physical root registry before scoped lockdown.
