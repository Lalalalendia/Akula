'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const files={
  gui:'aquilum-app/native/src/updater/windows_first_gui_ack.rs',
  dispatch:'aquilum-app/native/src/updater/frame_ack.rs',
  root:'aquilum-app/native/src/updater/mod.rs',
  frame:'aquilum-app/native/src/host/frame.rs',
  contract:'.github/scripts/video18b18c-first-frame.test.cjs',
  doc:'docs/release/video18b18c-first-frame-windows.md',
  workflow:'.github/workflows/akula-video18b14-protected-gui.yml'
};
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const hook='crate::updater::on_first_presented_frame();';
const allowedWindows='all(target_os = "windows", feature = "video-rust-bundle", feature = "video-ui-e2e")';
function verify(gui,dispatch,updater,frame,wf){
  const header=wf.split(/\r?\npermissions:/,1)[0];
  for(const key of ['gui','dispatch','contract','doc']){
    const src=files[key];
    assert.ok(header.includes("      - '"+src+"'"),'VIDEO18C_HOLD: missing source test trigger '+src);
  }
  assert.ok(!header.includes("      - '"+files.root+"'")&&!header.includes("      - '"+files.frame+"'"),
    'VIDEO18C_HOLD: do not silently increase old module CI fanout');
  assert.match(updater,/mod frame_ack;/);
  assert.match(updater,/pub\(crate\) use frame_ack::on_first_presented_frame;/);
  assert.match(dispatch,/\#\[path = "windows_first_gui_ack.rs"\]\s*mod windows_first_gui_ack;/);
  assert.equal((dispatch.match(/feature = "video-rust-bundle"/g)||[]).length,3,
    'VIDEO18C_HOLD: every Windows module/dispatch/fallback must require experimental bundle');
  assert.equal((dispatch.match(/feature = "video-ui-e2e"/g)||[]).length,3,
    'VIDEO18C_HOLD: every Windows module/dispatch/fallback must require test-only GUI');
  assert.ok(frame.includes(allowedWindows),'VIDEO18C_HOLD: Windows not guarded as test video');
  assert.equal(frame.split(hook).length,2,'VIDEO18C_HOLD: GUI callback must exist once');
  const idx=frame.indexOf(hook);
  assert.ok(frame.lastIndexOf('output.present(&drawn);',idx)>0
    && frame.lastIndexOf('self.reveal();',idx)>frame.lastIndexOf('output.present(&drawn);',idx),
    'VIDEO18C_HOLD: ACK before present/reveal');
  assert.ok(frame.lastIndexOf('if startup_frame_presented {',idx)>frame.lastIndexOf('self.reveal();',idx),
    'VIDEO18C_HOLD: empty/no-op frame can emit ACK');
  for(const term of [
    'AKULA-WIN-FIRST-PRESENTED-V1',
    'AKULA_VIDEO18C_ACK_WRITE_HANDLE',
    'AKULA_VIDEO18C_ACK_NONCE',
    'AKULA_VIDEO18C_GENERATION_SHA256',
    'AKULA_VIDEO18C_GENERATION_VERSION',
    'AKULA_VIDEO18C_CONTROLLER_PID',
    'static FIRST: AtomicBool',
    'FIRST.swap(true, Ordering::AcqRel)',
    'GetFileType(pipe)',
    'GetHandleInformation(pipe,&mut flags)',
    'HANDLE_FLAG_INHERIT',
    'fn WriteFile(',
    'body.len() > 256',
    'fn emit(challenge: &Challenge)',
    'std::thread::spawn',
    'Challenge::from_environment()',
    'noninherited_pipe_and_regular_windows_handle_cannot_ack',
    'partial_wrong_identity_and_noncanonical_version_refuse',
  ]){
    assert.ok(gui.includes(term),'VIDEO18C_HOLD: missing pipe, one-shot or identity evidence '+term);
  }
  assert.ok(!/confirm_candidate\s*\(|publish_beta|channel_ready\s*\(|gh release create/.test(gui.replace(/\/\/[^\n]*/g,'')),
    'VIDEO18C_HOLD: GUI emitter must never approve an update or publish');
  assert.ok(wf.includes('Windows inherited pipe ACK transport tests')&&
    wf.includes('updater::frame_ack::windows_first_gui_ack::tests'),
    'VIDEO18C_HOLD: no real Windows unit transport test');
  assert.ok(!wf.includes('workflow_dispatch:')&&!/contents:\s*write/.test(wf),
    'VIDEO18C_HOLD: no release or manual trigger');
}
test('VIDEO18C actual presented-frame hook, one-shot inherited pipe and no-release source contract',()=>{
  verify(read(files.gui),read(files.dispatch),read(files.root),read(files.frame),read(files.workflow));
});
test('VIDEO18C negative: cannot move callback ahead of reveal or send on empty frame',()=>{
  const [gui,dispatch,updater,frame,wf]=[files.gui,files.dispatch,files.root,files.frame,files.workflow].map(read);
  const beforeReveal=frame.replace('self.reveal();','');
  assert.notEqual(beforeReveal,frame);
  assert.throws(()=>verify(gui,dispatch,updater,beforeReveal,wf),/VIDEO18C_HOLD/);
  const noGate=frame.replace('if startup_frame_presented {','if true {');
  assert.notEqual(noGate,frame);
  assert.throws(()=>verify(gui,dispatch,updater,noGate,wf),/VIDEO18C_HOLD/);
});
test('VIDEO18C negative: do not remove inherited-pipe validation or experimental target gate',()=>{
  const [gui,dispatch,updater,frame,wf]=[files.gui,files.dispatch,files.root,files.frame,files.workflow].map(read);
  const cases=[
    [gui.replace('GetFileType(pipe)','GetFakeType(pipe)'),dispatch,updater,frame,wf],
    [gui.replace('static FIRST: AtomicBool','static SECOND: AtomicBool'),dispatch,updater,frame,wf],
    [gui,dispatch.replace('feature = "video-ui-e2e"','feature = "installed"'),updater,frame,wf],
    [gui,dispatch,updater,frame,wf.replace("      - '"+files.gui+"'\n",'')]
  ];
  for(const args of cases){
    assert.notDeepEqual(args,[gui,dispatch,updater,frame,wf]);
    assert.throws(()=>verify(...args),/VIDEO18C_HOLD/);
  }
});
