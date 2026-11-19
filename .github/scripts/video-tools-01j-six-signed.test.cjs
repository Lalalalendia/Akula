'use strict';
// VIDEO-TOOLS-01j: negative/source contracts for genuine test-only Minisign
// six-file NSIS plus actual native Akula discovery. No real release authority.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const locations={
  installer:'scripts/video-tools-01j-six-file-lab.nsi',
  runner:'scripts/windows-video-tools-01j-six-signed-lab.ps1',
  native:'labs/video-tools-01j/src/main.rs',
  cargo:'labs/video-tools-01j/Cargo.toml',
  workflow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  stock:'aquilum-app/native/packaging/akula-windows.json',
  trust:'aquilum-app/native/src/video/tools_package.rs',
  discovery:'aquilum-app/native/src/video/tools_package_probe.rs',
  pin:'aquilum-app/native/src/video/tools_package_pin_windows.rs',
  allow:'docs/release/video18b04-five-exe-allowlist.json'
};
const all=()=>Object.fromEntries(Object.entries(locations).map(([k,p])=>[k,
  fs.readFileSync(path.join(root,p),'utf8')]));
const hold=(v,m)=>assert.ok(v,'VIDEO_TOOLS_01J_HOLD: '+m);
function validate(s){
  const {installer,runner,native,cargo,workflow,stock,trust,discovery,pin,allow}=s;
  assert.deepEqual(JSON.parse(stock).binaries.map(x=>x.path),
    ['akula-app','shiva-pdf-preview'],'VIDEO_TOOLS_01J_HOLD: stock package was expanded');
  for(const token of [
    'Name "Akula Video Tools 01j NONRELEASE laboratory"',
    'InstallDir "$PROGRAMFILES64\\Akula Video Tools"',
    '!include "WinVer.nsh"',
    'RequestExecutionLevel admin',
    'File "/oname=akula-video-mux.exe"',
    'File "/oname=akula-yt-dlp.exe"',
    'File "/oname=akula-quickjs.exe"',
    'File "/oname=manifest.json"',
    'File "/oname=manifest.minisig"',
    'WriteUninstaller "$INSTDIR\\uninstall.exe"',
    'Delete "$INSTDIR\\uninstall.exe"',
    'RMDir "$INSTDIR"'
  ])hold(installer.includes(token),'six-file NSIS identity missing '+token);
  hold(!installer.includes('Delete /r')&&!installer.includes('RMDir /r')&&!installer.includes('WriteRegStr'),
    'uninstaller must never recursively delete arbitrary user data or touch stock registry');
  const actualFiles=installer.match(/^\s+File "\/oname=/gm)||[];
  hold(actualFiles.length===5,'exactly five staged entries and a generated uninstaller required');
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    '$head -cne $env:EXPECTED_HEAD_SHA',
    'WindowsBuiltInRole]::Administrator',
    'Get-AkulaVideoToolsOsProgramFiles',
    "Join-Path $program 'Akula Video Tools'",
    'protected product already exists',
    'scripts/video-tools-01i-unsigned-custody.cjs',
    "labs/video-lite/Cargo.lock",
    'minisign -G -W',
    'minisign -S -q',
    'minisign -Vm',
    "labs/video-tools-01j/Cargo.toml",
    'scripts/video-tools-01j-six-file-lab.nsi',
    "Run-Native -Mode 'missing'",
    "Run-Native -Mode 'valid' -Public $public",
    "Run-Native -Mode 'wrong-key' -Public $otherPublic",
    "Run-Native -Mode 'tamper-manifest' -Public $public",
    "Run-Native -Mode 'tamper-signature' -Public $public",
    "Run-Native -Mode 'tamper-pe' -Public $public",
    "Run-Native -Mode 'seventh-file' -Public $public",
    'Assert-Exact-Six',
    'privateNote',
    'private_notes_preserved=1 release=0',
    "VIDEO_TOOLS_01J_SIX_SIGNED_PASS",
    'if($installed -and',
    'Remove-Item -LiteralPath $work -Recurse -Force'
  ])hold(runner.includes(token),'real Windows signed six-file CI lab incomplete '+token);
  hold(!runner.includes('gh release create')&&!runner.includes('GITHUB_TOKEN'),
    'Windows test cannot create a publisher release');
  for(const token of [
    '#[path = "../../../aquilum-app/native/src/video/tools_package.rs"]',
    '#[path = "../../../aquilum-app/native/src/video/tools_package_probe.rs"]',
    '#[path = "../../../aquilum-app/native/src/video/tools_package_pin_windows.rs"]',
    '#[path = "../../../aquilum-app/native/src/video/tools_package_pin_admission.rs"]',
    'probe_installed_video_tools()',
    'ToolsDiscovery::PublisherNotApproved',
    'ToolsDiscovery::Missing',
    'ToolsDiscovery::Unsafe',
    'os_root::installed_root()',
    'os_root::check_installed_acl',
    'PinnedPackage::open(',
    'tools_package::verify_signed(root, &raw, signed, key)?',
    'held.inspect(|name, file|',
    'sha_of_held(file)? != attested.digests[index]',
    'tools_package::shipping_admission_enabled()',
    'tools_package_pin_admission::inspect_owner_approved_package(',
    'VIDEO_TOOLS_01J_NATIVE_SIGNED_INSTALLED_PASS',
    'VIDEO_TOOLS_01J_NATIVE_REJECT_PASS'
  ])hold(native.includes(token),'native Akula exact-source signed admission absent '+token);
  hold(!native.includes('CreateProcessW')&&!native.includes('Command::new('),
    'signed test must never execute video components');
  for(const token of [
    'minisign-verify = "0.2"','sha2 = "0.10"',
    'publish = false','[workspace]'
  ])hold(cargo.includes(token),'standalone nonpublishable crypto verifier missing '+token);
  hold(trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";') &&
    trust.includes('false // a future, separately reviewed release gate must replace this constant'),
    'production trust and release HOLD must not change');
  hold(discovery.includes('PublisherNotApproved')&&
    discovery.includes('if APPROVED_VIDEO_TOOLS_KEY.is_empty() || !shipping_admission_enabled()'),
    'production discovery must keep the unapproved publisher state');
  hold(pin.includes('share_mode(FILE_SHARE_READ)')&&pin.includes('SIGNED_NAMES'),
    'native verifier must retain the real 01g pinned handles');
  const source=JSON.parse(allow);
  hold(source.release_admitted===false&&source.installer_signing_authorized===false&&
    source.stage_binary_distribution_authorized===false,
    'upstream allowlist must remain unapproved');
  for(const token of [
    "      - 'scripts/windows-video-tools-01j-six-signed-lab.ps1'",
    "      - 'scripts/video-tools-01j-six-file-lab.nsi'",
    "      - '.github/scripts/video-tools-01j-six-signed.test.cjs'",
    'node --test .github/scripts/video-tools-01j-six-signed.test.cjs',
    'VIDEO_TOOLS_01J_SIX_SIGNED_PASS',
    'choco install minisign --version=0.8.0 --yes --no-progress',
    'runs-on: windows-2025', 'permissions:\n  contents: read',
    'persist-credentials: false', 'cancel-in-progress:'
  ])hold(workflow.includes(token),'PR-only real Windows signed install workflow incomplete '+token);
}
test('genuine six-file signed test installer remains blocked as a production authority',()=>{
  validate(all());
});
test('negative source mutations reject missing trust hold, crypto, ACL, binaries or cleanup',()=>{
  const s=all();
  const cases=[
    ['installer','manifest.minisig','manifest-untrusted'],
    ['installer','Delete "$INSTDIR\\uninstall.exe"','Delete "$INSTDIR\\unknown.exe"'],
    ['runner','minisign -S -q','echo skip'],
    ['runner',"Run-Native -Mode 'wrong-key' -Public $otherPublic",'Write-Host skip-key'],
    ['runner',"Run-Native -Mode 'tamper-manifest' -Public $public",'Write-Host skip-manifest'],
    ['runner',"Run-Native -Mode 'tamper-pe' -Public $public",'Write-Host skip-exe'],
    ['runner',"Run-Native -Mode 'seventh-file' -Public $public",'Write-Host skip-extra'],
    ['runner','private_notes_preserved=1 release=0','release=1'],
    ['native','ToolsDiscovery::PublisherNotApproved','ToolsDiscovery::ReadOnlyAttested'],
    ['native','tools_package::verify_signed(root, &raw, signed, key)?','Ok(())'],
    ['native','PinnedPackage::open(','OpenOptions::new('],
    ['native','tools_package::shipping_admission_enabled()','false'],
    ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";',
      'pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "temp";'],
    ['workflow','node --test .github/scripts/video-tools-01j-six-signed.test.cjs','echo skip'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [key,from,to] of cases){
    hold(s[key].includes(from),'negative mutation anchor missing '+key+': '+from);
    assert.throws(()=>validate({...s,[key]:s[key].replaceAll(from,to)}),
      /VIDEO_TOOLS_01J_HOLD/);
  }
});
