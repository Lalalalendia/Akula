# Akula B1b4a — read-only owner/CAS correlation, 20 October 2026

Dependent on [B1b3c PR #486](https://github.com/Lalalalendia/Akula/pull/486). Feature `server-bootstrap-b1b4a-cas-fence-lab` is never included in default Core or shipping installers.

The pure Rust gate correlates: real root-UID0 B1a-signed held Node descriptor with metadata and time checks; detached B1b1b owner Ed25519 authorization for exactly one host/InstallId/plan/release digest and nonce; and a separately signed short-lived CAS witness statement covering independent previous/accepted checkpoints, an independent owner-signature digest and fresh challenge. Canonical binary encoding has no command or filename. All comparisons must match, and a second separate readback of the accepted checkpoint is required. Witness claims older than 60 seconds are refused.

CI uses separate synthetic TEST release, owner and witness signing keys, real root UID file-descriptor verification, and negative signatures, host/package/owner-scope mismatch, nonce/challenge mismatch, stale independent state, epoch skipping and expiry.

**NOT PROVED:** independently protected/monotonic CAS witness implementation, challenge uniqueness, trusted origin of supplied before/after checkpoints, actual Windows consent, hostile root, time integrity, real installer, systemd changes, power-failure recovery. Signatures cannot substitute for an independent live CAS and revocation read. InertFenceEvidence is deliberately NOT an installation capability; it always requires fresh CAS under the actual eventual commit transaction lock. No privileged install opcode is exposed. This is a stacked Draft and must not merge until #486 is merged and its own exact-head checks pass.
