'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const rust='labs/video-lite/video18b13_windows_transaction_lab.rs';
const wfPath='.github/workflows/akula-video18b13-generation-crash-lab.yml';
const testPath='.github/scripts/video18b13-generation-crash-contract.test.cjs';
const docPath='docs/release/video18b13-five-exe-crash-recovery.md';
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const names=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
function validate(source,workflow){
  assert.match(source,/#!\[cfg\(target_os = "windows"\)\]/,'VIDEO18B13_HOLD: lab must be Windows-only');
  for(const name of names)assert.ok(source.includes('"'+name+'"'),'VIDEO18B13_HOLD: missing generation member '+name);
  for(const token of [
    'RUNNER_ENVIRONMENT','GITHUB_EVENT_NAME','RUNNER_OS',
    'AkulaVideo18b13-','lock.try_lock()','create_new(true)',
    'f.sync_all()','std::process::exit(73)',
    'after-receipt','after-backup','after-promote',
    'fs::rename(self.live(),self.backup())',
    'fs::rename(self.stage(),self.live())',
    'fs::rename(self.live(),self.failed())',
    'fs::rename(self.backup(),self.live())',
    'VIDEO18B13-FIRST-FRAME-TEST-ONLY',
    'VIDEO18B13-COMMITTED:',
    'if !journal {','if ack {','if committed {',
    'wrong_ack_and_corrupt_previous_never_delete_backup',
    'abrupt_child_exits_restore_old_exact_five_or_leave_old_intact',
  ])assert.ok(source.includes(token),'VIDEO18B13_HOLD: missing crash/rollback boundary '+token);
  assert.ok(!source.includes('std::process::Command::new("powershell"'),'VIDEO18B13_HOLD: no arbitrary external updater');
  const head=workflow.split(/\r?\npermissions:/,1)[0];
  assert.match(head,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const p of [rust,wfPath,testPath,docPath])assert.ok(head.includes("      - '"+p+"'"),'VIDEO18B13_HOLD: missing source trigger '+p);
  assert.match(workflow,/permissions:\s*\n  contents: read/);
  assert.match(workflow,/persist-credentials: false/);
  assert.match(workflow,/runs-on: windows-2022/);
  assert.match(workflow,/EXPECTED_HEAD_SHA/);
  assert.match(workflow,/rustc --edition=2021 --test -Dwarnings/);
  assert.match(workflow,/--test-threads=1/);
  assert.ok(!workflow.includes('workflow_dispatch:'),'VIDEO18B13_HOLD: no manual binary release');
  assert.ok(!/gh release create|cargo packager|publish_beta|upload-artifact|contents: write/i.test(workflow),
    'VIDEO18B13_HOLD: source test cannot publish installer or executable');
}
test('VIDEO18B13 requires real Windows process crash, receipt, five-member rollback and no-release CI',()=>validate(read(rust),read(wfPath)));
test('VIDEO18B13 rejects missing Windows OS lock or damaged generation receipt',()=>{
 const s=read(rust),w=read(wfPath);
 for(const token of ['lock.try_lock()','f.sync_all()','fs::rename(self.backup(),self.live())','VIDEO18B13-COMMITTED:']){
  const m=s.replace(token,'DISABLED_'+token);
  assert.notEqual(m,s);
  assert.throws(()=>validate(m,w),/VIDEO18B13_HOLD/);
 }
});
test('VIDEO18B13 rejects dropped member, missing source trigger or publishing',()=>{
 const s=read(rust),w=read(wfPath);
 const noSidecar=s.replace('"akula-quickjs.exe"','"attacker.exe"');
 assert.throws(()=>validate(noSidecar,w),/VIDEO18B13_HOLD/);
 const noTrigger=w.replace("      - '"+rust+"'\n",'');
 assert.notEqual(noTrigger,w);
 assert.throws(()=>validate(s,noTrigger),/VIDEO18B13_HOLD/);
 assert.throws(()=>validate(s,w+'\nworkflow_dispatch:\n'),/VIDEO18B13_HOLD/);
});
