'use strict';
// VIDEO-TOOLS-01d negative source contracts; genuine ACL mutations run on Windows.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const base = path.resolve(__dirname, '../..');
const sources = {
  guard: 'scripts/video-tools-01d-acl.ps1',
  poison: 'scripts/windows-video-tools-01d-acl-negative.ps1',
  runner: 'scripts/windows-video-tools-01a-isolated-lab.ps1',
  workflow: '.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  packaging: 'aquilum-app/native/packaging/akula-windows.json'
};
const hold = (ok, why) => assert.ok(ok, 'VIDEO_TOOLS_01D_HOLD: ' + why);
function verify(a) {
  const {guard,poison,runner,workflow,packaging} = a;
  const product=JSON.parse(packaging);
  assert.deepEqual(product.binaries.map(x=>x.path),['akula-app','shiva-pdf-preview'],
    'VIDEO_TOOLS_01D_HOLD: separate tools must not change stock release');
  for (const token of [
    'Assert-AkulaVideoToolsLabRoot -Path $Path -LabId $LabId -RequireExists',
    'Get-ChildItem -LiteralPath $Path -Force',
    'akula-video-mux.exe', 'akula-yt-dlp.exe', 'akula-quickjs.exe', 'uninstall.exe',
    'Get-Acl -LiteralPath $target', 'RawSecurityDescriptor', 'DiscretionaryAcl',
    'GetOwner([Security.Principal.SecurityIdentifier])',
    'GetAccessRules(', 'AccessControlType]::Allow', 'PropagationFlags]::InheritOnly',
    'FileSystemRights', 'WriteData', 'AppendData', 'WriteAttributes',
    'WriteExtendedAttributes', 'Delete', 'DeleteSubdirectoriesAndFiles',
    'ChangePermissions', 'TakeOwnership',
    'S-1-5-18','S-1-5-32-544','S-1-5-80-956008885',
    'SID=', 'VIDEO_TOOLS_01D_HOLD: unsafe writable ACL',
    'VIDEO_TOOLS_01D_HOLD: unsafe package owner'
  ]) hold(guard.includes(token), 'missing fail-closed ACL condition ' + token);
  hold(!/\$env:ProgramFiles(?:64)?\b|GetEnvironmentVariable\('ProgramFiles'/.test(guard),
    'ACL admission must not resolve a root through mutable environment');
  for (const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'", "GITHUB_EVENT_NAME -cne 'pull_request'",
    'EXPECTED_HEAD_SHA', '$actualHead -cne $env:EXPECTED_HEAD_SHA',
    'Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId',
    "Set-Acl -LiteralPath $Target -AclObject $poisoned",
    "Set-Acl -LiteralPath $Target -AclObject $saved",
    'S-1-1-0','S-1-5-32-545','S-1-5-11',
    'WriteData','Delete','WriteAttributes','ChangePermissions',
    'VIDEO_TOOLS_01D_ACL_MUTATION_PASS', 'release=0'
  ]) hold(poison.includes(token),'genuine Windows poison/restoration witness missing '+token);
  hold(poison.includes('finally {') && poison.includes('if ($changed)'),
    'the poisoned DACL must be restored even after exceptions');
  for (const token of [
    "scripts/video-tools-01d-acl.ps1",
    "scripts/windows-video-tools-01d-acl-negative.ps1",
    "Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag",
    "VIDEO_TOOLS_01D_ACL_MUTATION_PASS",
    "VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS"
  ]) hold(runner.includes(token),'actual protected NSIS path omitted ACL witness '+token);
  for (const token of [
    "      - 'scripts/video-tools-01d-acl.ps1'",
    "      - 'scripts/windows-video-tools-01d-acl-negative.ps1'",
    "      - '.github/scripts/video-tools-01d-acl.test.cjs'",
    'node --test .github/scripts/video-tools-01d-acl.test.cjs',
    'needs: contract','runs-on: windows-2025',
    'VIDEO_TOOLS_01D_ACL_MUTATION_PASS','VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS',
    'permissions:\n  contents: read','persist-credentials: false'
  ]) hold(workflow.includes(token), 'read-only exact-head Windows CI missing '+token);
  hold(!/workflow_dispatch:|^\s+push:|contents:\s*write|publish_beta|gh release create/m.test(workflow),
    'this PR-only workflow cannot authorize production release');
}
test('installed root and all four files reject unprivileged owners and writable ACLs',()=>{
  verify(Object.fromEntries(Object.entries(sources).map(([name,p])=>[name,fs.readFileSync(path.join(base,p),'utf8')])));
});
test('negative source mutations cannot delete an OS ACL, Windows poison or release hold',()=>{
  const a=Object.fromEntries(Object.entries(sources).map(([name,p])=>[name,fs.readFileSync(path.join(base,p),'utf8')]));
  const mutations=[
    ['guard','GetOwner([Security.Principal.SecurityIdentifier])','GetUnsafeOwner()'],
    ['guard','DiscretionaryAcl','SomeOtherAcl'],
    ['guard','ChangePermissions','ReadPermissions'],
    ['guard','WriteAttributes','ReadAttributes'],
    ['guard','S-1-5-32-544','S-1-5-32-545'],
    ['guard','unsafe writable ACL','safe writable ACL'],
    ['poison','Set-Acl -LiteralPath $Target -AclObject $saved','Write-Host no-restore'],
    ['poison','S-1-5-11','S-1-5-18'],
    ['poison','VIDEO_TOOLS_01D_ACL_MUTATION_PASS','FAKE_PASS'],
    ['runner','Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag','Write-Host skip-acl'],
    ['workflow','node --test .github/scripts/video-tools-01d-acl.test.cjs','echo skip-acl'],
    ['workflow','VIDEO_TOOLS_01D_ACL_MUTATION_PASS','FAKE_PASS'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [name,from,to] of mutations) {
    hold(a[name].includes(from),'mutation anchor missing '+name+': '+from);
    assert.throws(()=>verify({...a,[name]:a[name].replaceAll(from,to)}),/VIDEO_TOOLS_01D_HOLD/);
  }
});
