'use strict';
// VIDEO-TOOLS-01m: reject fake positive status, untrusted test keys and release promotion.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const root = path.resolve(__dirname, '../..');
const files = {
  probe: 'aquilum-app/native/src/video/tools_package_probe.rs',
  signed: 'aquilum-app/native/src/video/tools_package.rs',
  queue: 'aquilum-app/native/src/video/queue.rs',
  native: 'labs/video-tools-01j/src/main.rs',
  runner: 'scripts/windows-video-tools-01j-six-signed-lab.ps1',
  workflow: '.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  stock: 'aquilum-app/native/packaging/akula-windows.json',
  doc: 'docs/release/video-tools-01m-trust-state.md'
};
const read = () => Object.fromEntries(Object.entries(files).map(([k,v]) =>
  [k,fs.readFileSync(path.join(root,v),'utf8')]));
const hold = (condition, why) => assert.ok(condition,'VIDEO_TOOLS_01M_HOLD: '+why);
function verify(s) {
  const installed=s.probe.match(/if APPROVED_VIDEO_TOOLS_KEY\.is_empty\(\) \|\| !shipping_admission_enabled\(\)/);
  hold(installed,'ordinary installed Akula lost publisher and release approval barrier');
  hold(s.probe.indexOf('check_installed_acl(&root, &files)') >= 0 &&
       s.probe.indexOf('check_installed_acl(&root, &files)') < installed.index,
       'native Windows ACL must be checked before any publisher decision');
  hold(s.probe.includes('Ok(attested) => classify_authenticated_version(&attested.version)') &&
       s.probe.includes('match verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY)'),
       'only Minisign-authenticated manifest may be classified');
  hold(s.probe.includes('"0.1.0" => ToolsDiscovery::ReadOnlyAttested') &&
       s.probe.includes('_ => ToolsDiscovery::IncompatibleVersion'),
       'exact version classifier missing');
  hold(s.signed.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";') &&
       s.signed.includes('false // a future, separately reviewed release gate must replace this constant'),
       'real publisher authority and public release must remain absent');
  hold(s.queue.includes('if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))'),
       'ordinary installed download queue must stay closed');
  hold(JSON.stringify(JSON.parse(s.stock).binaries.map(x=>x.path)) ===
       JSON.stringify(['akula-app','shiva-pdf-preview']),
       'stock signed Akula installer must remain two-EXE');
  for(const token of [
    'fn exact_installed(root: &Path, key: &str) -> Result<String, String>',
    'tools_package::verify_signed(root, &raw, signed, key)?',
    'PinnedPackage::open(',
    'probe_installed_video_tools() != ToolsDiscovery::PublisherNotApproved',
    'tools_package_probe::classify_authenticated_version(&authenticated_version)',
    '"status-attested"|"status-incompatible"',
    'ToolsDiscovery::ReadOnlyAttested',
    'ToolsDiscovery::IncompatibleVersion',
    'VIDEO_TOOLS_01M_NATIVE_STATE_PASS state={label}',
    'publisher_unapproved=1 release=0'
  ])hold(s.native.includes(token),'real native signed-status witness missing: '+token);
  hold(!s.native.includes('CreateProcessW') && !s.native.includes('Command::new('),
       'signed diagnostic witness must not launch Video Tools');
  for(const token of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "$head -cne $env:EXPECTED_HEAD_SHA",
    'Run-Native -Mode \'status-attested\' -Public $public',
    'Run-Native -Mode \'status-incompatible\' -Public $public',
    '$toVersion=\'"version":"0.2.0"\'',
    '& $signer -S -q -s $private -m $candidate -x $candidateSignature',
    '& $signer -Vm $candidate -x $candidateSignature -p $public -Q',
    '[IO.File]::WriteAllBytes($installedManifest,$saved01mManifest)',
    '[IO.File]::WriteAllBytes($installedSig,$saved01mSignature)',
    'VIDEO_TOOLS_01M_REAL_SIGNED_STATES_PASS version_010=1 version_020=1',
    'publisher_approved=0 release=0',
    'VIDEO_TOOLS_01M_REAL_SIGNED_STATES_PASS`n'
  ])hold(s.runner.includes(token),'signed genuine Windows state/restore proof missing: '+token);
  for(const token of [
    "      - '.github/scripts/video-tools-01m-trust-state.test.cjs'",
    "      - 'docs/release/video-tools-01m-trust-state.md'",
    'node --test .github/scripts/video-tools-01m-trust-state.test.cjs',
    'VIDEO_TOOLS_01M_REAL_SIGNED_STATES_PASS`n',
    'Get-Content -LiteralPath $proofFile -Raw -ErrorAction Stop',
    'persist-credentials: false',
    'permissions:\n  contents: read',
    'runs-on: windows-2025',
    'cancel-in-progress:'
  ])hold(s.workflow.includes(token),'PR-only fail-closed CI proof missing: '+token);
  hold(!/workflow_dispatch:|^  push:|contents: write|gh release create/m.test(s.workflow),
       'CI workflow may not publish a release');
  hold(s.doc.includes('RELEASE HOLD') && s.doc.includes('ReadOnlyAttested') &&
       s.doc.includes('IncompatibleVersion') && s.doc.includes('ephemeral'),
       'scope documentation lost temporary signing and release limitation');
}
test('real signed versions are diagnostic only and production authority stays closed',()=>{
  verify(read());
});
test('mutating signer, classification, publisher gate or proof makes CI fail',()=>{
  const files=read();
  const changes=[
    ['probe','Ok(attested) => classify_authenticated_version(&attested.version)','Ok(_) => ToolsDiscovery::ReadOnlyAttested'],
    ['probe','check_installed_acl(&root, &files)','Ok(())'],
    ['signed','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "CI";'],
    ['queue','if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))','if false'],
    ['native','tools_package::verify_signed(root, &raw, signed, key)?','panic!("fake verify")'],
    ['native','PinnedPackage::open(','FakePinned::open('],
    ['native','probe_installed_video_tools() != ToolsDiscovery::PublisherNotApproved','false'],
    ['runner','Run-Native -Mode \'status-incompatible\' -Public $public','Write-Output not-tested'],
    ['runner','& $signer -Vm $candidate -x $candidateSignature -p $public -Q','Write-Output unverified'],
    ['runner','[IO.File]::WriteAllBytes($installedManifest,$saved01mManifest)','Write-Output skip-restore'],
    ['workflow','node --test .github/scripts/video-tools-01m-trust-state.test.cjs','echo skipped'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [key,original,replacement] of changes){
    hold(files[key].includes(original),'negative anchor missing: '+key);
    assert.throws(()=>verify({...files,[key]:files[key].replace(original,replacement)}),
      /VIDEO_TOOLS_01M_HOLD/,key+' must be rejected');
  }
});
