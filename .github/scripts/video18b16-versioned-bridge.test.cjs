'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../..');
const file={
 nsi:'scripts/video18b16-protected-versioned-bridge.nsi',
 ps:'scripts/windows-video18b16-protected-versioned-bridge.ps1',
 wf:'.github/workflows/akula-video18b16-versioned-bridge.yml',
 test:'.github/scripts/video18b16-versioned-bridge.test.cjs',
 doc:'docs/release/video18b16-protected-generation-layout.md',
 legacy:'scripts/video18b-protected-five-real.nsi',
 custody:'scripts/windows-video18b04-real-five-custody.ps1',
 allowlist:'docs/release/video18b04-five-exe-allowlist.json',
};
const names=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
function verify(nsi,ps,wf){
 const header=wf.split(/\r?\npermissions:/,1)[0];
 assert.match(header,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
 for(const p of Object.values(file))assert.ok(header.includes("      - '"+p+"'"),'VIDEO18B16_HOLD trigger: '+p);
 for(const term of [
   'RequestExecutionLevel admin','AkulaVideo18bRealNsisLab-${LAB_ID}',
   'SetOutPath "$INSTDIR\\current"','WriteUninstaller "$INSTDIR\\uninstall.exe"',
   'IfFileExists "$INSTDIR\\akula-app.exe"','Section "Uninstall"'
 ])assert.ok(nsi.includes(term),'VIDEO18B16_HOLD NSIS: '+term);
 for(const n of names){
   assert.ok(nsi.includes('File "/oname='+n+'" "${LAB_STAGE}\\'+n+'"'),'VIDEO18B16_HOLD current missing '+n);
   assert.ok(nsi.includes('Delete "$INSTDIR\\current\\'+n+'"'),'VIDEO18B16_HOLD current uninstall missing '+n);
   assert.ok(nsi.includes('Delete "$INSTDIR\\'+n+'"'),'VIDEO18B16_HOLD legacy uninstall missing '+n);
 }
 assert.match(nsi,/RMDir "\$INSTDIR\\current"/);
 assert.match(nsi,/RMDir "\$INSTDIR"/);
 for(const term of [
   'EXPECTED_HEAD_SHA',"'github-hosted'",'IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)',
   'Video18b16Stage-','Video18b16Media-','AkulaVideo18bRealNsisLab-',
   'Assert-Five $current','Assert-Flat $root',
   'VIDEO18B16_TORN_CURRENT_REJECTED_PASS',
   'WindowsIdentity.RunImpersonated','LogonUser','nonadmin-local-media.txt',
   'Get-FileHash -LiteralPath $note','installer_signing_authorized -ne $false',
   'stage_binary_distribution_authorized -ne $false',
   'VIDEO18B16_PROTECTED_VERSIONED_BRIDGE_PASS'
 ])assert.ok(ps.includes(term),'VIDEO18B16_HOLD Windows proof: '+term);
 assert.match(ps,/if \(\$gone\.ExitCode -ne 0 -or \(Test-Path -LiteralPath \$root\)\)/);
 for(const term of [
   'runs-on: windows-2022','choco install nsis --version=3.11.0',
   'video18b16-protected-versioned-bridge.ps1',
   'windows-video18b04-real-five-custody.ps1',
   'repository: Lalalalendia/shiva','ref: cb542d30d2c18a18b1ee457f5158d5b0f551757e',
   'cargo build --release --locked','persist-credentials: false','permissions:\n  contents: read'
 ])assert.ok(wf.includes(term),'VIDEO18B16_HOLD workflow: '+term);
 assert.ok(!wf.includes('workflow_dispatch:'),'VIDEO18B16_HOLD dispatch');
 assert.ok(!/contents:\s*write|gh release create|publish_beta|cargo packager/i.test(wf),'VIDEO18B16_HOLD release');
 assert.ok(!/WriteRegStr|CreateShortCut|ExecShell|RegDLL|System::Call/i.test(nsi),'VIDEO18B16_HOLD shipping installer identity');
}
test('VIDEO18B16 dual protected five-file NSIS contract',()=>verify(read(file.nsi),read(file.ps),read(file.wf)));
test('VIDEO18B16 negative incomplete uninstall and role weakening',()=>{
 const nsi=read(file.nsi),ps=read(file.ps),wf=read(file.wf);
 for(const term of ['Delete "$INSTDIR\\current\\akula-quickjs.exe"',
   'Delete "$INSTDIR\\akula-app.exe"','RequestExecutionLevel admin']){
   const bad=nsi.replace(term,'REMOVED_BY_NEGATIVE_TEST');assert.notEqual(bad,nsi);
   assert.throws(()=>verify(bad,ps,wf),/VIDEO18B16_HOLD/);
 }
 const noUser=ps.replace('WindowsIdentity.RunImpersonated','WindowsIdentity.NoImpersonation');
 assert.notEqual(noUser,ps);
 assert.throws(()=>verify(nsi,noUser,wf),/VIDEO18B16_HOLD/);
});
test('VIDEO18B16 negative source and publication changes',()=>{
 const nsi=read(file.nsi),ps=read(file.ps),wf=read(file.wf);
 const noTrigger=wf.replace("      - '"+file.nsi+"'\n",'');
 assert.notEqual(noTrigger,wf);
 assert.throws(()=>verify(nsi,ps,noTrigger),/VIDEO18B16_HOLD/);
 const unpin=wf.replace('ref: cb542d30d2c18a18b1ee457f5158d5b0f551757e','ref: main');
 assert.notEqual(unpin,wf);
 assert.throws(()=>verify(nsi,ps,unpin),/VIDEO18B16_HOLD/);
 assert.throws(()=>verify(nsi,ps,wf+'\nworkflow_dispatch:\n'),/VIDEO18B16_HOLD/);
});
