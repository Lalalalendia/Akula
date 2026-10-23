# AKULA-MEDIA-LAB-17 — Rust Artifact DAG contract

This is a **standalone, non-shipping Rust crate** matching the existing Akula
`blake3` / `rusqlite` / `serde` dependency versions. It does not change
`aquilum-app/Cargo.toml`, the production Sync DAG, the search worker, or CI.
It does **not** decode video, transcribe Japanese, translate, or enforce real
Akula File Gate rights. Its sample ASR and translation outputs are explicitly
labeled mock data. The `register_source` fence is lab metadata, not trusted
admission; production must inject verified immutable source snapshots and
revalidate access at read, publication, and delivery.

## Key model

- **SourceRef:** scope/source/revision/generation/content digest, independent of file path.
- **AudioBinding:** container-local track index, signed start PTS and rational
  time base mapping a canonical PCM digest into source presentation time.
- **StageRequest:** kind, code/model/parameters/schema, *role-sorted* content
  digests of inputs, plus scope as an isolation boundary.
- **StageOutput:** immutable actual BLAKE3 digest of output bytes, separate from
  BLAKE3 stage computation key. If another ASR version returns identical bytes,
  a translator depending on the same content digest is not recalculated.
- **ArtifactStore:** small SQLite source registration, bindings, outputs and
  exact parent-role edges; immediate transactions, generation/revoke fences,
  idempotent publication and fail-closed divergence under the same stage key.

## Reproduction

Requires Rust with Cargo plus standard SQLite C compiler/toolchain for
`rusqlite(bundled)`:

```bash
cargo test --manifest-path modules/media-artifact-lab/Cargo.toml
```

This crate's Rust tests use a **frozen text manifest** from LAB-16. They do
not rerun FFmpeg and cannot independently prove the manifest's correctness.
The actual AAC/PCM and source timeline equivalence was verified in the
previous LAB-16 (`AKULA-MEDIA-LAB-16.zip`).

## Integration boundaries

Do **not** merge the lab into Akula's production Core yet. The next deliberate
integration should adapt typed SourceRef to existing `DocumentId`, `RevisionId`
and source-read gate, reuse the opt-in `semantic_jobs.rs` lease/fencing ideas
rather than create another scheduler, and bind transactions to the host's
real rights epoch. An independent StageRequest must encode all context it
actually consumes; ASR based solely on PCM can be reused across remuxes,
while scene-conditioned ASR must explicitly depend on that visual context.

### Known non-goals / blockers

- No executor/daemon, no remote API, no background queue, no UI.
- No physical WAV/PNG cache or TTL; no actual audio parsing or model inference.
- No cross-process atomic authorization with an external File Gate database:
  the lab fence protects **only** its own SQLite registration state. It must
  not be used as a production permissions authority.
- No stable Unicode normalization for arbitrary model output: production
  needs a versioned *semantic* transcript projection for early cutoffs.
- No Rust build has been run in the creating environment: neither `rustc`
  nor `cargo` was installed, and the container could not resolve external
  hosts. Test suite results remain **unverified** until CI or a Rust host runs
  the command above. Python/SQLite manifest checks are separate.
