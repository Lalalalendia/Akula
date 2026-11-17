'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const root = path.resolve(__dirname, '../..');
const read = p => fs.readFileSync(path.join(root, p), 'utf8');
const PS = 'scripts/windows-video18b-protected-root-lab.ps1';
const WF = '.github/workflows/akula-video18b-protected-root-lab.yml';

function verifyLab(source, workflow) {
  for (const proof of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "RUNNER_OS -cne 'Windows'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    'EXPECTED_HEAD_SHA',
    "Join-Path $programFiles ('AkulaVideo18bLab-'",
    "'*S-1-5-32-545:(OI)(CI)RX'",
    "'*S-1-5-32-544:(OI)(CI)F'",
    "'*S-1-5-18:(OI)(CI)F'",
    'New-LocalUser -Name $accountName',
    'Remove-LocalUser -Name $accountName',
    'WindowsIdentity.RunImpersonated<string>',
    'WindowsBuiltInRole.Administrator',
    'ExpectDenied("overwrite:"',
    'ExpectDenied("delete"',
    'ExpectDenied("rename"',
    'ExpectDenied("inject helper"',
    'ExpectDenied("replace directory"',
    'ExpectDenied("create junction staging directory"',
    'local_media_writable=1',
    'Get-FileHash -LiteralPath $file -Algorithm SHA256',
    'Remove-Item -LiteralPath $root -Recurse',
    'VIDEO18B_PROTECTED_ROOT_LAB_PASS'
  ]) {
    assert.ok(source.includes(proof), 'VIDEO18B_HOLD: missing protected lab safety witness: ' + proof);
  }
  for (const forbidden of ['akula-windows-video17-lab.json', 'akula-windows.json', 'cargo packager', 'ShellExecuteW', 'publish_beta=true']) {
    assert.ok(!source.includes(forbidden), 'VIDEO18B_HOLD: script must not package/publish anything: ' + forbidden);
  }
  const header = workflow.split(/\r?\npermissions:/, 1)[0];
  assert.match(header, /^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  assert.ok(header.includes("      - '" + PS + "'"), 'VIDEO18B_HOLD: no source subscription');
  assert.ok(header.includes("      - '" + WF + "'"), 'VIDEO18B_HOLD: no workflow subscription');
  assert.match(workflow, /runs-on: windows-2022/, 'VIDEO18B_HOLD: real Windows runner required');
  assert.match(workflow, /persist-credentials: false/, 'VIDEO18B_HOLD: repository credentials must not persist');
  assert.match(workflow, /windows-video18b-protected-root-lab\.ps1/, 'VIDEO18B_HOLD: real ACL lab must run');
}

test('VIDEO18B protected lab is isolated, credential-free and exercises a real standard user', () => {
  verifyLab(read(PS), read(WF));
});
test('VIDEO18B changing ACL to allow user write must be rejected', () => {
  const before = read(PS);
  const weaker = before.replace("'*S-1-5-32-545:(OI)(CI)RX'", "'*S-1-5-32-545:(OI)(CI)M'");
  assert.notEqual(weaker, before);
  assert.throws(() => verifyLab(weaker, read(WF)), /VIDEO18B_HOLD/);
});
test('VIDEO18B removing user impersonation must be rejected', () => {
  const before = read(PS);
  const weaker = before.replace('WindowsIdentity.RunImpersonated<string>', 'UntrustedOperation');
  assert.notEqual(weaker, before);
  assert.throws(() => verifyLab(weaker, read(WF)), /VIDEO18B_HOLD/);
});
test('VIDEO18B losing pull-request source-trigger coverage must be rejected', () => {
  const wf = read(WF);
  const weaker = wf.replace("      - '" + PS + "'\n", '');
  assert.notEqual(weaker, wf);
  assert.throws(() => verifyLab(read(PS), weaker), /VIDEO18B_HOLD/);
});
