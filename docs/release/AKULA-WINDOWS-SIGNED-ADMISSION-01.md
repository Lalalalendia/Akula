# RELEASE-WINDOWS-APPLY-HOLD-01 — refuse unsafe automatic Windows install

**Status: DRAFT / FAIL-CLOSED / NO RELEASE.**

This PR isolates an existing unsafe compatibility mechanism. The inherited
Windows `updater::apply(&[u8], before_exit)` wrote a downloaded installer to
a PID-named directory in the ordinary OS temp location and attempted to
launch the installer with `/S /R /UPDATE /ARGS`. It had no durable
prior-generation backup, first-start GUI acknowledgement, or crash rollback.
The disabled `channel_ready()` gate prevented normal users from invoking it,
but an accidental release-channel toggle would have exposed that path.

## What is now enforced

- Windows `apply()` **always returns HOLD**, with an actionable error.
  It does not write an executable, launch a process, end the current app,
  invoke `before_exit`, or modify Akula/Aquilum data.
- A Windows-only Rust test invokes this function with synthetic unsigned bytes,
  confirms `HOLD` and verifies that the pre-exit closure is not called.
- An independent Windows beta Node policy test checks the source boundary
  against regression to `ShellExecuteW`, temp installer writes or process
  exit. The guard accepts both the currently disabled empty publisher key
  and a separately reviewed pinned public key; **key provisioning must not
  enable the release channel**.
- The GitHub release workflow and updater publication channel remain disabled.

## Avoid duplicate signing mechanisms

The complete signed-candidate, version, platform, manifest and SHA-256 checks
are being implemented separately in
[PR #522 — Windows real NSIS upgrade and signed candidate](https://github.com/Lalalalendia/Akula/pull/522).
The independently owner-provided Minisign public key and protected workflow
are handled in
[PR #527 — owner key pinning](https://github.com/Lalalalendia/Akula/pull/527).

Earlier experimental Windows admission work in this branch duplicated #522.
That duplicate implementation and its CI fixture **were reverted** before
acceptance. This PR must remain limited to forbidding unsafe install.

## Still missing before user-facing Windows auto-update

1. The **actual** verified Akula publisher manifest and download must be
   joined with the exact candidate bytes in the updater (no raw `Vec<u8>`
   without a typed proof at the installation boundary).
2. OS-protected staging with a single authoritative lock, package rehash,
   Akula-only identity and verified destination, durable journal and
   protected previous version.
3. Generation-bound first-GUI acknowledgement, reboot/crash detection,
   **automatic** rollback and tamper-resistant persisted version floor.
4. Complete Windows 10/11 installed and physical acceptance, separate
   publisher approval and only then enabling a signed release channel.

The real two-version NSIS laboratory in #522 proves upgrade compatibility
and **manual** restoration only. It is not equivalent to crash-safe
in-application update, and this change does not make it so.

The ordinary update flow must fail safely until the missing guarantees are
implemented and independently verified.

## Reuse baseline: existing rar2 updater, not another engine

**Reuse source (read-only, pinned):**
[`Lalalalendia/rar2@32a803e67244836d2794f9bbbdf9b24deab63d7e`](https://github.com/Lalalalendia/rar2/tree/32a803e67244836d2794f9bbbdf9b24deab63d7e).
Before implementing any new Windows transaction, review and port the appropriate
existing Rust code with an explicit product adapter; do **not** invent a
parallel updater state machine.

| Existing source | Proven responsibility | Adaptation for Akula |
| --- | --- | --- |
| `crates/chaptera-update-engine/` | `UpdatePhase` journal with synced `.next/.prev`, `begin_verified_candidate`, retain `current/` as previous, activation, confirmation, conservative `recover()`; Windows locked-file retry; `transaction_acceptance.rs` tests crash windows | Reuse state-machine/recovery logic, adapt installed paths, journal identity and installer layout |
| `crates/chaptera-update-orchestrator/` | Cross-process OS `fs4` lock, `quiesce`, candidate health check, rollback on failed health; `windows_product_acceptance.rs` exercises an actual installed Reader | Adapt Akula executable/NSIS lifetime and require generation-bound first-GUI ACK rather than treating mere process exit as success |
| `crates/chaptera-update-handoff/` | Runs a copied trusted U1 updater outside the active tree, revalidates hash and handoff request, restricts child environment | Retain this separate trusted control process during NSIS/installer transition; never execute an unauthenticated temp `exe` |
| `crates/chaptera-update-trust/` | Signed TUF repository validation, root/metadata expiry and rollback defence, product/channel/layout policy, authenticated payload receipt | Keep Akula's independently owner-pinned Minisign signature pipeline from #522/#527 unless a **reviewed protocol migration** explicitly changes it. Adapt semantics/receipt at a typed boundary |
| `crates/chaptera-update-ui-state/` | Separate update-state model including no-network default, download/progress, repair and rollback outcomes | Reference state machine only; its existence does not prove it is connected to the Akula UI |

### Compatibility boundaries — tests that must not be conflated

1. **Different installed layouts.** `rar2` Reader's Inno Setup
   [installer](https://github.com/Lalalalendia/rar2/blob/32a803e67244836d2794f9bbbdf9b24deab63d7e/installer/windows/chaptera-reader.iss)
   places the executable in `{app}/current/`, with sibling `.staging/` and
   `.rollback/`. Akula currently builds NSIS and uses its existing
   install location, so copying `UpdateEngine::activate_candidate()` unchanged
   would be unsafe. Either migrate the Akula binary layout with a verified
   installer bridge or provide an explicit NSIS transaction adapter.
2. **Recovery is not a publisher proof.** An unkeyed local journal digest
   helps detect corruption but is not an independent anti-rollback witness
   against hostile modification of the whole install tree. Keep a separately
   verified owner trust root and version floor.
3. **Hosted reboot harness is a self-test.**
   [`chaptera-local-updater-reboot-resume.yml`](https://github.com/Lalalalendia/rar2/blob/32a803e67244836d2794f9bbbdf9b24deab63d7e/.github/workflows/chaptera-local-updater-reboot-resume.yml)
   proves same-boot recovery and guards real product mutation. It
   explicitly does **not** claim a real reboot occurred. The real reboot
   path requires independent operator acknowledgement and physical evidence.
4. **A health smoke is not necessarily a first-render ACK.** The Reader
   `--product-smoke-v1` validates an executable; Akula must still prove
   actual GUI start/first usable frame and bound that acknowledgement to
   the exact candidate generation.
5. **Current sibling PRs:** [#504](https://github.com/Lalalalendia/Akula/pull/504)
   proves large-document GUI persistence, [#522](https://github.com/Lalalalendia/Akula/pull/522)
   proves real NSIS v1-to-v2 and manually controlled v1 recovery,
   [#527](https://github.com/Lalalalendia/Akula/pull/527) pins an Akula
   owner publisher root. This PR only removes a dangerous legacy Windows
   apply shortcut. Do not duplicate the signed-candidate engine in #522.

### Concrete next implementation milestone

In a subsequent **reviewed adapter change**, prove the following with a
disposable installed Akula v1: `signed Akula candidate → isolated staging
→ copied v1 control process → OS lock → retain v1 → activate v2 → bounded
generation-matched GUI ACK → confirm / automatic byte-identical rollback`.
Add a fault matrix for termination after every journal phase and stale ACK,
preserve Akula data/Aquilum identity, and verify a real NSIS-compatible layout
transition separately. Continue fail-closed `HOLD` until those tests and
the publisher/release gates pass.
