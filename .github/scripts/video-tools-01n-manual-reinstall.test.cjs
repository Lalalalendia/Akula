'use strict';
// VIDEO-TOOLS-01n: strict test-only manual install refusal + owner HOLD.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const repo = path.resolve(__dirname, '../..');
const paths = {
  nsis:'scripts/video-tools-01j-six-file-lab.nsi',
  runner:'scripts/windows-video-tools-01j-six-signed-lab.ps1',
  flow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  trust:'aquilum-app/native/src/video/tools_package.rs',
  queue:'aquilum-app/native/src/video/queue.rs',
  stock:'aquilum-app/native/packaging/akula-windows.json',
  doc:'docs/release/video-tools-01n-owner-and-manual-hold.md'
};
const load = () => Object.fromEntries(Object.entries(paths).map(([k,v]) =>
  [k,fs.readFileSync(path.join(repo,v),'utf8')]));
const hold = (v,m) => assert.ok(v,'VIDEO_TOOLS_01N_HOLD: '+m);
function check(s) {
  const {nsis,runner,flow,trust,queue,stock,doc}=s;
  hold(JSON.stringify(JSON.parse(stock).binaries.map(x=>x.path)) ===
    JSON.stringify(['akula-app','shiva-pdf-preview']),
    'stock installer no longer two EXEs');
  hold(trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";') &&
    trust.includes('false // a future, separately reviewed release gate must replace this constant'),
    'real publisher key/release HOLD was bypassed');
  hold(queue.includes('if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))'),
    'installed queue was unlocked');
  for(const t of [
    'Name "Akula Video Tools 01j NONRELEASE laboratory"',
    'InstallDir "$PROGRAMFILES64\\Akula Video Tools"',
    'RequestExecutionLevel admin',
    'IfFileExists "$INSTDIR\\*.*" 0 +3',
    'SetErrorLevel 77',
    'Abort "VIDEO_TOOLS_01N_HOLD: protected Video Tools already installed;',
    'WriteUninstaller "$INSTDIR\\uninstall.exe"',
    'RMDir "$INSTDIR"'
  ])hold(nsis.includes(t),'protected NSIS refusal missing '+t);
  hold(!/RMDir \/r|Delete \/r|WriteRegStr|nsExec::Exec/.test(nsis),
    'installer can mutate stock data, recurse or execute child');
  for(const t of [
    "$before01n=@{}",
    '(Get-FileHash -LiteralPath (Join-Path $root $itemName) -Algorithm SHA256).Hash',
    "$attempt01n=Start-Process -FilePath $installer -ArgumentList '/S' -PassThru -Wait",
    'if($attempt01n.ExitCode -ne 77)',
    "throw ('VIDEO_TOOLS_01N_HOLD: denied manual reinstall altered protected object '",
    'VIDEO_TOOLS_01N_REINSTALL_HOLD_PASS errorlevel=77 six_sha_unchanged=1',
    "$reinstall01n=Start-Process -FilePath $installer -ArgumentList '/S' -PassThru -Wait",
    'if($reinstall01n.ExitCode -ne 0 -or !$installed)',
    'if($reuninstall01n.ExitCode -ne 0)',
    'VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS denied_overwrite=1 explicit_uninstall=1 verified_reinstall=1 final_missing=1 ephemeral_key=1 release=0',
    'VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS'+String.fromCharCode(96)+'n'
  ])hold(runner.includes(t),'Windows signed reinstall witness missing '+t);
  const before=runner.indexOf('$before01n=@{}');
  const denied=runner.indexOf('if($attempt01n.ExitCode -ne 77)');
  const retry=runner.indexOf('$reinstall01n=Start-Process');
  const verified=runner.indexOf("Run-Native -Mode 'valid' -Public $public",retry);
  const marker=runner.indexOf('VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS denied_overwrite=1');
  hold(before>=0 && before<denied && denied<retry && retry<verified && verified<marker,
    'denial, deliberate reinstall, signature check or cleanup reordered');
  for(const t of [
    "      - '.github/scripts/video-tools-01n-manual-reinstall.test.cjs'",
    "      - 'docs/release/video-tools-01n-owner-and-manual-hold.md'",
    'node --test .github/scripts/video-tools-01n-manual-reinstall.test.cjs',
    'VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS'+String.fromCharCode(96)+'n',
    'Get-Content -LiteralPath $proofFile -Raw -ErrorAction Stop',
    'permissions:\n  contents: read',
    'runs-on: windows-2025','persist-credentials: false'
  ])hold(flow.includes(t),'exact-HEAD Windows CI proof missing '+t);
  hold(!/workflow_dispatch:|^  push:|contents: write|gh release create/m.test(flow),
    'nonshipping workflow can release');
  for(const t of [
    'VIDEO-18 RELEASE HOLD','publisher key','same-version','Windows 10/11',
    'manual','REVIEW_','version floor','RELEASE_READY=false'
  ])hold(doc.includes(t),'owner release boundary omitted '+t);
}
test('real six-file NSIS rejects overwrite and enables only deliberate reinstall',()=>{
  check(load());
});
test('negative source mutations must fail',()=>{
  const s=load();
  for(const [k,a,b] of [
    ['nsis','SetErrorLevel 77','SetErrorLevel 0'],
    ['nsis','IfFileExists "$INSTDIR\\*.*" 0 +3','IfFileExists "$INSTDIR\\*.*" +3 0'],
    ['nsis','Abort "VIDEO_TOOLS_01N_HOLD: protected Video Tools already installed;','DetailPrint "accepted existing install '],
    ['runner','if($attempt01n.ExitCode -ne 77)','if($false)'],
    ['runner','$before01n=@{}','$before01n=$null'],
    ['runner','if($reinstall01n.ExitCode -ne 0 -or !$installed)','if($false)'],
    ['runner','if($reuninstall01n.ExitCode -ne 0)','if($false)'],
    ['runner','VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS denied_overwrite=1','VIDEO_TOOLS_01N_MANUAL_REINSTALL_PASS denied_overwrite=0'],
    ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "CI";'],
    ['queue','if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))','if false'],
    ['flow','permissions:\n  contents: read','permissions:\n  contents: write'],
    ['flow','node --test .github/scripts/video-tools-01n-manual-reinstall.test.cjs','echo skipped']
  ]) {
    hold(s[k].includes(a),'mutation anchor missing '+k+': '+a);
    assert.throws(()=>check({...s,[k]:s[k].replace(a,b)}),/VIDEO_TOOLS_01N_HOLD/);
  }
});
