'use strict';
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const paths={
  orchestrator:'scripts/windows-video18b14-protected-installed-gui.ps1',
  pointer:'scripts/windows-video18b14-protected-gui.ps1',
  python:'labs/video-lite/video18b14/gui_acceptance.py',
  workflow:'.github/workflows/akula-video18b14-protected-gui.yml',
  contract:'.github/scripts/video18b14-protected-gui.test.cjs',
  doc:'docs/release/video18b14-protected-installed-gui.md',
};
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const names=['akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs'];
function verify(ps,mouse,py,wf){
  for(const n of names)assert.ok(ps.includes("'"+n+"'"),'VIDEO18B14_HOLD: missing genuine EXE '+n);
  for(const term of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    "RUNNER_OS -cne 'Windows'",
    'EXPECTED_HEAD_SHA',
    "IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
    "AkulaVideo18bRealNsisLab-",
    "Video18b14Stage-",
    'video18b-protected-five-real.nsi',
    'NSIS\\makensis.exe',
    "source outside hosted checkout/temp",
    'ReadByte() -ne 77','ReadByte() -ne 90',
    'release_admitted -ne $false',
    'AKULA_BUNDLED_MUX_SHA256',
    'Get-FileHash -LiteralPath $source',
    'Get-FileHash -LiteralPath $file',
    'video18b14/gui_acceptance.py',
    'VIDEO18B14_PROTECTED_GUI',
    'VIDEO18B14_PROTECTED_INSTALL_GUI_UNINSTALL_PASS',
    "uninstall.exe",
    'video18b14-output',
    'SHA256',
    'local media leaked to note vault',
    'installed uninstall lost or altered private local media'
  ])assert.ok(ps.includes(term),'VIDEO18B14_HOLD: missing real NSIS/GUI boundary '+term);
  assert.match(ps,/Start-Process -FilePath \$setup -ArgumentList '\/S' -Wait -PassThru/);
  assert.match(ps,/Start-Process -FilePath \$uninstaller -ArgumentList '\/S' -Wait -PassThru/);
  for(const term of [
    "AKULA_VIDEO18B14_PROTECTED_GUI",
    "ProgramFiles",
    "AkulaVideo18bRealNsisLab-",
    "[0-9a-f]{32}",
    "akula-app.exe",
    "ReparsePoint",
    "SendInput(1,down",
    "Wait-Event 'WINDOW_POINTER_DOWN'",
    "VIDEO16_RESTART_DECODED_GUI_FRAME_PASS",
  ])assert.ok(mouse.includes(term),'VIDEO18B14_HOLD: GUI cannot bypass native click, restart or protected path '+term);
  assert.ok(!mouse.includes("Join-Path $env:LOCALAPPDATA 'Akula\\akula-app.exe'"),
    'VIDEO18B14_HOLD: cannot accept per-user installed GUI');
  for(const term of [
    'AKULA_VIDEO18B14_PROTECTED_GUI','ProgramFiles','re.fullmatch',
    'AkulaVideo18bRealNsisLab-',
    'args.exe.resolve(strict=True) != installed',
    'scripts/windows-video18b14-protected-gui.ps1',
    'RestrictedLocalServer','http.server.ThreadingHTTPServer',
    'VIDEO16_REAL_MOUSE_DOWNLOAD_PASS',
    'VIDEO16_RESTART_DECODED_GUI_FRAME_PASS',
    'VIDEO18B14_PROTECTED_HOSTED_GUI_E2E_PASS',
  ])assert.ok(py.includes(term),'VIDEO18B14_HOLD: GUI harness lacks path and local video proof '+term);
  const hdr=wf.split(/\r?\npermissions:/,1)[0];
  assert.match(hdr,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const path of Object.values(paths))assert.ok(hdr.includes("      - '"+path+"'"),
    'VIDEO18B14_HOLD: missing exact source trigger '+path);
  assert.match(wf,/permissions:\s*\n  contents: read/);
  assert.match(wf,/persist-credentials: false/);
  assert.match(wf,/runs-on: windows-2022/);
  assert.match(wf,/EXPECTED_HEAD_SHA/);
  assert.match(wf,/cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e/);
  assert.match(wf,/windows-video18b04-real-five-custody.ps1/);
  assert.match(wf,/windows-video18b14-protected-installed-gui.ps1/);
  assert.match(wf,/python -m py_compile labs\/video-lite\/video18b14\/gui_acceptance.py/);
  assert.match(wf,/needs: \[contract, fixture\]/);
  assert.match(wf,/needs: real-gui/);
  assert.match(wf,/VIDEO18B14_PROTECTED_GUI_INDEPENDENT_DECODE_PASS/);
  assert.match(wf,/video18b14-output\/protected-installed\.mp4/);
  assert.ok(!wf.includes('workflow_dispatch:'),'VIDEO18B14_HOLD: no manual release');
  assert.ok(!/gh release create|publish_beta|contents: write|cargo packager signer/i.test(wf),
    'VIDEO18B14_HOLD: forbidden publication path');
  assert.ok(!/upload-artifact[\s\S]*path:[^\n]*\.(exe|msi|zip)/i.test(wf),
    'VIDEO18B14_HOLD: never upload NSIS or helpers');
}
test('VIDEO18B14 requires protected real PE install, physical GUI playback, uninstall and separate decode',()=>{
  verify(read(paths.orchestrator),read(paths.pointer),read(paths.python),read(paths.workflow));
});
test('VIDEO18B14 refuses weakening protected executable path or omitting genuine click',()=>{
  const ps=read(paths.orchestrator),mouse=read(paths.pointer),py=read(paths.python),wf=read(paths.workflow);
  const badMouse=mouse.replace("SendInput(1,down","DROPPED(1,down");
  assert.notEqual(mouse,badMouse);
  assert.throws(()=>verify(ps,badMouse,py,wf),/VIDEO18B14_HOLD/);
  const badPy=py.replace("AkulaVideo18bRealNsisLab-","WritableUserRoot-");
  assert.notEqual(py,badPy);
  assert.throws(()=>verify(ps,mouse,badPy,wf),/VIDEO18B14_HOLD/);
});
test('VIDEO18B14 rejects missing fifth binary or loss of CI source trigger',()=>{
  const ps=read(paths.orchestrator),mouse=read(paths.pointer),py=read(paths.python),wf=read(paths.workflow);
  const badPs=ps.replace("'akula-quickjs'","'unknown-program'");
  assert.notEqual(badPs,ps);
  assert.throws(()=>verify(badPs,mouse,py,wf),/VIDEO18B14_HOLD/);
  const remove="      - '"+paths.pointer+"'\n";
  const badWf=wf.replace(remove,'');
  assert.notEqual(wf,badWf);
  assert.throws(()=>verify(ps,mouse,py,badWf),/VIDEO18B14_HOLD/);
  assert.throws(()=>verify(ps,mouse,py,wf+'\nworkflow_dispatch:\n'),/VIDEO18B14_HOLD/);
});
