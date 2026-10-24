'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.join(__dirname, '../..');
const read = name => fs.readFileSync(path.join(ROOT, name), 'utf8');
const workflow = read('.github/workflows/s3-pair-onepass-ci.yml');
const prSection = workflow.split('  pull_request:\n')[1]?.split('  workflow_dispatch:\n')[0];

test('S3 smoke watches the actual native Windows modal and UIA ownership', () => {
  assert.ok(prSection, 'S3 smoke must subscribe to pull requests');
  const required = [
    '.github/workflows/s3-pair-onepass-ci.yml',
    '.github/scripts/s3-pair-routing.test.cjs',
    'scripts/windows-cloud-provider-uia.ps1',
    'scripts/windows-beta-gui-input.ps1',
    'aquilum-app/native/src/app/settings/cloud_action.rs',
    'aquilum-app/native/src/ui/settings/cloud.rs',
    'aquilum-app/native/src/ui/settings/binding.rs',
    'aquilum-app/native/src/ui/settings/schema/sync/yandex/pairing.rs',
  ];
  for (const file of required) {
    assert.ok(fs.existsSync(path.join(ROOT, file)), 'Missing watched source: ' + file);
    assert.ok(prSection.includes("      - '" + file + "'\n"), 'Unwatched S3 user path: ' + file);
  }
  // No-growth CI fanout: shared settings entrypoints already have their own
  // broad validation routes. This focused S3 lane watches the provider-owned
  // implementation, not every generic settings action or Yandex schema edit.
  for (const common of [
    'aquilum-app/native/src/app/settings/action.rs',
    'aquilum-app/native/src/ui/settings/schema/sync/yandex.rs',
  ]) {
    assert.ok(!prSection.includes("      - '" + common + "'\n"),
      'Do not add a workflow subscriber to common settings code: ' + common);
  }
  assert.doesNotMatch(prSection, /native\/src\/(?:app|ui)\/\*\*/,
    'Do not subscribe the new workflow to broad native paths');
});

test('S3 UIA is still executed against the real opt-in Windows-native build', () => {
  assert.match(workflow, /^  windows-native:\n/m);
  assert.match(workflow, /cargo check --locked -p aquilum-native --features installed,s3-pair-beta/);
  assert.match(workflow, /cargo build --locked -p aquilum-native --features installed,s3-pair-beta/);
  assert.match(workflow, /-File '\.\/scripts\/windows-cloud-provider-uia\.ps1'/);
  const uia = read('scripts/windows-cloud-provider-uia.ps1');
  assert.match(uia, /Bucket-Editor/);
  assert.match(uia, /Cloud bucket label not accessible/);
  assert.match(uia, /\[AkulaBetaGuiInput\]::TypeAsciiKeys/);
  assert.match(uia, /AKULA_CLOUD_SAVE_UIA_PASS/);
  assert.match(uia, /AKULA_CLOUD_DISCARD_UIA_PASS/);
  assert.match(uia, /AKULA_DROPBOX_MODAL_UIA_PASS/);
  // PowerShell 5.1 decodes BOM-less scripts as ANSI on Windows runners.
  assert.doesNotMatch(uia, /[^\x00-\x7F]/, 'UIA script must be ASCII or explicitly UTF-8 BOM');
});

test('PR cancellation is scoped; manually dispatched acceptance is preserved', () => {
  assert.match(workflow, /^  workflow_dispatch:\n/m);
  assert.match(workflow, /^concurrency:\n/m);
  assert.match(workflow, /github\.workflow/);
  assert.match(workflow, /github\.event\.pull_request\.number \|\| github\.ref/);
  assert.match(workflow, /cancel-in-progress: \$\{\{ github\.event_name == 'pull_request' \}\}/);
});
