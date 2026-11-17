'use strict';
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const psPath='scripts/windows-video18b11-protected-real-five.ps1';
const nsiPath='scripts/video18b-protected-five-real.nsi';
const wfPath='.github/workflows/akula-video18b11-protected-real-nsis.yml';
const testPath='.github/scripts/video18b11-protected-real-nsis.test.cjs';
const docPath='docs/release/video18b11-protected-real-five-nsis.md';
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const names=['akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs'];
function validate(ps,nsi,wf){
 for(const term of [
   'RequestExecutionLevel admin',
   'InstallDir "$PROGRAMFILES64\\AkulaVideo18bRealNsisLab-',
   'StrCpy $INSTDIR "$PROGRAMFILES64\\AkulaVideo18bRealNsisLab-',
   'WriteUninstaller "$INSTDIR\\uninstall.exe"',
   '!ifndef LAB_STAGE','!ifndef LAB_ID','!ifndef LAB_OUTPUT',
   'SilentInstall silent','SilentUnInstall silent',
 ]) assert.ok(nsi.includes(term),'VIDEO18B11_HOLD: missing protected NSIS boundary '+term);
 for(const n of names){
   assert.ok(nsi.includes('File "/oname='+n+'.exe"'),'VIDEO18B11_HOLD: missing real EXE '+n);
   assert.ok(nsi.includes('Delete "$INSTDIR\\'+n+'.exe"'),'VIDEO18B11_HOLD: missing uninstall '+n);
 }
 assert.ok(!/WriteRegStr|WriteRegDWORD|ExecWait|nsisdl::|inetc::|RequestExecutionLevel user/i.test(nsi),'VIDEO18B11_HOLD: lab cannot mutate product release state');
 for(const term of [
   "RUNNER_ENVIRONMENT -cne 'github-hosted'",
   "GITHUB_EVENT_NAME -cne 'pull_request'",
   'EXPECTED_HEAD_SHA','[IO.File]::OpenRead($source)','ReadByte() -ne 77',
   'Copy-Item -LiteralPath $source -Destination $file',
   'AKULA_BUNDLED_MUX_SHA256','stage does not contain exactly five real EXEs',
   'WindowsIdentity.RunImpersonated<string>',
   'New-LocalUser','Remove-LocalUser','blocked_mutations=11 readable_helpers=5 local_media_writable=1',
   'VIDEO18B11_GENUINE_PROTECTED_REAL_NSIS_INSTALL_PASS',
   'VIDEO18B11_GENUINE_NSIS_UNINSTALL_PASS',
   'VIDEO18B11_PROTECTED_REAL_FIVE_NSIS_PASS',
 ]) assert.ok(ps.includes(term),'VIDEO18B11_HOLD: missing trust/real installation evidence '+term);
 assert.ok(!ps.includes('VIDEO18B11-LAB-ONLY:'),'VIDEO18B11_HOLD: cannot use inert mock files');
 assert.match(ps,/real_exe_sha256=5 signed=0/);
 const hdr=wf.split(/\r?\npermissions:/,1)[0];
 assert.match(hdr,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
 for(const file of [psPath,nsiPath,wfPath,testPath,docPath])assert.ok(hdr.includes("      - '"+file+"'"),'VIDEO18B11_HOLD: lost source trigger '+file);
 assert.match(wf,/permissions:\s*\n  contents: read/);
 assert.match(wf,/persist-credentials: false/);
 assert.match(wf,/runs-on: windows-2022/);
 assert.match(wf,/--version=3\.11\.0/);
 assert.match(wf,/ref: \$\{\{ github\.event\.pull_request\.head\.sha \}\}/);
 assert.match(wf,/EXPECTED_HEAD_SHA:/);
 for(const term of [
   'cargo build --release --locked --manifest-path _shiva-preview/Cargo.toml -p cli',
   'cargo build --release --locked --manifest-path labs/video-lite/Cargo.toml --bin dash',
   'AKULA_BUNDLED_MUX_SHA256',
   '2026.10.26','v0.18.0',
   'cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e',
   'windows-video18b04-real-five-custody.ps1',
   'windows-video18b11-protected-real-five.ps1',
   'Remove-LocalUser',
 ]) assert.ok((wf+'\n'+ps).includes(term),'VIDEO18B11_HOLD: missing pinned real component '+term);
 assert.ok(!wf.includes('workflow_dispatch:'),'VIDEO18B11_HOLD: no manual workflow dispatch');
 assert.ok(!/gh release create|publish_beta\s*:\s*true|cargo packager signer|upload-artifact[\s\S]*\.(?:exe|msi|zip)/i.test(wf),'VIDEO18B11_HOLD: unsigned real EXEs must not be published');
}
test('VIDEO18B11 requires actual protected NSIS with five real executables and 11 attack denials',()=>validate(read(psPath),read(nsiPath),read(wfPath)));
test('VIDEO18B11 rejects unprotected per-user installation',()=>{
 const n=read(nsiPath).replaceAll('$PROGRAMFILES64\\AkulaVideo18bRealNsisLab-','$LOCALAPPDATA\\Akula');
 assert.throws(()=>validate(read(psPath),n,read(wfPath)),/VIDEO18B11_HOLD/);
});
test('VIDEO18B11 rejects unelevated installer and missing helper',()=>{
 const n=read(nsiPath).replace('RequestExecutionLevel admin','RequestExecutionLevel user');
 assert.throws(()=>validate(read(psPath),n,read(wfPath)),/VIDEO18B11_HOLD/);
 const weak=read(nsiPath).replace('File "/oname=akula-quickjs.exe"','File "/oname=unknown.exe"');
 assert.throws(()=>validate(read(psPath),weak,read(wfPath)),/VIDEO18B11_HOLD/);
});
test('VIDEO18B11 refuses replacing real Windows attacker proof with mock',()=>{
 const p=read(psPath).replace('WindowsIdentity.RunImpersonated<string>','// dropped attacker token');
 assert.throws(()=>validate(p,read(nsiPath),read(wfPath)),/VIDEO18B11_HOLD/);
});
test('VIDEO18B11 refuses lost exact-head trigger or release publishing',()=>{
 const wf=read(wfPath);
 const weak=wf.replace("      - '"+psPath+"'\n",'');
 assert.notEqual(weak,wf);
 assert.throws(()=>validate(read(psPath),read(nsiPath),weak),/VIDEO18B11_HOLD/);
 const publish=wf+'\nworkflow_dispatch:\n';
 assert.throws(()=>validate(read(psPath),read(nsiPath),publish),/VIDEO18B11_HOLD/);
});
