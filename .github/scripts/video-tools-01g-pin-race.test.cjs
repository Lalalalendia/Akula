'use strict';
// VIDEO-TOOLS-01g negative source/mutation contracts. Real race is Windows CI.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const repo=path.resolve(__dirname,'../..');
const paths={
  native:'aquilum-app/native/src/video/tools_package_pin_windows.rs',
  bridge:'aquilum-app/native/src/video/tools_package_pin_admission.rs',
  package:'aquilum-app/native/src/video/tools_package.rs',
  parent:'aquilum-app/native/src/video/mod.rs',
  probe:'aquilum-app/native/src/video/tools_package_probe.rs',
  lab:'scripts/windows-video-tools-01g-pin-race.ps1',
  runner:'scripts/windows-video-tools-01a-isolated-lab.ps1',
  workflow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  stock:'aquilum-app/native/packaging/akula-windows.json'
};
const read=k=>fs.readFileSync(path.join(repo,paths[k]),'utf8');
const hold=(pass,why)=>assert.ok(pass,'VIDEO_TOOLS_01G_HOLD: '+why);
function validate(x){
  const {native,bridge,package:pkg,parent,probe,lab,runner,workflow,stock}=x;
  assert.deepEqual(JSON.parse(stock).binaries.map(v=>v.path),
    ['akula-app','shiva-pdf-preview'],'VIDEO_TOOLS_01G_HOLD: default installer changed');
  for(const token of [
    'OpenOptionsExt','FILE_SHARE_READ','FILE_FLAG_OPEN_REPARSE_POINT',
    'FILE_FLAG_BACKUP_SEMANTICS','share_mode(FILE_SHARE_READ)',
    'safe_ancestors(root)?','fs::symlink_metadata',
    'PinnedPackage','_root: File','files: Vec<',
    'SIGNED_NAMES','LAB_NAMES','read_dir(root)',
    'reparse_metadata(&entry.path(), false)?',
    'fn inspect(','authenticate(name, file)?',
    'compare_reference(held, &mut reference)',
    'akula_video_tools_01g_witness',
    'video-tools-01g-ready','video-tools-01g-release',
    'VIDEO_TOOLS_01G_NATIVE_PIN_PASS source_bytes=3 root_held=1 files_held=4 release=0'
  ])hold(native.includes(token),'native OS pin and byte comparison missing '+token);
  hold(!native.includes('Command::new(')&&!native.includes('CreateProcessW'),
    'native pin must never launch a media executable');
  for(const token of [
    'inspect_owner_approved_package',
    'ReadOnlyPinnedAttestation','_handles: windows_pin::PinnedPackage',
    'APPROVED_VIDEO_TOOLS_KEY.is_empty()',
    '!shipping_admission_enabled()',
    'windows_discovery::installed_root()',
    'windows_discovery::check_installed_acl',
    'PinnedPackage::open(root, &windows_pin::SIGNED_NAMES)',
    'verify_signed(root, manifest, signed, APPROVED_VIDEO_TOOLS_KEY)',
    'held.inspect(|name, file|',
    'Sha256::new()','digest.update(&buf[..n])','attested.digests[i]',
    'manifest.json','manifest.minisig',
    'held PE digest differs from authenticated manifest'
  ])hold(bridge.includes(token),'signed manifest not bound to opened handles '+token);
  hold(!bridge.includes('Command::new(')&&!bridge.includes('CreateProcessW'),
    'signed admission cannot grant launch');
  for(const token of [
    'pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";',
    'pub(super) version: String','pub(super) digests: [String; 3]',
    'false // a future, separately reviewed release gate must replace this constant'
  ])hold(pkg.includes(token),'publisher RELEASE HOLD or original signature contract changed '+token);
  hold(probe.includes('pub(super) mod windows_discovery;'),'native OS-known root must be reused');
  hold(parent.includes('mod tools_package_pin_admission;'),'new pinned module not gated to installed Windows');
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    '$head -cne $env:EXPECTED_HEAD_SHA',
    'WindowsBuiltInRole]::Administrator',
    'Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists',
    'Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId',
    'rustc --edition=2024 --test',
    'rustc --edition=2024 --cfg akula_video_tools_01g_witness',
    'Start-Process -FilePath $witness',
    'GetLastWin32Error()',
    'ERROR_SHARING_VIOLATION=32','0x40000000','0x00010000',
    'video-tools-01g-ready','video-tools-01g-release',
    'Positive control','Write-Output',
    'VIDEO_TOOLS_01G_PIN_RACE_PASS','bytes_unchanged=1 release=0'
  ])hold(lab.includes(token),'real elevated process-race test missing '+token);
  for(const token of [
    'scripts/windows-video-tools-01g-pin-race.ps1',
    'VIDEO_TOOLS_01G_PIN_RACE_PASS',
    'VIDEO_TOOLS_01F_NONADMIN_PASS',
    'VIDEO_TOOLS_01A_ISOLATED_THREE_PE_PASS'
  ])hold(runner.includes(token),'actual protected NSIS run must call 01g '+token);
  for(const token of [
    "      - 'aquilum-app/native/src/video/tools_package_pin_windows.rs'",
    "      - 'aquilum-app/native/src/video/tools_package_pin_admission.rs'",
    "      - 'scripts/windows-video-tools-01g-pin-race.ps1'",
    "      - '.github/scripts/video-tools-01g-pin-race.test.cjs'",
    'node --test .github/scripts/video-tools-01g-pin-race.test.cjs',
    'VIDEO_TOOLS_01G_PIN_RACE_PASS',
    'runs-on: windows-2025','needs: contract','permissions:\n  contents: read',
    'persist-credentials: false'
  ])hold(workflow.includes(token),'exact-head no-release CI witness missing '+token);
  hold(!workflow.includes('workflow_dispatch:')&&!workflow.includes('contents: write')
    &&!workflow.includes('gh release create'),'this PR-only lab must not publish');
}
test('signed bytes are checked through held Windows handles and all existing release holds remain',()=>{
  validate(Object.fromEntries(Object.keys(paths).map(k=>[k,read(k)])));
});
test('negative source mutations reject missing pin, signature check, OS race or release guard',()=>{
  const x=Object.fromEntries(Object.keys(paths).map(k=>[k,read(k)]));
  const mutants=[
    ['native','share_mode(FILE_SHARE_READ)','share_mode(7)'],
    ['native','FILE_FLAG_OPEN_REPARSE_POINT','FILE_FLAG_NOT_OPEN_REPARSE_POINT'],
    ['native','safe_ancestors(root)?','Ok(())'],
    ['native','authenticate(name, file)?','Ok(())'],
    ['native','compare_reference(held, &mut reference)','Ok(())'],
    ['bridge','APPROVED_VIDEO_TOOLS_KEY.is_empty()','false'],
    ['bridge','!shipping_admission_enabled()','false'],
    ['bridge','verify_signed(root, manifest, signed, APPROVED_VIDEO_TOOLS_KEY)','Ok(())'],
    ['bridge','held.inspect(|name, file|','held.skip(|name, file|'],
    ['bridge','attested.digests[i]','untrusted_digest'],
    ['package','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "active";'],
    ['lab','GetLastWin32Error()','GetLastFriendlyError()'],
    ['lab','ERROR_SHARING_VIOLATION=32','any_error'],
    ['lab','Positive control','control omitted entirely'],
    ['runner','VIDEO_TOOLS_01G_PIN_RACE_PASS','FAKE_PASS'],
    ['workflow','node --test .github/scripts/video-tools-01g-pin-race.test.cjs','echo skip'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [part,oldText,newText] of mutants){
    hold(x[part].includes(oldText),'mutation anchor absent '+part+': '+oldText);
    assert.throws(()=>validate({...x,[part]:x[part].replaceAll(oldText,newText)}),
      /VIDEO_TOOLS_01G_HOLD/);
  }
});
