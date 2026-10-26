'use strict';

// RELEASE-LOCK-01 — fail closed if someone re-enables the inherited Aquilum
// release job or removes its guard before the independent Akula release exists.
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const file = (name) => readFileSync(path.join(root, name), 'utf8');

test('release.yml is a manually-triggered, read-only HOLD with one failing job', () => {
  const text = file('.github/workflows/release.yml');
  assert.match(text, /^name: Akula Release \(HOLD\)$/m);
  assert.match(text, /^on:\s*\n  workflow_dispatch:\s*$/m);
  assert.match(text, /^permissions:\s*\n  contents: read$/m);
  const jobs = text.split(/\njobs:\n/);
  assert.equal(jobs.length, 2, 'release.yml must contain one jobs stanza');
  const names = [...jobs[1].matchAll(/^  ([a-z][a-z0-9-]*):\s*$/gm)].map((match) => match[1]);
  assert.deepEqual(names, ['release-hold'], 'no executable packaging/publishing jobs');
  assert.match(jobs[1], /echo "::error title=Akula release HOLD::/);
  assert.match(jobs[1], /^\s+exit 1\s*$/m, 'manual invocation must always fail');
  assert.doesNotMatch(text, /Aquilum_0\.2\.6|cargo packager|gh release|TAURI_SIGNING|upload-artifact|secrets\./);
});

test('inherited workflow is preserved only as documentation', () => {
  const archive = file('docs/release/legacy-aquilum-release.yml');
  assert.match(archive, /gh release download v0\.2\.6 -R Freaction\/Aquilum/);
  assert.match(archive, /^  macos:/m);
  assert.match(archive, /^  linux:/m);
  assert.match(archive, /^  manifest:/m);
  assert.doesNotMatch(file('.github/workflows/release.yml'), /Aquilum_0\.2\.6/);
});

test('safe standalone Windows beta remains separate; updater remains off', () => {
  const beta = file('.github/workflows/windows-beta.yml');
  const updater = file('aquilum-app/native/src/updater/release.rs');
  assert.match(beta, /^name: Akula Windows beta candidate$/m);
  assert.match(beta, /--features installed/);
  assert.match(beta, /scripts\/test-windows-beta\.ps1/);
  assert.doesNotMatch(beta, /sync-installed-host/);
  const pinned = updater.match(/pub\(super\) const PUBLIC_KEY: &str = "([^"]+)";/);
  assert.ok(pinned, 'Akula must have an explicit pinned public key');
  // Exact key bytes, not merely the human-readable comment, are trust-critical.
  assert.equal(pinned[1], 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ3QTY2QjZENDA0NkY2QzUKUldURjlrWkFiV3VtMXh2SFFsdEljaUw0T0pUakVmd1krZG4wVmJ1REVmaEx1eWcrMFIzTjQzMi8K', 'Akula publisher root changed without a reviewed key rotation');
  const decoded = Buffer.from(pinned[1], 'base64').toString('utf8');
  assert.equal(decoded.split('\n')[0], 'untrusted comment: minisign public key: D7A66B6D4046F6C5');
  assert.match(decoded.split('\n')[1], /^RW[A-Za-z0-9+/]+$/);
  assert.equal(decoded.split('\n').filter(Boolean).length, 2);
  assert.match(updater, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*cfg!\(all\(target_os = "windows", feature = "installed"\)\)\s*\}/);
});

test('this policy runs when the release workflow is edited', () => {
  const ci = file('.github/workflows/release-isolation-policy.yml');
  assert.match(ci, /^name: Akula release isolation policy$/m);
  assert.match(ci, /^  pull_request:$/m);
  assert.match(ci, /\.github\/workflows\/release\.yml/);
  assert.match(ci, /node --test \.github\/scripts\/release-isolation-policy\.test\.cjs/);
});


test('publisher key proof cannot publish, run on PR code or bypass protected environment', () => {
  const yaml = file('.github/workflows/akula-publisher-key-proof.yml');
  assert.match(yaml, /^name: Akula publisher key proof \(no release\)$/m);
  assert.match(yaml, /^on:\n  workflow_dispatch:\s*$/m);
  assert.match(yaml, /^permissions:\n  contents: read$/m);
  assert.match(yaml, /if: github.ref == 'refs\/heads\/main'/);
  assert.match(yaml, /environment: akula-release/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD/);
  assert.match(yaml, /minisign -Vm /);
  assert.match(yaml, /tamper_refused=1 no_release=1/);
  assert.doesNotMatch(yaml, /gh release|create-release|contents:\s*write|upload-artifact|pull_request:|push:/);
});


test('signed Windows beta may publish only after manually approved independent verification', () => {
  const yaml = file('.github/workflows/akula-windows-signed-candidate.yml');
  assert.match(yaml, /^name: Akula Windows signed candidate \(review only\)$/m);
  assert.ok(
    yaml.includes("run-name: Akula Windows v${{ inputs.release_version }} - ${{ inputs.publish_beta && 'PUBLISH' || 'VERIFY' }}"),
    'Actions runs must show the version and publication intent',
  );
  assert.match(yaml, /^on:\n  workflow_dispatch:\n    inputs:\n      release_version:/m);
  assert.match(yaml, /release_version:\n        description:[^\n]+\n        type: string\n        required: true/);
  assert.match(yaml, /AKULA_REQUESTED_VERSION: \$\{\{ inputs\.release_version \}\}/);
  assert.ok(yaml.includes('Workflow display version $requested does not match the native Akula Cargo version'),
    'mismatched run-name versions must fail before building');
  assert.match(yaml, /m\.version!==process\.env\.AKULA_REQUESTED_VERSION/);
  assert.match(yaml, /publish_beta:\n        description:[^\n]+\n        type: boolean\n        required: true\n        default: false/);
  assert.match(yaml, /^permissions:\n  contents: read$/m);
  assert.match(yaml, /if: github.ref == 'refs\/heads\/main'/);
  assert.match(yaml, /environment: akula-release/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY/);
  assert.match(yaml, /secrets\.CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD/);
  assert.match(yaml, /test-windows-beta\.ps1/);
  assert.match(yaml, /node scripts\/akula-version\.cjs check/);
  assert.match(yaml, /schema = 1/);
  assert.match(yaml, /artifacts = \[ordered\]@\{/);
  assert.match(yaml, /filename = \$package\[0\]\.Name/);
  assert.doesNotMatch(yaml, /platforms = \[ordered\]@\{/);
  assert.match(yaml, /signed_manifest=1 independent_verify=1 tamper_refused=1 no_release=1/);
  assert.doesNotMatch(yaml, /pull_request:|push:/);

  const segments = yaml.split('\n  publish-windows-beta:\n');
  assert.equal(segments.length, 2, 'optional publishing must be a separate job');
  const [build, publish] = segments;
  assert.doesNotMatch(build, /gh release create|contents:\s*write/,
    'signing and verification jobs must remain read-only');
  assert.match(publish, /needs: independent-verification/);
  assert.match(publish, /inputs\.publish_beta == true/);
  assert.match(publish, /github\.ref == 'refs\/heads\/main'/);
  assert.match(publish, /environment: akula-release/);
  assert.match(publish, /permissions:\n      contents: write/);
  assert.match(publish, /minisign -Vm/);
  assert.match(publish, /m\.source_commit!==process\.env\.GITHUB_SHA/);
  assert.match(publish, /if gh release view/);
  assert.match(publish, /git ls-remote --exit-code --tags/);
  assert.match(publish, /gh api .*releases\/latest/);
  assert.match(publish, /node scripts\/akula-version\.cjs assert-newer/);
  assert.match(publish, /releases\/generate-notes/);
  assert.match(publish, /--notes-file/);
  assert.match(publish, /gh release create/);
  assert.match(publish, /--latest/);
});

test('every next signed Windows release verifies actual published Akula NSIS upgrade', () => {
  const wf = file('.github/workflows/akula-windows-signed-candidate.yml');
  const script = file('scripts/test-windows-signed-next-version-upgrade.ps1');
  const build = wf.split('\n  independent-verification:\n')[0];
  assert.match(build, /Test actual latest published Akula to next signed NSIS upgrade/);
  assert.match(build, /run: \.\/scripts\/test-windows-signed-next-version-upgrade\.ps1/);
  assert.match(build, /AKULA_SIGNED_NEXT_CANDIDATE_DISPOSABLE: '1'/);
  assert.doesNotMatch(build, /inputs\.release_version == '0\.3\.14'/);
  assert.match(script, /RUNNER_ENVIRONMENT -cne 'github-hosted'/);
  assert.match(script, /GITHUB_REF -cne 'refs\/heads\/main'/);
  assert.match(script, /AKULA_SIGNED_NEXT_CANDIDATE_DISPOSABLE -cne '1'/);
  assert.match(script, /releases\/latest/);
  assert.match(script, /scripts\/akula-version\.cjs assert-newer/);
  assert.match(script, /browser_download_url/);
  assert.match(script, /Get-FileHash/);
  assert.match(script, /'\/S','\/R','\/UPDATE','\/ARGS'/);
  assert.match(script, /Assert-Preserved/);
  assert.match(script, /shiva-pdf-preview\.exe/);
  assert.match(script, /AKULA_SIGNED_NEXT_UPGRADE_PASS/);
});


test('one-shot owner-requested Akula 0.3.15 dispatch keeps publisher environment isolated', () => {
  const dispatcher = file('.github/workflows/akula-v0315-owner-dispatch.yml');
  const request = JSON.parse(file('docs/release/akula-v0.3.15-dispatch-request.json'));
  assert.deepEqual(request, {
    schema: 1,
    request_id: 'akula-0315-owner-20261026',
    release_version: '0.3.15',
    publish_beta: true,
  });
  assert.match(dispatcher, /^on:\n  push:\n    branches: \[main\]\n    paths:/m);
  assert.match(dispatcher, /docs\/release\/akula-v0\.3\.15-dispatch-request\.json/);
  assert.match(dispatcher, /actions: write/);
  assert.match(dispatcher, /github\.repository == 'Lalalalendia\/Akula'/);
  assert.match(dispatcher, /'akula-0315-owner-20261026'/);
  assert.match(dispatcher, /latest.*v0\.3\.14|latest.*v0\.3\.15/s);
  assert.match(dispatcher, /actions\/workflows\/akula-windows-signed-candidate\.yml\/dispatches/);
  assert.match(dispatcher, /'inputs\[release_version\]=0\.3\.15'/);
  assert.match(dispatcher, /'inputs\[publish_beta\]=true'/);
  assert.match(dispatcher, /environment: akula-release/); // check protected publisher in source verification step
  assert.doesNotMatch(dispatcher, /gh release create|cargo packager signer|secrets\./);
});

// One signed Akula update is installed over the previous version. S3 is a
// user-invoked feature of that same executable, not a second release channel.
test('single published Akula installer includes opt-in S3 feature', () => {
  const signed = file('.github/workflows/akula-windows-signed-candidate.yml');
  const active = signed.split('\n').filter(line => !/^\s*#/.test(line)).join('\n');
  assert.match(active, /scripts\/akula-win7-signed-stage\.ps1/);
  assert.match(file('scripts/akula-win7-signed-stage.ps1'),
    /cargo \+nightly build[^\n]*--features installed,s3-pair-beta[^\n]*--target \$target[^\n]*-Z build-std=std,panic_abort/,
    'the one signed EXE must include opt-in S3 and use the audited Windows 7 target');
  assert.match(file('scripts/akula-win7-signed-stage.ps1'),
    /\$env:RUSTFLAGS = '--cfg windows_slim_errors -C target-feature=\+crt-static'/,
    'the protected Win7 signed candidate must not require a separately installed UCRT/VC runtime');
  assert.match(file('scripts/akula-win7-signed-stage.ps1'),
    /external_crt_imported_dlls/,
    'the signed candidate must reject dynamic UCRT/VC runtime imports');

  assert.match(active, /scripts\/test-windows-beta\.ps1/);
  assert.match(active, /--title "Akula v\$\{version\}" --latest/);
  assert.doesNotMatch(active, /--features installed,s3-pair-beta\s+--bin\s+different-app/);
});
