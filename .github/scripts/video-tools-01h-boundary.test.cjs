'use strict';
// VIDEO-TOOLS-01h source mutation gate. The real CreateProcessW test runs
// only on hosted Windows and executes ONLY a source-built benign child.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const base=path.resolve(__dirname,'../..');
const locations={
  native:'aquilum-app/native/src/video/tools_package_01h_process_boundary_windows.rs',
  benign:'scripts/video-tools-01h-benign-child.rs',
  powershell:'scripts/windows-video-tools-01h-process-boundary.ps1',
  parent:'aquilum-app/native/src/video/mod.rs',
  pin:'aquilum-app/native/src/video/tools_package_pin_windows.rs',
  signing:'aquilum-app/native/src/video/tools_package.rs',
  workflow:'.github/workflows/akula-video-tools-01h-process-boundary.yml',
  stock:'aquilum-app/native/packaging/akula-windows.json'
};
const read=name=>fs.readFileSync(path.join(base,locations[name]),'utf8');
const hold=(ok,reason)=>assert.ok(ok,'VIDEO_TOOLS_01H_HOLD: '+reason);
function verify(x){
  const {native,benign,powershell,parent,pin,signing,workflow,stock}=x;
  assert.deepEqual(JSON.parse(stock).binaries.map(v=>v.path),
    ['akula-app','shiva-pdf-preview'],'VIDEO_TOOLS_01H_HOLD: stock release must remain two EXEs');
  for(const token of [
    '#[path = "tools_package_pin_windows.rs"]',
    'pinned::PinnedPackage::open(&root, &pinned::LAB_NAMES)',
    'compare_installed_to_source(&mut guard, &source)?',
    'os_root::known_program_files_x64()?',
    'os_root::check_installed_acl(root, &pinned::LAB_NAMES)?',
    'root.parent() != Some(os_program_files.as_path())',
    'AkulaVideoToolsLab-','bInheritHandles = FALSE',
    'CREATE_NO_WINDOW', 'CreateProcessW(',
    'let (child, thread) = spawn_exact_benign_child(&executable, &work)?;',
    'await_barrier(&work.join("01h-child-started"), "benign image startup")?',
    'await_barrier(&work.join("01h-finish"), "second mutation probe")?',
    'WaitForSingleObject(child.0, WAIT_LIMIT_MS)',
    'GetExitCodeProcess(child.0, &mut exit_code)',
    'drop(guard);', 'akula_video_tools_01h_witness',
    'VIDEO_TOOLS_01H_CREATEPROCESS_PASS benign=1 signed_tools_executed=0',
  ])hold(native.includes(token),'native verified process boundary omitted '+token);
  hold(native.indexOf('let (child, thread) = spawn_exact_benign_child') <
       native.lastIndexOf('drop(guard);') &&
       native.indexOf('await_barrier(&work.join("01h-child-started")') <
       native.lastIndexOf('drop(guard);'),
       'same held guard must outlive CreateProcessW and child start');
  hold(!native.includes('akula_yt_dlp.exe')&&!native.includes('yt-dlp --'),
    'test must not execute media runtimes');
  for(const token of [
    '--akula-01h-child','GITHUB_EVENT_NAME',
    'RUNNER_ENVIRONMENT','01h-child-started','01h-finish',
    '01h-child-exited','VIDEO_TOOLS_01H_CHILD_HOLD'
  ])hold(benign.includes(token),'benign fixture must prove start and clean exit '+token);
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    '$head -cne $env:EXPECTED_HEAD_SHA',
    'WindowsBuiltInRole]::Administrator',
    'Get-AkulaVideoToolsOsProgramFiles',
    'Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists',
    'Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag',
    'S-1-5-32-544','S-1-5-32-545',
    'rustc --edition=2024 --test',
    'rustc --edition=2024 --cfg akula_video_tools_01h_witness',
    'scripts/video-tools-01h-benign-child.rs',
    'Start-Process -FilePath $witness',
    'GetLastWin32Error()',
    'ERROR_SHARING_VIOLATION=32',
    'AkulaVideoTools01hMutation',
    'Attempt-Rename','Check-WhilePinned',
    "Await-Witness -Marker '01h-pinned'",
    "Await-Witness -Marker '01h-launched'",
    "01h-child-started","01h-child-exited",
    'positive', 'if($createdRoot)',
    'refusing unsafe cleanup of unknown protected files',
    'VIDEO_TOOLS_01H_HANDOFF_PASS',
    'write_after_drop=1 hashes=4 release=0'
  ])hold(powershell.toLowerCase().includes(token.toLowerCase()),
    'Windows native race and safe cleanup omitted '+token);
  hold(parent.includes('mod tools_package_01h_process_boundary_windows;'),
    'native Windows leaf module not compiled');
  hold(parent.includes('all(windows, any(test, feature = "installed"))'),
    'module must be disabled outside Windows installed or tests');
  hold(pin.includes('share_mode(FILE_SHARE_READ)') && pin.includes('_root: File'),
    'must reuse existing held handle guard, not a path-only verifier');
  hold(signing.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";') &&
       signing.includes('false // a future, separately reviewed release gate must replace this constant'),
       'publisher and shipping HOLD must remain');
  for(const token of [
    "      - 'aquilum-app/native/src/video/tools_package_01h_process_boundary_windows.rs'",
    "      - 'scripts/windows-video-tools-01h-process-boundary.ps1'",
    "      - '.github/scripts/video-tools-01h-boundary.test.cjs'",
    'node --test .github/scripts/video-tools-01h-boundary.test.cjs',
    'VIDEO_TOOLS_01H_HANDOFF_PASS', 'runs-on: windows-2025',
    'needs: contract','permissions:\n  contents: read',
    'persist-credentials: false'
  ])hold(workflow.includes(token),'PR-only Windows gate missing '+token);
  hold(!/workflow_dispatch:|^\s+push:|contents:\s*write|gh release create|publish_beta/m.test(workflow),
    'CI must not gain any release trigger or write token');
}
test('native benign CreateProcessW is pinned until child start with no publisher approval',()=>{
  verify(Object.fromEntries(Object.keys(locations).map(k=>[k,read(k)])));
});
test('negative mutations cannot bypass pin, real Win32 race, source binding or release hold',()=>{
  const x=Object.fromEntries(Object.keys(locations).map(k=>[k,read(k)]));
  const mutations=[
    ['native','pinned::PinnedPackage::open(&root, &pinned::LAB_NAMES)','unprotected_open(&root)'],
    ['native','compare_installed_to_source(&mut guard, &source)?','untrusted_pinned_bytes()'],
    ['native','os_root::known_program_files_x64()?','std::env::temp_dir()'],
    ['native','CreateProcessW(','SkipLaunch('],
    ['native','bInheritHandles = FALSE','inherit untrusted handles'],
    ['native','await_barrier(&work.join("01h-child-started"), "benign image startup")?','Ok(())'],
    ['native','drop(guard);','drop_unverified();'],
    ['benign','01h-child-started','skip-child'],
    ['powershell','ERROR_SHARING_VIOLATION=32','any_err_is_pass'],
    ['powershell','Attempt-Rename','Skip-Rename'],
    ['powershell',"Await-Witness -Marker '01h-launched'",'Write-Host no-start'],
    ['powershell','VIDEO_TOOLS_01H_HANDOFF_PASS','FAKE_PASS'],
    ['parent','mod tools_package_01h_process_boundary_windows;','// removed module'],
    ['signing','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "key";'],
    ['workflow','node --test .github/scripts/video-tools-01h-boundary.test.cjs','echo skip'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write'],
    ['workflow','VIDEO_TOOLS_01H_HANDOFF_PASS','FAKE_PASS']
  ];
  for(const [key,original,replacement] of mutations){
    hold(x[key].includes(original),'mutation anchor absent '+key+': '+original);
    assert.throws(()=>verify({...x,[key]:x[key].replaceAll(original,replacement)}),
      /VIDEO_TOOLS_01H_HOLD/);
  }
});
