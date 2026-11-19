'use strict';
// VIDEO-TOOLS-01c: static negative contracts supplement actual hosted-Windows tests.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const base=path.resolve(__dirname,'../..');
const sources={
  guard:'scripts/video-tools-01c-os-known-root.ps1',
  win:'scripts/windows-video-tools-01c-root-lab.ps1',
  installer:'scripts/windows-video-tools-01a-isolated-lab.ps1',
  workflow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml'
};
const read = n => fs.readFileSync(path.join(base,sources[n]),'utf8');
const hold=(ok,msg)=>assert.ok(ok,'VIDEO_TOOLS_01C_HOLD: '+msg);
function check(a) {
  const guard=a.guard,win=a.win,installer=a.installer,wf=a.workflow;
  for(const token of [
    'Is64BitOperatingSystem','Is64BitProcess',
    'OSVersion.Version.Major -lt 10',
    'DllImport("shell32.dll"',
    'SHGetKnownFolderPath(ref Guid id',
    '6D809377-6AF0-444B-8957-A3773F02200E',
    'Marshal.FreeCoTaskMem(ptr)',
    '[IO.Path]::IsPathFullyQualified',
    'Get-AkulaVideoToolsOsProgramFiles',
    'Assert-AkulaVideoToolsNoReparseParents',
    '[IO.FileAttributes]::ReparsePoint',
    "RequireExists",
    '^[a-f0-9]{32}$',
    'outside exact OS-known product root',
    'OrdinalIgnoreCase'
  ])hold(guard.includes(token),'known-folder or reparse gate omitted '+token);
  hold(!/\$env:ProgramFiles(?:\b|64\b)|GetEnvironmentVariable\('ProgramFiles'/i.test(guard),
    'PATH/ProgramFiles environment may not authorize a protected root');
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    'EXPECTED_HEAD_SHA','git rev-parse HEAD',
    'WindowsBuiltInRole]::Administrator',
    'Get-AkulaVideoToolsOsProgramFiles',
    "SetEnvironmentVariable('ProgramFiles'",
    'SPOOF-PROGRAM-FILES',
    'New-Item -ItemType Junction',
    'Assert-AkulaVideoToolsLabRoot -Path $expected -LabId $id -RequireExists',
    'outside exact OS-known product root',
    '[IO.FileAttributes]::ReparsePoint',
    'Remove-Item -LiteralPath $expected -Force',
    'VIDEO_TOOLS_01C_OS_ROOT_PASS'
  ])hold(win.includes(token),'genuine Windows negative test missing '+token);
  for(const token of [
    ". (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')",
    '$program = Get-AkulaVideoToolsOsProgramFiles',
    'Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag',
    'Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS'
  ])hold(installer.includes(token),'actual 01a installer did not adopt OS root or broke witness '+token);
  hold(!/\$program\s*=\s*\[IO\.Path\]::GetFullPath\(\$env:ProgramFiles\)/.test(installer),
    'live installer still trusts spoofed ProgramFiles env');
  for(const token of [
    "      - 'scripts/video-tools-01c-os-known-root.ps1'",
    "      - 'scripts/windows-video-tools-01c-root-lab.ps1'",
    "      - '.github/scripts/video-tools-01c-root.test.cjs'",
    "node --test .github/scripts/video-tools-01c-root.test.cjs",
    'VIDEO_TOOLS_01C_OS_ROOT_PASS',
    'needs: contract',
    'runs-on: windows-2025',
    'permissions:\n  contents: read',
    'persist-credentials: false',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS'
  ])hold(wf.includes(token),'Windows CI root gate missing '+token);
  hold(!/workflow_dispatch:|^\s+push:|contents:\s*write|gh release create|publish_beta/.test(wf),
    '01c must never access publisher release gates');
}
test('protected Video Tools root comes from shell32 known-folder API, not environment',()=>{
  check(Object.fromEntries(Object.keys(sources).map(n=>[n,read(n)])));
});
test('negative mutations reject env-path spoof, deleted junction gate or missing real NSIS binding',()=>{
  const a=Object.fromEntries(Object.keys(sources).map(n=>[n,read(n)]));
  const mutations=[
    ['guard','SHGetKnownFolderPath(ref Guid id','GetEnvironmentVariable(ref Guid id'],
    ['guard','6D809377-6AF0-444B-8957-A3773F02200E','00000000-0000-0000-0000-000000000000'],
    ['guard','Is64BitProcess','REMOVED_BITNESS_GUARD'],
    ['guard','[IO.FileAttributes]::ReparsePoint','[IO.FileAttributes]::Normal'],
    ['guard','Marshal.FreeCoTaskMem(ptr)','// leaked pointer'],
    ['guard','outside exact OS-known product root','let the user choose root'],
    ['win','SPOOF-PROGRAM-FILES','SPOOF-REMOVED'],
    ['win','New-Item -ItemType Junction','New-Item -ItemType Directory'],
    ['win','VIDEO_TOOLS_01C_OS_ROOT_PASS','FAKE_PASS'],
    ['installer','$program = Get-AkulaVideoToolsOsProgramFiles','$program=$env:ProgramFiles'],
    ['installer','Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists','// not verified'],
    ['workflow','needs: contract','needs: manual'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write'],
    ['workflow','VIDEO_TOOLS_01C_OS_ROOT_PASS','NOT_A_PROOF']
  ];
  for(const [name,from,to] of mutations){
    const changed=a[name].replaceAll(from,to);
    hold(changed!==a[name],'mutation not applied '+name+': '+from);
    assert.throws(()=>check({...a,[name]:changed}),/VIDEO_TOOLS_01C_HOLD/);
  }
});
