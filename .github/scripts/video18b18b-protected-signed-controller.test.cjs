'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../..');
const files={
  gate:'labs/video-lite/video18b18-signed-five/src/lib.rs',
  manifest:'labs/video-lite/video18b18-signed-five/Cargo.toml',
  ctrl:'labs/video-lite/video18b18b-controller/src/lib.rs',
  ctrlmanifest:'labs/video-lite/video18b18b-controller/Cargo.toml',
  ps:'scripts/windows-video18b18b-protected-signed-controller.ps1',
  wf:'.github/workflows/akula-video18b18b-protected-signed-controller.yml',
  test:'.github/scripts/video18b18b-protected-signed-controller.test.cjs',
  doc:'docs/release/video18b18b-protected-signed-controller.md',
};
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const names=[
  'akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
  'akula-yt-dlp.exe','akula-quickjs.exe'
];
const pin='32a803e67244836d2794f9bbbdf9b24deab63d7e';

function verify(a,am,c,cm,ps,wf){
  const head=wf.split(/\r?\npermissions:/,1)[0];
  assert.match(head,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const p of Object.values(files)){
    assert.ok(head.includes("      - '"+p+"'"),'VIDEO18B18B_HOLD: missing source trigger '+p);
  }
  for(const word of [
    '#[cfg(all(windows, feature = "lab-only-signature"))]',
    'pub fn admit_hosted_lab_signature(',
    'admit_with_key(raw, signature, test_key, installed_floor)',
    '("RUNNER_ENVIRONMENT", "github-hosted")',
    'const VIDEO_FIVE_CHANNEL_READY: bool = false;',
    'pub fn source_commit(&self) -> &str',
    'pub fn cargo_lock_sha256(&self) -> &str',
    'pub fn verify_stage(&self, current: &Path)',
  ]){
    assert.ok(a.includes(word),'VIDEO18B18B_HOLD: signed trust boundary missing '+word);
  }
  assert.ok(am.includes('lab-only-signature = []')&&am.includes('publish = false'),
    'VIDEO18B18B_HOLD: trust key injection is available outside nonshipping feature');
  assert.ok(cm.includes('publish = false')&&cm.includes('[workspace]')&&
    cm.includes('features = ["lab-only-signature"]')&&
    cm.includes('path = "../../../_rar2-pinned/crates/chaptera-update-engine"')&&
    cm.includes('path = "../../../_rar2-pinned/crates/chaptera-update-orchestrator"'),
    'VIDEO18B18B_HOLD: signed crate and pinned rar2 are not bridged');
  for(const word of [
    'admit_hosted_lab_signature(&raw,&signature,&public_key,"0.3.20")',
    'admit_hosted_lab_signature(&raw,&signature,&other,"0.3.20")',
    'signed.source_commit(),required("EXPECTED_HEAD_SHA")',
    'signed.cargo_lock_sha256(),required("AKULA_VIDEO18B18B_CARGO_LOCK_SHA256")',
    'signed.generation_sha256(),manifest_hash',
    'signed.verify_stage(&candidate).unwrap()',
    'signed.verify_stage(&paths.staged_candidate).unwrap()',
    'signed.verify_stage(&root.join("current")).unwrap()',
    'InstallLock::try_acquire(&root)',
    'OrchestrationError::LockBusy',
    'root.parent()==Some(pf.as_path())',
    'fs::symlink_metadata(path)',
    'meta.file_attributes() & 0x400',
    'UpdatePhase::CandidateActivated',
    'RecoveryOutcome::UnconfirmedCandidateRolledBack',
    'recovery.recover().unwrap()',
    'cleanup_orphaned_transactions().unwrap()',
    'VIDEO18B18B_SIGNED_REAL_PE_PROTECTED_ROLLBACK_PASS',
    'first_frame_ACK=0 release=0',
  ]){
    assert.ok(c.includes(word),'VIDEO18B18B_HOLD: controller custody or forced rollback missing '+word);
  }
  for(const n of names)assert.ok(c.includes('"'+n+'"'),
    'VIDEO18B18B_HOLD: exact five PE names missing '+n);
  assert.match(c,/let owner=InstallLock::try_acquire\(&root\)\.unwrap\(\);/,
    'VIDEO18B18B_HOLD: primary OS lock must have one concrete owner');
  assert.ok(!/\bconfirm_candidate\s*\(/.test(c.replace(/\/\/[^\n]*/g,'')),
    'VIDEO18B18B_HOLD: cannot commit without a real presented-frame ACK');
  assert.ok(!/std::process::Command|ShellExecuteW|channel_ready\s*\(\s*\)/.test(c),
    'VIDEO18B18B_HOLD: candidate must never run or enter product release');
  for(const word of [
    "$pin='"+pin+"'",
    'git -C $upstream rev-parse HEAD',
    "version='0.3.21'",
    'source_commit=$env:EXPECTED_HEAD_SHA',
    'cargo_lock_sha256=$lockDigest',
    'components=$records',
    'Copy-Item -LiteralPath (Join-Path $current',
    'VIDEO18B18B_NONRELEASE_OVERLAY_',
    '& $mini -G -W -s $key -p $pub',
    '& $mini -S -s $key -m $manifestFile',
    '& $mini -Vm $manifestFile -x $signature',
    'cargo generate-lockfile --manifest-path $manifestPath',
    'cargo test --locked --manifest-path $manifestPath --lib',
    'VIDEO18B18B_EPHEMERAL_LOCK_NOT_RELEASE',
    'VIDEO18B16_PROTECTED_VERSIONED_BRIDGE_PASS',
    'WindowsIdentity.RunImpersonated',
    'VIDEO18B18B_SIGNED_CONTROLLER_POST_RECOVERY_FIVE_PASS',
  ]){
    assert.ok(ps.includes(word),'VIDEO18B18B_HOLD: missing real signed Windows proof '+word);
  }
  for(const word of [
    'runs-on: windows-2022',
    'repository: Lalalalendia/rar2',
    'ref: '+pin,
    'repository: Lalalalendia/shiva',
    'choco install nsis --version=3.11.0',
    'minisign-0.12-win64.zip',
    '37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479',
    'VIDEO18B18B_PINNED_MINISIGN_012_PASS',
    'Expand-Archive -LiteralPath $archive',
    'persist-credentials: false',
    'permissions:\n  contents: read',
    'node --test .github/scripts/video18b18b-protected-signed-controller.test.cjs',
    'windows-video18b18b-protected-signed-controller.ps1',
  ])assert.ok(wf.includes(word),'VIDEO18B18B_HOLD: Windows CI contract missing '+word);
  assert.ok(!wf.includes('choco install minisign --version=0.8.0'),
    'VIDEO18B18B_HOLD: legacy 0.8 cannot create noninteractive ephemeral keys');
  assert.ok(!wf.includes('workflow_dispatch:')&&!/contents:\s*write|gh release create|publish_beta|cargo packager/i.test(wf),
    'VIDEO18B18B_HOLD: nonshipping scope breached');
  assert.ok(!/path:\s*(?:\$\{\{.*\}\}\/)?[^\n]*\.(exe|msi|key|minisig)/.test(wf),
    'VIDEO18B18B_HOLD: executable/signature/private keys may not be uploaded');
}
test('VIDEO18B18B authenticated five real PE + strict lock/rollback controller source',()=>{
  verify(read(files.gate),read(files.manifest),read(files.ctrl),read(files.ctrlmanifest),read(files.ps),read(files.wf));
});
test('VIDEO18B18B negative: reject missing trust, replaced key or disabled OS lock',()=>{
  const a=read(files.gate),am=read(files.manifest),c=read(files.ctrl),cm=read(files.ctrlmanifest),ps=read(files.ps),wf=read(files.wf);
  const cases=[
    [a.replace('const VIDEO_FIVE_CHANNEL_READY: bool = false;','const VIDEO_FIVE_CHANNEL_READY: bool = true;'),am,c,cm,ps,wf],
    [a.replace('admit_with_key(raw, signature, test_key, installed_floor)','inspect_authenticated(raw, installed_floor)'),am,c,cm,ps,wf],
    [a,am,c.replace('InstallLock::try_acquire(&root)','InstallLock::acquire(&root)'),cm,ps,wf],
    [a,am,c.replace('RecoveryOutcome::UnconfirmedCandidateRolledBack','RecoveryOutcome::NothingToDo'),cm,ps,wf],
    [a,am,c,cm,ps.replace("$pin='"+pin+"'", "$pin='main'"),wf],
  ];
  for(const row of cases) {
    assert.notDeepEqual(row,[a,am,c,cm,ps,wf]);
    assert.throws(()=>verify(...row),/VIDEO18B18B_HOLD/);
  }
});
test('VIDEO18B18B negative: refuse version-less signature, missing trigger or publishing',()=>{
  const a=read(files.gate),am=read(files.manifest),c=read(files.ctrl),cm=read(files.ctrlmanifest),ps=read(files.ps),wf=read(files.wf);
  const unversioned=ps.replace('source_commit=$env:EXPECTED_HEAD_SHA','source_commit="a"*40');
  assert.notEqual(unversioned,ps);
  assert.throws(()=>verify(a,am,c,cm,unversioned,wf),/VIDEO18B18B_HOLD/);
  const filtered=wf.replace("      - '"+files.ctrl+"'\n",'');
  assert.notEqual(filtered,wf);
  assert.throws(()=>verify(a,am,c,cm,ps,filtered),/VIDEO18B18B_HOLD/);
  assert.throws(()=>verify(a,am,c,cm,ps,wf+'\nworkflow_dispatch:\n'),/VIDEO18B18B_HOLD/);
  assert.throws(()=>verify(a,am,c,cm,ps,wf+'\npermissions:\n  contents: write\n'),/VIDEO18B18B_HOLD/);
});
