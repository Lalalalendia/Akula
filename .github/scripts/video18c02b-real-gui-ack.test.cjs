'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const repo=path.resolve(__dirname,'../..');
const files={
  controller:'labs/video-lite/video18c02b_real_gui_ack.rs',
  script:'scripts/windows-video18b16-protected-versioned-bridge.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  test:'.github/scripts/video18c02b-real-gui-ack.test.cjs',
  doc:'docs/release/video18c02b-real-current-gui-ack.md',
  sender:'aquilum-app/native/src/updater/windows_first_gui_ack.rs',
  frame:'aquilum-app/native/src/host/frame.rs',
};
const read=p=>fs.readFileSync(path.join(repo,p),'utf8');

function verify(controller,script,workflow,sender,frame){
  const header=workflow.split(/\r?\npermissions:/,1)[0];
  for(const name of ['controller','script','workflow','test','doc']){
    const p=files[name];
    assert.ok(header.includes("      - '"+p+"'"),'VIDEO18C02B_HOLD: no trigger '+p);
  }
  for(const term of [
    'guarded_pr()', 'validate_app(', 'AkulaVideo18bRealNsisLab-',
    'if !lower_hex(tag, 32)', 'ProgramFiles', 'current',
    'REQUIRED: [&str; 5]', 'fs::canonicalize', 'fs::symlink_metadata',
    'CreatePipe(', 'SetHandleInformation(', 'HANDLE_FLAG_INHERIT',
    'PROC_THREAD_ATTRIBUTE_HANDLE_LIST', 'UpdateProcThreadAttribute(',
    'CreateProcessW(', 'CREATE_UNICODE_ENVIRONMENT',
    'GetProcessId(', 'GetProcessTimes(', 'BCryptGenRandom(',
    'AKULA_VIDEO18C_ACK_WRITE_HANDLE', 'AKULA_VIDEO18C_ACK_NONCE',
    'AKULA_VIDEO18C_GENERATION_SHA256', 'AKULA_VIDEO18C_GENERATION_VERSION',
    'AKULA_VIDEO18C_CONTROLLER_PID', 'PeekNamedPipe(', 'ReadFile(',
    'while Instant::now()<deadline', 'MAX_RECEIPT: usize = 256',
    'GetWindowThreadProcessId(', 'IsWindowVisible(', 'GetWindowRect(',
    'GetWindowTextW(', 'PostMessageW(', 'WM_CLOSE',
    'creation_time(child.0.0)?==first_creation',
    'VIDEO18C02B_REAL_GUI_ACK_PASS',
    'wrong-nonce', 'signed_generation=0 confirm=0 rollback=0 release=0'
  ]) assert.ok(controller.includes(term),'VIDEO18C02B_HOLD: controller missing '+term);
  assert.ok(controller.includes('if root.parent() != Some(program_files.as_path())'),
    'VIDEO18C02B_HOLD: not pinned Program Files path');
  assert.ok(controller.includes('if fs::read_dir(current).map_err(|e| e.to_string())?.count() != 5'),
    'VIDEO18C02B_HOLD: partial or extra PE generation accepted');
  assert.ok(controller.includes('let valid=receipt==expected.as_bytes()'),
    'VIDEO18C02B_HOLD: ACK not compared to trusted expected bytes');
  assert.ok(!/\b(confirm_candidate\s*\(|publish_beta|gh release create|remove_dir_all|WriteRegStr)\b/.test(
    controller.replace(/\/\/[^\n]*/g,'')), 'VIDEO18C02B_HOLD: release, journal or recursive deletion');
  assert.ok(script.includes('VIDEO18C02B_REAL_GUI_ACK_PASS') &&
    script.includes("labs/video-lite/video18c02b_real_gui_ack.rs") &&
    script.includes("Join-Path $current 'akula-app.exe'") &&
    script.includes("$expected['akula-app']") &&
    script.includes('Assert-Five $current') &&
    script.includes('Assert-Flat $root') &&
    script.includes('rustc --edition=2024'),
    'VIDEO18C02B_HOLD: no real protected current GUI IPC witness');
  assert.ok(workflow.includes('node --test .github/scripts/video18c02b-real-gui-ack.test.cjs'),
    'VIDEO18C02B_HOLD: negative source contract absent');
  assert.ok(workflow.includes('cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e'),
    'VIDEO18C02B_HOLD: wrong app features');
  assert.ok(!workflow.includes('workflow_dispatch:')&&!/contents:\s*write/.test(workflow),
    'VIDEO18C02B_HOLD: unauthorized release channel');
  assert.ok(sender.includes('AKULA-WIN-FIRST-PRESENTED-V1') &&
    sender.includes('static FIRST: AtomicBool') &&
    sender.includes('fn emit(challenge: &Challenge)') &&
    sender.includes('GetFileType(pipe)'),
    'VIDEO18C02B_HOLD: GUI ACK sender disconnected');
  assert.ok(frame.includes('if startup_frame_presented {') &&
    frame.includes('crate::updater::on_first_presented_frame();'),
    'VIDEO18C02B_HOLD: GUI ACK before actual first present');
}

test('Actual current/Akula GUI receives only private inherited ACK after first presented frame',()=>{
  verify(...['controller','script','workflow','sender','frame'].map(x=>read(files[x])));
});
test('Negative: missing private handle, pinned OS process and deadline are rejected',()=>{
  const [controller,script,wf,sender,frame]=['controller','script','workflow','sender','frame'].map(x=>read(files[x]));
  for(const changed of [
    controller.replace('PROC_THREAD_ATTRIBUTE_HANDLE_LIST','PROC_THREAD_ATTRIBUTE_ANY'),
    controller.replace('creation_time(child.0.0)?==first_creation','true'),
    controller.replace('MAX_RECEIPT: usize = 256','MAX_RECEIPT: usize = 65535'),
    controller.replace('while Instant::now()<deadline','while true'),
    controller.replace('if root.parent() != Some(program_files.as_path())','if false'),
    controller.replace('let valid=receipt==expected.as_bytes()','let valid=true'),
  ]){
    assert.notEqual(changed,controller);
    assert.throws(()=>verify(changed,script,wf,sender,frame),/VIDEO18C02B_HOLD/);
  }
});
test('Negative: modified workflow may not skip physical child or permit release',()=>{
  const [c,s,wf,emitter,frame]=['controller','script','workflow','sender','frame'].map(x=>read(files[x]));
  const variants=[
    wf.replace("      - '"+files.controller+"'\n",''),
    wf.replace('cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e',
      'cargo build --release -p aquilum-native --features installed'),
    wf+'\nworkflow_dispatch:\n',
  ];
  for(const changed of variants){
    assert.notEqual(changed,wf);
    assert.throws(()=>verify(c,s,changed,emitter,frame),/VIDEO18C02B_HOLD/);
  }
});
