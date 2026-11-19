'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const root = path.resolve(__dirname, '../..');
const sources = {
  nsis: 'scripts/video-tools-01a-isolated-lab.nsi',
  runner: 'scripts/windows-video-tools-01a-isolated-lab.ps1',
  workflow: '.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  release: 'aquilum-app/native/packaging/akula-windows.json',
  allow: 'docs/release/video18b04-five-exe-allowlist.json'
};
const read = name => fs.readFileSync(path.join(root, sources[name]), 'utf8');
const hold = (condition, why) => assert.ok(condition, 'VIDEO_TOOLS_01A_HOLD: ' + why);
const three = ['akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];

function verify(x) {
  const release = JSON.parse(x.release);
  const allow = JSON.parse(x.allow);
  hold(release.productName === 'Akula','stock product identity was replaced');
  assert.deepEqual(release.binaries.map(i => i.path), ['akula-app','shiva-pdf-preview'],
    'VIDEO_TOOLS_01A_HOLD: ordinary signed release must retain exactly two EXE');
  hold(allow.release_admitted === false &&
    allow.installer_signing_authorized === false &&
    allow.stage_binary_distribution_authorized === false &&
    allow.independent_owner_approval_required === true,
    'ephemeral binary provenance cannot authorize production distribution');
  assert.deepEqual(three.map(name => {
    const row = allow.components.filter(item => item.name === name);
    hold(row.length === 1, 'missing pinned individual component ' + name);
    return row[0].name;
  }),three);
  for (const name of three.slice(1)) {
    const item = allow.components.find(row=>row.name===name);
    hold(/^[a-f0-9]{64}$/.test(item.sha256) && item.size_bytes > 1000000,
      'upstream content hash or byte length is missing');
  }
  const nsi = x.nsis;
  const files = [...nsi.matchAll(/^\s*File\s+"\/oname=([^"]+)"\s+"[^"]+"/gm)]
    .map(match => match[1]);
  assert.deepEqual(files, three,'VIDEO_TOOLS_01A_HOLD: NSIS adds non-media payload');
  for (const token of [
    '!ifndef LAB_ID','!ifndef LAB_STAGE','!ifndef LAB_OUTPUT',
    'Name "Akula Video Tools 01a nonrelease laboratory"',
    'InstallDir "$PROGRAMFILES64\\AkulaVideoToolsLab-${LAB_ID}"',
    'RequestExecutionLevel admin','SilentInstall silent',
    'SilentUnInstall silent','${AtLeastWin10}',
    'IfFileExists "$INSTDIR\\*.*" 0 +2',
    'WriteUninstaller "$INSTDIR\\uninstall.exe"',
    'Delete "$INSTDIR\\akula-video-mux.exe"',
    'Delete "$INSTDIR\\akula-yt-dlp.exe"',
    'Delete "$INSTDIR\\akula-quickjs.exe"',
    'RMDir "$INSTDIR"'
  ]) hold(nsi.includes(token), 'isolated NSIS contract missing: '+token);
  hold(!/RMDir\s+\/[rR]|WriteReg|DeleteReg|Exec(?:Shell|Wait)?\b|CreateShortCut|SetShellVarContext|akula-app\.exe|shiva-pdf-preview\.exe/i.test(nsi),
    'lab installer must not mutate stock Akula, registry, shortcuts or arbitrary files');
  const ps = x.runner;
  for (const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    'EXPECTED_HEAD_SHA','git rev-parse HEAD',
    'WindowsBuiltInRole]::Administrator',
    'OSVersion.Version.Major -lt 10',
    "Join-Path $repo 'labs/video-lite/target/release/dash.exe'",
    'Assert-RealFile $source',
    'release_admitted -ne $false',
    'installer_signing_authorized -ne $false',
    'stage_binary_distribution_authorized -ne $false',
    'independent_owner_approval_required -ne $true',
    'Get-FileHash -LiteralPath $source -Algorithm SHA256',
    "'pinned-upstream'", 'size_bytes',
    'AkulaVideoToolsLab-',
    "Join-Path $stage $name",
    'Start-Process -FilePath $installer',
    'Get-Acl -LiteralPath $root',
    'S-1-5-32-545',
    "unrelated-private-note.md",
    'Get-FileHash -LiteralPath $note -Algorithm SHA256',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS',
    'release=0'
  ]) hold(ps.includes(token), 'real protected three-file Windows proof missing '+token);
  hold((ps.match(/Get-FileHash -LiteralPath \\$source -Algorithm SHA256/g) || []).length === 2,
    'both independent upstream pin verification and source digest witness are required');
  hold(ps.includes("$env:EXPECTED_HEAD_SHA -notmatch") &&
    ps.includes("$actualHead -cne $env:EXPECTED_HEAD_SHA"),
    'exact-head guard must both validate and compare the PR revision');
  hold(ps.includes("Set-Content -LiteralPath (Join-Path $root 'unrelated-private-note.md')") &&
    ps.includes("Test-Path -LiteralPath (Join-Path $root 'unrelated-private-note.md')"),
    'uninstall proof must both create and verify independent private note');
  hold(!/gh release create|Publish-Module|Upload-Artifact|workflow_dispatch/.test(ps),
    'no release or publishing is permitted in PR-only lab');
  for (const name of three) hold(ps.includes("'"+name+"'"),'runner missing '+name);
  const wf=x.workflow, trigger=wf.split(/\r?\npermissions:/,1)[0];
  for(const token of [
    "on:\n  pull_request:",
    "      - 'scripts/video-tools-01a-isolated-lab.nsi'",
    "      - 'scripts/windows-video-tools-01a-isolated-lab.ps1'",
    "      - '.github/scripts/video-tools-01a-isolated-nsis.test.cjs'",
    "      - '.github/workflows/akula-video-tools-01a-isolated-lab.yml'",
    "      - 'docs/release/video18b04-five-exe-allowlist.json'",
    'node --test .github/scripts/video-tools-01a-isolated-nsis.test.cjs',
    'needs: contract',
    'github-hosted','--locked',
    '2026.10.26','v0.18.0',
    'choco install nsis --version=3.11.0',
    './scripts/windows-video-tools-01a-isolated-lab.ps1',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS'
  ]) hold(wf.includes(token),'workflow missing exact-head admission or real Windows test '+token);
  hold(trigger.includes("      - 'aquilum-app/native/packaging/akula-windows.json'"),
    'stock package change must retrigger separation audit');
  hold(/permissions:\r?\n  contents: read/.test(wf),
    'workflow must remain read-only');
  hold(!/workflow_dispatch:|^\s+push:|contents:\s*write|environment:\s*akula-release|publish_beta|gh release create/m.test(wf),
    'workflow cannot obtain publisher authority or publish');
}
test('separate protected Video Tools lab installs only three pinned PE',()=>{
  verify(Object.fromEntries(Object.keys(sources).map(key=>[key,read(key)])));
});
test('negative mutations fail on identity, signing HOLD, Windows gate and protected uninstall',()=>{
  const base=Object.fromEntries(Object.keys(sources).map(key=>[key,read(key)]));
  const mutations=[
    ['nsis','RequestExecutionLevel admin','RequestExecutionLevel user'],
    ['nsis','${AtLeastWin10}','${AtLeastWin7}'],
    ['nsis','AkulaVideoToolsLab-${LAB_ID}','Akula'],
    ['nsis','File "/oname=akula-quickjs.exe"','File "/oname=akula-app.exe"'],
    ['nsis','RMDir "$INSTDIR"','RMDir /r "$INSTDIR"'],
    ['runner','EXPECTED_HEAD_SHA','UNVERIFIED_HEAD'],
    ['runner','Get-Acl -LiteralPath $root','Write-Host "skip ACL"'],
    ['runner',"installer_signing_authorized -ne $false","installer_signing_authorized -ne $true"],
    ['runner','Get-FileHash -LiteralPath $source -Algorithm SHA256','Write-Host no-hashes'],
    ['runner','unrelated-private-note.md','nuke-everything.txt'],
    ['runner','VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS','FAKE_PASS'],
    ['workflow','needs: contract','needs: manual'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write'],
    ['workflow','VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS','MISSING_PROOF'],
    ['release','"path": "shiva-pdf-preview"','"path": "akula-video-mux"'],
    ['allow','"release_admitted": false','"release_admitted": true']
  ];
  for(const [name,from,to] of mutations) {
    const changed=base[name].replace(from,to);
    hold(changed!==base[name], 'mutation missed '+name+': '+from);
    assert.throws(()=>verify({...base,[name]:changed}),/VIDEO_TOOLS_01A_HOLD/, name+': '+from);
  }
});
