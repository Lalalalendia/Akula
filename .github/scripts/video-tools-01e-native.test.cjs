'use strict';
// VIDEO-TOOLS-01e: negative source tests supplement genuine Windows native API CI.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const base=path.resolve(__dirname,'../..');
const files={
  rust:'aquilum-app/native/src/video/tools_package_windows.rs',
  module:'aquilum-app/native/src/video/tools_package.rs',
  lab:'scripts/windows-video-tools-01e-native-lab.ps1',
  installer:'scripts/windows-video-tools-01a-isolated-lab.ps1',
  workflow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  product:'aquilum-app/native/packaging/akula-windows.json'
};
const read = name => fs.readFileSync(path.join(base,files[name]),'utf8');
const hold=(ok,message)=>assert.ok(ok,'VIDEO_TOOLS_01E_HOLD: '+message);
function check(x){
  const {rust,module,lab,installer,workflow,product}=x;
  assert.deepEqual(JSON.parse(product).binaries.map(x=>x.path),['akula-app','shiva-pdf-preview']);
  for(const token of [
    'SHGetKnownFolderPath','PROGRAM_FILES_X64','0x6d809377',
    'RtlGetVersion','version.major < 10','size_of::<usize>() != 8',
    'CoTaskMemFree','LocalFree','GetNamedSecurityInfoW',
    'ConvertSidToStringSidW','GetAce',
    'check_ancestors(&root)','fs::symlink_metadata','file_attributes() & 0x400',
    'Akula Video Tools','GetNamedSecurityInfoW(', '1 | 4',
    'dacl.is_null()','privileged_sid','0x4000_0000','0x1000_0000',
    '0x0004_0000','0x0008_0000','0x0000_0040',
    'non-elevated trustee can mutate protected package',
    'unsupported or malformed Windows ACL entry','signed package is incomplete',
    'check_object_acl(&item.path())','check_object_acl(root)',
    'akula_video_tools_acl_witness',
    'VIDEO_TOOLS_01E_NATIVE_ACL_PASS'
  ])hold(rust.includes(token),'native OS/ACL gate missing '+token);
  hold(!/std::env::var(?:_os)?\\(("|')ProgramFiles|GetEnvironmentVariable\\(("|')ProgramFiles/.test(rust),
    'native discovery must not trust ambient ProgramFiles');
  for(const token of [
    '#[cfg(windows)]','tools_package_windows.rs','windows_discovery::installed_root()',
    'windows_discovery::check_installed_acl',
    'APPROVED_VIDEO_TOOLS_KEY.is_empty()','shipping_admission_enabled()',
    'verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY)',
    'manifest.json','manifest.minisig',
    'ReadOnlyAttested','PublisherNotApproved'
  ])hold(module.includes(token),'native signed-content binding missing '+token);
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    '$head -cne $env:EXPECTED_HEAD_SHA',
    'WindowsBuiltInRole]::Administrator',
    'rustc --edition=2024 --test',
    'rustc --edition=2024 --cfg akula_video_tools_acl_witness',
    'Start-Process -FilePath $witness',
    'Set-Acl -LiteralPath $Target -AclObject $modified',
    'Set-Acl -LiteralPath $Target -AclObject $before',
    'ProgramFiles','SPOOF-01E-PROGRAMFILES',
    'S-1-1-0','S-1-5-11','WriteData','ChangePermissions',
    'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS','release=0'
  ])hold(lab.includes(token),'real Windows native negative lab missing '+token);
  hold(lab.includes('finally {') && lab.includes('if ($changed)'),
    'ACL poison must be restored under failure');
  for(const token of [
    "scripts/windows-video-tools-01e-native-lab.ps1",
    'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS','VIDEO_TOOLS_01D_ACL_MUTATION_PASS'
  ])hold(installer.includes(token),'actual NSIS runner did not invoke native guard '+token);
  for(const token of [
    "      - 'aquilum-app/native/src/video/tools_package_windows.rs'",
    "      - 'scripts/windows-video-tools-01e-native-lab.ps1'",
    "      - '.github/scripts/video-tools-01e-native.test.cjs'",
    'node --test .github/scripts/video-tools-01e-native.test.cjs',
    'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS','needs: contract',
    'runs-on: windows-2025','permissions:\n  contents: read','persist-credentials: false'
  ])hold(workflow.includes(token),'CI release isolation/native proof missing '+token);
  hold(!/workflow_dispatch:|^\\s+push:|contents:\\s*write|gh release create|publish_beta/m.test(workflow),
    'nonrelease workflow cannot gain publishing authority');
}
test('OS known folder, real Win32 owner/DACL, and signed read-only boundary remain enforced',()=>{
  check(Object.fromEntries(Object.keys(files).map(k=>[k,read(k)])));
});
test('negative source mutations cannot bypass native Win32 gates or publish',()=>{
  const a=Object.fromEntries(Object.keys(files).map(k=>[k,read(k)]));
  for(const [file,before,after] of [
    ['rust','SHGetKnownFolderPath','SHGetFolderPath'],
    ['rust','GetNamedSecurityInfoW','SkipDacl'],
    ['rust','0x0004_0000','0x0000_0000'],
    ['rust','0x4000_0000','0x0000_0000'],
    ['rust','check_object_acl(&item.path())','// skipped native file rights'],
    ['rust','signed package is incomplete','ignored files'],
    ['module','APPROVED_VIDEO_TOOLS_KEY.is_empty()','false'],
    ['module','verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY)','Ok(())'],
    ['lab','Set-Acl -LiteralPath $Target -AclObject $before','Write-Host no-restore'],
    ['lab','VIDEO_TOOLS_01E_NATIVE_WIN32_PASS','FAKE_PASS'],
    ['installer','VIDEO_TOOLS_01E_NATIVE_WIN32_PASS','FAKE_PASS'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ]){
    hold(a[file].includes(before),'mutation anchor missing '+file+': '+before);
    assert.throws(()=>check({...a,[file]:a[file].replaceAll(before,after)}),/VIDEO_TOOLS_01E_HOLD/);
  }
});
