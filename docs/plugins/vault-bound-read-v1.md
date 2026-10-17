# PLUGIN-04 — Vault-scoped descriptor-safe read adapter

**Status:** proposal under active PR review (not a plugin runtime). Parent: [Akula PLUGIN-00–22](https://app.notion.com/p/3fc32a84beec8132bd9cd127ba517080), [phase B](https://app.notion.com/p/3fc32a84beec81988742d38ed545bbb7). Depends on merged PLUGIN-00–03.

## Objective / product value

Enable a future community plugin to read only notes explicitly permitted for its installed instance **without** ever receiving an ambient filesystem path or descriptor. Build atop the *existing* read-only, pinned-sync admission code; do not duplicate platform-specific sandbox code or add cap-std / a new runtime prematurely.

## Host-only data flow

1. Core admits **registered** workspaces via existing UiStateService and pinned filesystem root: the vault UUID comes from the saved workspace registration (not from an untrusted plugin parameter).
2. Trusted host document indexing calls HostPluginVaultReader::bind_host_note with one portable, canonically spelled relative path. It derives the note ID from the saved document registry. The resulting ScopedNoteHandle is opaque, not serde-serializable, and never sent as a path or host pointer across the future plugin ABI.
3. Trusted host code obtains live current-vault/note context. PermissionBroker.authorize checks that this plugin's **actual grant** covers NotesRead on this particular note in the matching registered vault; a manifest request is not a grant.
4. The registered admission reserves a closed document (refusing unsaved/open editors), acquires the Core File Gate mutex, traverses *one component at a time* under pinned OS handles, refuses intermediate/final symlink or Windows reparse points and hard links, reads bounded content, verifies file identity and bytes by a second descriptor-relative read, and rechecks the registered root identity.
5. The host rechecks workspace identity and the permission lease against **fresh** UI context and persistent revocation epoch before returning bytes. All failures return generic permission/unavailable results without paths or note content.

The pinning code is **reused** from aquilum-app/core/src/sync/pinned_read.rs, pinned_windows.rs, host_admission.rs, and SyncRelativePath. Windows uses NtCreateFile parent-handle-relative and refuses junction/reparse; Unix uses rustix openat with NOFOLLOW. No direct host path is accepted from an external plugin, and this is a **read-only** integration. Safe writes remain PLUGIN-09 with DocumentHub and File Gate.

## Security tests and threats

- T01: wrong vault, wrong plugin/grant, selected note vs folder vs whole-vault, revoked grant after read, switch of current note/context, forgotten/reissued workspace registration.
- T02: absolute/traversal path, Windows drive/ADS/UNC form, device aliases, decomposed Unicode, symlink leaf/parent, NTFS junction, hardlink, root directory replacement, file under edit, bounded content.
- Test matrix: existing Linux Main PR gate and **already existing** Sync pinned-read OS checks (Windows/macOS/Linux) extended with PLUGIN-04 read threat tests; it cancels stale PR runs. Paths unrelated to plugins/sync keep their existing lightweight CI.
- Red-team tests exercise the *actual host API*, not only string parser/permission methods.
- Document all checked SHA/status and residual risk. A green build is not a certification of a guest runtime, OS sandbox, or File Gate writes.

## Important unresolved limitations

- No guest runtime / public extension APIs / panel UI yet; this host adapter cannot be reached by arbitrary guest code.
- Binding now captures a pinned OS file identity without copying file contents; read revalidates that identity and rejects same-path replacement even with identical bytes. A legitimate atomic save replacing an inode must trigger a trusted rebind. Same-ID reuse and external process races remain platform residual risks.
- Normalized casing/collisions across portable filesystems require a complete host-index collision audit. The adapter rejects noncanonical Unicode path spelling and reserved Windows names; Windows file opens remain case-insensitive; an indexed-path collision audit must precede guest RPC exposure. Do not map raw plugin path strings to bind_host_note.
- No cross-process OS mutation lock: this is a read-only bounded preflight. A different process may change file bytes between operations; the adapter compares identity/bytes twice but is not a globally atomic snapshot.
- No exactly-once grants or permissions sync to another device.
- No secret, clipboard, MCP, network or write authority is delegated from these handles.

## Acceptance criteria

A reviewed PR with positive allowed-note fixtures and negative T01/T02 matrix on Linux/Windows/macOS; all required Rust workspace/sync-engine and native frame tests pass on exact PR head; scoped Windows/macOS checks pass. If file identity and collision tests remain incomplete, label the stage partial and create a follow-up instead of calling it release-ready.
