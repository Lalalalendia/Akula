'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const sourcePath='labs/video-lite/video18c02_windows_child_ipc.rs';
const testPath='.github/scripts/video18c02-windows-child-ipc.test.cjs';
const docPath='docs/release/video18c02-real-child-ipc.md';
const workflowPath='.github/workflows/akula-video18b14-protected-gui.yml';
const read=p=>fs.readFileSync(path.join(root,p),'utf8');

function contract(src,workflow) {
  const header=workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of [sourcePath,testPath,docPath]) {
    assert.ok(header.includes("      - '"+p+"'"),'VIDEO18C02_HOLD: missing PR path trigger '+p);
  }
  for(const s of [
    'hosted_pr_only()', 'CreatePipe(', 'SetHandleInformation(',
    'InitializeProcThreadAttributeList(', 'UpdateProcThreadAttribute(',
    'ATTRIBUTE_HANDLE_LIST', 'EXTENDED_STARTUPINFO_PRESENT',
    'CreateProcessW(', 'GetProcessId(', 'BCryptGenRandom(',
    'WaitForSingleObject(', 'DEADLINE_MS', 'DeleteProcThreadAttributeList(',
    'GetFileType(unrelated)', 'drop(writer);', 'wrong-nonce',
    'duplicate', 'silent', 'all == expected.as_bytes()',
    'VIDEO18C02_REAL_PARENT_CHILD_PIPE_PASS',
    'real_gui=0 signed_generation=0 confirm=0 release=0'
  ]) {
    assert.ok(src.includes(s),'VIDEO18C02_HOLD: missing authentic restricted child IPC '+s);
  }
  assert.ok(src.includes('unsafe { GetProcessId(process.0) } != info.pid'),
    'VIDEO18C02_HOLD: child claimed PID without OS HANDLE proof');
  assert.ok(src.includes('if all.len() > 256 { break; }'),
    'VIDEO18C02_HOLD: unlimited receipt length');
  assert.ok(!/\b(confirm_candidate|publish_beta|gh release create|release-channel|remove_dir_all)\b/.test(
    src.replace(/\/\/[^\n]*/g,'')
  ), 'VIDEO18C02_HOLD: IPC witness must not mutate product release/update state');
  assert.ok(workflow.includes('Windows real child process allowlisted pipe lab (no commit)'),
    'VIDEO18C02_HOLD: Windows runner never executes real child IPC');
  assert.ok(workflow.includes('node --test .github/scripts/video18c02-windows-child-ipc.test.cjs'),
    'VIDEO18C02_HOLD: source test not in required contract gate');
  assert.ok(workflow.includes('rustc --edition=2024') &&
    workflow.includes('VIDEO18C02_REAL_PARENT_CHILD_PIPE_PASS'),
    'VIDEO18C02_HOLD: no actual Windows rustc-run proof');
  assert.ok(!workflow.includes('workflow_dispatch:')&&!/contents:\s*write/.test(workflow),
    'VIDEO18C02_HOLD: no manual release or write permission');
}

test('Windows real OS parent/child handle allowlist with four receipt outcomes is lab-only',()=>{
  contract(read(sourcePath),read(workflowPath));
});
test('Negative: removing OS PID or handle allowlist must fail source contract',()=>{
  const src=read(sourcePath),wf=read(workflowPath);
  for(const [changedSource,changedWorkflow] of [
    [src.replace('GetProcessId(process.0)','GetCurrentProcessId()'),wf],
    [src.replace('ATTRIBUTE_HANDLE_LIST','ATTRIBUTE_BAD_LIST'),wf],
    [src.replace('if all.len() > 256 { break; }','if false { break; }'),wf],
    [src,wf.replace("      - '"+sourcePath+"'\n",'')]
  ]) {
    assert.throws(()=>contract(changedSource,changedWorkflow),/VIDEO18C02_HOLD/);
  }
});
