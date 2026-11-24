# AK-REUSE-01 — Doppelscan advisory audit

Purpose: discover candidates for reusing Akula's own Rust code, following the measured TOR Doppelscan 0.9.0 integration. This complements, not replaces, the existing architecture budget, no-growth ratchet, security watchdog, and Main PR gate.

## Run and evidence

No additional workflow file. Use existing **Branch hygiene** (weekly scheduled audit or manual audit mode on main). The isolated advisory job also runs on same-repository PRs that touch Branch hygiene's scoped tooling paths, including the two Doppelscan scripts. It never runs on pull_request_target or in the write-authorized pruning job. Ordinary source PRs do not trigger it. No automatic clone threshold, no doppelscan check, no rewriting.

The job verifies the exact checked-out commit, snapshots regular tracked Rust blobs from that SHA, downloads the version/checksum pinned Linux analyzer used by TOR, and runs it in an offline, bounded, read-only non-root Docker container. The source and tool are the only read-only mounts; no Git metadata, credentials, user vault, or network are available inside the scanner. The Ubuntu image ID is recorded; the base image tag is not a fixed digest. CI logs contain counts and up to five candidate groups per mode with bounded source locations. The artifact (7 days, repository-accessible) retains the input SHA-256 manifest, raw JSON reports, stderr, timing/status and summary; raw tool reports may contain code-derived values. It never contains the source snapshot or the downloaded executable. Failed scan/validation remains visibly failed, not an empty success.

## Scope and interpretation

- First-party aquilum-app/core/src, aquilum-app/native/src, and modules/*/src (including standalone module implementations, some experimental).
- Exclude directories vendor/generated/fixtures/tests/test/probes, standalone test/generated filenames and symlinks. Inline test modules remain included. Other labs, interop, examples and scripts outside these roots are outside this baseline.
- Exact: strict identifiers/literals, minimum 50 syntax tokens.
- Renamed: normalized structural matches, minimum 50 tokens.
- Near: 0.8 similarity, minimum 50 tokens.
- Record any scanner exclusions relative to the selected Git manifest; don't equate selected files with scanned files if they differ. Counts overlap; groups do not measure a percentage of useless code.

## Admission and follow-up

Review the first real scan on a pinned SHA before choosing refactor targets. Prioritize repeated production helpers with identical semantics. Independently validate security/access checks, storage ownership, cancellation/recovery and UI behavior. Intentional separate negative tests, compatibility code and variant policies are not automatically redundant. Extract a common helper only with an independently reviewed behavioral contract and relevant regression tests. Rebaseline using identical settings/scope; do not force a numerical duplicate-count target.

No runtime changes, dependency updates, release or deployment in this PR. Keep advisory audit independent from the existing security remediation.
