# AGENT-CAS-01 — MCP expectedRevision for conditional Markdown edits

## Why
The legacy `update_note` helper retries one File Gate conflict by recomposing the edit against the newer file. That can be convenient for single-agent append workflows, but it is not an agent-visible semantic compare-and-swap contract.

## Opt-in contract
- `read_note(includeRevision=true)` adds `revision`, the 64-character lowercase BLAKE3 hash of **raw bytes of the entire file**, even for a requested heading or truncated fragment. Other reads keep their old response format and avoid hashing.
- `update_note(expectedRevision=<hash>)` rejects a stale revision instead of silently rebasing. The exact raw hash also becomes the atomic File Gate expected hash, detecting intervening changes after the first check.
- A successful conditional write returns the newly published `revision`; a conditional no-op returns the unchanged revision.
- A conflict is an MCP `isError=true` result. Its `content[0].text` holds JSON with `code=MCP_REVISION_CONFLICT`, `expectedRevision`, `actualRevision` when readable, `path`, and `retryable=false`; the caller must re-read and replan, not blindly retry.
- Old calls without `expectedRevision` preserve the existing single-retry behavior. Multi-agent writers should use conditional updates.

## Explicit limits
This is **content-hash CAS**, not document identity or per-agent permissioning. Identical-byte ABA, rename/replacement with identical content, and operations outside `update_note` require further consideration. `set_frontmatter`, history restore, rename/move/delete still lack an equivalent agent-visible expected-revision contract.

The raw-byte hash matters for CRLF: normalized text alone must never be used as the File Gate expected hash. We intentionally avoid extra BLAKE3 work for legacy reads.

## Tests / acceptance
- Real disposable Core and File Gate fixture: agent A writes with current revision; agent B uses stale revision and is rejected with unchanged disk bytes; agent B rereads then succeeds.
- Invalid revision rejected without write.
- CRLF fixture verifies original raw bytes determine the revision.
- Main PR gate including default Rust workspace and sync-engine.
- Future: parallel HTTP clients on the same document, new-document identity/ABA policy, and expected revision support on all mutating tools.

Never claim full 100-agent safe concurrent mutation from this narrow opt-in contract.