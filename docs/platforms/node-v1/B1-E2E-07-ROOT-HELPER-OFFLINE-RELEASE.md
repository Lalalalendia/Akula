# B1-E2E-07 — offline root-helper signing handoff (HOLD)

**Status: verified data preparation only; not an authorized release, installer or VPS deployment.**

The [integrated B1-E2E-07 plan](B1-E2E-07-INTEGRATED-INSTALL.md)
requires independently trusted publisher Ed25519 authority. The current
client source still has `PRODUCTION_BOOTSTRAP_PUBKEY=None`. GitHub CI
only compiles and uploads `UNSIGNED-NOT-FOR-INSTALL` candidates; the CI
must never have a production signing private key or sign a release.

This tool completes the **offline reproducible handoff** from actual
candidate ELF to the already-existing B1a canonical signing format
and the exact `AKULA_ROOT_STAGE_V1` frame expected by the root-owned
`root_stage_v1.py` HOLD reader. It does not invent a second signature
format or release trust anchor.

## Release operator prerequisites (OUTSIDE this repo)

1. Independently approve the release commit and candidate SHA-256 on a trusted
   workstation. Verify the downloaded ELF against the expected CI commit,
   artifact digest and source review. The GitHub artifact is merely a
   candidate, not release authority.
2. Independently manage a production Ed25519 private signing key **offline**.
   Do not create or store it in GitHub Actions, Notion, GitHub Secrets, CI
   fixtures, the Windows desktop, a VPS, or alongside a candidate binary.
3. Establish the production public-key fingerprint using a second trusted
   channel. The `--trusted-pubkey-hex` flag requires this independently
   reviewed key. A key delivered in the same package as its signature
   is **not** a trust anchor.
4. Select a strictly increasing `release_sequence`, supported
   `major.minor.patch`, and a validity interval no longer than 90 days.
   The monotonic floor, key rotation/revocation and trusted time must be
   administered separately. The CLI cannot create those guarantees.

## Offline example on a trusted, isolated Linux machine

The actual commit/SHA, approved release sequence, publication dates and
Ed25519 key are **operator decisions**, not values supplied by the candidate.
No real key paths or signatures are recorded here.

```sh
# Only prepare canonical bytes; NO SIGNING occurs here.
python3 scripts/server-bootstrap/offline_helper_release.py prepare \
  --elf /trusted/review/akula-root-b1d0-helper \
  --version 0.3.13 \
  --sequence <INDEPENDENTLY_APPROVED_SEQUENCE> \
  --not-before <APPROVED_UNIX_TIME> \
  --expires <APPROVED_UNIX_EXPIRY> \
  --out-manifest /trusted/review/helper.manifest

# Separately, the authorized offline signing operator signs the exact
# helper.manifest (using their existing private key management procedure).
# Example for a trusted offline OpenSSL workstation ONLY:
openssl pkeyutl -sign -rawin \
  -inkey /secure/offline/production-signing-key.pem \
  -in /trusted/review/helper.manifest \
  -out /trusted/review/helper.signature

# Only the independently reviewed *public* key is supplied for verification.
python3 scripts/server-bootstrap/offline_helper_release.py frame \
  --elf /trusted/review/akula-root-b1d0-helper \
  --manifest /trusted/review/helper.manifest \
  --signature /trusted/review/helper.signature \
  --trusted-pubkey-hex <INDEPENDENTLY_APPROVED_64_HEX_ED25519_PUBLIC_KEY> \
  --out-frame /trusted/review/helper.stage-frame
```

The tool refuses an unsupported/changed ELF, a malformed or expired manifest,
wrong SHA-256/size, invalid signer identity, bad Ed25519 signature, wrong
trusted public key, symlink/hardlinked input and existing output file. It
performs the same signature and format check as the original root-stage
reader in a temporary scratch directory before emitting the frame.

The output is a **verified data frame**. It is not a deployable executable,
approved B0 v3 plan, root-owned install transaction, signed Windows release,
or systemd service. Do not send it to a VPS or activate a root helper until
independent production client pinning, root policy/owner approval, direct
administrator installation and full installed-Windows acceptance are proven.

## Automated test boundaries

`test_offline_helper_release.py` generates **ephemeral testing-only**
Ed25519 keys inside temporary directories, uses the system OpenSSL verifier,
and exercises canonical B1a parity with `root_stage_v1.receive_verified`.
These keys and signatures are destroyed with test fixtures and cannot be
used as Akula release identities. The test never opens any real VPS,
`/usr/libexec`, `/opt` or root-owned service and cannot activate the
Install Node button.

**Next independent blockers:** production signer enrollment and audited
publisher key rotation; signed immutable release floor/clock recovery;
explicit admin-only protected fixed-path helper installation; bounded B1d0
privileged IPC commit; systemd/reboot and Windows A→VPS→Windows B acceptance.
