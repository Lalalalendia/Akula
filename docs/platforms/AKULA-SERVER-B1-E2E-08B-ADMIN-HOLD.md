# Akula B1-E2E-08b — offline operator helper publication (HOLD)

## Actual capability

After B1-E2E-07 (#747) and B1-E2E-08a (#758) were merged, the feature-gated Rust library can copy an **already independently verified, root-owned, signed helper** from the fixed `bootstrap-v1` stage to the fixed root-only executable path:

- Staging source: `/var/lib/akula/server-bootstrap/bootstrap-v1/helper.staged` (root 0400).
- Destination: `/usr/libexec/akula/akula-root-b1d0-helper` (root 0500).
- Existing destination parent must be root 0700, with root-owned non-symlink ancestors.
- Signed stage and `b0v3.policy` MUST independently agree on the helper SHA-256.
- Publication uses exclusive pending creation, durable file and directory sync, `RENAME_NOREPLACE`, and readback. Failed/partial attempts are **HOLD without automated cleanup**.
- The separate non-shipping binary `akula-root-helper-publish-lab` has **zero arguments** and only calls the fixed-path, root-only implementation.

The destination is an inert on-disk file, **not** an enabled server. Publication cannot execute it, call B1d0 CAS, create an IPC listener, install a Node, invoke `sudo`, start systemd, or contact a VPS. Do not present a successful exit as "Install Node succeeded".

## Production trust: mandatory human-operated release ceremony

**As of 26 October 2026: NO production publisher key has been independently registered and NO production release is accepted.** The compiled Windows bootstrap signer pin is intentionally `None` in `server_first_root_stage.rs`. CI uses only ephemeral test keys.

The following is a protocol for an independently approved operator on trusted, separately controlled machines, not an automated CI job:

1. Freeze the exact source commit and build candidate ELF with the existing unsigned Linux helper workflow. Record the artifact SHA-256, ELF architecture, size and full Git commit. Do not sign a later or differently built artifact under the same receipt.
2. On the separately controlled offline signer, establish and retain the Ed25519 private key outside the repository, GitHub Actions, VPS and chat. Its **public** key fingerprint and raw 32-byte key must be authenticated out of band by the operator. A key copied from a candidate bundle, SSH response or manifest is not independent authority.
3. Generate the canonical, **unsigned** B1a manifest on a trusted machine using `scripts/server-bootstrap/offline_helper_release.py prepare`, with an explicit version, monotonically increasing release sequence and limited validity interval. Compare the manifest's embedded helper hash to the frozen candidate.
4. Sign the *exact manifest bytes* on the offline signer using its hardware-backed or otherwise protected private key, using Ed25519 `pkeyutl -sign -rawin` (or an independently verified equivalent). Export only the detached 64-byte signature; never export the private key.
5. On a different trusted verifier, use `offline_helper_release.py frame` with the ELF, signed manifest, detached signature and the **independently reviewed public key**. The verifier fails closed on a different digest, altered manifest, signature, clock window, ELF architecture or signer identity. Retain the verified frame and separate public-key approval record.
6. Only after review, separately enroll the exact helper digest and owner/release identities in the root-owned B0v3 policy on the intended host and pin the separately reviewed bootstrap signer public key into a signed Windows Akula release. Neither operation is performed by this PR.
7. Independently authenticate the administrator-only CLI artifact before ever running it as UID0. Review the root-owned fixed path chain; the CLI **does not create** `/usr/libexec/akula`. Creating that directory is a separate deliberate administrative operation; refuse symlinks and unexpected ancestor permissions.
8. After the signed stage and root trust exist, a separately authorized administrator may run `akula-root-helper-publish-lab` **without arguments**. The only successful marker is `AKULA_B1D0_ADMIN_HELPER_FILE_PUBLISHED_HOLD`. Re-running the command must refuse; no reset or cleanup is automatic.
9. For acceptance, test root-owned running-image verification and owner IPC preview independently. Keep Node CAS, activation, health/reboot, Windows UI Install Node and actual VPS operations disabled until individually reviewed and proven.

**No signing keys, candidate binary signatures, production key fingerprints or approvals are shipped by this PR.** All actual production registration requires independent operator action and evidence.

## CI proof versus production

The opt-in GitHub Actions gate runs on disposable Ubuntu hosts. It builds the default Core without installer authority, compiles and exercises the root-owned temporary staged-file tests under unprivileged and UID0 identities, builds the admin CLI, and verifies refusal for no trust and arbitrary arguments. It also rechecks that the root-helper runtime refuses `--install`.

These CI tests **do not** prove a trusted operator ceremony, production provenance, real VPS installation, systemd operation or Windows→VPS→Windows readiness.

### Required later gates

1. Independent publisher-key approval and matching signed release artifact (real, non-CI provenance).
2. Administrator verification of the exact helper-install CLI before privileged execution.
3. Owner detached consent + bounded IPC to the existing B1d0 CAS (one-time and durable replay protection).
4. Separate systemd unit setup, restart/health/reboot tests, then live Windows→VPS→Windows acceptance.
