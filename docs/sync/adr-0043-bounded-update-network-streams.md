# ADR-SYNC-043 — Bounded update manifest and signed asset downloads

**Status:** P0-C6e Draft. Independent from P0-C6d first-render ACK. The
Akula update channel remains intentionally disabled because no Akula
minisign public key or independent release publisher authority has
been provisioned.

## Observed risk

`updater::Updater::download()` formerly trusted the unverified HTTP
`Content-Length` to reserve a `Vec` and read until EOF with no size
budget. A malicious, corrupt, or misconfigured server could request
near-unbounded allocation and stream unlimited data even though the
package would ultimately fail signature verification. Similarly,
`release::find()` decoded unbounded JSON directly from the response
reader before validating the release version and target.

These are latent exposures in a *disabled* release channel and do not
mean that normal local Akula builds currently download such payloads.

## Safeguards

- `download_guard.rs` rejects invalid, zero and greater-than-budget
  `Content-Length`; accepts missing length only if the real stream is
  independently capped.
- The real signed updater payload has a default conservative **512 MiB
  maximum**. Every received chunk is checked against the remaining
  budget *before* appending to memory. Allocation proceeds in small
  bounded increments instead of allocating attacker-reported length.
- A nonempty body is required. A present `Content-Length` must match
  actual bytes; short, long or corrupt responses HOLD.
- Signature validation using the pinned Akula minisign key, when
  provisioned, remains after complete bounded receipt and before
  storing a pending application payload.
- The `latest.json` release manifest is limited to **256 KiB**
  before `serde_json` parsing.
- 14 direct tests cover known and unknown lengths, dishonest length,
  short read, limit boundary, invalid and zero headers, malformed
  manifest, and refusal of excessive JSON.

This size budget is an explicit product policy subject to review
against genuine signed release artifacts. It can be raised in a
reviewed change with evidence; it must not be disabled to "fix" an
unexpectedly large package.

## What this does not prove

This is HTTP memory/resource containment, not authentication of the
manifest or an actual signed updater release. Source URL pinning,
provisioning and auditing the Akula signing key, real Apple Developer
ID/notarization, power-loss tests, and P0-C6d strong post-startup
health/backup release remain separate gates.

Do not activate automatic update checks, delete preserved backup,
change S3 overwrite/delete or baseline custody, or automatically
clear HOLD as part of this PR.
