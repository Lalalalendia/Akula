# ADR-SYNC-038 — Verify a notarized, publisher-pinned macOS DMG before release upload

**Status:** staged in release P0-C5. No Akula Developer ID certificate, Apple
notarization credentials, app signing key or GitHub Release is provisioned
by this change. Protected S3 editing remains HOLD.

## Why P0-C4 is necessary but insufficient

The production user-side shell installer from P0-C4 rejects a downloaded
app unless the source-pinned Developer ID Team, signed bundle ID and macOS
Gatekeeper evaluation agree. Both source literals are deliberately empty,
so that installer currently refuses new remote installations.

However, the manual `release.yml` workflow still built and uploaded
macOS packages after only a generic `codesign --verify --deep --strict`.
That verifies structural signature integrity, **not** the exact Akula
publisher or a notarization ticket. As a result, a packaging workflow
could expose downloadable artifacts that the protected installer would
correctly reject. A manually dispatched release is not itself a trusted
publisher identity.

## Release-side fail-closed gate

The macOS package jobs now call
`scripts/verify-macos-release.sh scripts/install.sh <packaged-DMG>`
**after** cargo-packager produces the DMG and **before** the GUI smoke test
and `actions/upload-artifact`.

The gate:

1. Parses exactly one literal `AKULA_DEVELOPER_TEAM_ID` and one
   `AKULA_BUNDLE_ID` from the checked-out, pinned installer source,
   without `source`, `eval`, environment, API or DMG-supplied fallback.
   Missing/blank/ambiguous pins cause HOLD before any mount or signing work.
2. Requires a regular packaged DMG and validates a stapled Apple
   notarization ticket with `xcrun stapler validate`.
3. Attaches the DMG read-only, requires the expected `Aquilum.app`
   native executable and verifies the actual app's code signature
   using `codesign --verify --deep --strict -R` with exact Apple
   Developer ID Application, Team ID and bundle identifier.
4. Requires `spctl --assess --type execute` acceptance under local macOS
   security policy. Attempts to detach the mounted DMG on all exits.
5. Does not allow artifact upload if any step failed. The existing
   full release source-SHA pin, local-only feature contract and signer
   independent manifest checks remain in effect.

This is a **DMG upload safety check**, not yet a complete release
attestation. The separate `.app.tar.gz` updater payload and
`.app.tar.gz.sig` must also be authenticated, bound to the same reviewed
bundle identity and independently compared with the DMG before any updater
channel may be enabled. The upstream Aquilum signing identity is not
accepted as Akula.

## What CI actually proves

Application PR checks, Ubuntu docs and the real macOS shell CI execute a
test harness with a disposable mock DMG and fake command implementations.
The test first executes the REAL production verifier with blank pins and
requires refusal **before** notarization, mounting or replacing anything.
It then tests a separate temporary source copy with two synthetic static
pin values and mocked `xcrun`, `codesign`, `spctl` and `hdiutil`.

The negative matrix includes missing/ambiguous pins, a symlinked DMG,
missing stapled ticket, another signer and denied Gatekeeper assessment.
These tests prove the scripts' control flow and fail-closed behavior.
They **do not** show that a genuine Akula DMG has been signed or notarized.

The required CI contract also rejects a release workflow which removes,
moves after artifact upload, or redirects this pinned-source verifier.

## External acceptance still needed

- Enroll the authentic Akula Team ID and signed bundle ID in the reviewed
  source and provision the Apple Developer ID Application certificate.
- Sign all nested binary code under hardened runtime, submit the real DMG
  through Apple's notary service, wait for acceptance and staple the ticket.
- Verify the real downloaded/produced DMG on clean macOS with Gatekeeper
  and source-pinned Developer ID identity.
- Independently verify the app updater's tarball contents and minisign
  signature against a reviewed Akula public key. Verify reproducibility
  and content identity of the package, manifest and install update path.
- Test actual first install, update, interrupted update, restore and
  uninstall across both macOS architectures, with preserved Markdown
  vaults and existing history.
- Keep `sync-installed-host` off in standard `--features installed`
  packages until the distinct OS-owned custody and protected writer
  gates are satisfied.

## References

- Apple: https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements
- Apple: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- ADR-SYNC-035, ADR-SYNC-036, ADR-SYNC-037 and Akula PR #287/#297/#307/#315.

Do not equate Apple notarization, publisher pinning, GitHub provenance
or custody admission: they protect different boundaries.
