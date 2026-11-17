'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const root = path.resolve(__dirname, '../..');
const read = p => fs.readFileSync(path.join(root, p), 'utf8');
const psPath = 'scripts/windows-video18b03-protected-nsis-lab.ps1';
const nsiPath = 'scripts/video18b-protected-five-mock.nsi';
const wfPath = '.github/workflows/akula-video18b03-protected-nsis-lab.yml';
const self = '.github/scripts/video18b03-protected-nsis-contract.test.cjs';
function validate(ps, nsi, wf) {
  for (const value of [
    'RequestExecutionLevel admin', 'InstallDir "$PROGRAMFILES64\\AkulaVideo18bNsisLab-',
    'StrCpy $INSTDIR "$PROGRAMFILES64\\AkulaVideo18bNsisLab-',
    'WriteUninstaller "$INSTDIR\\uninstall.exe"', 'SilentInstall silent',
    'SilentUnInstall silent', 'Section "Uninstall"', '!ifndef LAB_ID',
  ]) assert.ok(nsi.includes(value), 'VIDEO18B03_HOLD: missing protected NSIS requirement ' + value);
  const names = ['akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs'];
  for (const name of names) {
    assert.ok(nsi.includes('File "/oname=' + name + '.exe"'), 'VIDEO18B03_HOLD: missing mock ' + name);
    assert.ok(nsi.includes('Delete "$INSTDIR\\' + name + '.exe"'), 'VIDEO18B03_HOLD: missing uninstall ' + name);
  }
  assert.ok(!/WriteRegStr|WriteRegDWORD|ExecWait|nsisdl::|inetc::/i.test(nsi),
    'VIDEO18B03_HOLD: forbidden release/update behavior in lab installer');
  for (const value of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'", 'EXPECTED_HEAD_SHA',
    'WindowsIdentity.RunImpersonated<string>', 'New-LocalUser', 'Remove-LocalUser',
    'Get-FileHash', 'blocked_mutations=11 readable_helpers=5 local_media_writable=1',
    'VIDEO18B03_GENUINE_NSIS_INSTALL_PASS', 'VIDEO18B03_GENUINE_NSIS_UNINSTALL_PASS',
    'VIDEO18B03_PROTECTED_NSIS_LAB_PASS',
  ]) assert.ok(ps.includes(value), 'VIDEO18B03_HOLD: missing real Windows test ' + value);
  const header = wf.split(/\r?\npermissions:/,1)[0];
  assert.match(header, /^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for (const name of [psPath,nsiPath,wfPath,self]) {
    assert.ok(header.includes("      - '" + name + "'"), 'VIDEO18B03_HOLD: missing source trigger ' + name);
  }
  assert.match(wf, /concurrency:\s*\n  group:/);
  assert.match(wf, /runs-on: windows-2022/);
  assert.match(wf, /--version=3\.11\.0/);
  assert.match(wf, /persist-credentials: false/);
  assert.match(wf, /EXPECTED_HEAD_SHA:/);
}
test('VIDEO18B03 protects real NSIS install and requires 11 standard-user denials', () =>
  validate(read(psPath),read(nsiPath),read(wfPath)));
test('VIDEO18B03 rejects mutable per-user install root', () => {
  const n = read(nsiPath), weak = n.replaceAll('$PROGRAMFILES64\\AkulaVideo18bNsisLab-','$LOCALAPPDATA\\Akula');
  assert.notEqual(weak,n);
  assert.throws(() => validate(read(psPath),weak,read(wfPath)),/VIDEO18B03_HOLD/);
});
test('VIDEO18B03 rejects unprivileged NSIS', () => {
  const n = read(nsiPath), weak = n.replace('RequestExecutionLevel admin','RequestExecutionLevel user');
  assert.notEqual(weak,n);
  assert.throws(() => validate(read(psPath),weak,read(wfPath)),/VIDEO18B03_HOLD/);
});
test('VIDEO18B03 rejects lost workflow source trigger', () => {
  const w = read(wfPath), weak = w.replace("      - '"+nsiPath+"'\n",'');
  assert.notEqual(weak,w);
  assert.throws(() => validate(read(psPath),read(nsiPath),weak),/VIDEO18B03_HOLD/);
});
