'use strict';
// VIDEO-TOOLS-01p: a genuinely signed-MANIFEST six-file package is probed
// by a distinct non-admin Windows SID, without enabling publisher admission.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const paths={
  user:'scripts/windows-video-tools-01f-standard-user.ps1',
  signed:'scripts/windows-video-tools-01j-six-signed-lab.ps1',
  wf:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  trust:'aquilum-app/native/src/video/tools_package.rs',
  queue:'aquilum-app/native/src/video/queue.rs',
  stock:'aquilum-app/native/packaging/akula-windows.json',
  doc:'docs/release/video-tools-01p-signed-six-nonadmin.md'
};
const read=()=>Object.fromEntries(Object.entries(paths).map(([k,v])=>
  [k,fs.readFileSync(path.join(root,v),'utf8')]));
const hold=(v,why)=>assert.ok(v,'VIDEO_TOOLS_01P_HOLD: '+why);
function verify(s){
  hold(JSON.stringify(JSON.parse(s.stock).binaries.map(x=>x.path))===
    JSON.stringify(['akula-app','shiva-pdf-preview']),
    'stock installer must remain two EXE');
  hold(s.trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";') &&
    s.trust.includes('false // a future, separately reviewed release gate must replace this constant'),
    'production publisher key or release permission opened');
  hold(s.queue.includes('if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))'),
    'installed Video Tools queue is no longer fail-closed');
  for(const token of [
    '[switch]$SignedSix',
    '$expectedRoot=Join-Path (Get-AkulaVideoToolsOsProgramFiles)',
    'Assert-AkulaVideoToolsNoReparseParents -Path $Root -RequireExists',
    '$items.Count -ne 6',
    "manifest.json','manifest.minisig','uninstall.exe'",
    '$signedSix=[bool]::Parse',
    "WindowsIdentity]::GetCurrent()",
    'WindowsBuiltInRole]::Administrator',
    'Start-Process -FilePath $pwsh -Credential $credential',
    'if($SignedSix) {',
    'Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256',
    "VIDEO_TOOLS_01P_CHILD_PASS standard_token=1 signed_six_read=6 denied_rights=24",
    "VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS token=standard_user signed_six_read=6 denied_rights=24 six_sha_unchanged=1 release=0",
    'if($denied -ne 24)',
    'if($code -ne 5)',
    'Remove-LocalUser -Name $user'
  ])hold(s.user.includes(token),'genuine standard-user six-file access proof missing '+token);
  hold(s.user.includes('foreach($access in @([uint32]0x2,[uint32]0x10000,[uint32]0x40000,[uint32]0x80000))'),
    'write/delete/DAC/owner Win32 rights must all be denied for each signed file');
  for(const token of [
    "Run-Native -Mode 'valid' -Public $public",
    "Join-Path $repo 'scripts/windows-video-tools-01f-standard-user.ps1'",
    '-SignedSix 2>&1',
    "VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS token=standard_user signed_six_read=6 denied_rights=24 six_sha_unchanged=1 release=0",
    'Assert-Exact-Six',
    'VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS'+String.fromCharCode(96)+'n'
  ])hold(s.signed.includes(token),'signed native witness integration missing '+token);
  const install=s.signed.indexOf('$installed=$true');
  const parent=s.signed.indexOf('-SignedSix 2>&1');
  const reverify=s.signed.indexOf("Run-Native -Mode 'valid' -Public $public",parent);
  const mutate=s.signed.indexOf("Run-Native -Mode 'wrong-key' -Public $otherPublic");
  hold(install>=0&&install<parent&&parent<reverify&&reverify<mutate,
    'user SID access must occur on fully signed protected package before mutation');
  for(const token of [
    "      - '.github/scripts/video-tools-01p-signed-nonadmin.test.cjs'",
    "      - 'docs/release/video-tools-01p-signed-six-nonadmin.md'",
    'node --test .github/scripts/video-tools-01p-signed-nonadmin.test.cjs',
    'VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS'+String.fromCharCode(96)+'n',
    'runs-on: windows-2025',
    'persist-credentials: false',
    'permissions:\n  contents: read'
  ])hold(s.wf.includes(token),'exact-HEAD Windows CI required proof missing '+token);
  hold(!/workflow_dispatch:|^  push:|contents: write|gh release create/m.test(s.wf),
    'nonrelease workflow may not distribute anything');
  for(const token of [
    'VIDEO-18 RELEASE HOLD','ephemeral','six','normal user','Windows 10/11',
    'signed manifest','unsigned NSIS','four-file'
  ])hold(s.doc.includes(token),'evidence/limitations missing '+token);
}
test('protected six-file signed manifest access denied from true standard Windows SID',()=>verify(read()));
test('reject removal of signed user proof, bytes/ACL safety or release HOLD',()=>{
  const s=read();
  for(const [key,from,to] of [
    ['user','$items.Count -ne 6','$items.Count -ne 7'],
    ['user','if($denied -ne 24)','if($denied -ne 0)'],
    ['user','if($code -ne 5)','if($false)'],
    ['user','-Credential $credential','-Credential $null'],
    ['user','[switch]$SignedSix','[switch]$NonValidated'],
    ['user','Remove-LocalUser -Name $user','Write-Output user-not-deleted'],
    ['signed','-SignedSix 2>&1','-SignedFour 2>&1'],
    ['signed','VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS token=standard_user signed_six_read=6 denied_rights=24 six_sha_unchanged=1 release=0','FAKE_PASS'],
    ['wf','node --test .github/scripts/video-tools-01p-signed-nonadmin.test.cjs','echo skipped'],
    ['wf','permissions:\n  contents: read','permissions:\n  contents: write'],
    ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "CI";'],
    ['queue','if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))','if false']
  ]){
    hold(s[key].includes(from),'negative mutation target missing '+key+': '+from);
    assert.throws(()=>verify({...s,[key]:s[key].replace(from,to)}),/VIDEO_TOOLS_01P_HOLD/);
  }
});
