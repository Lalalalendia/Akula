'use strict';
// VIDEO-TOOLS-01f source/mutation contracts. The real Win32 probe runs in CI.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const repo=path.resolve(__dirname,'../..');
const paths={
 user:'scripts/windows-video-tools-01f-standard-user.ps1',
 runner:'scripts/windows-video-tools-01a-isolated-lab.ps1',
 wf:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
 stock:'aquilum-app/native/packaging/akula-windows.json'
};
const source=k=>fs.readFileSync(path.join(repo,paths[k]),'utf8');
const hold=(ok,m)=>assert.ok(ok,'VIDEO_TOOLS_01F_HOLD: '+m);
function verify(a){
  const {user,runner,wf,stock}=a;
  assert.deepEqual(JSON.parse(stock).binaries.map(x=>x.path),['akula-app','shiva-pdf-preview'],
    'VIDEO_TOOLS_01F_HOLD: stock release gained extra EXE');
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    '$head -cne $env:EXPECTED_HEAD_SHA',
    'WindowsBuiltInRole]::Administrator',
    'Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists',
    'Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId',
    'New-LocalUser -Name $user -Password $secret',
    'Get-LocalUser -Name $user -ErrorAction SilentlyContinue',
    'Remove-LocalUser -Name $user',
    'S-1-5-32-545','Get-LocalGroupMember',
    'RandomNumberGenerator]::GetBytes(24)',
    'WindowsIdentity]::GetCurrent()','CreateFileW',
    'GetLastWin32Error()','ERROR_ACCESS_DENIED=5',
    'root_create','root_deletechild','exe_write','exe_attributes',
    'exe_delete','exe_change_acl','exe_take_ownership',
    'Start-Process -FilePath $pwsh -Credential $credential',
    'RedirectStandardOutput $stdout','RedirectStandardError $stderr',
    'VIDEO_TOOLS_01F_CHILD_PASS standard_token=1 read=1 denied_rights=7',
    'Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256',
    'VIDEO_TOOLS_01F_NONADMIN_PASS','restored=1 release=0','finally {','if ($created)'
  ])hold(user.includes(token),'actual Windows non-admin witness missing '+token);
  hold(!/New-LocalUser\s+.*-NoPassword/.test(user),
    'no-password child accounts are disallowed');
  hold(!/Set-Acl -LiteralPath \$Root|Remove-Item -LiteralPath \$Root/.test(user),
    'test must not directly modify protected release bytes or ACL');
  for(const token of [
    "scripts/windows-video-tools-01f-standard-user.ps1",
    'VIDEO_TOOLS_01F_NONADMIN_PASS',
    'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS'
  ])hold(runner.includes(token),'existing protected NSIS run not testing the real child '+token);
  for(const token of [
    "      - 'scripts/windows-video-tools-01f-standard-user.ps1'",
    "      - '.github/scripts/video-tools-01f-standard-user.test.cjs'",
    'node --test .github/scripts/video-tools-01f-standard-user.test.cjs',
    'VIDEO_TOOLS_01F_NONADMIN_PASS','runs-on: windows-2025',
    'needs: contract','permissions:\n  contents: read','persist-credentials: false'
  ])hold(wf.includes(token),'read-only exact-head CI integration missing '+token);
  hold(!/workflow_dispatch:|^\s+push:|contents:\s*write|gh release create|publish_beta/m.test(wf),
    'non-release CI cannot publish binaries or use write token');
}
test('real nonadmin child SID, Win32 rights denial, package hashes and user cleanup enforced',()=>{
  verify(Object.fromEntries(Object.keys(paths).map(k=>[k,source(k)])));
});
test('negative mutations reject skipped OS checks, account cleanup and release hold',()=>{
  const a=Object.fromEntries(Object.keys(paths).map(k=>[k,source(k)]));
  const cases=[
    ['user','New-LocalUser -Name $user -Password $secret','Write-Host fake'],
    ['user','WindowsIdentity]::GetCurrent()','WindowsIdentity]::GetAnonymous()'],
    ['user','GetLastWin32Error()','GetPlausibleError()'],
    ['user','root_create','root_read'],
    ['user','exe_change_acl','exe_read_acl'],
    ['user','Remove-LocalUser -Name $user','Write-Host keep-user'],
    ['user','VIDEO_TOOLS_01F_CHILD_PASS standard_token=1 read=1 denied_rights=7','FAKE_CHILD'],
    ['user','VIDEO_TOOLS_01F_NONADMIN_PASS','FAKE_PASS'],
    ['runner','VIDEO_TOOLS_01F_NONADMIN_PASS','FAKE_PASS'],
    ['wf','node --test .github/scripts/video-tools-01f-standard-user.test.cjs','echo skip'],
    ['wf','VIDEO_TOOLS_01F_NONADMIN_PASS','FAKE_PASS'],
    ['wf','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [part,from,to] of cases){
    hold(a[part].includes(from),'mutation anchor missing '+part+': '+from);
    assert.throws(()=>verify({...a,[part]:a[part].replaceAll(from,to)}),/VIDEO_TOOLS_01F_HOLD/);
  }
});
