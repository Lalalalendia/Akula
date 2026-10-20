# MCP-PATH-01 — symlink and reparse-point preflight

## Threat and confirmed code path

`Vault::any_path` previously joined a relative pathname to the registered workspace root without examining existing filesystem components. `Vault::note` then followed the resulting path via `is_file`; `read_note` ultimately uses `std::fs::read`, which follows filesystem symbolic links. A preexisting `vault/Escape.md -> ../private/secrets.md` could therefore expose a document outside the chosen workspace to an MCP client holding the local token.

The same relative path mechanism feeds some mutating MCP calls. Rejecting links is appropriate even when a link points to an in-vault file because alias paths can bypass path-identity assumptions in File Gate, history and sync.

## What this PR enforces

- For relative note and folder paths, preflight `symlink_metadata` for each existing component of the path, not following the link.
- Recheck the effective `.md` candidate after appending the default extension. Otherwise the final `Escape.md` symlink is missed for argument `Escape`.
- Windows: reject any metadata carrying `FILE_ATTRIBUTE_REPARSE_POINT`, not only recognized symlink kinds; this is deliberately conservative and includes junction/mount-point redirection.
- Fail closed on permission/metadata errors other than `NotFound`; nonexistent children remain creatable in legitimate new folders.
- Reject `..` components in absolute inputs before a canonical-path fallback when parents do not yet exist.
- Unix regression tests for symlinked files, directories, and in-vault aliases; Windows symbolic-directory test when the host permits creating symlinks; ordinary relative Markdown resolution tests must remain green.

## Important: NOT a completed security boundary

This is a defense against **preexisting** redirection and malformed absolute paths. It still checks path components before `File::open` or file mutation; a cooperating or malicious *local* process can potentially swap a directory for a link between the check and use (TOCTOU). The root itself is trusted from the workspace registration machinery; other aliases and code paths need independent audit.

For a strong sandbox/remote-agent promise, design handle-relative, no-follow traversal and file operations, with OS-specific behavior and tests: Unix openat/openat2 path restrictions or equivalent, and Windows handles opened with explicit reparse-point semantics. Include create/rename/delete/history/trash, hard-link policy, mount boundaries and cross-process races. Avoid describing this preflight alone as secure containment or proof of 100-agent readiness.

## Evidence needed before production

- Windows native CI confirms reparse/junction rejection; "could not create symlink" does not count as positive evidence.
- Fuzz relevant path forms and adversarial reparse behavior.
- Verify File Gate with concurrent local directory rename/replacement and crash/restart.
- Audit paths created by direct trash/history helpers that bypass `Vault::note_path`.
