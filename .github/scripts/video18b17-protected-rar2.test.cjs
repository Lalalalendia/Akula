'use strict';
const {test}=require('node:test'), assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path');
const base=path.resolve(__dirname,'../..');
const files={
  rs:'labs/video-lite/video18b17_protected_rar2_real_pe.rs',
  ps:'scripts/windows-video18b17-protected-rar2-real-pe.ps1',
  wf:'.github/workflows/akula-video18b17-protected-rar2.yml',
  test:'.github/scripts/video18b17-protected-rar2.test.cjs',
  doc:'docs/release/video18b17-protected-rar2-real-pe.md',
  nsis:'scripts/video18b16-protected-versioned-bridge.nsi',
};
const read=p=>fs.readFileSync(path.join(base,p),'utf8');
const pin='32a803e67244836d2794f9bbbdf9b24deab63d7e';
const five=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
function verify(rs,ps,wf){
 const header=wf.split(/\r?\npermissions:/,1)[0];
 assert.match(header,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
 for(const file of Object.values(files))assert.ok(header.includes("      - '"+file+"'"),
   'VIDEO18B17_HOLD: missing source trigger '+file);
 for(const word of [
   'repository: Lalalalendia/rar2',
   'ref: '+pin,
   'repository: Lalalalendia/shiva',
   'runs-on: windows-2022',
   'permissions:\n  contents: read',
   'cargo build --release --locked',
   'persist-credentials: false',
   'video18b17-protected-rar2-real-pe.ps1',
   'video18b17-protected-rar2.test.cjs',
   'choco install nsis --version=3.11.0',
 ]){
   assert.ok(wf.includes(word),'VIDEO18B17_HOLD: unpinned or disabled CI contract '+word);
 }
 for(const word of [
   "$pin='"+pin+"'",
   'git -C $upstream rev-parse HEAD',
   'VIDEO18B17_EPHEMERAL_LOCK_NOT_RELEASE',
   'cargo generate-lockfile',
   'cargo test --locked -p chaptera-update-orchestrator --test video18b17_akula_protected',
   'AKULA_VIDEO18B17_INSTALL_ROOT',
   'AKULA_VIDEO18B17_STAGE_ROOT',
   'Assert-Flat $root',
   'Assert-Five $current',
   'VIDEO18B17_RAR2_ENGINE_POST_RECOVERY_EXACT_FIVE_PASS',
   'WindowsIdentity.RunImpersonated',
   'VIDEO18B16_PROTECTED_VERSIONED_BRIDGE_PASS',
 ]){
   assert.ok(ps.includes(word),'VIDEO18B17_HOLD: missing real-PE protected test seam '+word);
 }
 for(const word of [
   'use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase}',
   'use chaptera_update_orchestrator::{InstallLock, OrchestrationError}',
   'InstallLock::try_acquire(&root)',
   'OrchestrationError::LockBusy',
   'root.parent() == Some(pf.as_path())',
   'LAB_PREFIX',
   'RUNNER_ENVIRONMENT',
   'fs::symlink_metadata',
   'Sha256::new()',
   'MZ',
   'stage_candidate',
   'assert_old(&root,&old)',
   'assert_new(root, next, old)',
   'UpdatePhase::Prepared',
   'RecoveryOutcome::PreparedTransactionAborted',
   'RecoveryOutcome::UnconfirmedCandidateRolledBack',
   'run_phase(&root,&candidate,&old,&next,"prepared")',
   'run_phase(&root,&candidate,&old,&next,"retained")',
   'run_phase(&root,&candidate,&old,&next,"activated")',
   'VIDEO18B17_PROTECTED_REAL_PE_RAR2_RECOVERY_PASS',
   'engine.cleanup_orphaned_transactions().unwrap()',
 ]){
   assert.ok(rs.includes(word),'VIDEO18B17_HOLD: missing lock or rollback witness '+word);
 }
 assert.match(rs,/let owner=InstallLock::try_acquire\(&root\)\.unwrap\(\);/,
   'VIDEO18B17_HOLD: primary owner must use nonblocking OS lock');
 assert.match(rs,/let loser=InstallLock::try_acquire\(&root\);/,
   'VIDEO18B17_HOLD: second lock owner must be actively denied');
 assert.match(rs,/let new_owner=InstallLock::try_acquire\(&root\)\.unwrap\(\);/,
   'VIDEO18B17_HOLD: lock recovery after release must be observed');
 assert.match(rs,/const COMPONENTS: \[&str; 5\] = \[\s*"akula-app.exe",\s*"shiva-pdf-preview.exe",\s*"akula-video-mux.exe",\s*"akula-yt-dlp.exe",\s*"akula-quickjs.exe",?\s*\];/,
   'VIDEO18B17_HOLD: exact five-member component declaration changed');
 for(const name of five)assert.ok(rs.includes('"'+name+'"'),
    'VIDEO18B17_HOLD: incomplete exact five PE allowlist '+name);
 assert.ok(!/confirm_candidate\s*\(/.test(rs),
   'VIDEO18B17_HOLD: non-shipping code must not confirm without GUI frame ACK');
 assert.ok(!wf.includes('workflow_dispatch:')&&!/contents:\s*write|gh release create|publish_beta/i.test(wf),
   'VIDEO18B17_HOLD: release and write capability forbidden');
 assert.ok(!/WriteRegStr|CreateShortCut|ExecShell|RegDLL/i.test(ps),
   'VIDEO18B17_HOLD: no real installer identity');
}
test('VIDEO18B17 pinned rar2 protected real PE OS lock and recovery contract',()=>{
 verify(read(files.rs),read(files.ps),read(files.wf));
});
test('VIDEO18B17 rejects source mutation removing lock, recovery or complete PE inventory',()=>{
 const rs=read(files.rs),ps=read(files.ps),wf=read(files.wf);
 const cases=[
   rs.replace('InstallLock::try_acquire(&root)','InstallLock::acquire(&root)'),
   rs.replace('run_phase(&root,&candidate,&old,&next,"activated")','// removed'),
   rs.replace('"akula-quickjs.exe"','"unknown.exe"'),
 ];
 assert.ok(!cases.some(x=>x===rs));
 for(const mutated of cases)assert.throws(()=>verify(mutated,ps,wf),/VIDEO18B17_HOLD/);
});
test('VIDEO18B17 rejects floating upstream, disabled PR path and publisher enablement',()=>{
 const rs=read(files.rs),ps=read(files.ps),wf=read(files.wf);
 const upstream=wf.replace('ref: '+pin,'ref: main');
 assert.notEqual(upstream,wf);
 assert.throws(()=>verify(rs,ps,upstream),/VIDEO18B17_HOLD/);
 const missing=wf.replace("      - '"+files.rs+"'\n",'');
 assert.notEqual(missing,wf);
 assert.throws(()=>verify(rs,ps,missing),/VIDEO18B17_HOLD/);
 assert.throws(()=>verify(rs,ps,wf+'\npermissions:\n  contents: write\n'),/VIDEO18B17_HOLD/);
 assert.throws(()=>verify(rs,ps,wf+'\nworkflow_dispatch:\n'),/VIDEO18B17_HOLD/);
});
